use crate::{
    domain::{
        api_token::ApiToken,
        application::{Application, Status},
    },
    infrastructure::secrets,
    repositories::{api_tokens::ApiTokensRepository, applications::ApplicationsRepository},
    services::{
        error::{Result, ServiceError},
        release::authorize,
        sessions::Identity,
    },
};

pub const TOKEN_PREFIX: &str = "vrp_";
#[derive(Clone)]
pub struct ApiTokens {
    repository: ApiTokensRepository,
    applications: ApplicationsRepository,
}
/// Only the indexed digest survives authentication; never retain a bearer secret.
pub struct Publisher {
    hash: String,
    pub token_id: String,
    pub application: Application,
    pub tauri_public_key: Option<String>,
}
pub struct IssuedApiToken {
    pub metadata: ApiToken,
    pub secret: String,
}
impl ApiTokens {
    pub fn new(repository: ApiTokensRepository, applications: ApplicationsRepository) -> Self {
        Self {
            repository,
            applications,
        }
    }
    pub async fn list(&self, actor: &Identity, app: &str) -> Result<Vec<ApiToken>> {
        let mut tx = self.applications.write().await?;
        authorize(&mut tx, actor).await?;
        ApplicationsRepository::get_in(&mut tx, app).await?;
        let list = ApiTokensRepository::list_in(&mut tx, app).await?;
        tx.commit().await?;
        Ok(list)
    }
    pub async fn create(
        &self,
        actor: &Identity,
        app: &str,
        name: &str,
        expires_at: Option<i64>,
    ) -> Result<IssuedApiToken> {
        let now = secrets::now();
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > 80
            || name.chars().any(char::is_control)
            || expires_at.is_some_and(|at| at <= now)
        {
            return Err(ServiceError::Invalid);
        }
        let secret = format!("{TOKEN_PREFIX}{}", secrets::random()?);
        let mut tx = self.applications.write().await?;
        let user = authorize(&mut tx, actor).await?;
        let application = ApplicationsRepository::get_in(&mut tx, app).await?;
        if application.status != Status::Active {
            return Err(ServiceError::ReleaseLocked);
        }
        let metadata = ApiToken {
            id: uuid::Uuid::new_v4().to_string(),
            application_id: app.into(),
            name: name.into(),
            created_at: now,
            created_by: user.id,
            expires_at,
            revoked_at: None,
            revoked_by: None,
        };
        ApiTokensRepository::insert(&mut tx, &metadata, &secrets::digest(&secret)).await?;
        tx.commit().await?;
        Ok(IssuedApiToken { metadata, secret })
    }
    pub async fn revoke(&self, actor: &Identity, app: &str, id: &str) -> Result<()> {
        let mut tx = self.applications.write().await?;
        let user = authorize(&mut tx, actor).await?;
        ApiTokensRepository::revoke(&mut tx, app, id, &user.id, secrets::now()).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn authenticate(&self, secret: &str, slug: &str) -> Result<Publisher> {
        if !secret
            .strip_prefix(TOKEN_PREFIX)
            .is_some_and(secrets::valid)
        {
            return Err(ServiceError::Unauthorized);
        }
        let hash = secrets::digest(secret);
        let token = self.repository.authenticate(&hash, secrets::now()).await?;
        // Resolve by the authenticated UUID: unknown and foreign slugs are identical.
        let application = self.applications.get(&token.application_id).await?;
        if application.slug != slug {
            return Err(ServiceError::Forbidden);
        }
        if application.status != Status::Active {
            return Err(ServiceError::ReleaseLocked);
        }
        let (tauri_public_key, _) = self.applications.tauri_settings(&application.id).await?;
        Ok(Publisher {
            hash,
            token_id: token.id,
            application,
            tauri_public_key,
        })
    }
    pub async fn reauthorize(
        c: &mut sqlx::SqliteConnection,
        publisher: &Publisher,
    ) -> Result<Application> {
        let token =
            ApiTokensRepository::authenticate_in(c, &publisher.hash, secrets::now()).await?;
        if token.application_id != publisher.application.id || token.id != publisher.token_id {
            return Err(ServiceError::Forbidden);
        }
        let application = ApplicationsRepository::get_in(c, &token.application_id).await?;
        if application.status != Status::Active {
            return Err(ServiceError::ReleaseLocked);
        }
        Ok(application)
    }
}
