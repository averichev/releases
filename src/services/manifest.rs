//! Protocol documents are built here independently of HTTP transport.
use crate::{
    domain::{
        application::{Application, Status, Visibility},
        standalone,
    },
    services::{
        distribution::{PublicDistribution, latest_downloads},
        error::{Result, ServiceError},
    },
};
use serde::Serialize;
use std::collections::BTreeMap;
#[derive(Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub app: String,
    pub latest_by_target: BTreeMap<String, ManifestRelease>,
    pub latest_by_type: BTreeMap<String, BTreeMap<String, ManifestRelease>>,
    pub releases: Vec<ManifestRelease>,
}
#[derive(Clone, Serialize)]
pub struct ManifestRelease {
    pub version: String,
    pub published_at: String,
    pub notes: String,
    pub artifacts: Vec<ManifestArtifact>,
}
#[derive(Clone, Serialize)]
pub struct ManifestArtifact {
    pub id: String,
    pub target: String,
    pub format: String,
    pub purpose: String,
    pub size_bytes: i64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_metadata: Option<String>,
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
    async fn document(
        &self,
        app: &Application,
        release: &crate::domain::release::Release,
        artifacts: &[crate::domain::artifact::Artifact],
        available: bool,
    ) -> Result<ManifestRelease> {
        let mut files = Vec::new();
        for a in artifacts {
            self.distribution.check_artifact(a).await?;
            let signature = a
                .standalone_signature
                .as_deref()
                .filter(|_| available && a.distribution_role == "executable");
            if let Some(jws) = signature {
                standalone::verify(jws, app, release, a)
                    .map_err(|_| ServiceError::StandaloneUnavailable)?;
            }
            files.push(ManifestArtifact {
                download_url: available
                    .then(|| self.distribution.artifact_url(&app.slug, release, a)),
                signed_metadata: signature.map(str::to_owned),
                id: a.id.clone(),
                target: a.target.clone(),
                format: a.file_type.clone(),
                purpose: a.distribution_role.clone(),
                size_bytes: a.size_bytes,
                sha256: a.sha256.clone(),
            });
        }
        Ok(ManifestRelease {
            version: release.version.to_string(),
            published_at: rfc3339(
                release
                    .published_at
                    .ok_or(ServiceError::StorageUnavailable)?,
            )?,
            notes: release.release_notes.clone(),
            artifacts: files,
        })
    }
    pub async fn generate(&self, key: &str, public: bool, limit: u32) -> Result<Manifest> {
        if !(1..=100).contains(&limit) {
            return Err(ServiceError::Invalid);
        }
        let catalog = self.distribution.published(key, public).await?;
        let app = &catalog.application;
        let available = app.status == Status::Active && app.visibility == Visibility::Public;
        let mut latest_by_target = BTreeMap::new();
        let mut latest_by_type = BTreeMap::new();
        // Compute before pagination so an older platform is never hidden by limit.
        for (kind, targets) in latest_downloads(&catalog) {
            let mut documents = BTreeMap::new();
            for (target, (release, artifacts)) in targets {
                documents.insert(
                    target,
                    self.document(app, &release, &artifacts, available).await?,
                );
            }
            if kind == "installer" {
                latest_by_target = documents.clone();
            }
            latest_by_type.insert(kind.to_owned(), documents);
        }
        let mut releases = Vec::new();
        for (release, artifacts) in catalog.releases.iter().take(limit as usize) {
            releases.push(self.document(app, release, artifacts, available).await?);
        }
        if public {
            self.distribution.recheck(&app.id).await?;
        }
        Ok(Manifest {
            schema_version: 2,
            app: app.slug.clone(),
            latest_by_target,
            latest_by_type,
            releases,
        })
    }
}
