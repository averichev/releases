use super::{AppState, delivery, error::ApiError, releases::ids};
use crate::{
    domain::{artifact::Artifact, release::Release},
    services::{
        distribution::{ArtifactSelection, PublicDistribution},
        manifest::rfc3339,
    },
};
use axum::{
    Extension, Json, Router,
    extract::{
        Path, Query,
        rejection::{PathRejection, QueryRejection},
    },
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/public/applications", get(applications))
        .route("/public/applications/{slug}", get(application))
        .route("/public/applications/{slug}/releases", get(history))
        .route("/public/applications/{slug}/icon", get(icon))
        .route(
            "/updates/tauri/{slug}/{target}/{arch}/{version}",
            get(tauri),
        )
}
pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/admin/applications/{id}/distribution", get(settings))
        .route(
            "/admin/applications/{id}/distribution/releases/{release}",
            get(release_settings),
        )
}
pub fn documents_router() -> Router<AppState> {
    Router::new()
        .route("/{slug}", get(page))
        .route("/{slug}/latest", get(latest_page))
        .route("/{slug}/latest/{target}", get(latest))
        .route(
            "/{slug}/releases/{version}/artifacts/{artifact}/download",
            get(download),
        )
        .route("/{slug}/{version}/{target}", get(version))
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DownloadQuery {
    target: Option<String>,
    format: Option<String>,
    #[serde(alias = "type")]
    purpose: Option<String>,
}
impl DownloadQuery {
    fn selection(&self) -> ArtifactSelection {
        ArtifactSelection {
            target: self.target.clone(),
            format: self.format.clone(),
            purpose: self.purpose.clone(),
        }
    }
}
fn redirect(location: String) -> Response {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [
            (header::LOCATION, location),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
    )
        .into_response()
}
async fn latest_page(
    Extension(state): Extension<AppState>,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<DownloadQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Path(slug) = path.map_err(|_| ApiError::NotFound)?;
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    if q.target.is_some() || q.format.is_some() || q.purpose.is_some() {
        let d = state.distribution.latest(&slug, &q.selection()).await?;
        return Ok(redirect(state.distribution.artifact_path(
            &slug,
            &d.release,
            &d.artifact,
        )));
    }
    state.distribution.application(&slug, true).await?;
    Ok(redirect(format!("/{slug}")))
}
async fn latest(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 2]>, PathRejection>,
    query: Result<Query<DownloadQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Path([slug, target]) = path.map_err(|_| ApiError::NotFound)?;
    let Query(mut q) = query.map_err(|_| ApiError::BadRequest)?;
    if q.target.is_some() {
        return Err(ApiError::BadRequest);
    }
    q.target = Some(target);
    let d = state.distribution.latest(&slug, &q.selection()).await?;
    Ok(redirect(state.distribution.artifact_path(
        &slug,
        &d.release,
        &d.artifact,
    )))
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
        .version(&slug, &version, &q.selection(), None)
        .await?;
    Ok((
        StatusCode::TEMPORARY_REDIRECT,
        [
            (
                header::LOCATION,
                state
                    .distribution
                    .artifact_path(&slug, &d.release, &d.artifact),
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
        .version(&slug, &version, &q.selection(), Some(&artifact))
        .await?;
    delivery::deliver(&state, d, method, headers).await
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CatalogQuery {
    limit: Option<u32>,
    offset: Option<u32>,
}
fn artifact_dto(d: &PublicDistribution, slug: &str, r: &Release, a: &Artifact) -> Value {
    json!({"id":a.id,"filename":a.filename,"target":a.target,"format":a.file_type,"purpose":a.distribution_role,"size_bytes":a.size_bytes,"sha256":a.sha256,"download_url":d.artifact_url(slug,r,a)})
}
async fn applications(
    Extension(state): Extension<AppState>,
    query: Result<Query<CatalogQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    let limit = q.limit.unwrap_or(20);
    let offset = q.offset.unwrap_or(0);
    let page = state.distribution.applications(limit, offset).await?;
    let items: Vec<_> = page.items.iter().map(|a| json!({
        "name": a.name,
        "slug": a.slug,
        "icon_url": a.icon_key.as_ref().map(|_| format!("/api/v1/public/applications/{}/icon", a.slug)),
    })).collect();
    Ok(Json(
        json!({"items": items, "total": page.total, "limit": limit, "offset": offset}),
    ))
}
async fn release_dto(
    d: &PublicDistribution,
    slug: &str,
    r: &Release,
    artifacts: &[Artifact],
) -> Result<Value, ApiError> {
    let mut files = Vec::new();
    for a in artifacts {
        d.check_artifact(a).await?;
        files.push(artifact_dto(d, slug, r, a));
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
    let catalog = state.distribution.published(&slug, true).await?;
    let a = &catalog.application;
    let mut latest_by_target = std::collections::BTreeMap::new();
    let mut latest_by_type = std::collections::BTreeMap::new();
    for (kind, targets) in crate::services::distribution::latest_downloads(&catalog) {
        let mut documents = std::collections::BTreeMap::new();
        for (target, (r, artifacts)) in targets {
            documents.insert(
                target,
                release_dto(&state.distribution, &slug, &r, &artifacts).await?,
            );
        }
        if kind == "installer" {
            latest_by_target = documents.clone();
        }
        latest_by_type.insert(kind, documents);
    }
    state.distribution.recheck(&a.id).await?;
    let manifest_url = format!(
        "{}/{slug}/manifest.json",
        state.distribution.config.public_base_url
    );
    Ok(Json(
        json!({"name":a.name,"description":a.description,"slug":a.slug,"icon_url":a.icon_key.as_ref().map(|_|format!("/api/v1/public/applications/{}/icon",a.slug)),"manifest_url":manifest_url,"latest_by_target":latest_by_target,"latest_by_type":latest_by_type}),
    ))
}
async fn history(
    Extension(state): Extension<AppState>,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<CatalogQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Path(slug) = path.map_err(|_| ApiError::NotFound)?;
    let Query(q) = query.map_err(|_| ApiError::BadRequest)?;
    let limit = q.limit.unwrap_or(20);
    let offset = q.offset.unwrap_or(0);
    let h = state
        .distribution
        .history(&slug, true, limit, offset)
        .await?;
    let mut items = Vec::new();
    for (r, a) in &h.releases {
        items.push(release_dto(&state.distribution, &slug, r, a).await?);
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
    let icon = state.applications.icon(&app.id).await?;
    state.distribution.recheck(&app.id).await?;
    Ok(super::icons::response(icon))
}
async fn tauri(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 4]>, PathRejection>,
) -> Result<Response, ApiError> {
    let Path([slug, target, arch, version]) = path.map_err(|_| ApiError::BadRequest)?;
    match state
        .updater
        .update(&slug, &format!("{target}-{arch}"), &version)
        .await?
    {
        Some(response) => Ok(Json(response).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}
async fn settings(
    Extension(state): Extension<AppState>,
    path: Result<Path<[String; 1]>, PathRejection>,
) -> Result<Json<Value>, ApiError> {
    let [id] = ids(path)?;
    let a = state.applications.get(&id).await?;
    let base = &state.distribution.config.public_base_url;
    let slug = &a.slug;
    let catalog = state.distribution.published(&id, false).await?;
    let downloads: Vec<_> = crate::services::distribution::latest_installers(&catalog).into_iter().map(|(target, (r, files))| {
        json!({"target":target,"version":r.version.to_string(),"formats":files.iter().map(|a| &a.file_type).collect::<Vec<_>>(),"url":format!("{base}/{slug}/latest/{target}"),"format_urls":files.iter().map(|a|json!({"format":a.file_type,"url":format!("{base}/{slug}/latest/{target}?format={}",a.file_type)})).collect::<Vec<_>>()})
    }).collect();
    let artifact_downloads: Vec<_> = crate::services::distribution::latest_downloads(&catalog)
        .flat_map(|(kind, targets)| targets.into_iter().map(move |(target, (release, files))| {
            json!({"type":kind,"target":target,"version":release.version.to_string(),"files":files.iter().map(|a|json!({"format":a.file_type,"url":format!("{base}/{slug}/latest/{target}?type={kind}&format={}",a.file_type)})).collect::<Vec<_>>()})
        }))
        .collect();
    Ok(Json(
        json!({"public_available":crate::services::distribution::public_application(&a).is_ok(),"application_url":format!("{base}/{slug}"),"latest_url":format!("{base}/{slug}/latest"),"manifest_url":format!("{base}/{slug}/manifest.json"),"tauri_url":crate::services::tauri_integration::endpoint(base,slug)?,"downloads":downloads,"artifact_downloads":artifact_downloads}),
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
    let public = published && crate::services::distribution::public_application(&app).is_ok();
    let files=artifacts.iter().map(|a|json!({"id":a.id,"url":published.then(||state.distribution.artifact_url(&app.slug,&release,a)),"tauri_compatible":a.distribution_role=="updater" && crate::domain::publication::compatible_tauri_target(a).is_some(),"signature_present":a.updater_signature.is_some()})).collect::<Vec<_>>();
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
