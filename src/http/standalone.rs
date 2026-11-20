use super::{AppState, artifacts::ArtifactDto, auth, error::ApiError, releases::ids};
use crate::services::sessions::Identity;
use axum::{
    Extension, Json, Router,
    extract::{
        Path, Query,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, put},
};
use serde::Deserialize;
use serde_json::{Value, json};

const SETTINGS: &str = "/admin/applications/{id}/integrations/standalone";
pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route(SETTINGS, put(save_settings))
        .route(
            "/admin/applications/{id}/releases/{release}/artifacts/{artifact}/standalone",
            put(save_signature),
        )
        .layer(middleware::from_fn(auth::require_admin))
        .merge(Router::new().route(SETTINGS, get(settings)))
}
pub fn api_router() -> Router<AppState> {
    Router::new().route("/updates/standalone/{slug}/{target}", get(update))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateQuery {
    current_version: String,
}
async fn update(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 2]>, PathRejection>,
    query: Result<Query<UpdateQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Path([slug, target]) = path.map_err(|_| ApiError::BadRequest)?;
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    match state.standalone_updater.update(&slug, &target, &q.current_version).await? {
        Some(update) => Ok(Json(json!({"version":update.version,"url":update.url,"signed_metadata":update.signed_metadata})).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsRequest {
    #[serde(default, deserialize_with = "setting")]
    public_key: Option<Option<String>>,
    #[serde(default, deserialize_with = "setting")]
    product_id: Option<Option<String>>,
}
// Serde's Option alone maps missing and null to the same value.
fn setting<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}
async fn save_settings(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 1]>, PathRejection>,
    payload: Result<Json<SettingsRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let [app] = ids(path)?;
    let p = auth::json(payload)?;
    state
        .applications
        .set_standalone_settings(
            &actor,
            &app,
            p.product_id.as_ref().map(|v| v.as_deref()),
            p.public_key.as_ref().map(|v| v.as_deref()),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn settings(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 1]>, PathRejection>,
) -> Result<Json<Value>, ApiError> {
    let [id] = ids(path)?;
    let app = state.applications.get(&id).await?;
    let key_id = app
        .standalone_public_key
        .as_deref()
        .and_then(crate::domain::standalone::key_id);
    Ok(Json(
        json!({"application_id":app.id,"product_id":app.product_id,"public_key":app.standalone_public_key,"key_id":key_id}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignatureRequest {
    signed_metadata: Option<String>,
}
async fn save_signature(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 3]>, PathRejection>,
    payload: Result<Json<SignatureRequest>, JsonRejection>,
) -> Result<Json<ArtifactDto>, ApiError> {
    let [app, release, artifact] = ids(path)?;
    let p = auth::json(payload)?;
    Ok(Json(
        state
            .artifacts
            .set_standalone(
                &actor,
                &app,
                &release,
                &artifact,
                p.signed_metadata.as_deref(),
            )
            .await?
            .into(),
    ))
}
