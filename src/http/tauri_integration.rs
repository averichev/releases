use super::{AppState, auth, error::ApiError, releases::ids};
use crate::services::sessions::Identity;
use axum::{
    Extension, Json, Router,
    extract::{
        Path, Query,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::StatusCode,
    middleware,
    routing::{get, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
const PATH: &str = "/admin/applications/{id}/integrations/tauri";
pub fn router() -> Router<AppState> {
    Router::new()
        .route(PATH, put(save))
        .layer(middleware::from_fn(auth::require_admin))
        .merge(
            Router::new()
                .route(PATH, get(settings))
                .route(&format!("{PATH}/readiness"), axum::routing::post(readiness)),
        )
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SettingsQuery {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsRequest {
    public_key: Option<String>,
}
async fn details(
    state: AppState,
    path: Result<Path<[String; 1]>, PathRejection>,
    query: Result<Query<SettingsQuery>, QueryRejection>,
    full: bool,
) -> Result<Json<Value>, ApiError> {
    let [app] = ids(path)?;
    let Query(_q) = query.map_err(|_| ApiError::BadRequest)?;
    let d = state.tauri_integration.details(&app, full).await?;
    Ok(Json(
        json!({"public_key":d.public_key,"public_base_url":d.public_base_url,"endpoint":d.endpoint,"configuration":d.configuration,
        "published_release_exists":d.published_release_exists,
        "packages":d.packages.into_iter().map(|p| json!({"id":p.id,"version":p.version,"filename":p.filename,"target":p.target,"compatible":p.compatible,"signature_present":p.signature_present})).collect::<Vec<_>>(),
        "ready":d.blockers.is_empty(),"blockers":d.blockers.into_iter().map(|i| json!({"code":i.code,"artifact_id":i.artifact_id,"message":i.message})).collect::<Vec<_>>(),
        "validation":d.validation.as_ref().map(super::publication::validation_dto),"full_integrity_checked":full && d.validation.is_some()}),
    ))
}
async fn settings(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 1]>, PathRejection>,
    query: Result<Query<SettingsQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    details(state, path, query, false).await
}
async fn readiness(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 1]>, PathRejection>,
    query: Result<Query<SettingsQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    details(state, path, query, true).await
}
async fn save(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 1]>, PathRejection>,
    payload: Result<Json<SettingsRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let [app] = ids(path)?;
    let p = auth::json(payload)?;
    state
        .applications
        .set_tauri_key(&actor, &app, p.public_key.as_deref())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
