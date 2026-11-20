use crate::services::error::Result;
use sqlx::{SqliteConnection, SqlitePool};
#[derive(sqlx::FromRow)]
pub struct PublicationEvent {
    pub id: i64,
    pub application_id: String,
    pub release_id: String,
    pub version: String,
    pub user_id: String,
    pub username: String,
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
        Ok(sqlx::query_as("SELECT h.*,r.version,u.username FROM publication_history h JOIN releases r ON r.id=h.release_id JOIN users u ON u.id=h.user_id WHERE h.application_id=? AND (? IS NULL OR h.id<?) ORDER BY h.id DESC LIMIT ?")
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
}
