use super::{
    AppState,
    error::ApiError,
    upload_body::{self, ParserBudget},
};
use crate::{
    domain::{artifact, publication::compatible_tauri_format, release},
    services::api_publishing::{
        BundleArtifact, MAX_ARTIFACTS, MAX_JSON_BYTES, PublicationBundle, UploadedFile,
    },
};
use axum::{
    Extension, Json, Router,
    extract::{
        DefaultBodyLimit, Path, Request,
        multipart::{Field, MultipartError},
        rejection::PathRejection,
    },
    http::{StatusCode, header},
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::time::timeout;

pub fn router(state: &AppState) -> Router<AppState> {
    Router::new()
        .route("/publish/{slug}", post(publish))
        .layer(DefaultBodyLimit::max(
            state.storage_config.max_request_bytes,
        ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseRequest {
    version: String,
    notes: String,
    artifacts: Vec<ArtifactRequest>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactRequest {
    target: String,
    distribution_role: String,
    file_type: String,
    file: String,
    standalone_signature: Option<String>,
    signature: Option<String>,
    size_bytes: Option<u64>,
    sha256: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum PartKind {
    File,
    Jws,
    Sig,
}
fn part_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name != "release"
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
impl ReleaseRequest {
    fn parts(&mut self) -> Result<HashMap<String, PartKind>, ApiError> {
        release::version(&self.version).ok_or(ApiError::InvalidRelease)?;
        release::notes(&self.notes).ok_or(ApiError::InvalidRelease)?;
        if self.artifacts.len() > MAX_ARTIFACTS {
            return Err(ApiError::PayloadTooLarge);
        }
        if self.artifacts.is_empty() {
            return Err(ApiError::InvalidArtifact);
        }
        let mut parts = HashMap::new();
        let mut files: HashMap<String, Vec<usize>> = HashMap::new();
        let mut tuples = HashSet::new();
        let mut updaters = HashSet::new();
        for (i, a) in self.artifacts.iter_mut().enumerate() {
            a.file_type = a.file_type.trim().to_ascii_lowercase();
            if !part_name(&a.file)
                || !artifact::TARGETS.contains(&a.target.as_str())
                || !artifact::ARTIFACT_TYPES.contains(&a.distribution_role.as_str())
                || !artifact::valid_file_type(&a.file_type)
                || !tuples.insert((
                    a.target.clone(),
                    a.file_type.clone(),
                    a.distribution_role.clone(),
                ))
                || a.sha256
                    .as_deref()
                    .is_some_and(|hash| !crate::infrastructure::secrets::valid(hash))
                || a.size_bytes == Some(0)
            {
                return Err(ApiError::InvalidArtifact);
            }
            if a.distribution_role == "updater" {
                if a.signature.is_none() {
                    return Err(ApiError::UpdaterSignatureRequired);
                }
                if compatible_tauri_format(&a.target, &a.file_type).is_none()
                    || !updaters.insert(a.target.clone())
                {
                    return Err(ApiError::InvalidUpdater);
                }
            } else if a.signature.is_some() {
                return Err(ApiError::InvalidUpdater);
            }
            if a.standalone_signature.is_some()
                && !matches!(a.distribution_role.as_str(), "executable" | "installer")
            {
                return Err(ApiError::InvalidStandalone);
            }
            if let Some(kind) = parts.insert(a.file.clone(), PartKind::File)
                && kind != PartKind::File
            {
                return Err(ApiError::InvalidArtifact);
            }
            files.entry(a.file.clone()).or_default().push(i);
            for (name, kind) in [
                (&a.standalone_signature, PartKind::Jws),
                (&a.signature, PartKind::Sig),
            ] {
                if let Some(name) = name
                    && (!part_name(name) || parts.insert(name.clone(), kind).is_some())
                {
                    return Err(ApiError::InvalidArtifact);
                }
            }
        }
        for uses in files.values().filter(|uses| uses.len() > 1) {
            if uses.len() != 2 {
                return Err(ApiError::InvalidArtifact);
            }
            let a = &self.artifacts[uses[0]];
            let b = &self.artifacts[uses[1]];
            if a.target != b.target
                || a.file_type != b.file_type
                || !matches!(
                    (a.distribution_role.as_str(), b.distribution_role.as_str()),
                    ("installer", "updater") | ("updater", "installer")
                )
            {
                return Err(ApiError::InvalidArtifact);
            }
        }
        Ok(parts)
    }
}
fn multipart_error(e: MultipartError, budget: &ParserBudget) -> ApiError {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE || budget.exceeded() {
        ApiError::PayloadTooLarge
    } else {
        ApiError::BadRequest
    }
}
async fn small(
    field: &mut Field<'_>,
    max: usize,
    idle: Duration,
    budget: &ParserBudget,
) -> Result<Vec<u8>, ApiError> {
    budget.file(max + 16 * 1024);
    let mut bytes = Vec::new();
    while let Some(chunk) = timeout(idle, field.chunk())
        .await
        .map_err(|_| ApiError::UploadTimeout)?
        .map_err(|e| multipart_error(e, budget))?
    {
        if bytes.len() + chunk.len() > max {
            return Err(ApiError::PayloadTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn publish(
    Extension(state): Extension<AppState>,
    path: Result<Path<String>, PathRejection>,
    request: Request,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let Path(slug) = path.map_err(|_| ApiError::BadRequest)?;
    super::origin::publication_transport(&request, &state)?;
    let mut headers = request.headers().get_all(header::AUTHORIZATION).iter();
    let auth = headers
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    if headers.next().is_some() {
        return Err(ApiError::Unauthorized);
    }
    let (scheme, secret) = auth.split_once(' ').ok_or(ApiError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err(ApiError::Unauthorized);
    }
    let publisher = state.api_tokens.authenticate(secret, &slug).await?;
    let _permit = state.api_publishing.permit()?;
    let idle = Duration::from_secs(state.storage_config.upload_idle_seconds);
    let (mut multipart, budget) =
        upload_body::multipart(request, state.storage_config.max_request_bytes)
            .await
            .map_err(|e| {
                if matches!(e, ApiError::InvalidArtifact) {
                    ApiError::BadRequest
                } else {
                    e
                }
            })?;
    let mut first = timeout(idle, multipart.next_field())
        .await
        .map_err(|_| ApiError::UploadTimeout)?
        .map_err(|e| multipart_error(e, &budget))?
        .ok_or(ApiError::BadRequest)?;
    if first.name() != Some("release")
        || first.content_type().is_some_and(|value| {
            !value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
    {
        return Err(ApiError::BadRequest);
    }
    let bytes = small(&mut first, MAX_JSON_BYTES, idle, &budget).await?;
    let mut input: ReleaseRequest =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::BadRequest)?;
    drop(first);
    let parts = input.parts()?;
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    let mut file_index = HashMap::new();
    let mut signatures = HashMap::new();
    budget.headers();
    while let Some(mut field) = timeout(idle, multipart.next_field())
        .await
        .map_err(|_| ApiError::UploadTimeout)?
        .map_err(|e| multipart_error(e, &budget))?
    {
        let name = field.name().ok_or(ApiError::BadRequest)?.to_owned();
        if !seen.insert(name.clone()) || seen.len() > parts.len() {
            return Err(ApiError::InvalidArtifact);
        }
        let kind = parts.get(&name).ok_or(ApiError::InvalidArtifact)?;
        let filename = field
            .file_name()
            .ok_or(ApiError::InvalidArtifact)?
            .to_owned();
        if !artifact::valid_filename(&filename) {
            return Err(ApiError::InvalidArtifact);
        }
        match kind {
            PartKind::File => {
                budget.file(state.storage_config.max_request_bytes);
                let mut pending = state.api_publishing.begin_file().await?;
                while let Some(chunk) = timeout(idle, field.chunk())
                    .await
                    .map_err(|_| ApiError::UploadTimeout)?
                    .map_err(|e| multipart_error(e, &budget))?
                {
                    pending.write(&chunk).await?;
                }
                pending.seal().await?;
                file_index.insert(name, files.len());
                files.push(UploadedFile { filename, pending });
            }
            PartKind::Jws | PartKind::Sig => {
                let (extension, max) = if *kind == PartKind::Jws {
                    (".jws", crate::domain::standalone::MAX_SIGNATURE_BYTES)
                } else {
                    (".sig", crate::services::updater::MAX_SIGNATURE_BYTES)
                };
                if !filename.to_ascii_lowercase().ends_with(extension) {
                    return Err(if *kind == PartKind::Jws {
                        ApiError::InvalidStandalone
                    } else {
                        ApiError::InvalidUpdater
                    });
                }
                signatures.insert(name, small(&mut field, max, idle, &budget).await?);
            }
        }
        budget.headers();
    }
    if seen.len() != parts.len() {
        return Err(ApiError::InvalidArtifact);
    }
    let artifacts = input
        .artifacts
        .into_iter()
        .map(|a| {
            Ok(BundleArtifact {
                file: *file_index.get(&a.file).ok_or(ApiError::InvalidArtifact)?,
                target: a.target,
                file_type: a.file_type,
                distribution_role: a.distribution_role,
                size_bytes: a.size_bytes,
                sha256: a.sha256,
                signature: a
                    .signature
                    .map(|name| signatures.remove(&name).ok_or(ApiError::InvalidArtifact))
                    .transpose()?,
                standalone_signature: a
                    .standalone_signature
                    .map(|name| signatures.remove(&name).ok_or(ApiError::InvalidArtifact))
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let result = state
        .api_publishing
        .publish(
            publisher,
            PublicationBundle {
                version: input.version,
                notes: input.notes,
                files,
                artifacts,
            },
        )
        .await?;
    let release = result.release;
    Ok((
        if result.created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({
            "release_id":release.id,"version":release.version.to_string(),"status":release.status.as_str(),"published_at":release.published_at
        })),
    ))
}
