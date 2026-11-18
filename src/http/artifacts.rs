use super::{AppState, auth, error::ApiError, releases::ids};
use crate::{
    domain::artifact::{self, Artifact, UploadMetadata},
    services::sessions::Identity,
};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Path, Request,
        multipart::{Field, MultipartError},
        rejection::PathRejection,
    },
    http::{StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::Serialize;
use std::time::Duration;
use tokio::time::timeout;
use tokio_util::io::ReaderStream;
const BASE: &str = "/admin/applications/{id}/releases/{release}/artifacts";
const DETAIL: &str = "/admin/applications/{id}/releases/{release}/artifacts/{artifact}";
pub fn router(state: &AppState) -> Router<AppState> {
    Router::new()
        .route(BASE, post(upload))
        .layer(DefaultBodyLimit::max(
            state.storage_config.max_request_bytes,
        ))
        .merge(
            Router::new()
                .route(DETAIL, delete(remove))
                .route(&format!("{DETAIL}/verify"), post(verify)),
        )
        .layer(middleware::from_fn(auth::require_admin))
        .merge(
            Router::new()
                .route(BASE, get(list))
                .route(&format!("{DETAIL}/download"), get(download))
                .route("/admin/artifact-options", get(options)),
        )
        .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
}
#[derive(Serialize)]
pub(super) struct ArtifactDto {
    id: String,
    release_id: String,
    filename: String,
    target: String,
    file_type: String,
    size_bytes: i64,
    sha256: String,
    created_at: i64,
    distribution_role: String,
    signature_present: bool,
}
impl From<Artifact> for ArtifactDto {
    fn from(a: Artifact) -> Self {
        Self {
            id: a.id,
            release_id: a.release_id,
            filename: a.filename,
            target: a.target,
            file_type: a.file_type,
            size_bytes: a.size_bytes,
            sha256: a.sha256,
            created_at: a.created_at,
            distribution_role: a.distribution_role,
            signature_present: a.updater_signature.is_some(),
        }
    }
}
#[derive(Serialize)]
struct OptionsDto {
    targets: &'static [&'static str],
    file_types: &'static [&'static str],
    max_file_bytes: u64,
    upload_idle_seconds: u64,
}
async fn options(Extension(state): Extension<AppState>) -> Json<OptionsDto> {
    Json(OptionsDto {
        targets: artifact::TARGETS,
        file_types: artifact::FILE_TYPES,
        max_file_bytes: state.storage_config.max_file_bytes,
        upload_idle_seconds: state.storage_config.upload_idle_seconds,
    })
}
async fn list(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 2]>, PathRejection>,
) -> Result<Json<Vec<ArtifactDto>>, ApiError> {
    let [app, release] = ids(path)?;
    Ok(Json(
        state
            .artifacts
            .list(&app, &release)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
fn multipart_error(error: MultipartError) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::PayloadTooLarge
    } else {
        ApiError::InvalidArtifact
    }
}
async fn text(field: &mut Field<'_>, idle: Duration) -> Result<String, ApiError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = timeout(idle, field.chunk())
        .await
        .map_err(|_| ApiError::UploadTimeout)?
        .map_err(multipart_error)?
    {
        if bytes.len() + chunk.len() > 128 {
            return Err(ApiError::InvalidArtifact);
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| ApiError::InvalidArtifact)
}
async fn upload(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 2]>, PathRejection>,
    request: Request,
) -> Result<(StatusCode, Json<ArtifactDto>), ApiError> {
    let [app, release] = ids(path)?;
    // Resolve ownership/status before consuming any request fields.
    state.releases.get(&app, &release).await?;
    let (mut multipart, budget) =
        super::upload_body::multipart(request, state.storage_config.max_request_bytes).await?;
    let idle = Duration::from_secs(state.storage_config.upload_idle_seconds);
    let mut target = None;
    let mut file_type = None;
    let mut upload = None;
    while let Some(mut field) = timeout(idle, multipart.next_field())
        .await
        .map_err(|_| ApiError::UploadTimeout)?
        .map_err(multipart_error)?
    {
        if upload.is_some() {
            return Err(ApiError::InvalidArtifact);
        }
        // Parsing a header and consuming its field are separate bounded phases.
        budget.headers();
        match field.name() {
            Some("target") if target.is_none() && field.file_name().is_none() => {
                target = Some(text(&mut field, idle).await?)
            }
            Some("file_type") if file_type.is_none() && field.file_name().is_none() => {
                file_type = Some(text(&mut field, idle).await?)
            }
            Some("file") => {
                let filename = field
                    .file_name()
                    .ok_or(ApiError::InvalidArtifact)?
                    .to_owned();
                budget.file(state.storage_config.max_request_bytes);
                let mut pending = state
                    .artifacts
                    .begin(
                        &actor,
                        &app,
                        &release,
                        UploadMetadata {
                            filename,
                            target: target.take().ok_or(ApiError::InvalidArtifact)?,
                            file_type: file_type.take(),
                        },
                    )
                    .await?;
                while let Some(chunk) = timeout(idle, field.chunk())
                    .await
                    .map_err(|_| ApiError::UploadTimeout)?
                    .map_err(multipart_error)?
                {
                    pending.write(&chunk).await?;
                }
                upload = Some(pending);
            }
            _ => return Err(ApiError::InvalidArtifact),
        }
        budget.headers();
    }
    let result = state
        .artifacts
        .complete(&actor, upload.ok_or(ApiError::InvalidArtifact)?)
        .await?;
    Ok((StatusCode::CREATED, Json(result.into())))
}
async fn download(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 3]>, PathRejection>,
) -> Result<Response, ApiError> {
    let [app, release, id] = ids(path)?;
    let (artifact, file) = state.artifacts.download(&app, &release, &id).await?;
    let encoded = artifact
        .filename
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect::<String>();
    let disposition = format!(
        "attachment; filename=\"artifact.{}\"; filename*=UTF-8''{encoded}",
        artifact.file_type
    );
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CONTENT_LENGTH, artifact.size_bytes.to_string()),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}
async fn remove(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 3]>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let [app, release, id] = ids(path)?;
    state.artifacts.delete(&actor, &app, &release, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Serialize)]
struct VerificationDto {
    valid: bool,
}
async fn verify(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 3]>, PathRejection>,
) -> Result<Json<VerificationDto>, ApiError> {
    let [app, release, id] = ids(path)?;
    Ok(Json(VerificationDto {
        valid: state.artifacts.verify(&actor, &app, &release, &id).await?,
    }))
}
