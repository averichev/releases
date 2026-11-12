use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

pub enum ApiError {
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
    PayloadTooLarge,
    UploadTimeout,
    StorageFull,
    StorageUnavailable,
    StorageBusy,
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
                "Invalid filename, target, format or multipart fields",
            ),
            Self::ArtifactConflict => (
                StatusCode::CONFLICT,
                "artifact_exists",
                "Artifact target and format already exist in this release",
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
                "Use a valid PNG, JPEG or WebP up to 2 MiB and 1024x1024 pixels",
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
