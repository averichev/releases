use super::{AppState, auth, error::ApiError, releases::ids};
use crate::{
    domain::publication::{PublicationValidation, Readiness},
    services::{publishing::PublicationCommand, sessions::Identity},
};
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Path, Query,
        rejection::{BytesRejection, JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
const APP: &str = "/admin/applications/{id}";
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            &format!("{APP}/releases/{{release}}/validate-publication"),
            post(validate),
        )
        .route(
            &format!("{APP}/releases/{{release}}/publish"),
            post(publish),
        )
        .route(&format!("{APP}/manifests/preview"), get(manifest_preview))
        .route(
            &format!("{APP}/releases/{{release}}/artifacts/{{artifact}}/updater"),
            put(updater_role),
        )
        .route(
            &format!("{APP}/releases/{{release}}/artifacts/{{artifact}}/signature"),
            put(signature).delete(delete_signature),
        )
        .layer(DefaultBodyLimit::max(
            crate::services::updater::MAX_SIGNATURE_BYTES,
        ))
        .layer(middleware::from_fn(auth::require_admin))
        .merge(
            Router::new()
                .route(&format!("{APP}/updater/tauri/preview"), post(tauri_preview))
                .route(&format!("{APP}/publication-history"), get(history)),
        )
        .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
}
pub(super) fn validation_dto(v: &PublicationValidation) -> Value {
    let issues = |items: &[crate::domain::publication::PublicationIssue]| {
        items
            .iter()
            .map(|i| json!({"code":i.code,"artifact_id":i.artifact_id,"message":i.message}))
            .collect::<Vec<_>>()
    };
    json!({"state":match v.state {Readiness::Ready=>"ready",Readiness::Warnings=>"warnings",Readiness::Errors=>"errors"},"warnings":issues(&v.warnings),"errors":issues(&v.errors)})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishRequest {
    expected_release_revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidateRequest {}
async fn validate(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 2]>, PathRejection>,
    payload: Result<Json<ValidateRequest>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let [app, id] = ids(path)?;
    auth::json(payload)?;
    let check = state.publishing.validate(&actor, &app, &id).await?;
    Ok(Json(
        json!({"release_revision":check.release_revision,"validation":validation_dto(&check.validation)}),
    ))
}
async fn publish(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 2]>, PathRejection>,
    payload: Result<Json<PublishRequest>, JsonRejection>,
) -> Result<Json<super::releases::ReleaseDto>, ApiError> {
    let [app, id] = ids(path)?;
    let p = auth::json(payload)?;
    Ok(Json(super::releases::ReleaseDto::from(
        state
            .publishing
            .publish(
                &actor,
                &app,
                &id,
                PublicationCommand {
                    expected_release_revision: p.expected_release_revision,
                },
            )
            .await?,
    )))
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct HistoryQuery {
    limit: Option<u32>,
    before: Option<i64>,
}
async fn history(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 1]>, PathRejection>,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let [app] = ids(path)?;
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    let rows = state
        .publishing
        .history(&app, q.limit.unwrap_or(50), q.before)
        .await?;
    Ok(Json(Value::Array(rows.into_iter().map(|h| {
        let author = h.token_name.as_ref().map(|name|format!("Токен {name}")).or_else(||h.username.clone());
        json!({"id":h.id,"application_id":h.application_id,"release_id":h.release_id,"version":h.version,
            "user_id":h.user_id,"username":h.username,"token_id":h.token_id,"token_name":h.token_name,
            "author":author,"created_at":h.created_at})
    }).collect())))
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ManifestQuery {
    limit: Option<u32>,
}
async fn manifest_preview(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 1]>, PathRejection>,
    query: Result<Query<ManifestQuery>, QueryRejection>,
) -> Result<Json<crate::services::manifest::Manifest>, ApiError> {
    let [app] = ids(path)?;
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    Ok(Json(
        state
            .manifests
            .generate(&app, false, q.limit.unwrap_or(20))
            .await?,
    ))
}
pub async fn public_manifest(
    Extension(state): Extension<AppState>,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<ManifestQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Path(slug) = path.map_err(|_| ApiError::NotFound)?;
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    let document = state
        .manifests
        .generate(&slug, true, q.limit.unwrap_or(20))
        .await?;
    let bytes = serde_json::to_vec(&document).map_err(|_| ApiError::Internal)?;
    let etag = format!("W/\"{:x}\"", Sha256::digest(&bytes));
    // Authorization/visibility and integrity always precede conditional caching.
    let matched = headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|s| s.split(','))
        .map(str::trim)
        .any(|s| {
            s == "*"
                || s.strip_prefix("W/").unwrap_or(s) == etag.strip_prefix("W/").unwrap_or(&etag)
        });
    let cache = [
        (header::ETAG, etag),
        (header::CACHE_CONTROL, "public, no-cache".into()),
    ];
    if matched {
        return Ok((StatusCode::NOT_MODIFIED, cache).into_response());
    }
    Ok((cache, [(header::CONTENT_TYPE, "application/json")], bytes).into_response())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TauriRequest {
    target: String,
    current_version: String,
}
async fn tauri_preview(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 1]>, PathRejection>,
    payload: Result<Json<TauriRequest>, JsonRejection>,
) -> Result<Json<crate::services::updater::TauriPreview>, ApiError> {
    let [app] = ids(path)?;
    let p = auth::json(payload)?;
    Ok(Json(
        state
            .updater
            .preview(&app, &p.target, &p.current_version)
            .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdaterRole {
    distribution_role: String,
}
async fn updater_role(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 3]>, PathRejection>,
    payload: Result<Json<UpdaterRole>, JsonRejection>,
) -> Result<Json<super::artifacts::ArtifactDto>, ApiError> {
    let [app, release, id] = ids(path)?;
    let p = auth::json(payload)?;
    Ok(Json(
        state
            .artifacts
            .set_updater(
                &actor,
                &app,
                &release,
                &id,
                Some(&p.distribution_role),
                None,
            )
            .await?
            .into(),
    ))
}
async fn signature(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 3]>, PathRejection>,
    bytes: Result<Bytes, BytesRejection>,
) -> Result<Json<super::artifacts::ArtifactDto>, ApiError> {
    let [app, release, id] = ids(path)?;
    let bytes = bytes.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::PayloadTooLarge
        } else {
            ApiError::InvalidUpdater
        }
    })?;
    Ok(Json(
        state
            .artifacts
            .set_updater(&actor, &app, &release, &id, None, Some(Some(&bytes)))
            .await?
            .into(),
    ))
}
async fn delete_signature(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 3]>, PathRejection>,
) -> Result<Json<super::artifacts::ArtifactDto>, ApiError> {
    let [app, release, id] = ids(path)?;
    Ok(Json(
        state
            .artifacts
            .set_updater(&actor, &app, &release, &id, None, Some(None))
            .await?
            .into(),
    ))
}
