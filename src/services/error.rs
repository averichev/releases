#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
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
    #[error("Artifact target and format already exist")]
    ArtifactConflict,
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
    #[error("Internal failure")]
    Internal(#[source] anyhow::Error),
}
impl From<sqlx::Error> for ServiceError {
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(ref db) = error {
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
