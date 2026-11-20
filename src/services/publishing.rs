use crate::{
    domain::{
        publication::PublicationValidation,
        release::{Release, ReleaseStatus},
    },
    infrastructure::secrets,
    repositories::{
        distribution::DistributionRepository,
        publication_history::{PublicationEvent, PublicationHistoryRepository},
        release::ReleasesRepository,
    },
    services::{
        error::{Result, ServiceError},
        publication_validation::PublicationValidator,
        release::authorize,
        sessions::Identity,
    },
};
#[derive(Clone)]
pub struct ReleasePublishing {
    releases: ReleasesRepository,
    snapshots: DistributionRepository,
    validator: PublicationValidator,
    history: PublicationHistoryRepository,
}
pub struct PublicationCommand {
    pub expected_release_revision: i64,
}
pub struct PublicationCheck {
    pub release_revision: i64,
    pub validation: PublicationValidation,
}
impl ReleasePublishing {
    pub fn new(
        releases: ReleasesRepository,
        snapshots: DistributionRepository,
        validator: PublicationValidator,
        history: PublicationHistoryRepository,
    ) -> Self {
        Self {
            releases,
            snapshots,
            validator,
            history,
        }
    }
    pub async fn history(
        &self,
        app: &str,
        limit: u32,
        before: Option<i64>,
    ) -> Result<Vec<PublicationEvent>> {
        if !(1..=100).contains(&limit) || before.is_some_and(|v| v < 1) {
            return Err(ServiceError::Invalid);
        }
        self.snapshots.application(app, false).await?;
        self.history.list(app, limit, before).await
    }
    pub async fn validate(
        &self,
        actor: &Identity,
        app: &str,
        id: &str,
    ) -> Result<PublicationCheck> {
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        tx.commit().await?;
        let snapshot = self.snapshots.snapshot(app, id).await?;
        let validation = self.validator.validate(&snapshot, true).await?;
        Ok(PublicationCheck {
            release_revision: snapshot.release.revision,
            validation,
        })
    }
    pub async fn publish(
        &self,
        actor: &Identity,
        app: &str,
        id: &str,
        command: PublicationCommand,
    ) -> Result<Release> {
        // Hash without a write lock, then reauthorize and compare the entire snapshot.
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        tx.commit().await?;
        let snapshot = self.snapshots.snapshot(app, id).await?;
        let validation = self.validator.validate(&snapshot, false).await?;
        if !validation.errors.is_empty() {
            return Err(ServiceError::PublicationBlocked(validation));
        }
        let mut tx = self.releases.write().await?;
        let user = authorize(&mut tx, actor).await?;
        let current = DistributionRepository::snapshot_in(&mut tx, app, id).await?;
        if current.application.status != snapshot.application.status
            || current.application.product_id != snapshot.application.product_id
            || current.application.standalone_public_key
                != snapshot.application.standalone_public_key
            || current.release != snapshot.release
            || current.artifacts != snapshot.artifacts
        {
            return Err(ServiceError::PublicationConflict);
        }
        // A completed request can be retried without another publication event.
        if current.release.status == ReleaseStatus::Published {
            tx.commit().await?;
            return Ok(current.release);
        }
        if current.release.revision != command.expected_release_revision {
            return Err(ServiceError::PublicationConflict);
        }
        let now = secrets::now().max(current.release.created_at);
        ReleasesRepository::publish(&mut tx, id, now).await?;
        PublicationHistoryRepository::append(&mut tx, app, id, &user.id, now).await?;
        let release = ReleasesRepository::get_in(&mut tx, app, id).await?;
        tx.commit().await?;
        Ok(release)
    }
}
