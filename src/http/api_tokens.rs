use super::{AppState, auth, error::ApiError, releases::ids};
use crate::{domain::api_token::ApiToken, infrastructure::secrets, services::sessions::Identity};
use axum::{
    Extension, Json, Router,
    extract::{
        Path,
        rejection::{JsonRejection, PathRejection},
    },
    http::StatusCode,
    middleware,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/applications/{id}/publication-tokens",
            get(list).post(create),
        )
        .route(
            "/admin/applications/{id}/publication-tokens/{token}/revoke",
            post(revoke),
        )
        .layer(middleware::from_fn(auth::require_admin))
}
fn dto(t: ApiToken) -> Value {
    let state = t.state(secrets::now());
    json!({"id":t.id,"application_id":t.application_id,"name":t.name,
        "created_at":t.created_at,"created_by":t.created_by,"expires_at":t.expires_at,
        "revoked_at":t.revoked_at,"revoked_by":t.revoked_by,"state":state})
}
async fn list(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 1]>, PathRejection>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let [app] = ids(path)?;
    Ok(Json(
        state
            .api_tokens
            .list(&actor, &app)
            .await?
            .into_iter()
            .map(dto)
            .collect(),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateToken {
    name: String,
    expires_at: Option<i64>,
}
async fn create(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 1]>, PathRejection>,
    body: Result<Json<CreateToken>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let [app] = ids(path)?;
    let input = auth::json(body)?;
    let issued = state
        .api_tokens
        .create(&actor, &app, &input.name, input.expires_at)
        .await?;
    let mut value = dto(issued.metadata);
    value["token"] = Value::String(issued.secret);
    Ok((StatusCode::CREATED, Json(value)))
}
async fn revoke(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 2]>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let [app, token] = ids(path)?;
    state.api_tokens.revoke(&actor, &app, &token).await?;
    Ok(StatusCode::NO_CONTENT)
}
