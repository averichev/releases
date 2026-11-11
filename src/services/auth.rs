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
pub struct Authentication {
    users: UsersRepository,
    sessions: SessionsRepository,
    passwords: Passwords,
    config: AuthConfig,
}
pub struct Login {
    pub user: User,
    pub token: String,
}
impl Authentication {
    pub fn new(
        users: UsersRepository,
        sessions: SessionsRepository,
        passwords: Passwords,
        config: AuthConfig,
    ) -> Self {
        Self {
            users,
            sessions,
            passwords,
            config,
        }
    }
    pub async fn admin_exists(&self) -> Result<bool> {
        self.users.admin_exists().await
    }
    pub async fn bootstrap(&self, username: &str, password: String) -> Result<bool> {
        if self.admin_exists().await? {
            return Ok(false);
        }
        let username = user::username(username).ok_or(ServiceError::Invalid)?;
        let hash = self.passwords.hash(password).await?;
        let mut tx = self.users.write().await?;
        // Serialize both the initial check and insert across CLI processes.
        let exists = UsersRepository::admin_exists_in(&mut tx).await?;
        if exists {
            return Ok(false);
        }
        let now = secrets::now();
        UsersRepository::insert(
            &mut tx,
            &User {
                id: uuid::Uuid::new_v4().to_string(),
                username,
                role: Role::Admin,
                status: Status::Active,
                created_at: now,
                updated_at: now,
                last_login_at: None,
            },
            Some(&hash),
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }
    pub async fn login(
        &self,
        username: &str,
        password: String,
        previous_hash: Option<String>,
    ) -> Result<Login> {
        let username = user::username(username).ok_or(ServiceError::Credentials)?;
        let bucket = secrets::digest(&username);
        self.sessions.reserve_login(&bucket, secrets::now()).await?;
        let stored = self.users.by_username(&username).await?;
        let hash = stored.as_ref().and_then(|u| u.password_hash.clone());
        let valid = self.passwords.verify(password, hash.clone()).await?;
        let stored = stored.filter(|_| valid).ok_or(ServiceError::Credentials)?;
        if stored.user()?.status != Status::Active {
            return Err(ServiceError::Credentials);
        }
        let token = secrets::random()?;
        let mut tx = self.users.write().await?;
        let current = UsersRepository::by_id(&mut tx, &stored.id).await?;
        if current.password_hash != hash || current.user()?.status != Status::Active {
            return Err(ServiceError::Credentials);
        }
        let now = secrets::now();
        if let Some(previous) = previous_hash {
            // Always replace a presented session instead of adopting its identifier.
            SessionsRepository::delete_one(&mut tx, &previous).await?;
        }
        SessionsRepository::insert(
            &mut tx,
            &secrets::digest(&token),
            &stored.id,
            now,
            now + self.config.session_ttl_seconds,
        )
        .await?;
        SessionsRepository::successful_login(&mut tx, &bucket).await?;
        tx.commit().await?;
        let mut user = current.user()?;
        user.last_login_at = Some(now);
        Ok(Login { user, token })
    }
    pub async fn change_password(
        &self,
        identity: &Identity,
        current_password: String,
        new_password: String,
    ) -> Result<()> {
        let stored = self
            .users
            .by_username(&identity.user.username)
            .await?
            .ok_or(ServiceError::Unauthorized)?;
        if !self
            .passwords
            .verify(current_password, stored.password_hash.clone())
            .await?
        {
            return Err(ServiceError::Credentials);
        }
        let hash = self.passwords.hash(new_password).await?;
        let mut tx = self.users.write().await?;
        SessionsRepository::authorize(&mut tx, &identity.session_hash, secrets::now()).await?;
        if UsersRepository::by_id(&mut tx, &identity.user.id)
            .await?
            .password_hash
            != stored.password_hash
        {
            return Err(ServiceError::Unauthorized);
        }
        UsersRepository::set_password(&mut tx, &identity.user.id, &hash, secrets::now()).await?;
        SessionsRepository::revoke(&mut tx, &identity.user.id).await?;
        UsersRepository::clear_tokens(&mut tx, &identity.user.id).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn inspect_token(&self, token: &str) -> Result<(String, String)> {
        if !secrets::valid(token) {
            return Err(ServiceError::Token);
        }
        let (_, username, purpose) = self
            .users
            .token(&secrets::digest(token), secrets::now())
            .await?
            .ok_or(ServiceError::Token)?;
        Ok((username, purpose))
    }
    pub async fn activate(&self, token: &str, password: String) -> Result<()> {
        self.inspect_token(token).await?;
        let hash = self.passwords.hash(password).await?;
        let mut tx = self.users.write().await?;
        let id = UsersRepository::consume_token(&mut tx, &secrets::digest(token), secrets::now())
            .await?;
        UsersRepository::set_password(&mut tx, &id, &hash, secrets::now()).await?;
        SessionsRepository::revoke(&mut tx, &id).await?;
        tx.commit().await?;
        Ok(())
    }
}
