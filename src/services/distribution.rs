//! Shared resolution, selection and download authorization. No HTTP types or disk paths.
use crate::{
    config::DownloadConfig,
    domain::{
        application::{Application, DistributionSelector, Status, Visibility, valid_slug},
        artifact::Artifact,
        publication::ReleaseChannel,
        release::{Release, ReleaseStatus},
    },
    infrastructure::storage::{ArtifactStorage, LocalArtifactStorage},
    repositories::{
        channel::{ChannelsRepository, PublicationSnapshot},
        distribution::DistributionRepository,
    },
    services::error::{Result, ServiceError},
};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub fn channel(value: &str) -> Result<()> {
    if matches!(value, "stable" | "beta") {
        Ok(())
    } else {
        Err(ServiceError::Invalid)
    }
}
pub fn public_application(app: &Application) -> Result<()> {
    if app.status == Status::Active && app.visibility == Visibility::Public {
        Ok(())
    } else {
        Err(ServiceError::NotFound)
    }
}
#[derive(Default)]
pub struct ArtifactSelection {
    pub target: Option<String>,
    pub format: Option<String>,
    pub purpose: Option<String>,
}
/// Apply explicit filters first; a default must never override a user's target.
pub fn select_artifact(
    artifacts: &[Artifact],
    query: &ArtifactSelection,
    default: Option<&DistributionSelector>,
) -> Result<Artifact> {
    if query
        .target
        .as_ref()
        .is_some_and(|t| !crate::domain::artifact::TARGETS.contains(&t.as_str()))
        || query
            .format
            .as_ref()
            .is_some_and(|f| !crate::domain::artifact::valid_file_type(f))
        || query
            .purpose
            .as_ref()
            .is_some_and(|p| !matches!(p.as_str(), "installer" | "updater"))
    {
        return Err(ServiceError::InvalidArtifact);
    }
    let candidates: Vec<_> = artifacts
        .iter()
        .filter(|a| {
            a.distribution_role == query.purpose.as_deref().unwrap_or("installer")
                && query.target.as_ref().is_none_or(|t| *t == a.target)
                && query.format.as_ref().is_none_or(|f| *f == a.file_type)
        })
        .collect();
    match candidates.as_slice() {
        [] => Err(ServiceError::NotFound),
        [only] => Ok((*only).clone()),
        _ => {
            if let Some(s) = default {
                let selected: Vec<_> = candidates.into_iter().filter(|a| s.matches(a)).collect();
                if let [only] = selected.as_slice() {
                    return Ok((*only).clone());
                }
            }
            Err(ServiceError::AmbiguousArtifact)
        }
    }
}
pub struct DistributionHistory {
    pub application: Application,
    pub channel: ReleaseChannel,
    pub releases: Vec<(Release, Vec<Artifact>)>,
    pub total: i64,
}
/// One context for every public download, ready for future request-level analytics.
/// It intentionally does not equate GET, HEAD, ranges or redirects to installations.
pub struct PublicDownload {
    pub application: Application,
    pub release: Release,
    pub artifact: Artifact,
    pub channel: String,
}
#[derive(Clone)]
pub struct PublicDistribution {
    repository: DistributionRepository,
    channels: ChannelsRepository,
    storage: LocalArtifactStorage,
    pub config: DownloadConfig,
    workers: Arc<Semaphore>,
}
impl PublicDistribution {
    pub fn new(
        repository: DistributionRepository,
        channels: ChannelsRepository,
        storage: LocalArtifactStorage,
        config: DownloadConfig,
    ) -> Self {
        Self {
            workers: Arc::new(Semaphore::new(config.max_concurrent_downloads)),
            repository,
            channels,
            storage,
            config,
        }
    }
    pub async fn application(&self, key: &str, public: bool) -> Result<Application> {
        if public && !valid_slug(key) {
            return Err(ServiceError::NotFound);
        }
        let app = self.repository.application(key, public).await?;
        if public {
            public_application(&app)?;
        }
        Ok(app)
    }
    pub async fn current(
        &self,
        key: &str,
        name: &str,
        public: bool,
    ) -> Result<(Application, ReleaseChannel, Option<PublicationSnapshot>)> {
        channel(name)?;
        if public && !valid_slug(key) {
            return Err(ServiceError::NotFound);
        }
        let (app, channel, snapshot) = self.channels.current_snapshot(key, name, public).await?;
        if public {
            public_application(&app)?;
        }
        if let Some(s) = &snapshot
            && s.release.status != ReleaseStatus::Published
        {
            return Err(ServiceError::NotFound);
        }
        Ok((app, channel, snapshot))
    }
    pub async fn history(
        &self,
        key: &str,
        name: &str,
        public: bool,
        limit: u32,
        offset: u32,
    ) -> Result<DistributionHistory> {
        channel(name)?;
        if !(1..=100).contains(&limit) || offset > 100_000 {
            return Err(ServiceError::Invalid);
        }
        let (application, channel, releases, total) = self
            .repository
            .history(key, name, public, limit, offset)
            .await?;
        if public {
            public_application(&application)?;
        }
        Ok(DistributionHistory {
            application,
            channel,
            releases,
            total,
        })
    }
    pub async fn default_selector(&self, app: &str) -> Result<Option<DistributionSelector>> {
        self.repository.selector(app).await
    }
    pub async fn latest(
        &self,
        slug: &str,
        name: &str,
        query: &ArtifactSelection,
    ) -> Result<PublicDownload> {
        let (_, _, snapshot) = self.current(slug, name, true).await?;
        let snapshot = snapshot.ok_or(ServiceError::NotFound)?;
        let default = self.default_selector(&snapshot.application.id).await?;
        let artifact = select_artifact(&snapshot.artifacts, query, default.as_ref())?;
        self.authorize_download(snapshot, artifact).await
    }
    pub async fn version(
        &self,
        slug: &str,
        version: &str,
        name: &str,
        query: &ArtifactSelection,
        id: Option<&str>,
    ) -> Result<PublicDownload> {
        channel(name)?;
        if !valid_slug(slug) || crate::domain::release::version(version).is_none() {
            return Err(ServiceError::NotFound);
        }
        let snapshot = self.repository.version(slug, version, name).await?;
        public_application(&snapshot.application)?;
        let artifact = if let Some(id) = id {
            snapshot
                .artifacts
                .iter()
                .find(|a| a.id == id)
                .cloned()
                .ok_or(ServiceError::NotFound)?
        } else {
            let default = self.default_selector(&snapshot.application.id).await?;
            select_artifact(&snapshot.artifacts, query, default.as_ref())?
        };
        self.authorize_download(snapshot, artifact).await
    }
    async fn authorize_download(
        &self,
        snapshot: PublicationSnapshot,
        artifact: Artifact,
    ) -> Result<PublicDownload> {
        public_application(&snapshot.application)?;
        if snapshot.release.status != ReleaseStatus::Published
            || artifact.release_id != snapshot.release.id
        {
            return Err(ServiceError::NotFound);
        }
        self.check_artifact(&artifact).await?;
        self.recheck(&snapshot.application.id).await?;
        Ok(PublicDownload {
            application: snapshot.application,
            release: snapshot.release,
            artifact,
            channel: snapshot.channel.name,
        })
    }
    pub async fn recheck(&self, app: &str) -> Result<()> {
        if self.channels.public_active(app).await? {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }
    /// Checks immutable metadata, never rehashes a multi-gigabyte download.
    pub async fn check_artifact(&self, artifact: &Artifact) -> Result<()> {
        drop(
            self.storage
                .open(&artifact.storage_key, artifact.size_bytes)
                .await?,
        );
        Ok(())
    }
    pub async fn open(&self, download: &PublicDownload) -> Result<tokio::fs::File> {
        let file = self
            .storage
            .open(&download.artifact.storage_key, download.artifact.size_bytes)
            .await?;
        self.recheck(&download.application.id).await?;
        Ok(file)
    }
    pub fn permit(&self) -> Result<OwnedSemaphorePermit> {
        self.workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| ServiceError::StorageBusy)
    }
    pub fn internal_uri(&self, artifact: &Artifact) -> Result<String> {
        self.storage.internal_uri(&artifact.storage_key)
    }
    pub fn artifact_path(
        &self,
        slug: &str,
        release: &Release,
        artifact: &Artifact,
        channel: &str,
    ) -> String {
        let suffix = if channel == "beta" {
            "?channel=beta"
        } else {
            ""
        };
        format!(
            "/{slug}/releases/{}/artifacts/{}/download{suffix}",
            release.version, artifact.id
        )
    }
    pub fn artifact_url(
        &self,
        slug: &str,
        release: &Release,
        artifact: &Artifact,
        channel: &str,
    ) -> String {
        format!(
            "{}{}",
            self.config.public_base_url,
            self.artifact_path(slug, release, artifact, channel)
        )
    }
}
