use super::{AppState, error::ApiError};
use crate::{
    domain::user::{Role, User},
    infrastructure::secrets,
    services::{error::ServiceError, sessions::Identity},
};
use axum::{
    Extension, Json, Router,
    extract::{Request, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

#[derive(Serialize)]
pub(super) struct UserDto {
    id: String,
    username: String,
    role: &'static str,
    status: &'static str,
    created_at: i64,
    updated_at: i64,
    last_login_at: Option<i64>,
}
impl From<User> for UserDto {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            username: u.username,
            role: u.role.as_str(),
            status: u.status.as_str(),
            created_at: u.created_at,
            updated_at: u.updated_at,
            last_login_at: u.last_login_at,
        }
    }
}
#[derive(Serialize)]
struct Me {
    user: UserDto,
    csrf_token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    username: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangePassword {
    current_password: String,
    new_password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Activate {
    token: String,
    password: String,
}
pub(super) fn json<T>(value: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    value.map(|Json(v)| v).map_err(|_| ApiError::BadRequest)
}

pub fn router() -> Router<AppState> {
    let protected = Router::new()
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/change-password", post(change_password))
        .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
        .layer(axum::middleware::from_fn(require_session));
    Router::new()
        .route("/login", post(login))
        .route("/token/check", post(check_token))
        .route("/activate", post(activate))
        .merge(protected)
}
pub(super) fn cookie_name(state: &AppState) -> &'static str {
    if state.auth_config.secure_cookie() {
        "__Host-veruna_session"
    } else {
        "veruna_session"
    }
}
fn session_cookie(headers: &HeaderMap, state: &AppState) -> Option<String> {
    let name = cookie_name(state);
    let mut values = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .filter_map(|v| v.trim().split_once('='))
        .filter(|(key, _)| *key == name)
        .map(|(_, value)| value);
    let first = values.next()?;
    if values.next().is_some() || !secrets::valid(first) {
        return None;
    }
    Some(first.to_owned())
}
fn set_cookie(
    state: &AppState,
    token: &str,
    ttl: i64,
) -> Result<axum::http::HeaderValue, ApiError> {
    format!(
        "{}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={ttl}{}",
        cookie_name(state),
        if state.auth_config.secure_cookie() {
            "; Secure"
        } else {
            ""
        }
    )
    .parse()
    .map_err(|_| ApiError::Internal)
}
// Applied to anonymous mutations too: a fixed custom header requires same-origin
// JavaScript/preflight; the exact configured Origin rejects login CSRF and DNS rebinding.
pub async fn same_origin(
    Extension(state): Extension<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if matches!(
        request.uri().path(),
        "/api/v1/auth/login" | "/api/v1/auth/token/check" | "/api/v1/auth/activate"
    ) && request.method() == axum::http::Method::POST
        && !valid_origin(request.headers(), &state)
    {
        return Err(ApiError::Forbidden);
    }
    Ok(next.run(request).await)
}
fn valid_origin(headers: &HeaderMap, state: &AppState) -> bool {
    headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        == Some(state.auth_config.public_origin.as_str())
        && headers
            .get("x-veruna-request")
            .and_then(|v| v.to_str().ok())
            == Some("1")
}
pub async fn require_session(
    Extension(state): Extension<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = session_cookie(request.headers(), &state).ok_or(ApiError::Unauthorized)?;
    let identity = state.sessions.authenticate(&token).await?;
    if !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) {
        let supplied = request
            .headers()
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !valid_origin(request.headers(), &state)
            || !bool::from(secrets::csrf(&token).as_bytes().ct_eq(supplied.as_bytes()))
        {
            return Err(ApiError::Forbidden);
        }
    }
    request.extensions_mut().insert(identity);
    Ok(next.run(request).await)
}
pub async fn require_admin(request: Request, next: Next) -> Result<Response, ApiError> {
    if request
        .extensions()
        .get::<Identity>()
        .is_none_or(|i| i.user.role != Role::Admin)
    {
        return Err(ApiError::Forbidden);
    }
    Ok(next.run(request).await)
}
async fn login(
    Extension(state): Extension<AppState>,
    headers: HeaderMap,
    payload: Result<Json<Credentials>, JsonRejection>,
) -> Result<Response, ApiError> {
    let payload = json(payload)?;
    if payload.username.len() > 128 || payload.password.len() > 512 {
        return Err(ApiError::BadRequest);
    }
    let previous = session_cookie(&headers, &state).map(|v| secrets::digest(&v));
    let login = state
        .authentication
        .login(&payload.username, payload.password, previous)
        .await?;
    Ok((
        [(
            header::SET_COOKIE,
            set_cookie(&state, &login.token, state.auth_config.session_ttl_seconds)?,
        )],
        Json(Me {
            user: login.user.into(),
            csrf_token: secrets::csrf(&login.token),
        }),
    )
        .into_response())
}
async fn me(
    Extension(state): Extension<AppState>,
    Extension(identity): Extension<Identity>,
    headers: HeaderMap,
) -> Result<Json<Me>, ApiError> {
    let token = session_cookie(&headers, &state).ok_or(ApiError::Unauthorized)?;
    Ok(Json(Me {
        user: identity.user.into(),
        csrf_token: secrets::csrf(&token),
    }))
}
async fn logout(
    Extension(state): Extension<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response, ApiError> {
    state.sessions.logout(&identity).await?;
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, set_cookie(&state, "", 0)?)],
    )
        .into_response())
}
async fn change_password(
    Extension(state): Extension<AppState>,
    Extension(identity): Extension<Identity>,
    payload: Result<Json<ChangePassword>, JsonRejection>,
) -> Result<Response, ApiError> {
    let payload = json(payload)?;
    if payload.current_password.len() > 512 {
        return Err(ApiError::BadRequest);
    }
    state
        .authentication
        .change_password(&identity, payload.current_password, payload.new_password)
        .await?;
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, set_cookie(&state, "", 0)?)],
    )
        .into_response())
}
async fn check_token(
    Extension(state): Extension<AppState>,
    payload: Result<Json<Token>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (username, purpose) = state
        .authentication
        .inspect_token(&json(payload)?.token)
        .await?;
    Ok(Json(
        serde_json::json!({"username":username,"purpose":purpose}),
    ))
}
async fn activate(
    Extension(state): Extension<AppState>,
    payload: Result<Json<Activate>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let payload = json(payload)?;
    state
        .authentication
        .activate(&payload.token, payload.password)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
impl From<ServiceError> for ApiError {
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::Invalid => Self::BadRequest,
            ServiceError::Credentials => Self::Credentials,
            ServiceError::Unauthorized => Self::Unauthorized,
            ServiceError::Forbidden => Self::Forbidden,
            ServiceError::RateLimited => Self::RateLimited,
            ServiceError::Conflict => Self::Conflict,
            ServiceError::NotFound => Self::NotFound,
            ServiceError::LastAdmin => Self::LastAdmin,
            ServiceError::Token => Self::Token,
            ServiceError::InvalidApplication => Self::InvalidApplication,
            ServiceError::ApplicationSlugConflict => Self::SlugConflict,
            ServiceError::InvalidIcon => Self::InvalidIcon,
            ServiceError::InvalidRelease => Self::InvalidRelease,
            ServiceError::ReleaseVersionConflict => Self::ReleaseVersionConflict,
            ServiceError::ReleaseLocked => Self::ReleaseLocked,
            ServiceError::InvalidArtifact => Self::InvalidArtifact,
            ServiceError::ArtifactConflict => Self::ArtifactConflict,
            ServiceError::UpdaterSignatureRequired => Self::UpdaterSignatureRequired,
            ServiceError::PayloadTooLarge => Self::PayloadTooLarge,
            ServiceError::UploadTimeout => Self::UploadTimeout,
            ServiceError::StorageFull => Self::StorageFull,
            ServiceError::StorageUnavailable => Self::StorageUnavailable,
            ServiceError::StorageBusy => Self::StorageBusy,
            ServiceError::PublicationConflict => Self::PublicationConflict,
            ServiceError::PublicationBlocked(validation) => Self::PublicationBlocked(validation),
            ServiceError::InvalidUpdater => Self::InvalidUpdater,
            ServiceError::AmbiguousArtifact => Self::AmbiguousArtifact,
            ServiceError::UpdaterUnavailable => Self::UpdaterUnavailable,
            ServiceError::Internal(cause) => {
                tracing::error!(error = ?cause, "Service operation failed");
                Self::Internal
            }
        }
    }
}
