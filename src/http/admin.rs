use super::{
    AppState,
    auth::{self, UserDto, json},
    error::ApiError,
    not_found,
};
use crate::{
    domain::user::{Role, Status},
    services::{sessions::Identity, users::IssuedToken},
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    middleware,
    routing::{any, get, patch, post},
};
use serde::Deserialize;
pub fn router(state: AppState) -> Router<AppState> {
    let administrators = Router::new()
        .route("/admin/users", get(list).post(create))
        .route("/admin/users/invitations", post(invite))
        .route("/admin/users/{id}/role", patch(role))
        .route("/admin/users/{id}/status", patch(status))
        .route("/admin/users/{id}/invitation", post(reinvite))
        .route("/admin/users/{id}/password-reset", post(reset))
        .route("/admin/users/{id}/revoke-sessions", post(revoke))
        .route("/admin", any(not_found))
        .route("/admin/", any(not_found))
        .route("/admin/{*path}", any(not_found))
        .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
        .layer(middleware::from_fn(auth::require_admin));
    administrators
        .merge(super::applications::router())
        .layer(middleware::from_fn_with_state(state, auth::require_session))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    username: String,
    role: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    username: String,
    role: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleRequest {
    role: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusRequest {
    status: String,
}
fn token_dto(state: &AppState, token: IssuedToken) -> serde_json::Value {
    serde_json::json!({"activation_url":format!("{}/admin/activate#token={}",state.auth_config.public_origin,token.token),"expires_at":token.expires_at})
}
async fn list(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
) -> Result<Json<Vec<UserDto>>, ApiError> {
    Ok(Json(
        state
            .users
            .list(&actor)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
async fn create(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    payload: Result<Json<Create>, JsonRejection>,
) -> Result<(StatusCode, Json<UserDto>), ApiError> {
    let p = json(payload)?;
    let role = Role::parse(&p.role).ok_or(ApiError::BadRequest)?;
    let (user, _) = state
        .users
        .create(&actor, &p.username, role, Some(p.password))
        .await?;
    Ok((StatusCode::CREATED, Json(user.into())))
}
async fn invite(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    payload: Result<Json<Invite>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let p = json(payload)?;
    let role = Role::parse(&p.role).ok_or(ApiError::BadRequest)?;
    let (_, token) = state.users.create(&actor, &p.username, role, None).await?;
    Ok((
        StatusCode::CREATED,
        Json(token_dto(&state, token.ok_or(ApiError::Internal)?)),
    ))
}
async fn role(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    Path(id): Path<String>,
    payload: Result<Json<RoleRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let role = Role::parse(&json(payload)?.role).ok_or(ApiError::BadRequest)?;
    state.users.access(&actor, &id, Some(role), None).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn status(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    Path(id): Path<String>,
    payload: Result<Json<StatusRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let status = Status::parse(&json(payload)?.status)
        .filter(|s| *s != Status::Invited)
        .ok_or(ApiError::BadRequest)?;
    state.users.access(&actor, &id, None, Some(status)).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn reinvite(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(token_dto(
        &state,
        state.users.reissue(&actor, &id, false).await?,
    )))
}
async fn reset(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(token_dto(
        &state,
        state.users.reissue(&actor, &id, true).await?,
    )))
}
async fn revoke(
    State(state): State<AppState>,
    Extension(actor): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.users.revoke(&actor, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
