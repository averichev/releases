use crate::services::error::Result;
use sqlx::{SqliteConnection, SqlitePool};
#[derive(sqlx::FromRow)]
pub struct PublicationEvent {
    pub id: i64,
    pub application_id: String,
    pub release_id: String,
    pub version: String,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub token_id: Option<String>,
    pub token_name: Option<String>,
    pub created_at: i64,
}
#[derive(Clone)]
pub struct PublicationHistoryRepository {
    pool: SqlitePool,
}
impl PublicationHistoryRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn list(
        &self,
        app: &str,
        limit: u32,
        before: Option<i64>,
    ) -> Result<Vec<PublicationEvent>> {
        Ok(sqlx::query_as("SELECT h.*,r.version,u.username,t.name AS token_name FROM publication_history h JOIN releases r ON r.id=h.release_id LEFT JOIN users u ON u.id=h.user_id LEFT JOIN api_tokens t ON t.id=h.token_id WHERE h.application_id=? AND (? IS NULL OR h.id<?) ORDER BY h.id DESC LIMIT ?")
            .bind(app).bind(before).bind(before).bind(limit).fetch_all(&self.pool).await?)
    }
    pub async fn append(
        c: &mut SqliteConnection,
        app: &str,
        release: &str,
        user: &str,
        now: i64,
    ) -> Result<()> {
        sqlx::query("INSERT INTO publication_history(application_id,release_id,user_id,created_at) VALUES(?,?,?,?)")
            .bind(app).bind(release).bind(user).bind(now).execute(c).await?;
        Ok(())
    }
    pub async fn append_token(
        c: &mut SqliteConnection,
        app: &str,
        release: &str,
        token: &str,
        now: i64,
    ) -> Result<()> {
        sqlx::query("INSERT INTO publication_history(application_id,release_id,token_id,created_at) VALUES(?,?,?,?)")
            .bind(app).bind(release).bind(token).bind(now).execute(c).await?;
        Ok(())
    }
}
