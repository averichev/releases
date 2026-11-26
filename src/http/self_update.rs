use axum::{
    Extension, Json, Router,
    extract::rejection::JsonRejection,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use standalone_updater::{CheckOutcome, OperationStatus, Stage};

use super::{AppState, auth::json, error::ApiError};
use crate::{
    config::SelfUpdateMode,
    infrastructure::self_update::target,
    services::{self_update::Action, sessions::Identity},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/self-update", get(summary))
        .route("/admin/self-update/check", post(check))
        .route("/admin/self-update/prepare", post(prepare))
        .route("/admin/self-update/install", post(install))
        .route("/admin/self-update/cancel", post(cancel))
}

#[derive(Serialize)]
struct SummaryDto {
    current_version: &'static str,
    target: String,
    mode: Option<SelfUpdateMode>,
    installation_supported: bool,
    installation_reason: Option<String>,
    status: Option<StatusDto>,
}

#[derive(Serialize)]
struct StatusDto {
    operation_id: Option<String>,
    state: Stage,
    check_outcome: CheckOutcome,
    last_checked: Option<u64>,
    available_version: Option<String>,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    message: String,
    recovery_required: bool,
}

impl From<OperationStatus> for StatusDto {
    fn from(s: OperationStatus) -> Self {
        Self {
            operation_id: s.operation_id,
            state: s.state,
            check_outcome: s.check_outcome,
            last_checked: s.last_checked,
            available_version: s.available.map(|a| a.version),
            downloaded_bytes: s.downloaded_bytes,
            total_bytes: s.total_bytes,
            message: s.message,
            recovery_required: s.recovery_required,
        }
    }
}

async fn summary(Extension(state): Extension<AppState>) -> Result<Json<SummaryDto>, ApiError> {
    let updates = state.service_updates.summary().await?;
    Ok(Json(SummaryDto {
        current_version: crate::VERSION,
        target: target(),
        mode: updates.mode,
        installation_supported: updates.installation_supported,
        installation_reason: updates.installation_reason,
        status: updates.status.map(Into::into),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepare {
    check_id: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    operation_id: String,
}

type Accepted = Result<(StatusCode, Json<StatusDto>), ApiError>;

async fn execute(state: AppState, actor: Identity, action: Action) -> Accepted {
    let status = state
        .service_updates
        .execute(&state.sessions, &actor, action)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(status.into())))
}

async fn check(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
) -> Accepted {
    execute(state, actor, Action::Check).await
}

async fn prepare(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    request: Result<Json<Prepare>, JsonRejection>,
) -> Accepted {
    let Prepare { check_id, version } = json(request)?;
    execute(state, actor, Action::Prepare { check_id, version }).await
}

async fn install(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    request: Result<Json<Confirmation>, JsonRejection>,
) -> Accepted {
    execute(
        state,
        actor,
        Action::Install {
            operation_id: json(request)?.operation_id,
        },
    )
    .await
}

async fn cancel(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    request: Result<Json<Confirmation>, JsonRejection>,
) -> Accepted {
    execute(
        state,
        actor,
        Action::Cancel {
            operation_id: json(request)?.operation_id,
        },
    )
    .await
}
