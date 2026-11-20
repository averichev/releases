use crate::{
    domain::release::{self, Release, ReleaseStatus},
    services::error::{Result, ServiceError},
};
use sqlx::{Sqlite, SqliteConnection, SqlitePool, Transaction};

const SELECT: &str = "SELECT r.*, (SELECT count(*) FROM artifacts a WHERE a.release_id=r.id) AS artifact_count, (SELECT coalesce(sum(size_bytes),0) FROM artifacts a WHERE a.release_id=r.id) AS total_size_bytes FROM releases r";
#[derive(sqlx::FromRow)]
struct StoredRelease {
    id: String,
    application_id: String,
    version: String,
    title: Option<String>,
    release_notes: String,
    status: String,
    revision: i64,
    created_at: i64,
    updated_at: i64,
    published_at: Option<i64>,
    artifact_count: i64,
    total_size_bytes: i64,
}
impl StoredRelease {
    fn release(self) -> Result<Release> {
        Ok(Release {
            id: self.id,
            application_id: self.application_id,
            version: semver::Version::parse(&self.version)
                .map_err(|e| ServiceError::Internal(e.into()))?,
            title: self.title,
            release_notes: self.release_notes,
            status: ReleaseStatus::parse(&self.status).ok_or_else(|| {
                ServiceError::Internal(anyhow::anyhow!("Invalid stored release status"))
            })?,
            revision: self.revision,
            created_at: self.created_at,
            updated_at: self.updated_at,
            published_at: self.published_at,
            artifact_count: self.artifact_count,
            total_size_bytes: self.total_size_bytes,
        })
    }
}
#[derive(Clone)]
pub struct ReleasesRepository {
    pool: SqlitePool,
}
impl ReleasesRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn write(&self) -> Result<Transaction<'static, Sqlite>> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }
    pub async fn get(&self, app: &str, id: &str) -> Result<Release> {
        sqlx::query_as::<_, StoredRelease>(&format!("{SELECT} WHERE r.application_id=? AND r.id=?"))
            .bind(app)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(ServiceError::NotFound)?
            .release()
    }
    pub async fn get_in(c: &mut SqliteConnection, app: &str, id: &str) -> Result<Release> {
        sqlx::query_as::<_, StoredRelease>(&format!("{SELECT} WHERE r.application_id=? AND r.id=?"))
            .bind(app)
            .bind(id)
            .fetch_optional(c)
            .await?
            .ok_or(ServiceError::NotFound)?
            .release()
    }
    pub async fn list(&self, app: &str) -> Result<Vec<Release>> {
        let mut tx = self.pool.begin().await?;
        super::applications::ApplicationsRepository::get_in(&mut tx, app).await?;
        let rows =
            sqlx::query_as::<_, StoredRelease>(&format!("{SELECT} WHERE r.application_id=?"))
                .bind(app)
                .fetch_all(&mut *tx)
                .await?;
        tx.commit().await?;
        rows.into_iter().map(StoredRelease::release).collect()
    }
    pub async fn insert(c: &mut SqliteConnection, r: &Release) -> Result<()> {
        conflict(sqlx::query("INSERT INTO releases(id,application_id,version,version_key,title,release_notes,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)")
            .bind(&r.id).bind(&r.application_id).bind(r.version.to_string()).bind(release::version_key(&r.version)).bind(&r.title).bind(&r.release_notes).bind(r.status.as_str()).bind(r.created_at).bind(r.updated_at).execute(c).await)?;
        Ok(())
    }
    pub async fn update(c: &mut SqliteConnection, r: &Release) -> Result<()> {
        conflict(sqlx::query("UPDATE releases SET version=?,version_key=?,title=?,release_notes=?,updated_at=?,revision=revision+1 WHERE id=?")
            .bind(r.version.to_string()).bind(release::version_key(&r.version)).bind(&r.title).bind(&r.release_notes).bind(r.updated_at).bind(&r.id).execute(c).await)?;
        Ok(())
    }
    pub async fn touch(c: &mut SqliteConnection, id: &str, now: i64) -> Result<()> {
        sqlx::query(
            "UPDATE releases SET updated_at=CASE WHEN status='draft' THEN max(updated_at,?) ELSE updated_at END,revision=revision+1 WHERE id=?",
        )
        .bind(now)
        .bind(id)
        .execute(c)
        .await?;
        Ok(())
    }
    pub async fn publish(c: &mut SqliteConnection, id: &str, now: i64) -> Result<()> {
        sqlx::query("UPDATE releases SET status='published',published_at=?,updated_at=max(updated_at,?),revision=revision+1 WHERE id=? AND status='draft'")
            .bind(now).bind(now).bind(id).execute(c).await?;
        Ok(())
    }
    pub async fn delete(c: &mut SqliteConnection, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM releases WHERE id=?")
            .bind(id)
            .execute(c)
            .await?;
        Ok(())
    }
}
fn conflict<T>(result: std::result::Result<T, sqlx::Error>) -> Result<T> {
    match result {
        Err(sqlx::Error::Database(ref e)) if e.is_unique_violation() => {
            Err(ServiceError::ReleaseVersionConflict)
        }
        other => Ok(other?),
    }
}
