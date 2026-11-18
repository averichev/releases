use axum::{Extension, Json};
use serde::Serialize;

use super::{AppState, error::ApiError};
use crate::services;

#[derive(Serialize)]
pub struct HealthResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

pub async fn get(Extension(state): Extension<AppState>) -> Result<Json<HealthResponse>, ApiError> {
    services::health::check(&state.database)
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "Readiness check failed");
            ApiError::DatabaseUnavailable
        })?;
    Ok(Json(HealthResponse {
        status: "ok",
        service: "veruna-releases",
        version: crate::VERSION,
    }))
}
