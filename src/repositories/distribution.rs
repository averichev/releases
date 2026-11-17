use super::{
    applications::ApplicationsRepository,
    artifact::ArtifactsRepository,
    channel::{ChannelsRepository, PublicationSnapshot},
    release::ReleasesRepository,
};
use crate::{
    domain::{
        application::DistributionSelector,
        release::{Release, ReleaseStatus},
    },
    services::error::{Result, ServiceError},
};
use sqlx::{SqliteConnection, SqlitePool};

#[derive(Clone)]
pub struct DistributionRepository {
    pool: SqlitePool,
}
impl DistributionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn selector(&self, app: &str) -> Result<Option<DistributionSelector>> {
        Self::selector_in(&mut *self.pool.acquire().await?, app).await
    }
    pub async fn selector_in(
        c: &mut SqliteConnection,
        app: &str,
    ) -> Result<Option<DistributionSelector>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT target,format,purpose FROM application_distribution WHERE application_id=?",
        )
        .bind(app)
        .fetch_optional(c)
        .await?;
        Ok(row.map(|(target, format, purpose)| DistributionSelector {
            target,
            format,
            purpose,
        }))
    }
    pub async fn set_selector(
        c: &mut SqliteConnection,
        app: &str,
        value: Option<&DistributionSelector>,
    ) -> Result<()> {
        if let Some(s) = value {
            sqlx::query("INSERT INTO application_distribution(application_id,target,format,purpose) VALUES(?,?,?,?) ON CONFLICT(application_id) DO UPDATE SET target=excluded.target,format=excluded.format,purpose=excluded.purpose")
                .bind(app).bind(&s.target).bind(&s.format).bind(&s.purpose).execute(c).await?;
        } else {
            sqlx::query("DELETE FROM application_distribution WHERE application_id=?")
                .bind(app)
                .execute(c)
                .await?;
        }
        Ok(())
    }
    pub async fn version(
        &self,
        slug: &str,
        version: &str,
        channel: &str,
    ) -> Result<PublicationSnapshot> {
        let mut tx = self.pool.begin().await?;
        let app = ApplicationsRepository::by_slug_in(&mut tx, slug).await?;
        let channel = ChannelsRepository::get_in(&mut tx, &app.id, channel).await?;
        let id: String = sqlx::query_scalar("SELECT r.id FROM releases r JOIN channel_releases cr ON cr.release_id=r.id WHERE r.application_id=? AND r.version=? AND r.status='published' AND cr.channel_id=?")
            .bind(&app.id).bind(version).bind(&channel.id).fetch_optional(&mut *tx).await?.ok_or(ServiceError::NotFound)?;
        let snapshot =
            ChannelsRepository::snapshot_in(&mut tx, &app.id, &id, &channel.name).await?;
        tx.commit().await?;
        Ok(snapshot)
    }
    pub async fn application(
        &self,
        key: &str,
        public: bool,
    ) -> Result<crate::domain::application::Application> {
        let mut tx = self.pool.begin().await?;
        let app = if public {
            ApplicationsRepository::by_slug_in(&mut tx, key).await?
        } else {
            ApplicationsRepository::get_in(&mut tx, key).await?
        };
        tx.commit().await?;
        Ok(app)
    }
    pub async fn history(
        &self,
        key: &str,
        channel: &str,
        public: bool,
        limit: u32,
        offset: u32,
    ) -> Result<(
        crate::domain::application::Application,
        crate::domain::publication::ReleaseChannel,
        Vec<(Release, Vec<crate::domain::artifact::Artifact>)>,
        i64,
    )> {
        let mut tx = self.pool.begin().await?;
        let app = if public {
            ApplicationsRepository::by_slug_in(&mut tx, key).await?
        } else {
            ApplicationsRepository::get_in(&mut tx, key).await?
        };
        let channel = ChannelsRepository::get_in(&mut tx, &app.id, channel).await?;
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM channel_releases WHERE channel_id=?")
                .bind(&channel.id)
                .fetch_one(&mut *tx)
                .await?;
        let ids:Vec<String>=sqlx::query_scalar("SELECT r.id FROM releases r JOIN channel_releases cr ON cr.release_id=r.id WHERE cr.channel_id=? AND r.status='published' ORDER BY r.published_at DESC,r.id LIMIT ? OFFSET ?")
            .bind(&channel.id).bind(limit).bind(offset).fetch_all(&mut *tx).await?;
        let mut releases = Vec::new();
        for id in ids {
            let r = ReleasesRepository::get_in(&mut tx, &app.id, &id).await?;
            if r.status != ReleaseStatus::Published {
                return Err(ServiceError::NotFound);
            }
            releases.push((r, ArtifactsRepository::list_in(&mut tx, &id).await?));
        }
        tx.commit().await?;
        Ok((app, channel, releases, total))
    }
}
