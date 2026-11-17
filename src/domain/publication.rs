use super::{artifact::Artifact, release::Release};

#[derive(Clone, Debug)]
pub struct ReleaseChannel {
    pub id: String,
    pub application_id: String,
    pub name: String,
    pub current_release_id: Option<String>,
    pub current_version: Option<String>,
    pub published_at: Option<i64>,
    pub release_count: i64,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Warnings,
    Errors,
}
#[derive(Clone, Debug)]
pub struct PublicationIssue {
    pub code: &'static str,
    pub artifact_id: Option<String>,
    pub message: String,
}
#[derive(Clone, Debug)]
pub struct PublicationValidation {
    pub state: Readiness,
    pub warnings: Vec<PublicationIssue>,
    pub errors: Vec<PublicationIssue>,
}
impl PublicationValidation {
    pub fn new(warnings: Vec<PublicationIssue>, errors: Vec<PublicationIssue>) -> Self {
        Self {
            state: if !errors.is_empty() {
                Readiness::Errors
            } else if !warnings.is_empty() {
                Readiness::Warnings
            } else {
                Readiness::Ready
            },
            warnings,
            errors,
        }
    }
}
/// Channel version is an explicit pointer; version ordering only describes the action.
pub fn switch_action(previous: Option<&Release>, next: &Release) -> &'static str {
    if previous.is_some_and(|r| next.version.cmp_precedence(&r.version).is_lt()) {
        "channel_rolled_back"
    } else {
        "channel_promoted"
    }
}
pub fn compatible_tauri_target(artifact: &Artifact) -> Option<&str> {
    let (os, arch) = artifact.target.split_once('-')?;
    if !matches!(arch, "x86_64" | "aarch64" | "i686" | "armv7") {
        return None;
    }
    let compatible = match os {
        "windows" => matches!(artifact.file_type.as_str(), "exe" | "msi"),
        "linux" => artifact.file_type == "appimage",
        "darwin" => artifact.file_type == "tar.gz",
        _ => false,
    };
    compatible.then_some(artifact.target.as_str())
}
pub fn tauri_target(value: &str) -> Option<String> {
    let (os, arch) = value.split_once('-')?;
    let os = match os {
        "windows" => "windows",
        "linux" => "linux",
        "darwin" | "macos" => "darwin",
        _ => return None,
    };
    let arch = match arch {
        "x86_64" | "x64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        "i686" | "armv7" => arch,
        _ => return None,
    };
    Some(format!("{os}-{arch}"))
}
