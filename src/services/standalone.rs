use crate::{
    domain::{release, standalone},
    services::{
        distribution::PublicDistribution,
        error::{Result, ServiceError},
    },
};

pub struct StandaloneUpdate {
    pub version: String,
    pub url: String,
    pub signed_metadata: String,
}

#[derive(Clone)]
pub struct StandaloneUpdater {
    distribution: PublicDistribution,
}
impl StandaloneUpdater {
    pub fn new(distribution: PublicDistribution) -> Self {
        Self { distribution }
    }
    pub async fn update(
        &self,
        slug: &str,
        target: &str,
        version: &str,
    ) -> Result<Option<StandaloneUpdate>> {
        if !standalone::target(target) {
            return Err(ServiceError::InvalidStandalone);
        }
        let current = release::version(version).ok_or(ServiceError::InvalidRelease)?;
        // Reuse the same public/active/published snapshot and SemVer ordering as
        // manifest v2, latest_by_target and Tauri. Only eligibility differs.
        let catalog = self.distribution.published(slug, true).await?;
        for (release, artifacts) in &catalog.releases {
            if !release.version.pre.is_empty() {
                continue;
            }
            if !release.version.cmp_precedence(&current).is_gt() {
                break;
            }
            let mut candidates = artifacts
                .iter()
                .filter(|a| a.target == target && a.standalone_signature.is_some());
            let Some(artifact) = candidates.next() else {
                continue;
            };
            if candidates.next().is_some() {
                return Err(ServiceError::StandaloneUnavailable);
            }
            let signature = artifact
                .standalone_signature
                .as_deref()
                .ok_or(ServiceError::StandaloneUnavailable)?;
            if catalog
                .application
                .standalone_public_key
                .as_deref()
                .and_then(|key| {
                    standalone::verify(signature, key, &catalog.application.id, release, artifact)
                })
                .is_none()
            {
                // A broken designated update blocks; never silently roll back
                // to an older candidate or offer an unsigned installer.
                return Err(ServiceError::StandaloneUnavailable);
            }
            self.distribution.check_artifact(artifact).await?;
            self.distribution.recheck(&catalog.application.id).await?;
            return Ok(Some(StandaloneUpdate {
                version: release.version.to_string(),
                url: self.distribution.artifact_url(slug, release, artifact),
                signed_metadata: signature.to_owned(),
            }));
        }
        Ok(None)
    }
}
