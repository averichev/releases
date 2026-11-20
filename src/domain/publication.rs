use super::artifact::Artifact;

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
pub fn compatible_tauri_target(artifact: &Artifact) -> Option<&str> {
    compatible_tauri_format(&artifact.target, &artifact.file_type)
}
pub fn compatible_tauri_format<'a>(target: &'a str, file_type: &str) -> Option<&'a str> {
    let (os, arch) = target.split_once('-')?;
    if !(matches!(arch, "x86_64" | "aarch64" | "i686" | "armv7")
        || os == "darwin" && arch == "universal")
    {
        return None;
    }
    let compatible = match os {
        "windows" => matches!(file_type, "exe" | "msi"),
        "linux" => file_type == "appimage",
        "darwin" => file_type == "tar.gz",
        _ => false,
    };
    compatible.then_some(target)
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
        "universal" if os == "darwin" => arch,
        _ => return None,
    };
    Some(format!("{os}-{arch}"))
}
