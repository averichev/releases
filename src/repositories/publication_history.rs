use crate::services::error::Result;
use sqlx::{SqliteConnection, SqlitePool};
#[derive(sqlx::FromRow)]
pub struct PublicationEvent {
    pub id: i64,
    pub application_id: String,
    pub release_id: String,
    pub channel: String,
    pub action: String,
    pub previous_release_id: Option<String>,
    pub previous_version: Option<String>,
    pub new_release_id: String,
    pub new_version: String,
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
        Ok(sqlx::query_as("SELECT h.*,c.name AS channel,p.version AS previous_version,n.version AS new_version,u.username FROM publication_history h JOIN release_channels c ON c.id=h.channel_id LEFT JOIN releases p ON p.id=h.previous_release_id JOIN releases n ON n.id=h.new_release_id JOIN users u ON u.id=h.user_id WHERE h.application_id=? AND (? IS NULL OR h.id<?) ORDER BY h.id DESC LIMIT ?").bind(app).bind(before).bind(before).bind(limit).fetch_all(&self.pool).await?)
    }
    pub async fn append(
        c: &mut SqliteConnection,
        channel: &crate::domain::publication::ReleaseChannel,
        release: &str,
        action: &str,
        user: &str,
        now: i64,
    ) -> Result<()> {
        sqlx::query("INSERT INTO publication_history(application_id,channel_id,release_id,action,previous_release_id,new_release_id,user_id,created_at) VALUES(?,?,?,?,?,?,?,?)").bind(&channel.application_id).bind(&channel.id).bind(release).bind(action).bind(&channel.current_release_id).bind(release).bind(user).bind(now).execute(c).await?;
        Ok(())
    }
}
