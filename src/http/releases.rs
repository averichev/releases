use super::{AppState, auth, error::ApiError};
use crate::{
    domain::release::{Release, ReleaseChanges, ReleaseInput, ReleaseQuery, ReleaseSort},
    services::sessions::Identity,
};
use axum::{
    Extension, Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::StatusCode,
    middleware,
    routing::{get, patch, post},
};
use serde::{Deserialize, Deserializer, Serialize};
pub const BASE: &str = "/admin/applications/{id}/releases";
pub const DETAIL: &str = "/admin/applications/{id}/releases/{release}";
pub fn router() -> Router<AppState> {
    Router::new()
        .route(BASE, post(create))
        .route(DETAIL, patch(update).delete(delete))
        .layer(DefaultBodyLimit::max(128 * 1024))
        .layer(middleware::from_fn(auth::require_admin))
        .merge(
            Router::new()
                .route(BASE, get(list))
                .route(DETAIL, get(get_one)),
        )
        .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
}
#[derive(Serialize)]
pub(super) struct ReleaseDto {
    id: String,
    application_id: String,
    version: String,
    title: Option<String>,
    release_notes: String,
    status: &'static str,
    revision: i64,
    created_at: i64,
    updated_at: i64,
    published_at: Option<i64>,
    channels: Vec<String>,
    artifact_count: i64,
    total_size_bytes: i64,
}
impl From<Release> for ReleaseDto {
    fn from(r: Release) -> Self {
        Self {
            id: r.id,
            application_id: r.application_id,
            version: r.version.to_string(),
            title: r.title,
            release_notes: r.release_notes,
            status: r.status.as_str(),
            revision: r.revision,
            created_at: r.created_at,
            updated_at: r.updated_at,
            published_at: r.published_at,
            channels: r.channels,
            artifact_count: r.artifact_count,
            total_size_bytes: r.total_size_bytes,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    version: String,
    title: Option<String>,
    #[serde(default)]
    release_notes: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Changes {
    #[serde(default, deserialize_with = "present_string")]
    version: Option<String>,
    #[serde(default, deserialize_with = "present_title")]
    title: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_string")]
    release_notes: Option<String>,
}
fn present_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
fn present_title<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct ListRequest {
    page: Option<u32>,
    per_page: Option<u32>,
    search: String,
    status: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}
#[derive(Serialize)]
struct PageDto {
    items: Vec<ReleaseDto>,
    total: usize,
    page: u32,
    per_page: u32,
}
fn json<T>(value: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    value.map(|Json(v)| v).map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::PayloadTooLarge
        } else {
            ApiError::InvalidRelease
        }
    })
}
pub(super) fn ids<const N: usize>(
    path: Result<Path<[String; N]>, PathRejection>,
) -> Result<[String; N], ApiError> {
    let Path(mut values) = path.map_err(|_| ApiError::NotFound)?;
    for value in &mut values {
        *value = uuid::Uuid::parse_str(value)
            .map_err(|_| ApiError::NotFound)?
            .to_string();
    }
    Ok(values)
}
async fn list(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 1]>, PathRejection>,
    query: Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<PageDto>, ApiError> {
    let [app] = ids(path)?;
    let Query(q) = query.map_err(|_| ApiError::InvalidRelease)?;
    let page = q.page.unwrap_or(1);
    let per_page = q.per_page.unwrap_or(20);
    let result = state
        .releases
        .list(
            &app,
            ReleaseQuery {
                page,
                per_page,
                search: q.search,
                status: q
                    .status
                    .map(|s| {
                        crate::domain::release::ReleaseStatus::parse(&s)
                            .ok_or(ApiError::InvalidRelease)
                    })
                    .transpose()?,
                sort: match q.sort.as_deref().unwrap_or("version") {
                    "version" => ReleaseSort::Version,
                    "created_at" => ReleaseSort::CreatedAt,
                    "updated_at" => ReleaseSort::UpdatedAt,
                    _ => return Err(ApiError::InvalidRelease),
                },
                descending: match q.order.as_deref().unwrap_or("desc") {
                    "asc" => false,
                    "desc" => true,
                    _ => return Err(ApiError::InvalidRelease),
                },
            },
        )
        .await?;
    Ok(Json(PageDto {
        items: result.items.into_iter().map(Into::into).collect(),
        total: result.total,
        page,
        per_page,
    }))
}
async fn create(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 1]>, PathRejection>,
    payload: Result<Json<CreateRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ReleaseDto>), ApiError> {
    let [app] = ids(path)?;
    let p = json(payload)?;
    Ok((
        StatusCode::CREATED,
        Json(
            state
                .releases
                .create(
                    &actor,
                    &app,
                    ReleaseInput {
                        version: p.version,
                        title: p.title,
                        release_notes: p.release_notes,
                    },
                )
                .await?
                .into(),
        ),
    ))
}
async fn get_one(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 2]>, PathRejection>,
) -> Result<Json<ReleaseDto>, ApiError> {
    let [app, id] = ids(path)?;
    Ok(Json(state.releases.get(&app, &id).await?.into()))
}
async fn update(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 2]>, PathRejection>,
    payload: Result<Json<Changes>, JsonRejection>,
) -> Result<Json<ReleaseDto>, ApiError> {
    let [app, id] = ids(path)?;
    let p = json(payload)?;
    Ok(Json(
        state
            .releases
            .update(
                &actor,
                &app,
                &id,
                ReleaseChanges {
                    version: p.version,
                    title: p.title,
                    release_notes: p.release_notes,
                },
            )
            .await?
            .into(),
    ))
}
async fn delete(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 2]>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let [app, id] = ids(path)?;
    state.releases.delete(&actor, &app, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
