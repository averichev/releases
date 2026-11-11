#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Admin,
    Operator,
}
impl Role {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "admin" => Some(Self::Admin),
            "operator" => Some(Self::Operator),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Operator => "operator",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Invited,
    Active,
    Disabled,
}
impl Status {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "invited" => Some(Self::Invited),
            "active" => Some(Self::Active),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invited => "invited",
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Clone)]
pub struct User {
    pub id: String,
    pub username: String,
    pub role: Role,
    pub status: Status,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_login_at: Option<i64>,
}

pub fn username(value: &str) -> Option<String> {
    let value = value.trim();
    ((3..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
    .then(|| value.to_ascii_lowercase())
}

pub fn valid_password(value: &str) -> bool {
    (12..=128).contains(&value.chars().count()) && value.len() <= 512
}
