use super::{AppState, auth, delivery, error::ApiError, releases::ids};
use crate::{
    domain::{application::DistributionSelector, artifact::Artifact, release::Release},
    services::{
        distribution::{ArtifactSelection, PublicDistribution},
        manifest::rfc3339,
        sessions::Identity,
    },
};
use axum::{
    Extension, Json, Router,
    extract::{
        Path, Query,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, Method, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, put},
};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/public/applications/{slug}", get(application))
        .route("/public/applications/{slug}/releases", get(history))
        .route("/public/applications/{slug}/icon", get(icon))
        .route(
            "/updates/tauri/{slug}/{channel}/{target}/{arch}/{version}",
            get(tauri),
        )
}
pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/applications/{id}/distribution/default",
            put(set_default),
        )
        .layer(middleware::from_fn(auth::require_admin))
        .merge(
            Router::new()
                .route("/admin/applications/{id}/distribution", get(settings))
                .route(
                    "/admin/applications/{id}/distribution/releases/{release}",
                    get(release_settings),
                ),
        )
}
pub fn documents_router() -> Router<AppState> {
    Router::new()
        .route("/{slug}", get(page))
        .route("/{slug}/latest", get(latest))
        .route(
            "/{slug}/releases/{version}/artifacts/{artifact}/download",
            get(download),
        )
        .route("/{slug}/{version}/{target}", get(version))
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DownloadQuery {
    channel: Option<String>,
    target: Option<String>,
    format: Option<String>,
    purpose: Option<String>,
}
impl DownloadQuery {
    fn channel(&self) -> &str {
        self.channel.as_deref().unwrap_or("stable")
    }
    fn selection(&self) -> ArtifactSelection {
        ArtifactSelection {
            target: self.target.clone(),
            format: self.format.clone(),
            purpose: self.purpose.clone(),
        }
    }
}
async fn latest(
    Extension(state): Extension<AppState>,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<DownloadQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Path(slug) = path.map_err(|_| ApiError::NotFound)?;
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    let d = state
        .distribution
        .latest(&slug, q.channel(), &q.selection())
        .await?;
    Ok((
        StatusCode::TEMPORARY_REDIRECT,
        [
            (
                header::LOCATION,
                state
                    .distribution
                    .artifact_path(&slug, &d.release, &d.artifact, &d.channel),
            ),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
    )
        .into_response())
}
async fn version(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 3]>, PathRejection>,
    query: Result<Query<DownloadQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Path([slug, version, target]) = path.map_err(|_| ApiError::NotFound)?;
    let Query(mut q) = query.map_err(|_| ApiError::BadRequest)?;
    if q.target.is_some() {
        return Err(ApiError::BadRequest);
    }
    q.target = Some(target);
    let d = state
        .distribution
        .version(&slug, &version, q.channel(), &q.selection(), None)
        .await?;
    Ok((
        StatusCode::TEMPORARY_REDIRECT,
        [
            (
                header::LOCATION,
                state
                    .distribution
                    .artifact_path(&slug, &d.release, &d.artifact, &d.channel),
            ),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
    )
        .into_response())
}
async fn download(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 3]>, PathRejection>,
    query: Result<Query<DownloadQuery>, QueryRejection>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Path([slug, version, artifact]) = path.map_err(|_| ApiError::NotFound)?;
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    if q.target.is_some() || q.format.is_some() || q.purpose.is_some() {
        return Err(ApiError::BadRequest);
    }
    uuid::Uuid::parse_str(&artifact).map_err(|_| ApiError::NotFound)?;
    let d = state
        .distribution
        .version(
            &slug,
            &version,
            q.channel(),
            &q.selection(),
            Some(&artifact),
        )
        .await?;
    delivery::deliver(&state, d, method, headers).await
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CatalogQuery {
    channel: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}
fn artifact_dto(
    d: &PublicDistribution,
    slug: &str,
    r: &Release,
    a: &Artifact,
    channel: &str,
) -> Value {
    json!({"id":a.id,"filename":a.filename,"target":a.target,"format":a.file_type,"purpose":a.distribution_role,"size_bytes":a.size_bytes,"sha256":a.sha256,"download_url":d.artifact_url(slug,r,a,channel)})
}
async fn release_dto(
    d: &PublicDistribution,
    slug: &str,
    r: &Release,
    artifacts: &[Artifact],
    channel: &str,
) -> Result<Value, ApiError> {
    let mut files = Vec::new();
    for a in artifacts {
        d.check_artifact(a).await?;
        files.push(artifact_dto(d, slug, r, a, channel));
    }
    Ok(
        json!({"version":r.version.to_string(),"published_at":rfc3339(r.published_at.ok_or(ApiError::StorageUnavailable)?)?,"notes":r.release_notes,"artifacts":files}),
    )
}
async fn application(
    Extension(state): Extension<AppState>,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<CatalogQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Path(slug) = path.map_err(|_| ApiError::NotFound)?;
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    if q.limit.is_some() || q.offset.is_some() {
        return Err(ApiError::BadRequest);
    }
    let channel = q.channel.as_deref().unwrap_or("stable");
    let (a, _, s) = state.distribution.current(&slug, channel, true).await?;
    let latest = if let Some(s) = &s {
        Some(
            release_dto(
                &state.distribution,
                &a.slug,
                &s.release,
                &s.artifacts,
                channel,
            )
            .await?,
        )
    } else {
        None
    };
    let selector = state.distribution.default_selector(&a.id).await?;
    let default_artifact = s
        .as_ref()
        .and_then(|s| {
            crate::services::distribution::select_artifact(
                &s.artifacts,
                &ArtifactSelection::default(),
                selector.as_ref(),
            )
            .ok()
        })
        .map(|a| a.id);
    let platforms: std::collections::BTreeSet<_> = s
        .as_ref()
        .map(|s| s.artifacts.iter().map(|a| a.target.clone()).collect())
        .unwrap_or_default();
    state.distribution.recheck(&a.id).await?;
    Ok(Json(
        json!({"name":a.name,"description":a.description,"slug":a.slug,"icon_url":a.icon_key.map(|_|format!("/api/v1/public/applications/{}/icon",a.slug)),"channel":channel,"latest":latest,"platforms":platforms,"default_artifact_id":default_artifact}),
    ))
}
async fn history(
    Extension(state): Extension<AppState>,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<CatalogQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Path(slug) = path.map_err(|_| ApiError::NotFound)?;
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    let channel = q.channel.as_deref().unwrap_or("stable");
    let limit = q.limit.unwrap_or(20);
    let offset = q.offset.unwrap_or(0);
    let h = state
        .distribution
        .history(&slug, channel, true, limit, offset)
        .await?;
    let mut items = Vec::new();
    for (r, a) in &h.releases {
        items.push(release_dto(&state.distribution, &slug, r, a, channel).await?);
    }
    state.distribution.recheck(&h.application.id).await?;
    Ok(Json(
        json!({"items":items,"total":h.total,"limit":limit,"offset":offset}),
    ))
}
async fn page(
    Extension(state): Extension<AppState>,
    path: Result<Path<String>, PathRejection>,
    method: Method,
) -> Result<Response, ApiError> {
    let Path(slug) = path.map_err(|_| ApiError::NotFound)?;
    match state.distribution.application(&slug, true).await {
        Ok(_) => Ok(super::assets::document(&method)),
        Err(crate::services::error::ServiceError::NotFound)
            if crate::domain::application::valid_slug(&slug) =>
        {
            let mut response = super::assets::document(&method);
            *response.status_mut() = StatusCode::NOT_FOUND;
            Ok(response)
        }
        Err(error) => Err(error.into()),
    }
}
async fn icon(
    Extension(state): Extension<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Response, ApiError> {
    let Path(slug) = path.map_err(|_| ApiError::NotFound)?;
    let app = state.distribution.application(&slug, true).await?;
    let bytes = state.applications.icon(&app.id).await?;
    state.distribution.recheck(&app.id).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "private, no-cache, must-revalidate"),
        ],
        bytes,
    )
        .into_response())
}
async fn tauri(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 5]>, PathRejection>,
) -> Result<Response, ApiError> {
    let Path([slug, channel, target, arch, version]) = path.map_err(|_| ApiError::BadRequest)?;
    match state
        .updater
        .update(&slug, &channel, &format!("{target}-{arch}"), &version)
        .await?
    {
        Some(response) => Ok(Json(response).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectorDto {
    target: String,
    format: String,
    purpose: String,
}
async fn set_default(
    Extension(state): Extension<AppState>,
    Extension(actor): Extension<Identity>,
    path: Result<Path<[String; 1]>, PathRejection>,
    payload: Result<Json<Option<SelectorDto>>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let [id] = ids(path)?;
    let value = auth::json(payload)?.map(|s| DistributionSelector {
        target: s.target,
        format: s.format,
        purpose: s.purpose,
    });
    state
        .applications
        .set_distribution(&actor, &id, value)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn settings(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 1]>, PathRejection>,
) -> Result<Json<Value>, ApiError> {
    let [id] = ids(path)?;
    let a = state.applications.get(&id).await?;
    let default = state.distribution.default_selector(&id).await?;
    let base = &state.distribution.config.public_base_url;
    let slug = &a.slug;
    let channels = state.publishing.channels(&id).await?;
    Ok(Json(
        json!({"public_available":crate::services::distribution::public_application(&a).is_ok(),"application_url":format!("{base}/{slug}"),"latest_url":format!("{base}/{slug}/latest"),"stable_manifest_url":format!("{base}/{slug}/manifest.json"),"beta_manifest_url":format!("{base}/{slug}/manifest.json?channel=beta"),"tauri_url":format!("{base}/api/v1/updates/tauri/{slug}/stable/{{{{target}}}}/{{{{arch}}}}/{{{{current_version}}}}"),"default_distribution":default.map(|s|json!({"target":s.target,"format":s.format,"purpose":s.purpose})),"channels":channels.into_iter().map(|c|json!({"name":c.name,"version":c.current_version})).collect::<Vec<_>>()}),
    ))
}

async fn release_settings(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 2]>, PathRejection>,
) -> Result<Json<Value>, ApiError> {
    let [id, release] = ids(path)?;
    let app = state.applications.get(&id).await?;
    let release = state.releases.get(&id, &release).await?;
    let artifacts = state.artifacts.list(&id, &release.id).await?;
    let published = release.status == crate::domain::release::ReleaseStatus::Published;
    let channel = if release.channels.iter().any(|c| c == "stable") {
        "stable"
    } else {
        "beta"
    };
    let public = published && crate::services::distribution::public_application(&app).is_ok();
    let files=artifacts.iter().map(|a|json!({"id":a.id,"url":published.then(||state.distribution.artifact_url(&app.slug,&release,a,channel)),"tauri_compatible":a.distribution_role=="updater" && crate::domain::publication::compatible_tauri_target(a).is_some(),"signature_present":a.updater_signature.is_some()})).collect::<Vec<_>>();
    let targets: std::collections::BTreeSet<_> =
        artifacts.iter().map(|a| a.target.as_str()).collect();
    let has_updater = artifacts.iter().any(|a| a.distribution_role == "updater");
    let warnings = targets
        .into_iter()
        .filter(|_| has_updater)
        .filter_map(|target| {
            let updaters: Vec<_> = artifacts
                .iter()
                .filter(|a| {
                    a.target == target
                        && a.distribution_role == "updater"
                        && crate::domain::publication::compatible_tauri_target(a).is_some()
                })
                .collect();
            if updaters.is_empty() {
                Some(format!("{target}: Tauri updater-пакет отсутствует."))
            } else if updaters.iter().any(|a| a.updater_signature.is_none()) {
                Some(format!("{target}: отсутствует подпись updater-пакета."))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    Ok(Json(
        json!({"public_available":public,"files":files,"warnings":warnings}),
    ))
}
