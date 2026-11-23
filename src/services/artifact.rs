use crate::{
    domain::{
        application::Status,
        artifact::{self, Artifact, UploadMetadata},
        publication::{compatible_tauri_format, compatible_tauri_target},
        release::ReleaseStatus,
        user::Role,
    },
    infrastructure::{
        secrets,
        storage::{ArtifactStorage, LocalArtifactStorage, PendingUpload},
    },
    repositories::{
        applications::ApplicationsRepository, artifact::ArtifactsRepository,
        release::ReleasesRepository,
    },
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
    distribution_role: String,
    updater_signature: Option<String>,
    standalone_signature: Option<String>,
}
impl ArtifactUpload {
    pub fn set_standalone_signature(&mut self, bytes: &[u8]) -> Result<()> {
        if !matches!(self.distribution_role.as_str(), "executable" | "installer")
            || bytes.is_empty()
            || bytes.len() > crate::domain::standalone::MAX_SIGNATURE_BYTES
        {
            return Err(ServiceError::InvalidStandalone);
        }
        self.standalone_signature = Some(
            std::str::from_utf8(bytes)
                .map_err(|_| ServiceError::InvalidStandalone)?
                .trim()
                .to_owned(),
        );
        Ok(())
    }
    pub fn set_signature(&mut self, bytes: &[u8]) -> Result<()> {
        if self.distribution_role != "updater" {
            return Err(ServiceError::InvalidUpdater);
        }
        self.updater_signature = Some(crate::services::updater::signature(bytes)?);
        Ok(())
    }
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
        let file_type = metadata.file_type().ok_or(ServiceError::InvalidArtifact)?;
        let role = metadata.distribution_role.as_deref().unwrap_or("installer");
        if !artifact::ARTIFACT_TYPES.contains(&role) {
            return Err(ServiceError::InvalidArtifact);
        }
        if role == "updater" && compatible_tauri_format(&metadata.target, &file_type).is_none() {
            return Err(ServiceError::InvalidUpdater);
        }
        // This short transaction finishes before receiving file bytes.
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        ReleasesRepository::get_in(&mut tx, app, release).await?;
        ArtifactsRepository::check_conflict(
            &mut tx,
            release,
            &metadata.target,
            &file_type,
            role,
            None,
        )
        .await?;
        tx.commit().await?;
        let pending = self.storage.begin().await?;
        Ok(ArtifactUpload {
            pending,
            app: app.to_owned(),
            release: release.to_owned(),
            filename: metadata.filename,
            target: metadata.target,
            file_type,
            distribution_role: role.to_owned(),
            updater_signature: None,
            standalone_signature: None,
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
            distribution_role: upload.distribution_role,
            updater_signature: upload.updater_signature,
            standalone_signature: upload.standalone_signature,
            download_count: 0,
        };
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        let release = ReleasesRepository::get_in(&mut tx, &upload.app, &upload.release).await?;
        if let Some(signature) = &artifact.standalone_signature {
            let app = ApplicationsRepository::get_in(&mut tx, &upload.app).await?;
            if crate::domain::standalone::verify(signature, &app, &release, &artifact).is_none() {
                return Err(ServiceError::InvalidStandalone);
            }
        }
        ArtifactsRepository::check_conflict(
            &mut tx,
            &upload.release,
            &artifact.target,
            &artifact.file_type,
            &artifact.distribution_role,
            None,
        )
        .await?;
        if release.status == ReleaseStatus::Published {
            if ApplicationsRepository::get_in(&mut tx, &upload.app)
                .await?
                .status
                != Status::Active
            {
                return Err(ServiceError::ReleaseLocked);
            }
            if artifact.distribution_role == "updater" {
                if compatible_tauri_target(&artifact).is_none() {
                    return Err(ServiceError::InvalidUpdater);
                }
                if artifact.updater_signature.is_none() {
                    return Err(ServiceError::UpdaterSignatureRequired);
                }
            }
        }
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
            if !artifact::ARTIFACT_TYPES.contains(&role) {
                return Err(ServiceError::InvalidArtifact);
            }
            if role == "updater"
                && crate::domain::publication::compatible_tauri_target(&artifact).is_none()
            {
                return Err(ServiceError::InvalidUpdater);
            }
            artifact.distribution_role = role.into();
            if artifact.standalone_signature.is_some()
                && !crate::domain::standalone::compatible(&artifact)
            {
                return Err(ServiceError::InvalidStandalone);
            }
            if role != "updater" {
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
        ArtifactsRepository::check_conflict(
            &mut tx,
            release,
            &artifact.target,
            &artifact.file_type,
            &artifact.distribution_role,
            Some(id),
        )
        .await?;
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
    /// Attach/clear standalone intent on drafts. Published artifacts remain immutable;
    /// signed additions to a published release must use the atomic upload path.
    pub async fn set_standalone(
        &self,
        actor: &Identity,
        app: &str,
        release: &str,
        id: &str,
        signature: Option<&str>,
    ) -> Result<Artifact> {
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        let r = ReleasesRepository::get_in(&mut tx, app, release).await?;
        draft(&r)?;
        let mut artifact = ArtifactsRepository::list_in(&mut tx, release)
            .await?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or(ServiceError::NotFound)?;
        if let Some(signature) = signature {
            let app = ApplicationsRepository::get_in(&mut tx, app).await?;
            if crate::domain::standalone::verify(signature, &app, &r, &artifact).is_none() {
                return Err(ServiceError::InvalidStandalone);
            }
        }
        ArtifactsRepository::standalone(&mut tx, id, signature).await?;
        ReleasesRepository::touch(&mut tx, release, secrets::now()).await?;
        tx.commit().await?;
        artifact.standalone_signature = signature.map(str::to_owned);
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
