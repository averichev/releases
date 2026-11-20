//! Shared resolution, selection and download authorization. No HTTP types or disk paths.
use crate::{
    config::DownloadConfig,
    domain::{
        application::{Application, Status, Visibility, valid_slug},
        artifact::Artifact,
        release::{Release, ReleaseStatus},
    },
    infrastructure::storage::{ArtifactStorage, LocalArtifactStorage},
    repositories::distribution::{DistributionRepository, PublicationSnapshot, PublishedCatalog},
    services::error::{Result, ServiceError},
};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

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
/// Resolve one role and explicit filters; ambiguous formats remain downloadable by ID.
fn validate_selection(query: &ArtifactSelection) -> Result<()> {
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
    Ok(())
}
pub fn select_artifact(artifacts: &[Artifact], query: &ArtifactSelection) -> Result<Artifact> {
    validate_selection(query)?;
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
        _ => Err(ServiceError::AmbiguousArtifact),
    }
}
pub struct DistributionHistory {
    pub application: Application,
    pub releases: Vec<(Release, Vec<Artifact>)>,
    pub total: i64,
}
/// Latest installers for each target, retaining all formats of that version.
pub fn latest_installers(
    catalog: &PublishedCatalog,
) -> std::collections::BTreeMap<String, (Release, Vec<Artifact>)> {
    let mut targets = std::collections::BTreeMap::new();
    for (release, artifacts) in &catalog.releases {
        if !release.version.pre.is_empty() {
            continue;
        }
        for a in artifacts
            .iter()
            .filter(|a| a.distribution_role == "installer")
        {
            let entry = targets
                .entry(a.target.clone())
                .or_insert_with(|| (release.clone(), Vec::new()));
            if entry.0.id == release.id {
                entry.1.push(a.clone());
            }
        }
    }
    targets
}
/// One context for every public download, ready for future request-level analytics.
/// It intentionally does not equate GET, HEAD, ranges or redirects to installations.
pub struct PublicDownload {
    pub application: Application,
    pub release: Release,
    pub artifact: Artifact,
}
#[derive(Clone)]
pub struct PublicDistribution {
    repository: DistributionRepository,
    storage: LocalArtifactStorage,
    pub config: DownloadConfig,
    workers: Arc<Semaphore>,
}
impl PublicDistribution {
    pub fn new(
        repository: DistributionRepository,
        storage: LocalArtifactStorage,
        config: DownloadConfig,
    ) -> Self {
        Self {
            workers: Arc::new(Semaphore::new(config.max_concurrent_downloads)),
            repository,
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
    pub async fn published(&self, key: &str, public: bool) -> Result<PublishedCatalog> {
        if public && !valid_slug(key) {
            return Err(ServiceError::NotFound);
        }
        let catalog = self.repository.published(key, public).await?;
        if public {
            public_application(&catalog.application)?;
        }
        Ok(catalog)
    }
    pub async fn history(
        &self,
        key: &str,
        public: bool,
        limit: u32,
        offset: u32,
    ) -> Result<DistributionHistory> {
        if !(1..=100).contains(&limit) || offset > 100_000 {
            return Err(ServiceError::Invalid);
        }
        let catalog = self.published(key, public).await?;
        Ok(DistributionHistory {
            total: catalog.releases.len() as i64,
            application: catalog.application,
            releases: catalog
                .releases
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .collect(),
        })
    }
    pub async fn latest(&self, slug: &str, query: &ArtifactSelection) -> Result<PublicDownload> {
        // latest is an installer shortcut; updater has its own signature-aware resolver.
        if query.target.is_none() || query.purpose.as_deref().is_some_and(|p| p != "installer") {
            return Err(ServiceError::InvalidArtifact);
        }
        validate_selection(query)?;
        let catalog = self.published(slug, true).await?;
        for (release, artifacts) in catalog.releases {
            if !release.version.pre.is_empty() {
                continue;
            }
            match select_artifact(&artifacts, query) {
                Ok(artifact) => {
                    return self
                        .authorize_download(
                            PublicationSnapshot {
                                application: catalog.application,
                                release,
                                artifacts,
                            },
                            artifact,
                        )
                        .await;
                }
                Err(ServiceError::NotFound) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(ServiceError::NotFound)
    }
    pub async fn version(
        &self,
        slug: &str,
        version: &str,
        query: &ArtifactSelection,
        id: Option<&str>,
    ) -> Result<PublicDownload> {
        if !valid_slug(slug) || crate::domain::release::version(version).is_none() {
            return Err(ServiceError::NotFound);
        }
        let snapshot = self.repository.version(slug, version).await?;
        public_application(&snapshot.application)?;
        let artifact = if let Some(id) = id {
            snapshot
                .artifacts
                .iter()
                .find(|a| a.id == id)
                .cloned()
                .ok_or(ServiceError::NotFound)?
        } else {
            select_artifact(&snapshot.artifacts, query)?
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
        })
    }
    pub async fn recheck(&self, app: &str) -> Result<()> {
        if self.repository.public_active(app).await? {
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
    pub fn artifact_path(&self, slug: &str, release: &Release, artifact: &Artifact) -> String {
        format!(
            "/{slug}/releases/{}/artifacts/{}/download",
            release.version, artifact.id
        )
    }
    pub fn artifact_url(&self, slug: &str, release: &Release, artifact: &Artifact) -> String {
        format!(
            "{}{}",
            self.config.public_base_url,
            self.artifact_path(slug, release, artifact)
        )
    }
}
