use crate::{
    config::AuthConfig,
    domain::user::{self, Role, Status, User},
    infrastructure::{passwords::Passwords, secrets},
    repositories::{sessions::SessionsRepository, users::UsersRepository},
    services::{
        error::{Result, ServiceError},
        sessions::Identity,
    },
};
#[derive(Clone)]
pub struct UserManagement {
    repository: UsersRepository,
    passwords: Passwords,
    config: AuthConfig,
}
pub struct IssuedToken {
    pub token: String,
    pub expires_at: i64,
}
impl UserManagement {
    pub fn new(repository: UsersRepository, passwords: Passwords, config: AuthConfig) -> Self {
        Self {
            repository,
            passwords,
            config,
        }
    }
    async fn authorize(c: &mut sqlx::SqliteConnection, actor: &Identity) -> Result<()> {
        let user = SessionsRepository::authorize(c, &actor.session_hash, secrets::now()).await?;
        if user.role != Role::Admin {
            return Err(ServiceError::Forbidden);
        }
        Ok(())
    }
    pub async fn list(&self, actor: &Identity) -> Result<Vec<User>> {
        if actor.user.role != Role::Admin {
            return Err(ServiceError::Forbidden);
        }
        self.repository.list().await
    }
    pub async fn create(
        &self,
        actor: &Identity,
        username: &str,
        role: Role,
        password: Option<String>,
    ) -> Result<(User, Option<IssuedToken>)> {
        let username = user::username(username).ok_or(ServiceError::Invalid)?;
        let hash = match password {
            Some(password) => Some(self.passwords.hash(password).await?),
            None => None,
        };
        let now = secrets::now();
        let user = User {
            id: uuid::Uuid::new_v4().to_string(),
            username,
            role,
            status: if hash.is_some() {
                Status::Active
            } else {
                Status::Invited
            },
            created_at: now,
            updated_at: now,
            last_login_at: None,
        };
        let mut tx = self.repository.write().await?;
        Self::authorize(&mut tx, actor).await?;
        UsersRepository::insert(&mut tx, &user, hash.as_deref()).await?;
        let token = if hash.is_none() {
            Some(self.issue(&mut tx, &user.id, "invitation").await?)
        } else {
            None
        };
        tx.commit().await?;
        Ok((user, token))
    }
    async fn issue(
        &self,
        c: &mut sqlx::SqliteConnection,
        id: &str,
        purpose: &str,
    ) -> Result<IssuedToken> {
        let token = secrets::random()?;
        let expires_at = secrets::now() + self.config.invitation_ttl_seconds;
        UsersRepository::put_token(c, id, &secrets::digest(&token), purpose, expires_at).await?;
        Ok(IssuedToken { token, expires_at })
    }
    /// Local owner recovery only. Never expose this entry point through HTTP.
    pub async fn recover_administrator_from_console(&self, username: &str) -> Result<IssuedToken> {
        let username = user::username(username).ok_or(ServiceError::Invalid)?;
        let mut tx = self.repository.write().await?;
        let user = UsersRepository::by_username_in(&mut tx, &username)
            .await?
            .user()?;
        if user.role != Role::Admin || user.status != Status::Active {
            return Err(ServiceError::Invalid);
        }
        let token = self.issue(&mut tx, &user.id, "reset").await?;
        tx.commit().await?;
        Ok(token)
    }
    pub async fn reissue(&self, actor: &Identity, id: &str, reset: bool) -> Result<IssuedToken> {
        let mut tx = self.repository.write().await?;
        Self::authorize(&mut tx, actor).await?;
        let user = UsersRepository::by_id(&mut tx, id).await?.user()?;
        if user.status
            != if reset {
                Status::Active
            } else {
                Status::Invited
            }
        {
            return Err(ServiceError::Invalid);
        }
        let token = self
            .issue(&mut tx, id, if reset { "reset" } else { "invitation" })
            .await?;
        tx.commit().await?;
        Ok(token)
    }
    pub async fn access(
        &self,
        actor: &Identity,
        id: &str,
        role: Option<Role>,
        status: Option<Status>,
    ) -> Result<()> {
        let mut tx = self.repository.write().await?;
        Self::authorize(&mut tx, actor).await?;
        let stored = UsersRepository::by_id(&mut tx, id).await?;
        let user = stored.user()?;
        let requested = status.unwrap_or(user.status);
        let status = if requested == Status::Active && stored.password_hash.is_none() {
            if user.status != Status::Disabled {
                return Err(ServiceError::Invalid);
            }
            Status::Invited
        } else {
            requested
        };
        UsersRepository::set_access(
            &mut tx,
            id,
            role.unwrap_or(user.role),
            status,
            secrets::now(),
        )
        .await?;
        SessionsRepository::revoke(&mut tx, id).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn revoke(&self, actor: &Identity, id: &str) -> Result<()> {
        let mut tx = self.repository.write().await?;
        Self::authorize(&mut tx, actor).await?;
        UsersRepository::by_id(&mut tx, id).await?;
        SessionsRepository::revoke(&mut tx, id).await?;
        tx.commit().await?;
        Ok(())
    }
}
