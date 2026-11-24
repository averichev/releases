#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("At most two active publication tokens are allowed")]
    ApiTokenLimit,
    #[error("Invalid standalone product identity, metadata, signature, key or platform")]
    InvalidStandalone,
    #[error("Standalone verification failed")]
    StandaloneVerification(#[from] crate::domain::standalone::VerificationError),
    #[error("Standalone key is already used or pinned by signed artifacts")]
    StandaloneKeyConflict,
    #[error("Product identity is already used or pinned by signed artifacts")]
    StandaloneProductConflict,
    #[error("Standalone update is blocked")]
    StandaloneUnavailable,
    #[error("Invalid request")]
    Invalid,
    #[error("Invalid credentials")]
    Credentials,
    #[error("Authentication required")]
    Unauthorized,
    #[error("Insufficient permissions")]
    Forbidden,
    #[error("Too many attempts")]
    RateLimited,
    #[error("User already exists")]
    Conflict,
    #[error("User not found")]
    NotFound,
    #[error("Last active administrator is protected")]
    LastAdmin,
    #[error("Invalid or expired token")]
    Token,
    #[error("Invalid application data")]
    InvalidApplication,
    #[error("Application slug already exists")]
    ApplicationSlugConflict,
    #[error("Invalid icon")]
    InvalidIcon,
    #[error("Invalid release fields or query")]
    InvalidRelease,
    #[error("Release version already exists")]
    ReleaseVersionConflict,
    #[error("Release must be a draft or application is archived")]
    ReleaseLocked,
    #[error("Invalid artifact metadata or multipart upload")]
    InvalidArtifact,
    #[error("Artifact target, format and role already exist, or target already has an updater")]
    ArtifactConflict,
    #[error("Published updater requires a signature in the same upload")]
    UpdaterSignatureRequired,
    #[error("Upload exceeds size limit")]
    PayloadTooLarge,
    #[error("Upload timed out")]
    UploadTimeout,
    #[error("Storage is full")]
    StorageFull,
    #[error("Stored artifact is missing or has an unexpected size")]
    StorageUnavailable,
    #[error("Storage operations are busy; retry later")]
    StorageBusy,
    #[error("Release changed; reload and review")]
    PublicationConflict,
    #[error("Release failed publication checks")]
    PublicationBlocked(crate::domain::publication::PublicationValidation),
    #[error("Multiple distributions match; specify target, format or purpose")]
    AmbiguousArtifact,
    #[error("Published updater configuration is invalid")]
    UpdaterUnavailable,
    #[error("Invalid updater signature or compatibility metadata")]
    InvalidUpdater,
    #[error("Internal failure")]
    Internal(#[source] anyhow::Error),
}
impl From<sqlx::Error> for ServiceError {
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(ref db) = error {
            if db.message() == "api_token_limit" {
                return Self::ApiTokenLimit;
            }
            if db.message() == "release_locked" {
                return Self::ReleaseLocked;
            }
            if db.message() == "artifact_exists" {
                return Self::ArtifactConflict;
            }
            if db.message() == "updater_signature_required" {
                return Self::UpdaterSignatureRequired;
            }
            if db.message() == "last_active_admin" {
                return Self::LastAdmin;
            }
            if db.is_unique_violation() {
                return Self::Conflict;
            }
        }
        Self::Internal(error.into())
    }
}
pub type Result<T> = std::result::Result<T, ServiceError>;

impl From<std::io::Error> for ServiceError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::StorageFull {
            Self::StorageFull
        } else {
            Self::Internal(error.into())
        }
    }
}
impl From<anyhow::Error> for ServiceError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}
