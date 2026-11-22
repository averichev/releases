use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

pub enum ApiError {
    InvalidStandalone,
    StandaloneKeyConflict,
    StandaloneProductConflict,
    StandaloneUnavailable,
    NotFound,
    MethodNotAllowed,
    BadRequest,
    Unauthorized,
    Credentials,
    Forbidden,
    RateLimited,
    Conflict,
    LastAdmin,
    Token,
    DatabaseUnavailable,
    Internal,
    InvalidApplication,
    SlugConflict,
    InvalidIcon,
    InvalidRelease,
    ReleaseVersionConflict,
    ReleaseLocked,
    InvalidArtifact,
    ArtifactConflict,
    UpdaterSignatureRequired,
    PayloadTooLarge,
    UploadTimeout,
    StorageFull,
    StorageUnavailable,
    StorageBusy,
    PublicationConflict,
    PublicationBlocked(crate::domain::publication::PublicationValidation),
    InvalidUpdater,
    AmbiguousArtifact,
    UpdaterUnavailable,
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::InvalidStandalone => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_standalone",
                "Invalid standalone product identity, metadata, signature, key or platform",
            ),
            Self::StandaloneKeyConflict => (
                StatusCode::CONFLICT,
                "standalone_key_conflict",
                "Key is already used or pinned by signed artifacts",
            ),
            Self::StandaloneProductConflict => (
                StatusCode::CONFLICT,
                "standalone_product_conflict",
                "Product identity is already used or pinned by signed artifacts",
            ),
            Self::StandaloneUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "standalone_unavailable",
                "Standalone update is blocked; contact the application administrator",
            ),
            Self::AmbiguousArtifact => (
                StatusCode::CONFLICT,
                "ambiguous_artifact",
                "Multiple artifacts match; specify target, type (or purpose), and format",
            ),
            Self::UpdaterUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "updater_unavailable",
                "Updater package is unavailable; contact the application administrator",
            ),
            Self::PublicationBlocked(validation) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    [(axum::http::header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({
                        "error": {"code": "publication_blocked", "message": "Release failed publication checks"},
                        "validation": super::publication::validation_dto(&validation)
                    })),
                ).into_response();
            }
            Self::PublicationConflict => (
                StatusCode::CONFLICT,
                "publication_conflict",
                "Release changed; reload and review",
            ),
            Self::InvalidUpdater => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_updater",
                "Invalid updater role, platform, format or signature; upload the generated .sig, never a private key",
            ),
            Self::InvalidRelease => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_release",
                "Invalid SemVer, release fields or query",
            ),
            Self::ReleaseVersionConflict => (
                StatusCode::CONFLICT,
                "release_version_exists",
                "Release with the same version precedence already exists",
            ),
            Self::ReleaseLocked => (
                StatusCode::CONFLICT,
                "release_locked",
                "Release must be a draft; new releases require an active application",
            ),
            Self::InvalidArtifact => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_artifact",
                "Invalid filename, target, artifact type, format or multipart fields",
            ),
            Self::UpdaterSignatureRequired => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "updater_signature_required",
                "Published updater requires a valid .sig in the same upload",
            ),
            Self::ArtifactConflict => (
                StatusCode::CONFLICT,
                "artifact_exists",
                "Artifact target, format and role already exist, or this target already has an updater",
            ),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "Upload exceeds configured size limit",
            ),
            Self::UploadTimeout => (
                StatusCode::REQUEST_TIMEOUT,
                "upload_timeout",
                "Upload idle timeout exceeded",
            ),
            Self::StorageFull => (
                StatusCode::INSUFFICIENT_STORAGE,
                "storage_full",
                "Insufficient storage space",
            ),
            Self::StorageUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "storage_unavailable",
                "Artifact storage needs repair",
            ),
            Self::StorageBusy => (
                StatusCode::SERVICE_UNAVAILABLE,
                "storage_busy",
                "Storage is busy; retry later",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "Resource not found"),
            Self::MethodNotAllowed => (
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "Method not allowed",
            ),
            Self::BadRequest => (
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "Invalid request",
            ),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication required",
            ),
            Self::Credentials => (
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "Invalid username or password",
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "Insufficient permissions or invalid CSRF protection",
            ),
            Self::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many attempts; try again later",
            ),
            Self::Conflict => (
                StatusCode::CONFLICT,
                "user_exists",
                "Username already exists",
            ),
            Self::LastAdmin => (
                StatusCode::CONFLICT,
                "last_active_admin",
                "Cannot remove the last active administrator",
            ),
            Self::Token => (
                StatusCode::BAD_REQUEST,
                "invalid_token",
                "Invalid or expired token",
            ),
            Self::DatabaseUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
                "Service is not ready",
            ),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Internal server error",
            ),
            Self::InvalidApplication => (
                StatusCode::BAD_REQUEST,
                "invalid_application",
                "Invalid application fields or query",
            ),
            Self::SlugConflict => (
                StatusCode::CONFLICT,
                "slug_exists",
                "Application slug already exists",
            ),
            Self::InvalidIcon => (
                StatusCode::BAD_REQUEST,
                "invalid_icon",
                "Use a valid SVG, PNG, JPEG or WebP up to 2 MiB; raster images up to 1024x1024 pixels",
            ),
        };
        (
            status,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(ErrorEnvelope {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}
