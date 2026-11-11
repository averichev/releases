use crate::{
    domain::user::{Role, Status, User},
    services::error::{Result, ServiceError},
};
use sqlx::{Sqlite, SqliteConnection, SqlitePool, Transaction};

#[derive(sqlx::FromRow)]
pub struct StoredUser {
    pub id: String,
    pub username: String,
    pub password_hash: Option<String>,
    role: String,
    status: String,
    created_at: i64,
    updated_at: i64,
    last_login_at: Option<i64>,
}
impl StoredUser {
    pub fn user(&self) -> Result<User> {
        Ok(User {
            id: self.id.clone(),
            username: self.username.clone(),
            role: Role::parse(&self.role)
                .ok_or_else(|| ServiceError::Internal(anyhow::anyhow!("Invalid stored role")))?,
            status: Status::parse(&self.status)
                .ok_or_else(|| ServiceError::Internal(anyhow::anyhow!("Invalid stored status")))?,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_login_at: self.last_login_at,
        })
    }
}
#[derive(Clone)]
pub struct UsersRepository {
    pub(crate) pool: SqlitePool,
}
impl UsersRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn write(&self) -> Result<Transaction<'static, Sqlite>> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }
    pub async fn admin_exists(&self) -> Result<bool> {
        Ok(
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM users WHERE role = 'admin')",
            )
            .fetch_one(&self.pool)
            .await?,
        )
    }
    pub async fn admin_exists_in(c: &mut SqliteConnection) -> Result<bool> {
        Ok(
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE role='admin')")
                .fetch_one(c)
                .await?,
        )
    }
    pub async fn by_username(&self, username: &str) -> Result<Option<StoredUser>> {
        Ok(sqlx::query_as("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&self.pool)
            .await?)
    }
    pub async fn by_username_in(c: &mut SqliteConnection, username: &str) -> Result<StoredUser> {
        sqlx::query_as("SELECT * FROM users WHERE username=?")
            .bind(username)
            .fetch_optional(c)
            .await?
            .ok_or(ServiceError::NotFound)
    }
    pub async fn by_id(c: &mut SqliteConnection, id: &str) -> Result<StoredUser> {
        sqlx::query_as("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(c)
            .await?
            .ok_or(ServiceError::NotFound)
    }
    pub async fn list(&self) -> Result<Vec<User>> {
        let rows: Vec<StoredUser> =
            sqlx::query_as("SELECT * FROM users ORDER BY created_at, username")
                .fetch_all(&self.pool)
                .await?;
        rows.iter().map(StoredUser::user).collect()
    }
    pub async fn insert(c: &mut SqliteConnection, user: &User, hash: Option<&str>) -> Result<()> {
        sqlx::query("INSERT INTO users(id,username,password_hash,role,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
            .bind(&user.id).bind(&user.username).bind(hash).bind(user.role.as_str()).bind(user.status.as_str())
            .bind(user.created_at).bind(user.updated_at).execute(c).await?;
        Ok(())
    }
    pub async fn set_access(
        c: &mut SqliteConnection,
        id: &str,
        role: Role,
        status: Status,
        now: i64,
    ) -> Result<()> {
        sqlx::query("UPDATE users SET role=?, status=?, updated_at=? WHERE id=?")
            .bind(role.as_str())
            .bind(status.as_str())
            .bind(now)
            .bind(id)
            .execute(&mut *c)
            .await?;
        if status == Status::Disabled {
            sqlx::query("DELETE FROM user_tokens WHERE user_id=?")
                .bind(id)
                .execute(c)
                .await?;
        }
        Ok(())
    }
    pub async fn set_password(
        c: &mut SqliteConnection,
        id: &str,
        hash: &str,
        now: i64,
    ) -> Result<()> {
        sqlx::query("UPDATE users SET password_hash=?, status='active', updated_at=? WHERE id=?")
            .bind(hash)
            .bind(now)
            .bind(id)
            .execute(c)
            .await?;
        Ok(())
    }
    pub async fn put_token(
        c: &mut SqliteConnection,
        id: &str,
        hash: &str,
        purpose: &str,
        expires: i64,
    ) -> Result<()> {
        sqlx::query("INSERT INTO user_tokens(user_id,token_hash,purpose,expires_at) VALUES(?,?,?,?) ON CONFLICT(user_id) DO UPDATE SET token_hash=excluded.token_hash,purpose=excluded.purpose,expires_at=excluded.expires_at")
            .bind(id).bind(hash).bind(purpose).bind(expires).execute(c).await?;
        Ok(())
    }
    pub async fn token(&self, hash: &str, now: i64) -> Result<Option<(String, String, String)>> {
        Ok(sqlx::query_as("SELECT u.id,u.username,t.purpose FROM user_tokens t JOIN users u ON u.id=t.user_id WHERE t.token_hash=? AND t.expires_at>? AND ((t.purpose='invitation' AND u.status='invited') OR (t.purpose='reset' AND u.status='active'))")
            .bind(hash).bind(now).fetch_optional(&self.pool).await?)
    }
    pub async fn consume_token(c: &mut SqliteConnection, hash: &str, now: i64) -> Result<String> {
        sqlx::query_scalar("DELETE FROM user_tokens WHERE token_hash=? AND expires_at>? AND user_id IN (SELECT id FROM users WHERE (status='invited' AND user_tokens.purpose='invitation') OR (status='active' AND user_tokens.purpose='reset')) RETURNING user_id")
            .bind(hash).bind(now).fetch_optional(c).await?.ok_or(ServiceError::Token)
    }
    pub async fn clear_tokens(c: &mut SqliteConnection, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM user_tokens WHERE user_id=?")
            .bind(id)
            .execute(c)
            .await?;
        Ok(())
    }
}
