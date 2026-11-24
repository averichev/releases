use crate::services::error::Result;
use sqlx::SqliteConnection;

pub struct ApiPublicationsRepository;
impl ApiPublicationsRepository {
    /// Includes manual drafts and published releases without an API manifest.
    pub async fn existing(
        c: &mut SqliteConnection,
        app: &str,
        version_key: &str,
    ) -> Result<Option<(String, Option<String>)>> {
        Ok(sqlx::query_as("SELECT r.id,p.manifest FROM releases r LEFT JOIN api_publications p ON p.release_id=r.id WHERE r.application_id=? AND r.version_key=?")
            .bind(app).bind(version_key).fetch_optional(c).await?)
    }
    pub async fn insert(c: &mut SqliteConnection, release: &str, manifest: &str) -> Result<()> {
        sqlx::query("INSERT INTO api_publications(release_id,manifest) VALUES(?,?)")
            .bind(release)
            .bind(manifest)
            .execute(c)
            .await?;
        Ok(())
    }
}
