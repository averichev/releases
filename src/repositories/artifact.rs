use crate::{
    domain::artifact::Artifact,
    services::error::{Result, ServiceError},
};
use sqlx::{SqliteConnection, SqlitePool};
#[derive(sqlx::FromRow)]
struct StoredArtifact {
    id: String,
    release_id: String,
    filename: String,
    target: String,
    file_type: String,
    size_bytes: i64,
    sha256: String,
    storage_key: String,
    created_at: i64,
    distribution_role: String,
    updater_signature: Option<String>,
}
impl From<StoredArtifact> for Artifact {
    fn from(v: StoredArtifact) -> Self {
        Self {
            id: v.id,
            release_id: v.release_id,
            filename: v.filename,
            target: v.target,
            file_type: v.file_type,
            size_bytes: v.size_bytes,
            sha256: v.sha256,
            storage_key: v.storage_key,
            created_at: v.created_at,
            distribution_role: v.distribution_role,
            updater_signature: v.updater_signature,
        }
    }
}
#[derive(Clone)]
pub struct ArtifactsRepository {
    pool: SqlitePool,
}
impl ArtifactsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn references(&self) -> Result<Vec<(String, i64)>> {
        Ok(
            sqlx::query_as("SELECT storage_key,size_bytes FROM artifacts")
                .fetch_all(&self.pool)
                .await?,
        )
    }
    pub async fn list(&self, release: &str) -> Result<Vec<Artifact>> {
        Ok(sqlx::query_as::<_, StoredArtifact>(
            "SELECT * FROM artifacts WHERE release_id=? ORDER BY created_at,id",
        )
        .bind(release)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
    }
    pub async fn list_in(c: &mut SqliteConnection, release: &str) -> Result<Vec<Artifact>> {
        Ok(sqlx::query_as::<_, StoredArtifact>(
            "SELECT * FROM artifacts WHERE release_id=? ORDER BY created_at,id",
        )
        .bind(release)
        .fetch_all(c)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
    }
    pub async fn get(&self, app: &str, release: &str, id: &str) -> Result<Artifact> {
        Ok(sqlx::query_as::<_,StoredArtifact>("SELECT a.* FROM artifacts a JOIN releases r ON r.id=a.release_id WHERE r.application_id=? AND a.release_id=? AND a.id=?")
            .bind(app).bind(release).bind(id).fetch_optional(&self.pool).await?.ok_or(ServiceError::NotFound)?.into())
    }
    pub async fn check_conflict(
        c: &mut SqliteConnection,
        release: &str,
        target: &str,
        format: &str,
        role: &str,
        exclude: Option<&str>,
    ) -> Result<()> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM artifacts WHERE release_id=? AND target=? AND distribution_role=? AND (file_type=? OR distribution_role='updater') AND (? IS NULL OR id!=?))",
        ).bind(release).bind(target).bind(role).bind(format).bind(exclude).bind(exclude)
            .fetch_one(c).await?;
        if exists {
            return Err(ServiceError::ArtifactConflict);
        }
        Ok(())
    }
    pub async fn insert(c: &mut SqliteConnection, a: &Artifact) -> Result<()> {
        let result=sqlx::query("INSERT INTO artifacts(id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at,distribution_role,updater_signature) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
            .bind(&a.id).bind(&a.release_id).bind(&a.filename).bind(&a.target).bind(&a.file_type).bind(a.size_bytes).bind(&a.sha256).bind(&a.storage_key).bind(a.created_at).bind(&a.distribution_role).bind(&a.updater_signature).execute(c).await;
        match result {
            Err(sqlx::Error::Database(ref e)) if e.is_unique_violation() => {
                return Err(ServiceError::ArtifactConflict);
            }
            other => {
                other?;
            }
        }
        Ok(())
    }
    pub async fn updater(
        c: &mut SqliteConnection,
        id: &str,
        role: &str,
        signature: Option<&str>,
    ) -> Result<()> {
        let result =
            sqlx::query("UPDATE artifacts SET distribution_role=?,updater_signature=? WHERE id=?")
                .bind(role)
                .bind(signature)
                .bind(id)
                .execute(c)
                .await;
        match result {
            Err(sqlx::Error::Database(ref e)) if e.is_unique_violation() => {
                Err(ServiceError::ArtifactConflict)
            }
            other => {
                other?;
                Ok(())
            }
        }
    }
    pub async fn delete(c: &mut SqliteConnection, release: &str, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM artifacts WHERE release_id=? AND id=?")
            .bind(release)
            .bind(id)
            .execute(c)
            .await?;
        Ok(())
    }
}
