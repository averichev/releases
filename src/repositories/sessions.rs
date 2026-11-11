use crate::{
    domain::user::User,
    repositories::users::StoredUser,
    services::error::{Result, ServiceError},
};
use sqlx::{SqliteConnection, SqlitePool};
#[derive(Clone)]
pub struct SessionsRepository {
    pool: SqlitePool,
}
impl SessionsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn find(&self, hash: &str, now: i64) -> Result<User> {
        let row: Option<StoredUser> = sqlx::query_as("SELECT u.* FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=? AND s.expires_at>? AND u.status='active'")
            .bind(hash).bind(now).fetch_optional(&self.pool).await?;
        row.ok_or(ServiceError::Unauthorized)?.user()
    }
    pub async fn authorize(c: &mut SqliteConnection, hash: &str, now: i64) -> Result<User> {
        let row: Option<StoredUser> = sqlx::query_as("SELECT u.* FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=? AND s.expires_at>? AND u.status='active'")
            .bind(hash).bind(now).fetch_optional(c).await?;
        row.ok_or(ServiceError::Unauthorized)?.user()
    }
    pub async fn insert(
        c: &mut SqliteConnection,
        hash: &str,
        id: &str,
        now: i64,
        expires: i64,
    ) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE expires_at<=?")
            .bind(now)
            .execute(&mut *c)
            .await?;
        // Bound persistent session growth, retaining the 19 most recent sessions.
        sqlx::query("DELETE FROM sessions WHERE user_id=? AND token_hash NOT IN (SELECT token_hash FROM sessions WHERE user_id=? ORDER BY created_at DESC,token_hash DESC LIMIT 19)")
            .bind(id).bind(id).execute(&mut *c).await?;
        sqlx::query(
            "INSERT INTO sessions(token_hash,user_id,created_at,expires_at) VALUES(?,?,?,?)",
        )
        .bind(hash)
        .bind(id)
        .bind(now)
        .bind(expires)
        .execute(&mut *c)
        .await?;
        sqlx::query("UPDATE users SET last_login_at=? WHERE id=?")
            .bind(now)
            .bind(id)
            .execute(c)
            .await?;
        Ok(())
    }
    pub async fn revoke(c: &mut SqliteConnection, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE user_id=?")
            .bind(id)
            .execute(c)
            .await?;
        Ok(())
    }
    pub async fn delete_one(c: &mut SqliteConnection, hash: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash=?")
            .bind(hash)
            .execute(c)
            .await?;
        Ok(())
    }
    pub async fn logout(&self, hash: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash=?")
            .bind(hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn reserve_login(&self, username_hash: &str, now: i64) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM login_attempts WHERE expires_at<=?")
            .bind(now)
            .execute(&mut *tx)
            .await?;
        for (bucket, limit) in [("global", 100), (username_hash, 5)] {
            let count: i64 = sqlx::query_scalar(
                "SELECT COALESCE((SELECT attempts FROM login_attempts WHERE bucket=?),0)",
            )
            .bind(bucket)
            .fetch_one(&mut *tx)
            .await?;
            if count >= limit {
                return Err(ServiceError::RateLimited);
            }
        }
        for bucket in ["global", username_hash] {
            sqlx::query("INSERT INTO login_attempts(bucket,attempts,expires_at) VALUES(?,1,?) ON CONFLICT(bucket) DO UPDATE SET attempts=attempts+1")
                .bind(bucket).bind(now+300).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn successful_login(c: &mut SqliteConnection, username_hash: &str) -> Result<()> {
        sqlx::query(
            "DELETE FROM login_attempts WHERE bucket=? OR (bucket='global' AND attempts=1)",
        )
        .bind(username_hash)
        .execute(&mut *c)
        .await?;
        sqlx::query("UPDATE login_attempts SET attempts=attempts-1 WHERE bucket='global'")
            .execute(c)
            .await?;
        Ok(())
    }
}
