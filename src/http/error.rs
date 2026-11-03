use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

pub enum ApiError {
    NotFound,
    MethodNotAllowed,
    AdministrationUnavailable,
    DatabaseUnavailable,
    Internal,
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
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "Resource not found"),
            Self::MethodNotAllowed => (
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "Method not allowed",
            ),
            Self::AdministrationUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "administration_unavailable",
                "Administrative API is disabled until authentication is implemented",
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
        };
        (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}
