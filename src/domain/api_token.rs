/// Public token metadata. Credentials and hashes never belong in this type.
#[derive(Clone)]
pub struct ApiToken {
    pub id: String,
    pub application_id: String,
    pub name: String,
    pub created_at: i64,
    pub created_by: String,
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub revoked_by: Option<String>,
}
impl ApiToken {
    pub fn state(&self, now: i64) -> &'static str {
        if self.revoked_at.is_some() {
            "revoked"
        } else if self.expires_at.is_some_and(|at| at <= now) {
            "expired"
        } else {
            "active"
        }
    }
}
