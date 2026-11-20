use crate::{
    domain::{
        artifact::Artifact,
        publication::{compatible_tauri_target, tauri_target},
        release::{self, Release},
    },
    services::{
        distribution::{PublicDistribution, public_application},
        error::{Result, ServiceError},
        manifest::rfc3339,
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
pub const MAX_SIGNATURE_BYTES: usize = 16 * 1024;
/// Accept the actual Tauri .sig (base64-encoded minisign document), never keys,
/// filenames, URLs or a SHA-256 substituted for a digital signature.
pub fn signature(bytes: &[u8]) -> Result<String> {
    if bytes.is_empty() || bytes.len() > MAX_SIGNATURE_BYTES {
        return Err(ServiceError::InvalidUpdater);
    }
    let value = std::str::from_utf8(bytes)
        .map_err(|_| ServiceError::InvalidUpdater)?
        .trim();
    let decoded = STANDARD
        .decode(value)
        .map_err(|_| ServiceError::InvalidUpdater)?;
    let text = std::str::from_utf8(&decoded).map_err(|_| ServiceError::InvalidUpdater)?;
    let lines: Vec<_> = text.lines().collect();
    if lines.len() != 4
        || !lines[0].starts_with("untrusted comment: ")
        || !lines[2].starts_with("trusted comment: ")
    {
        return Err(ServiceError::InvalidUpdater);
    }
    let packet = STANDARD
        .decode(lines[1])
        .map_err(|_| ServiceError::InvalidUpdater)?;
    let global = STANDARD
        .decode(lines[3])
        .map_err(|_| ServiceError::InvalidUpdater)?;
    if packet.len() != 74 || !matches!(&packet[..2], b"Ed" | b"ED") || global.len() != 64 {
        return Err(ServiceError::InvalidUpdater);
    }
    Ok(value.to_owned())
}
pub struct UpdateSelection {
    pub release: Release,
    pub artifact: Artifact,
}
pub enum UpdateDecision {
    Available(Box<UpdateSelection>),
    NoUpdate,
    MissingPackage,
    MissingSignature,
    AmbiguousPackage,
}
pub fn select(
    release: Release,
    artifacts: Vec<Artifact>,
    target: &str,
    client_version: &str,
) -> Result<UpdateDecision> {
    if release.status != crate::domain::release::ReleaseStatus::Published {
        return Err(ServiceError::ReleaseLocked);
    }
    let current = release::version(client_version).ok_or(ServiceError::InvalidRelease)?;
    let target = tauri_target(target).ok_or(ServiceError::InvalidUpdater)?;
    if !release.version.cmp_precedence(&current).is_gt() {
        return Ok(UpdateDecision::NoUpdate);
    }
    // Prefer the exact architecture. Universal is a fallback for modern Macs only;
    // a broken or ambiguous exact package must still block the update.
    let compatible: Vec<_> = artifacts
        .into_iter()
        .filter(|a| a.distribution_role == "updater" && compatible_tauri_target(a).is_some())
        .collect();
    let selected_target = if matches!(target.as_str(), "darwin-x86_64" | "darwin-aarch64")
        && !compatible.iter().any(|a| a.target == target)
    {
        "darwin-universal"
    } else {
        target.as_str()
    };
    let mut candidates = compatible
        .into_iter()
        .filter(|a| a.target == selected_target);
    let Some(artifact) = candidates.next() else {
        return Ok(UpdateDecision::MissingPackage);
    };
    if candidates.next().is_some() {
        return Ok(UpdateDecision::AmbiguousPackage);
    }
    if artifact
        .updater_signature
        .as_deref()
        .is_none_or(|s| signature(s.as_bytes()).is_err())
    {
        return Ok(UpdateDecision::MissingSignature);
    }
    Ok(UpdateDecision::Available(Box::new(UpdateSelection {
        release,
        artifact,
    })))
}
/// Published catalog is ordered once by SemVer in the distribution repository.
/// Skip releases without a usable signed updater; installers never participate.
pub fn select_latest(
    catalog: &crate::repositories::distribution::PublishedCatalog,
    target: &str,
    version: &str,
) -> Result<UpdateDecision> {
    release::version(version).ok_or(ServiceError::InvalidRelease)?;
    tauri_target(target).ok_or(ServiceError::InvalidUpdater)?;
    let mut missing = UpdateDecision::MissingPackage;
    for (r, artifacts) in &catalog.releases {
        if !r.version.pre.is_empty() {
            continue;
        }
        match select(r.clone(), artifacts.clone(), target, version)? {
            available @ UpdateDecision::Available(_) => return Ok(available),
            UpdateDecision::NoUpdate => return Ok(UpdateDecision::NoUpdate),
            UpdateDecision::MissingPackage => (),
            other => missing = other,
        }
    }
    Ok(missing)
}
#[derive(Serialize)]
pub struct TauriResponse {
    pub version: String,
    pub pub_date: String,
    pub url: String,
    pub signature: String,
    pub notes: String,
}
/// The distribution adapter supplies the permanent path for this exact artifact.
/// The origin must be resolved and validated at the HTTP/configuration boundary.
pub fn response(
    selection: &UpdateSelection,
    public_origin: &str,
    download_path: &str,
) -> Result<TauriResponse> {
    if selection.release.status != crate::domain::release::ReleaseStatus::Published
        || selection.artifact.release_id != selection.release.id
        || selection.artifact.distribution_role != "updater"
        || compatible_tauri_target(&selection.artifact).is_none()
    {
        return Err(ServiceError::InvalidUpdater);
    }
    let origin = url::Url::parse(public_origin).map_err(|_| ServiceError::InvalidUpdater)?;
    if !(origin.scheme() == "https"
        || (origin.scheme() == "http"
            && matches!(origin.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
        || origin.path() != "/"
        || !download_path.starts_with('/')
        || download_path.starts_with("//")
    {
        return Err(ServiceError::InvalidUpdater);
    }
    let url = origin
        .join(download_path)
        .map_err(|_| ServiceError::InvalidUpdater)?;
    if url.origin() != origin.origin()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url
            .path_segments()
            .is_some_and(|mut p| p.any(|part| part == selection.artifact.id))
    {
        return Err(ServiceError::InvalidUpdater);
    }
    let signature = signature(
        selection
            .artifact
            .updater_signature
            .as_deref()
            .ok_or(ServiceError::InvalidUpdater)?
            .as_bytes(),
    )?;
    Ok(TauriResponse {
        version: selection.release.version.to_string(),
        pub_date: rfc3339(
            selection
                .release
                .published_at
                .ok_or(ServiceError::InvalidUpdater)?,
        )?,
        url: url.to_string(),
        signature,
        notes: selection.release.release_notes.clone(),
    })
}
#[derive(Serialize)]
pub struct TauriPreview {
    pub state: &'static str,
    pub reason: &'static str,
    pub message: &'static str,
    pub selected_version: Option<String>,
    pub artifact_id: Option<String>,
    pub artifact_filename: Option<String>,
    pub signature_present: bool,
    pub public_downloads_available: bool,
    pub response: Option<TauriResponse>,
}
#[derive(Clone)]
pub struct TauriUpdater {
    distribution: PublicDistribution,
}
impl TauriUpdater {
    pub fn new(distribution: PublicDistribution) -> Self {
        Self { distribution }
    }
    pub async fn update(
        &self,
        slug: &str,
        target: &str,
        version: &str,
    ) -> Result<Option<TauriResponse>> {
        release::version(version).ok_or(ServiceError::InvalidRelease)?;
        tauri_target(target).ok_or(ServiceError::InvalidUpdater)?;
        let catalog = self.distribution.published(slug, true).await?;
        match select_latest(&catalog, target, version)? {
            UpdateDecision::NoUpdate | UpdateDecision::MissingPackage => Ok(None),
            UpdateDecision::MissingSignature | UpdateDecision::AmbiguousPackage => {
                Err(ServiceError::UpdaterUnavailable)
            }
            UpdateDecision::Available(selection) => {
                self.distribution
                    .check_artifact(&selection.artifact)
                    .await?;
                self.distribution.recheck(&catalog.application.id).await?;
                Ok(Some(response(
                    &selection,
                    &self.distribution.config.public_base_url,
                    &self
                        .distribution
                        .artifact_path(slug, &selection.release, &selection.artifact),
                )?))
            }
        }
    }
    pub async fn preview(&self, app: &str, target: &str, version: &str) -> Result<TauriPreview> {
        release::version(version).ok_or(ServiceError::InvalidRelease)?;
        tauri_target(target).ok_or(ServiceError::InvalidUpdater)?;
        let catalog = self.distribution.published(app, false).await?;
        let app = &catalog.application;
        let mut p = TauriPreview {
            state: "no_update",
            reason: "no_published_releases",
            message: "Нет опубликованных релизов.",
            selected_version: None,
            artifact_id: None,
            artifact_filename: None,
            signature_present: false,
            public_downloads_available: public_application(app).is_ok(),
            response: None,
        };
        if !p.public_downloads_available {
            p.state = "blocked";
            p.reason = "application_not_public";
            p.message = "Приложение приватное или архивировано. Публичный updater недоступен.";
            return Ok(p);
        }
        if catalog.releases.is_empty() {
            return Ok(p);
        }
        match select_latest(&catalog, target, version)? {
            UpdateDecision::NoUpdate => {
                p.reason = "version_not_newer";
                p.message =
                    "Опубликованная версия не новее версии клиента. Downgrade не выполняется.";
            }
            UpdateDecision::MissingPackage => {
                p.reason = "updater_package_missing";
                p.message = "Для платформы нет совместимого updater-пакета.";
            }
            UpdateDecision::MissingSignature => {
                p.state = "blocked";
                p.reason = "signature_missing";
                p.message = "У updater-пакета нет корректной подписи .sig.";
            }
            UpdateDecision::AmbiguousPackage => {
                p.state = "blocked";
                p.reason = "ambiguous_package";
                p.message = "Платформе соответствует несколько updater-пакетов.";
            }
            UpdateDecision::Available(selection) => {
                p.selected_version = Some(selection.release.version.to_string());
                p.artifact_id = Some(selection.artifact.id.clone());
                p.artifact_filename = Some(selection.artifact.filename.clone());
                p.signature_present = true;
                match self.distribution.check_artifact(&selection.artifact).await {
                    Ok(()) => {
                        p.state = "available";
                        p.reason = "update_available";
                        p.message = "Подписанное обновление доступно через публичный endpoint.";
                        p.response = Some(response(
                            &selection,
                            &self.distribution.config.public_base_url,
                            &self.distribution.artifact_path(
                                &app.slug,
                                &selection.release,
                                &selection.artifact,
                            ),
                        )?);
                    }
                    Err(ServiceError::StorageUnavailable) => {
                        p.state = "blocked";
                        p.reason = "artifact_corrupted";
                        p.message =
                            "Updater-пакет отсутствует или размер не совпадает. Восстановите файл.";
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(p)
    }
}
