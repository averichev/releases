/// Application identity is permanent; future releases reference `id`, never `slug`.
#[derive(Clone, Debug)]
pub struct Application {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub icon_key: Option<String>,
    pub visibility: Visibility,
    pub status: Status,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
}
impl Visibility {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "public" => Some(Self::Public),
            "private" => Some(Self::Private),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Active,
    Archived,
}
impl Status {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }
}

pub const RESERVED_SLUGS: &[&str] = &[
    "api", "admin", "assets", "auth", "login", "logout", "static", "health", "activate", "favicon",
    "index",
];
pub fn valid_slug(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !RESERVED_SLUGS.contains(&value)
}
pub fn name(value: &str) -> Option<String> {
    let value = value.trim();
    ((1..=120).contains(&value.chars().count()) && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}
pub fn description(value: &str) -> Option<String> {
    let value = value.trim();
    (value.chars().count() <= 2000
        && !value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t'))
    .then(|| value.to_owned())
}

#[derive(Clone, Copy)]
pub enum Sort {
    Name,
    CreatedAt,
    UpdatedAt,
}
#[derive(Clone)]
pub struct ListQuery {
    pub page: u32,
    pub per_page: u32,
    pub search: String,
    pub status: Option<Status>,
    pub visibility: Option<Visibility>,
    pub sort: Sort,
    pub descending: bool,
}
pub struct ApplicationPage {
    pub items: Vec<Application>,
    pub total: i64,
}
pub struct NewApplication {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub visibility: Visibility,
}
#[derive(Default)]
pub struct ApplicationChanges {
    pub name: Option<String>,
    pub description: Option<String>,
    pub visibility: Option<Visibility>,
}

/// Mutable application preference; published artifacts remain immutable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistributionSelector {
    pub target: String,
    pub format: String,
    pub purpose: String,
}
impl DistributionSelector {
    pub fn valid(&self) -> bool {
        super::artifact::TARGETS.contains(&self.target.as_str())
            && super::artifact::valid_file_type(&self.format)
            && self.purpose == "installer"
    }
    pub fn matches(&self, artifact: &super::artifact::Artifact) -> bool {
        artifact.target == self.target
            && artifact.file_type == self.format
            && artifact.distribution_role == self.purpose
    }
}
