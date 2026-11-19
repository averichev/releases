//! Protocol documents are built here independently of HTTP transport.
use crate::{
    domain::application::{Status, Visibility},
    services::{
        distribution::PublicDistribution,
        error::{Result, ServiceError},
    },
};
use serde::Serialize;
#[derive(Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub app: String,
    pub channel: String,
    pub latest: String,
    pub releases: Vec<ManifestRelease>,
}
#[derive(Serialize)]
pub struct ManifestRelease {
    pub version: String,
    pub published_at: String,
    pub notes: String,
    pub artifacts: Vec<ManifestArtifact>,
}
#[derive(Serialize)]
pub struct ManifestArtifact {
    pub id: String,
    pub target: String,
    pub format: String,
    pub purpose: String,
    pub size_bytes: i64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
}
pub fn rfc3339(timestamp: i64) -> Result<String> {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .ok_or_else(|| ServiceError::Internal(anyhow::anyhow!("Invalid stored timestamp")))
}
#[derive(Clone)]
pub struct ManifestGeneration {
    distribution: PublicDistribution,
}
impl ManifestGeneration {
    pub fn new(distribution: PublicDistribution) -> Self {
        Self { distribution }
    }
    pub async fn generate(
        &self,
        key: &str,
        channel: &str,
        public: bool,
        limit: u32,
    ) -> Result<Manifest> {
        if !(1..=100).contains(&limit) {
            return Err(ServiceError::Invalid);
        }
        let history = self
            .distribution
            .manifest_history(key, channel, public, limit)
            .await?;
        let app = history.application;
        let releases = history.releases;
        let latest = releases
            .first()
            .ok_or(ServiceError::NotFound)?
            .0
            .version
            .to_string();
        let available = app.status == Status::Active && app.visibility == Visibility::Public;
        let mut documents = Vec::new();
        for (release, artifacts) in releases {
            let mut files = Vec::new();
            for a in artifacts {
                self.distribution.check_artifact(&a).await?;
                files.push(ManifestArtifact {
                    download_url: available.then(|| {
                        self.distribution
                            .artifact_url(&app.slug, &release, &a, channel)
                    }),
                    id: a.id,
                    target: a.target,
                    format: a.file_type,
                    purpose: a.distribution_role,
                    size_bytes: a.size_bytes,
                    sha256: a.sha256,
                });
            }
            documents.push(ManifestRelease {
                version: release.version.to_string(),
                published_at: rfc3339(
                    release
                        .published_at
                        .ok_or(ServiceError::StorageUnavailable)?,
                )?,
                notes: release.release_notes,
                artifacts: files,
            });
        }
        if public {
            self.distribution.recheck(&app.id).await?;
        }
        Ok(Manifest {
            schema_version: 1,
            app: app.slug,
            channel: channel.into(),
            latest,
            releases: documents,
        })
    }
}
