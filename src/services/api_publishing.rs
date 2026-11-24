//! One durable publication per bundle. All network and filesystem work precedes
//! BEGIN IMMEDIATE; only authorization and metadata changes hold the write lock.
use crate::{
    domain::{
        artifact::{self, Artifact, UploadMetadata},
        release::{self, Release, ReleaseStatus},
    },
    infrastructure::{
        secrets,
        storage::{ArtifactStorage, LocalArtifactStorage, PendingUpload},
    },
    repositories::{
        api_publications::ApiPublicationsRepository, applications::ApplicationsRepository,
        artifact::ArtifactsRepository, distribution::PublicationSnapshot,
        publication_history::PublicationHistoryRepository, release::ReleasesRepository,
    },
    services::{
        api_tokens::{ApiTokens, Publisher},
        error::{Result, ServiceError},
        publication_validation::PublicationValidator,
    },
};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const MAX_ARTIFACTS: usize = 64;
pub const MAX_JSON_BYTES: usize = 256 * 1024;

pub struct UploadedFile {
    pub filename: String,
    pub pending: PendingUpload,
}
pub struct BundleArtifact {
    pub file: usize,
    pub target: String,
    pub file_type: String,
    pub distribution_role: String,
    pub size_bytes: Option<u64>,
    pub sha256: Option<String>,
    pub signature: Option<Vec<u8>>,
    pub standalone_signature: Option<Vec<u8>>,
}
pub struct PublicationBundle {
    pub version: String,
    pub notes: String,
    pub files: Vec<UploadedFile>,
    pub artifacts: Vec<BundleArtifact>,
}
pub struct BundlePublication {
    pub release: Release,
    pub created: bool,
}
#[derive(Clone)]
pub struct ApiPublishing {
    releases: ReleasesRepository,
    storage: LocalArtifactStorage,
    validator: PublicationValidator,
    uploads: Arc<Semaphore>,
}
#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct OriginalArtifact<'a> {
    filename: &'a str,
    target: &'a str,
    file_type: &'a str,
    distribution_role: &'a str,
    size_bytes: i64,
    sha256: &'a str,
    signature: Option<&'a str>,
    standalone_signature: Option<&'a str>,
}
fn manifest(release: &Release, artifacts: &[Artifact]) -> Result<String> {
    let mut artifacts: Vec<_> = artifacts
        .iter()
        .map(|a| OriginalArtifact {
            filename: &a.filename,
            target: &a.target,
            file_type: &a.file_type,
            distribution_role: &a.distribution_role,
            size_bytes: a.size_bytes,
            sha256: &a.sha256,
            signature: a.updater_signature.as_deref(),
            standalone_signature: a.standalone_signature.as_deref(),
        })
        .collect();
    artifacts.sort();
    serde_json::to_string(&serde_json::json!({"version":release.version.to_string(),
        "notes":release.release_notes,"artifacts":artifacts}))
    .map_err(|e| ServiceError::Internal(e.into()))
}
impl ApiPublishing {
    pub fn new(
        releases: ReleasesRepository,
        storage: LocalArtifactStorage,
        validator: PublicationValidator,
        max_uploads: usize,
    ) -> Self {
        Self {
            releases,
            storage,
            validator,
            uploads: Arc::new(Semaphore::new(max_uploads)),
        }
    }
    pub fn permit(&self) -> Result<OwnedSemaphorePermit> {
        self.uploads
            .clone()
            .try_acquire_owned()
            .map_err(|_| ServiceError::StorageBusy)
    }
    pub async fn begin_file(&self) -> Result<PendingUpload> {
        self.storage.begin().await
    }
    pub async fn publish(
        &self,
        publisher: Publisher,
        bundle: PublicationBundle,
    ) -> Result<BundlePublication> {
        let version = release::version(&bundle.version).ok_or(ServiceError::InvalidRelease)?;
        let notes = release::notes(&bundle.notes).ok_or(ServiceError::InvalidRelease)?;
        if bundle.artifacts.len() > MAX_ARTIFACTS {
            return Err(ServiceError::PayloadTooLarge);
        }
        if bundle.artifacts.is_empty() {
            return Err(ServiceError::InvalidArtifact);
        }
        let now = secrets::now();
        let release = Release {
            id: uuid::Uuid::new_v4().to_string(),
            application_id: publisher.application.id.clone(),
            version,
            title: None,
            release_notes: notes,
            status: ReleaseStatus::Draft,
            revision: 0,
            created_at: now,
            updated_at: now,
            published_at: None,
            artifact_count: 0,
            total_size_bytes: 0,
            download_count: 0,
        };
        let mut artifacts = Vec::with_capacity(bundle.artifacts.len());
        let mut objects = Vec::with_capacity(bundle.artifacts.len());
        let mut files: Vec<_> = bundle.files.into_iter().map(Some).collect();
        // Count references before moving originals; shared parts get independent
        // storage keys through ArtifactStorage, preserving the existing DB model.
        let mut uses = vec![0usize; files.len()];
        for a in &bundle.artifacts {
            *uses.get_mut(a.file).ok_or(ServiceError::InvalidArtifact)? += 1;
        }
        if uses.contains(&0) {
            return Err(ServiceError::InvalidArtifact);
        }
        for input in bundle.artifacts {
            let source = files
                .get_mut(input.file)
                .and_then(Option::as_mut)
                .ok_or(ServiceError::InvalidArtifact)?;
            source.pending.seal().await?;
            let metadata = UploadMetadata {
                filename: source.filename.clone(),
                target: input.target,
                file_type: Some(input.file_type),
                distribution_role: Some(input.distribution_role),
            };
            if !artifact::valid_filename(&metadata.filename)
                || !artifact::TARGETS.contains(&metadata.target.as_str())
            {
                return Err(ServiceError::InvalidArtifact);
            }
            let file_type = metadata.file_type().ok_or(ServiceError::InvalidArtifact)?;
            let role = metadata
                .distribution_role
                .ok_or(ServiceError::InvalidArtifact)?;
            if !artifact::ARTIFACT_TYPES.contains(&role.as_str())
                || input
                    .size_bytes
                    .is_some_and(|size| size != source.pending.size_bytes())
                || input
                    .sha256
                    .as_deref()
                    .is_some_and(|hash| !secrets::valid(hash) || hash != source.pending.sha256())
            {
                return Err(ServiceError::InvalidArtifact);
            }
            let signature = input
                .signature
                .as_deref()
                .map(crate::services::updater::signature)
                .transpose()?;
            if role == "updater" && signature.is_none() {
                return Err(ServiceError::UpdaterSignatureRequired);
            }
            if role != "updater" && signature.is_some() {
                return Err(ServiceError::InvalidUpdater);
            }
            let standalone_signature = input
                .standalone_signature
                .as_deref()
                .map(|bytes| {
                    if bytes.is_empty()
                        || bytes.len() > crate::domain::standalone::MAX_SIGNATURE_BYTES
                    {
                        return Err(crate::domain::standalone::VerificationError::InvalidJws);
                    }
                    std::str::from_utf8(bytes)
                        .map(str::trim)
                        .map(str::to_owned)
                        .map_err(|_| crate::domain::standalone::VerificationError::InvalidJws)
                })
                .transpose()?;
            uses[input.file] -= 1;
            let object = if uses[input.file] == 0 {
                files[input.file]
                    .take()
                    .ok_or(ServiceError::InvalidArtifact)?
                    .pending
            } else {
                self.storage.duplicate(&source.pending).await?
            };
            let a = Artifact {
                id: uuid::Uuid::new_v4().to_string(),
                release_id: release.id.clone(),
                filename: metadata.filename,
                target: metadata.target,
                file_type,
                distribution_role: role,
                size_bytes: object.size_bytes() as i64,
                sha256: object.sha256(),
                storage_key: object.storage_key().into(),
                created_at: now,
                updater_signature: signature,
                standalone_signature,
                download_count: 0,
            };
            if let Some(signature) = &a.standalone_signature {
                crate::domain::standalone::verify(signature, &publisher.application, &release, &a)?;
            }
            artifacts.push(a);
            objects.push(object);
        }
        let original = manifest(&release, &artifacts)?;
        for object in &mut objects {
            self.storage.promote(object).await?;
        }
        let snapshot = PublicationSnapshot {
            application: publisher.application.clone(),
            release,
            artifacts,
        };
        // Streaming ingestion computed SHA-256; inspect reuses the publication
        // rules and checks regular files/size without rereading the whole batch.
        let validation = self.validator.inspect(&snapshot).await?;
        if !validation.errors.is_empty() {
            return Err(ServiceError::PublicationBlocked(validation));
        }
        let mut tx = self.releases.write().await?;
        let current = ApiTokens::reauthorize(&mut tx, &publisher).await?;
        if current.product_id != publisher.application.product_id
            || current.standalone_public_key != publisher.application.standalone_public_key
            || ApplicationsRepository::tauri_key_in(&mut tx, &current.id).await?
                != publisher.tauri_public_key
        {
            return Err(ServiceError::PublicationConflict);
        }
        let release = &snapshot.release;
        if let Some((id, accepted)) = ApiPublicationsRepository::existing(
            &mut tx,
            &current.id,
            &release::version_key(&release.version),
        )
        .await?
        {
            if accepted.as_deref() != Some(&original) {
                return Err(ServiceError::ReleaseVersionConflict);
            }
            let release = ReleasesRepository::get_in(&mut tx, &current.id, &id).await?;
            tx.commit().await?;
            return Ok(BundlePublication {
                release,
                created: false,
            });
        }
        ReleasesRepository::insert(&mut tx, release).await?;
        for artifact in &snapshot.artifacts {
            ArtifactsRepository::insert(&mut tx, artifact).await?;
        }
        let published_at = secrets::now().max(now);
        ReleasesRepository::publish(&mut tx, &release.id, published_at).await?;
        PublicationHistoryRepository::append_token(
            &mut tx,
            &current.id,
            &release.id,
            &publisher.token_id,
            published_at,
        )
        .await?;
        ApiPublicationsRepository::insert(&mut tx, &release.id, &original).await?;
        let release = ReleasesRepository::get_in(&mut tx, &current.id, &release.id).await?;
        // Ambiguous/cancelled commit must never remove files that may be durable.
        // Startup reconciliation reclaims unreferenced objects after a crash.
        for object in &mut objects {
            object.preserve_on_commit();
        }
        tx.commit().await?;
        Ok(BundlePublication {
            release,
            created: true,
        })
    }
}
