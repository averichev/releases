use super::{
    applications::ApplicationsRepository, artifact::ArtifactsRepository,
    release::ReleasesRepository,
};
use crate::{
    domain::{
        application::{Application, ApplicationPage},
        artifact::Artifact,
        release::Release,
    },
    services::error::{Result, ServiceError},
};
use sqlx::{SqliteConnection, SqlitePool};

pub struct PublicationSnapshot {
    pub application: Application,
    pub release: Release,
    pub artifacts: Vec<Artifact>,
}
/// Application and published files share one SQLite read snapshot.
pub struct PublishedCatalog {
    pub application: Application,
    pub releases: Vec<(Release, Vec<Artifact>)>,
}
#[derive(Clone)]
pub struct DistributionRepository {
    pool: SqlitePool,
}
impl DistributionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn applications(&self, limit: u32, offset: u32) -> Result<ApplicationPage> {
        let mut tx = self.pool.begin().await?;
        let page = ApplicationsRepository::public_page_in(&mut tx, limit, offset).await?;
        tx.commit().await?;
        Ok(page)
    }
    pub async fn snapshot(&self, app: &str, release: &str) -> Result<PublicationSnapshot> {
        let mut tx = self.pool.begin().await?;
        let snapshot = Self::snapshot_in(&mut tx, app, release).await?;
        tx.commit().await?;
        Ok(snapshot)
    }
    pub async fn snapshot_in(
        c: &mut SqliteConnection,
        app: &str,
        release: &str,
    ) -> Result<PublicationSnapshot> {
        Ok(PublicationSnapshot {
            application: ApplicationsRepository::get_in(c, app).await?,
            release: ReleasesRepository::get_in(c, app, release).await?,
            artifacts: ArtifactsRepository::list_in(c, release).await?,
        })
    }
    pub async fn version(&self, slug: &str, version: &str) -> Result<PublicationSnapshot> {
        let mut tx = self.pool.begin().await?;
        let app = ApplicationsRepository::by_slug_in(&mut tx, slug).await?;
        let id: String = sqlx::query_scalar(
            "SELECT id FROM releases WHERE application_id=? AND version=? AND status='published'",
        )
        .bind(&app.id)
        .bind(version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ServiceError::NotFound)?;
        let snapshot = Self::snapshot_in(&mut tx, &app.id, &id).await?;
        tx.commit().await?;
        Ok(snapshot)
    }
    pub async fn application(&self, key: &str, public: bool) -> Result<Application> {
        let mut c = self.pool.acquire().await?;
        if public {
            ApplicationsRepository::by_slug_in(&mut c, key).await
        } else {
            ApplicationsRepository::get_in(&mut c, key).await
        }
    }
    pub async fn published(&self, key: &str, public: bool) -> Result<PublishedCatalog> {
        let mut tx = self.pool.begin().await?;
        let app = if public {
            ApplicationsRepository::by_slug_in(&mut tx, key).await?
        } else {
            ApplicationsRepository::get_in(&mut tx, key).await?
        };
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM releases WHERE application_id=? AND status='published'",
        )
        .bind(&app.id)
        .fetch_all(&mut *tx)
        .await?;
        let mut releases = Vec::with_capacity(ids.len());
        for id in ids {
            releases.push((
                ReleasesRepository::get_in(&mut tx, &app.id, &id).await?,
                ArtifactsRepository::list_in(&mut tx, &id).await?,
            ));
        }
        tx.commit().await?;
        // SQLite strings do not implement SemVer precedence.
        releases.sort_by(|(a, _), (b, _)| b.version.cmp_precedence(&a.version));
        Ok(PublishedCatalog {
            application: app,
            releases,
        })
    }
    pub async fn public_active(&self, id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM applications WHERE id=? AND status='active' AND visibility='public'").bind(id).fetch_one(&self.pool).await? == 1)
    }
}
