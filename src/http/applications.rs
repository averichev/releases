use super::{AppState, auth, error::ApiError};
use crate::{
    domain::application::{
        Application, ApplicationChanges, ListQuery, NewApplication, Sort, Status, Visibility,
    },
    infrastructure::icons::MAX_ICON_BYTES,
    services::sessions::Identity,
};
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{BytesRejection, JsonRejection, PathRejection, QueryRejection},
    },
    http::{StatusCode, header},
    middleware,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Deserializer, Serialize};

pub fn router() -> Router<AppState> {
    let writes = Router::new()
        .route("/admin/applications", post(create))
        .route("/admin/applications/{id}", axum::routing::patch(update))
        .route("/admin/applications/{id}/archive", post(archive))
        .route("/admin/applications/{id}/restore", post(restore))
        .layer(DefaultBodyLimit::max(32 * 1024));
    let icons = Router::new()
        .route(
            "/admin/applications/{id}/icon",
            post(upload_icon).delete(remove_icon),
        )
        .layer(DefaultBodyLimit::max(MAX_ICON_BYTES));
    writes
        .merge(icons)
        .layer(middleware::from_fn(auth::require_admin))
        .merge(
            Router::new()
                .route("/admin/applications", get(list))
                .route("/admin/applications/{id}", get(get_one))
                .route("/admin/applications/{id}/icon", get(icon)),
        )
        .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
}
#[derive(Serialize)]
struct ApplicationDto {
    id: String,
    slug: String,
    name: String,
    description: String,
    icon_url: Option<String>,
    visibility: &'static str,
    status: &'static str,
    created_at: i64,
    updated_at: i64,
}
impl From<Application> for ApplicationDto {
    fn from(app: Application) -> Self {
        Self {
            icon_url: app
                .icon_key
                .map(|key| format!("/api/v1/admin/applications/{}/icon?v={key}", app.id)),
            id: app.id,
            slug: app.slug,
            name: app.name,
            description: app.description,
            visibility: app.visibility.as_str(),
            status: app.status.as_str(),
            created_at: app.created_at,
            updated_at: app.updated_at,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    name: String,
    slug: String,
    #[serde(default)]
    description: String,
    #[serde(default = "private")]
    visibility: String,
}
fn private() -> String {
    "private".into()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateRequest {
    #[serde(default, deserialize_with = "present_string")]
    name: Option<String>,
    #[serde(default, deserialize_with = "present_string")]
    description: Option<String>,
    #[serde(default, deserialize_with = "present_string")]
    visibility: Option<String>,
}
fn present_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct ListRequest {
    page: Option<u32>,
    per_page: Option<u32>,
    search: String,
    status: Option<String>,
    visibility: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}
#[derive(Serialize)]
struct PageDto {
    items: Vec<ApplicationDto>,
    total: i64,
    page: u32,
    per_page: u32,
}
fn id(path: Result<Path<String>, PathRejection>) -> Result<String, ApiError> {
    let Path(value) = path.map_err(|_| ApiError::BadRequest)?;
    uuid::Uuid::parse_str(&value).map_err(|_| ApiError::NotFound)?;
    Ok(value)
}
fn visibility(value: &str) -> Result<Visibility, ApiError> {
    Visibility::parse(value).ok_or(ApiError::InvalidApplication)
}
fn payload<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    payload
        .map(|Json(v)| v)
        .map_err(|_| ApiError::InvalidApplication)
}
async fn list(
    State(state): State<AppState>,
    query: Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<PageDto>, ApiError> {
    let Query(q) = query.map_err(|_| ApiError::InvalidApplication)?;
    let query = ListQuery {
        page: q.page.unwrap_or(1),
        per_page: q.per_page.unwrap_or(24),
        search: q.search.trim().to_owned(),
        status: match q.status.as_deref().unwrap_or("active") {
            "all" => None,
            value => Some(Status::parse(value).ok_or(ApiError::InvalidApplication)?),
        },
        visibility: match q.visibility.as_deref().unwrap_or("all") {
            "all" => None,
            value => Some(visibility(value)?),
        },
        sort: match q.sort.as_deref().unwrap_or("updated_at") {
            "name" => Sort::Name,
            "created_at" => Sort::CreatedAt,
            "updated_at" => Sort::UpdatedAt,
            _ => return Err(ApiError::InvalidApplication),
        },
        descending: match q.order.as_deref().unwrap_or("desc") {
            "asc" => false,
            "desc" => true,
            _ => return Err(ApiError::InvalidApplication),
        },
    };
    let page = query.page;
    let per_page = query.per_page;
    let result = state.applications.list(query).await?;
    Ok(Json(PageDto {
        items: result.items.into_iter().map(Into::into).collect(),
        total: result.total,
        page,
        per_page,
    }))
}
async fn get_one(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<ApplicationDto>, ApiError> {
    Ok(Json(state.applications.get(&id(path)?).await?.into()))
}
async fn create(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    request: Result<Json<CreateRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApplicationDto>), ApiError> {
    let p = payload(request)?;
    let app = state
        .applications
        .create(
            &actor,
            NewApplication {
                name: p.name,
                slug: p.slug,
                description: p.description,
                visibility: visibility(&p.visibility)?,
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(app.into())))
}
async fn update(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<String>, PathRejection>,
    request: Result<Json<UpdateRequest>, JsonRejection>,
) -> Result<Json<ApplicationDto>, ApiError> {
    let p = payload(request)?;
    Ok(Json(
        state
            .applications
            .update(
                &actor,
                &id(path)?,
                ApplicationChanges {
                    name: p.name,
                    description: p.description,
                    visibility: p.visibility.map(|v| visibility(&v)).transpose()?,
                },
            )
            .await?
            .into(),
    ))
}
async fn archive(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<ApplicationDto>, ApiError> {
    Ok(Json(
        state
            .applications
            .set_status(&actor, &id(path)?, Status::Archived)
            .await?
            .into(),
    ))
}
async fn restore(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<ApplicationDto>, ApiError> {
    Ok(Json(
        state
            .applications
            .set_status(&actor, &id(path)?, Status::Active)
            .await?
            .into(),
    ))
}
async fn upload_icon(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<String>, PathRejection>,
    bytes: Result<Bytes, BytesRejection>,
) -> Result<Json<ApplicationDto>, ApiError> {
    Ok(Json(
        state
            .applications
            .set_icon(
                &actor,
                &id(path)?,
                Some(bytes.map_err(|_| ApiError::InvalidIcon)?.to_vec()),
            )
            .await?
            .into(),
    ))
}
async fn remove_icon(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<ApplicationDto>, ApiError> {
    Ok(Json(
        state
            .applications
            .set_icon(&actor, &id(path)?, None)
            .await?
            .into(),
    ))
}
async fn icon(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<impl IntoResponse, ApiError> {
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        state.applications.icon(&id(path)?).await?,
    ))
}
