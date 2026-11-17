use crate::{
    domain::{
        artifact::{self, Artifact, UploadMetadata},
        user::Role,
    },
    infrastructure::{
        secrets,
        storage::{ArtifactStorage, LocalArtifactStorage, PendingUpload},
    },
    repositories::{artifact::ArtifactsRepository, release::ReleasesRepository},
    services::{
        error::{Result, ServiceError},
        release::{authorize, draft, remove_after_commit},
        sessions::Identity,
    },
};

#[derive(Clone)]
pub struct ArtifactManagement {
    repository: ArtifactsRepository,
    releases: ReleasesRepository,
    storage: LocalArtifactStorage,
}
pub struct ArtifactUpload {
    pending: PendingUpload,
    app: String,
    release: String,
    filename: String,
    target: String,
    file_type: String,
}
impl ArtifactUpload {
    pub async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.pending.write(bytes).await
    }
}
impl ArtifactManagement {
    pub fn new(
        repository: ArtifactsRepository,
        releases: ReleasesRepository,
        storage: LocalArtifactStorage,
    ) -> Self {
        Self {
            repository,
            releases,
            storage,
        }
    }
    pub async fn list(&self, app: &str, release: &str) -> Result<Vec<Artifact>> {
        self.releases.get(app, release).await?;
        self.repository.list(release).await
    }
    pub async fn begin(
        &self,
        actor: &Identity,
        app: &str,
        release: &str,
        metadata: UploadMetadata,
    ) -> Result<ArtifactUpload> {
        if actor.user.role != Role::Admin {
            return Err(ServiceError::Forbidden);
        }
        if !artifact::valid_filename(&metadata.filename)
            || !artifact::TARGETS.contains(&metadata.target.as_str())
        {
            return Err(ServiceError::InvalidArtifact);
        }
        let file_type = artifact::file_type(&metadata.filename, metadata.file_type.as_deref())
            .ok_or(ServiceError::InvalidArtifact)?;
        // This short transaction finishes before receiving file bytes.
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        draft(&ReleasesRepository::get_in(&mut tx, app, release).await?)?;
        if ArtifactsRepository::duplicate(&mut tx, release, &metadata.target, &file_type).await? {
            return Err(ServiceError::ArtifactConflict);
        }
        tx.commit().await?;
        let pending = self.storage.begin().await?;
        Ok(ArtifactUpload {
            pending,
            app: app.to_owned(),
            release: release.to_owned(),
            filename: metadata.filename,
            target: metadata.target,
            file_type,
        })
    }
    pub async fn complete(&self, actor: &Identity, mut upload: ArtifactUpload) -> Result<Artifact> {
        self.storage.promote(&mut upload.pending).await?;
        let now = secrets::now();
        let artifact = Artifact {
            id: uuid::Uuid::new_v4().to_string(),
            release_id: upload.release.clone(),
            filename: upload.filename,
            target: upload.target,
            file_type: upload.file_type,
            size_bytes: upload.pending.size_bytes as i64,
            sha256: upload.pending.sha256(),
            storage_key: upload.pending.key.clone(),
            created_at: now,
            distribution_role: "installer".into(),
            updater_signature: None,
        };
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        draft(&ReleasesRepository::get_in(&mut tx, &upload.app, &upload.release).await?)?;
        ArtifactsRepository::insert(&mut tx, &artifact).await?;
        ReleasesRepository::touch(&mut tx, &upload.release, now).await?;
        // From this point a failed/cancelled commit may already be durable.
        upload.pending.preserve_on_commit();
        tx.commit().await?;
        Ok(artifact)
    }
    pub async fn download(
        &self,
        app: &str,
        release: &str,
        id: &str,
    ) -> Result<(Artifact, tokio::fs::File)> {
        let artifact = self.repository.get(app, release, id).await?;
        let file = self
            .storage
            .open(&artifact.storage_key, artifact.size_bytes)
            .await?;
        Ok((artifact, file))
    }
    pub async fn delete(&self, actor: &Identity, app: &str, release: &str, id: &str) -> Result<()> {
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        draft(&ReleasesRepository::get_in(&mut tx, app, release).await?)?;
        let artifact = ArtifactsRepository::list_in(&mut tx, release)
            .await?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or(ServiceError::NotFound)?;
        ArtifactsRepository::delete(&mut tx, release, id).await?;
        ReleasesRepository::touch(&mut tx, release, secrets::now()).await?;
        tx.commit().await?;
        remove_after_commit(&self.storage, &artifact.storage_key).await;
        Ok(())
    }
    pub async fn set_updater(
        &self,
        actor: &Identity,
        app: &str,
        release: &str,
        id: &str,
        role: Option<&str>,
        signature: Option<Option<&[u8]>>,
    ) -> Result<Artifact> {
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        draft(&ReleasesRepository::get_in(&mut tx, app, release).await?)?;
        let mut artifact = ArtifactsRepository::list_in(&mut tx, release)
            .await?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or(ServiceError::NotFound)?;
        if let Some(role) = role {
            if !matches!(role, "installer" | "updater")
                || (role == "updater"
                    && crate::domain::publication::compatible_tauri_target(&artifact).is_none())
            {
                return Err(ServiceError::InvalidUpdater);
            }
            artifact.distribution_role = role.into();
            if role == "installer" {
                artifact.updater_signature = None;
            }
        }
        if let Some(value) = signature {
            if artifact.distribution_role != "updater" {
                return Err(ServiceError::InvalidUpdater);
            }
            artifact.updater_signature =
                value.map(crate::services::updater::signature).transpose()?;
        }
        ArtifactsRepository::updater(
            &mut tx,
            id,
            &artifact.distribution_role,
            artifact.updater_signature.as_deref(),
        )
        .await?;
        ReleasesRepository::touch(&mut tx, release, secrets::now()).await?;
        tx.commit().await?;
        Ok(artifact)
    }
    pub async fn verify(
        &self,
        actor: &Identity,
        app: &str,
        release: &str,
        id: &str,
    ) -> Result<bool> {
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        ReleasesRepository::get_in(&mut tx, app, release).await?;
        let artifact = ArtifactsRepository::list_in(&mut tx, release)
            .await?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or(ServiceError::NotFound)?;
        tx.commit().await?;
        self.storage
            .verify(&artifact.storage_key, artifact.size_bytes, &artifact.sha256)
            .await
    }
}
