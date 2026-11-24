use crate::{
    domain::api_token::ApiToken,
    services::error::{Result, ServiceError},
};
use sqlx::{SqliteConnection, SqlitePool};

const SELECT: &str = "SELECT id,application_id,name,created_at,created_by,expires_at,revoked_at,revoked_by FROM api_tokens";
#[derive(sqlx::FromRow)]
struct StoredToken {
    id: String,
    application_id: String,
    name: String,
    created_at: i64,
    created_by: String,
    expires_at: Option<i64>,
    revoked_at: Option<i64>,
    revoked_by: Option<String>,
}
impl From<StoredToken> for ApiToken {
    fn from(t: StoredToken) -> Self {
        Self {
            id: t.id,
            application_id: t.application_id,
            name: t.name,
            created_at: t.created_at,
            created_by: t.created_by,
            expires_at: t.expires_at,
            revoked_at: t.revoked_at,
            revoked_by: t.revoked_by,
        }
    }
}
#[derive(Clone)]
pub struct ApiTokensRepository {
    pool: SqlitePool,
}
impl ApiTokensRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn list_in(c: &mut SqliteConnection, app: &str) -> Result<Vec<ApiToken>> {
        Ok(sqlx::query_as::<_, StoredToken>(&format!(
            "{SELECT} WHERE application_id=? ORDER BY created_at DESC,id"
        ))
        .bind(app)
        .fetch_all(c)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
    }
    pub async fn insert(c: &mut SqliteConnection, t: &ApiToken, hash: &str) -> Result<()> {
        sqlx::query("INSERT INTO api_tokens(id,application_id,token_hash,name,created_at,created_by,expires_at) VALUES(?,?,?,?,?,?,?)")
            .bind(&t.id).bind(&t.application_id).bind(hash).bind(&t.name).bind(t.created_at)
            .bind(&t.created_by).bind(t.expires_at).execute(c).await?;
        Ok(())
    }
    pub async fn revoke(
        c: &mut SqliteConnection,
        app: &str,
        id: &str,
        user: &str,
        now: i64,
    ) -> Result<()> {
        let found: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM api_tokens WHERE application_id=? AND id=?)",
        )
        .bind(app)
        .bind(id)
        .fetch_one(&mut *c)
        .await?;
        if !found {
            return Err(ServiceError::NotFound);
        }
        sqlx::query("UPDATE api_tokens SET revoked_at=max(created_at,?),revoked_by=? WHERE application_id=? AND id=? AND revoked_at IS NULL")
            .bind(now).bind(user).bind(app).bind(id).execute(c).await?;
        Ok(())
    }
    pub async fn authenticate(&self, hash: &str, now: i64) -> Result<ApiToken> {
        let mut c = self.pool.acquire().await?;
        Self::authenticate_in(&mut c, hash, now).await
    }
    pub async fn authenticate_in(
        c: &mut SqliteConnection,
        hash: &str,
        now: i64,
    ) -> Result<ApiToken> {
        Ok(sqlx::query_as::<_, StoredToken>(&format!("{SELECT} WHERE token_hash=? AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>?)"))
            .bind(hash).bind(now).fetch_optional(c).await?.ok_or(ServiceError::Unauthorized)?.into())
    }
}
