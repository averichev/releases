use semver::{BuildMetadata, Version};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseStatus {
    Draft,
    Published,
}
impl ReleaseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "published" => Some(Self::Published),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub id: String,
    pub application_id: String,
    pub version: Version,
    pub title: Option<String>,
    pub release_notes: String,
    pub status: ReleaseStatus,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub published_at: Option<i64>,
    pub artifact_count: i64,
    pub total_size_bytes: i64,
}
pub struct ReleaseInput {
    pub version: String,
    pub title: Option<String>,
    pub release_notes: String,
}
#[derive(Default)]
pub struct ReleaseChanges {
    pub version: Option<String>,
    pub title: Option<Option<String>>,
    pub release_notes: Option<String>,
}
pub struct ReleaseQuery {
    pub page: u32,
    pub per_page: u32,
    pub search: String,
    pub status: Option<ReleaseStatus>,
    pub sort: ReleaseSort,
    pub descending: bool,
}
pub enum ReleaseSort {
    Version,
    CreatedAt,
    UpdatedAt,
}
pub struct ReleasePage {
    pub items: Vec<Release>,
    pub total: usize,
}

pub fn version(value: &str) -> Option<Version> {
    let value = value.trim();
    let value = value.strip_prefix('v').unwrap_or(value);
    (value.len() <= 200)
        .then(|| Version::parse(value).ok())
        .flatten()
}
/// Build metadata is retained for display, but does not create another release
/// with identical SemVer precedence within an application.
pub fn version_key(version: &Version) -> String {
    let mut key = version.clone();
    key.build = BuildMetadata::EMPTY;
    key.to_string()
}
pub fn title(value: Option<&str>) -> Option<Option<String>> {
    let value = value.unwrap_or("").trim();
    if value.chars().count() > 120 || value.chars().any(char::is_control) {
        return None;
    }
    Some((!value.is_empty()).then(|| value.to_owned()))
}
pub fn notes(value: &str) -> Option<String> {
    (value.chars().count() <= 20000
        && !value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')))
    .then(|| value.trim().to_owned())
}
