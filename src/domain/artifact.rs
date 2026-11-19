/// Extend this list in one place; the administrative UI obtains it from the API.
pub const TARGETS: &[&str] = &[
    "windows-x86_64",
    "windows-aarch64",
    "linux-x86_64",
    "linux-aarch64",
    "darwin-x86_64",
    "darwin-aarch64",
    "darwin-universal",
    "any",
];
pub const FILE_TYPES: &[&str] = &[
    "exe", "msi", "bin", "zip", "tar.gz", "dmg", "appimage", "deb", "rpm",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    pub id: String,
    pub release_id: String,
    pub filename: String,
    pub target: String,
    pub file_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_key: String,
    pub created_at: i64,
    pub distribution_role: String,
    pub updater_signature: Option<String>,
}
pub struct UploadMetadata {
    pub filename: String,
    pub target: String,
    pub file_type: Option<String>,
}
pub fn valid_filename(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 255
        && !matches!(value, "." | "..")
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
}
pub fn valid_file_type(value: &str) -> bool {
    (1..=32).contains(&value.len())
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
        && !value.ends_with('.')
        && !value.contains("..")
}
/// Format is a server-normalized metadata token, not a malware/content verdict.
/// Unknown extensions remain supported. Extensionless files need an explicit type.
pub fn file_type(filename: &str, explicit: Option<&str>) -> Option<String> {
    let lower = filename.to_ascii_lowercase();
    let detected = ["tar.gz", "tar.xz", "tar.bz2", "tar.zst"]
        .iter()
        .find(|ext| lower.ends_with(&format!(".{ext}")))
        .map(|v| (*v).to_owned())
        .or_else(|| {
            lower
                .rsplit_once('.')
                .filter(|(base, _)| !base.is_empty())
                .map(|(_, ext)| ext.to_owned())
        });
    let explicit = explicit.map(|v| v.trim().to_ascii_lowercase());
    if let (Some(found), Some(given)) = (&detected, &explicit)
        && found != given
    {
        return None;
    }
    let value = explicit.or(detected)?;
    valid_file_type(&value).then_some(value)
}
