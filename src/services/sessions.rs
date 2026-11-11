use crate::{
    domain::user::User,
    infrastructure::secrets,
    repositories::sessions::SessionsRepository,
    services::error::{Result, ServiceError},
};
#[derive(Clone)]
pub struct Sessions {
    pub(crate) repository: SessionsRepository,
}
#[derive(Clone)]
pub struct Identity {
    pub user: User,
    pub session_hash: String,
}
impl Sessions {
    pub fn new(repository: SessionsRepository) -> Self {
        Self { repository }
    }
    pub async fn authenticate(&self, token: &str) -> Result<Identity> {
        if !secrets::valid(token) {
            return Err(ServiceError::Unauthorized);
        }
        let session_hash = secrets::digest(token);
        let user = self.repository.find(&session_hash, secrets::now()).await?;
        Ok(Identity { user, session_hash })
    }
    pub async fn logout(&self, identity: &Identity) -> Result<()> {
        self.repository.logout(&identity.session_hash).await
    }
}
