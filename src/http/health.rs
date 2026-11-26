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
    check(&state).await?;
    Ok(Json(HealthResponse {
        status: "ok",
        service: "veruna-releases",
        version: crate::VERSION,
    }))
}

async fn check(state: &AppState) -> Result<(), ApiError> {
    services::health::check(&state.database)
        .await
        .map_err(|error| {
            tracing::error!(error = ?error, "Readiness check failed");
            ApiError::DatabaseUnavailable
        })
}

#[cfg(target_os = "linux")]
pub async fn update_readiness(
    Extension(state): Extension<AppState>,
    request: axum::extract::Request,
) -> Result<Json<standalone_updater::runtime::ReadyResponse>, ApiError> {
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;
    let local = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|peer| peer.0.ip().is_loopback());
    let direct_host = request
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .and_then(|host| host.parse::<SocketAddr>().ok())
        .is_some_and(|host| host.ip().is_loopback() && host.port() == state.server_bind.port());
    let proxied = [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-proto",
        "x-forwarded-host",
    ]
    .iter()
    .any(|name| request.headers().contains_key(*name));
    if !local || !direct_host || proxied {
        return Err(ApiError::NotFound);
    }
    let services::self_update::ServiceUpdates::Linux(manager) = &state.service_updates else {
        return Err(ApiError::NotFound);
    };
    check(&state).await?;
    Ok(Json(manager.ready_response()))
}
