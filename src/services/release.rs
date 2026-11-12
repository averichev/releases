use crate::{
    domain::{
        application::Status,
        release::{
            self, Release, ReleaseChanges, ReleaseInput, ReleasePage, ReleaseQuery, ReleaseSort,
            ReleaseStatus,
        },
        user::Role,
    },
    infrastructure::{
        secrets,
        storage::{ArtifactStorage, LocalArtifactStorage},
    },
    repositories::{
        applications::ApplicationsRepository, artifact::ArtifactsRepository,
        release::ReleasesRepository, sessions::SessionsRepository,
    },
    services::{
        error::{Result, ServiceError},
        sessions::Identity,
    },
};
#[derive(Clone)]
pub struct ReleaseManagement {
    repository: ReleasesRepository,
    storage: LocalArtifactStorage,
}
impl ReleaseManagement {
    pub fn new(repository: ReleasesRepository, storage: LocalArtifactStorage) -> Self {
        Self {
            repository,
            storage,
        }
    }
    pub async fn get(&self, app: &str, id: &str) -> Result<Release> {
        self.repository.get(app, id).await
    }
    pub async fn list(&self, app: &str, q: ReleaseQuery) -> Result<ReleasePage> {
        if q.page == 0 || !(1..=100).contains(&q.per_page) || q.search.chars().count() > 200 {
            return Err(ServiceError::InvalidRelease);
        }
        // Metadata only. Sorting in Rust uses actual SemVer precedence, including
        // arbitrarily large numeric prerelease identifiers. No lexicographic SQL.
        let mut items = self.repository.list(app).await?;
        items.retain(|r| r.version.to_string().contains(q.search.trim()));
        items.sort_by(|a, b| {
            let order = match q.sort {
                ReleaseSort::Version => a.version.cmp_precedence(&b.version),
                ReleaseSort::CreatedAt => a.created_at.cmp(&b.created_at),
                ReleaseSort::UpdatedAt => a.updated_at.cmp(&b.updated_at),
            };
            (if q.descending { order.reverse() } else { order }).then(a.id.cmp(&b.id))
        });
        let total = items.len();
        Ok(ReleasePage {
            items: items
                .into_iter()
                .skip(
                    (u64::from(q.page - 1) * u64::from(q.per_page)).min(usize::MAX as u64) as usize,
                )
                .take(q.per_page as usize)
                .collect(),
            total,
        })
    }
    pub async fn create(
        &self,
        actor: &Identity,
        app: &str,
        input: ReleaseInput,
    ) -> Result<Release> {
        let version = release::version(&input.version).ok_or(ServiceError::InvalidRelease)?;
        let title = release::title(input.title.as_deref()).ok_or(ServiceError::InvalidRelease)?;
        let release_notes =
            release::notes(&input.release_notes).ok_or(ServiceError::InvalidRelease)?;
        let now = secrets::now();
        let release = Release {
            id: uuid::Uuid::new_v4().to_string(),
            application_id: app.to_owned(),
            version,
            title,
            release_notes,
            status: ReleaseStatus::Draft,
            created_at: now,
            updated_at: now,
            published_at: None,
            artifact_count: 0,
            total_size_bytes: 0,
        };
        let mut tx = self.repository.write().await?;
        authorize(&mut tx, actor).await?;
        if ApplicationsRepository::get_in(&mut tx, app).await?.status != Status::Active {
            return Err(ServiceError::ReleaseLocked);
        }
        ReleasesRepository::insert(&mut tx, &release).await?;
        tx.commit().await?;
        Ok(release)
    }
    pub async fn update(
        &self,
        actor: &Identity,
        app: &str,
        id: &str,
        input: ReleaseChanges,
    ) -> Result<Release> {
        if input.version.is_none() && input.title.is_none() && input.release_notes.is_none() {
            return Err(ServiceError::InvalidRelease);
        }
        let mut tx = self.repository.write().await?;
        authorize(&mut tx, actor).await?;
        let mut release = ReleasesRepository::get_in(&mut tx, app, id).await?;
        draft(&release)?;
        if let Some(value) = input.version {
            release.version = release::version(&value).ok_or(ServiceError::InvalidRelease)?;
        }
        if let Some(value) = input.title {
            release.title = release::title(value.as_deref()).ok_or(ServiceError::InvalidRelease)?;
        }
        if let Some(value) = input.release_notes {
            release.release_notes = release::notes(&value).ok_or(ServiceError::InvalidRelease)?;
        }
        release.updated_at = secrets::now().max(release.updated_at);
        ReleasesRepository::update(&mut tx, &release).await?;
        tx.commit().await?;
        Ok(release)
    }
    pub async fn delete(&self, actor: &Identity, app: &str, id: &str) -> Result<()> {
        let mut tx = self.repository.write().await?;
        authorize(&mut tx, actor).await?;
        let release = ReleasesRepository::get_in(&mut tx, app, id).await?;
        draft(&release)?;
        let artifacts = ArtifactsRepository::list_in(&mut tx, id).await?;
        ReleasesRepository::delete(&mut tx, id).await?;
        // Do not remove a file before a confirmed commit. Cancellation or an
        // ambiguous commit leaves only safe orphans, reclaimed at next startup.
        tx.commit().await?;
        for artifact in artifacts {
            remove_after_commit(&self.storage, &artifact.storage_key).await;
        }
        Ok(())
    }
}
pub(super) async fn authorize(c: &mut sqlx::SqliteConnection, actor: &Identity) -> Result<()> {
    let user = SessionsRepository::authorize(c, &actor.session_hash, secrets::now()).await?;
    if user.role != Role::Admin {
        return Err(ServiceError::Forbidden);
    }
    Ok(())
}
pub(super) fn draft(release: &Release) -> Result<()> {
    if release.status != ReleaseStatus::Draft {
        return Err(ServiceError::ReleaseLocked);
    }
    Ok(())
}
pub(super) async fn remove_after_commit(storage: &impl ArtifactStorage, key: &str) {
    if let Err(error) = storage.remove(key).await {
        tracing::warn!(error=?error,"Unreferenced artifact cleanup deferred until startup");
    }
}
