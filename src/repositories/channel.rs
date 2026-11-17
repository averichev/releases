use super::{
    applications::ApplicationsRepository, artifact::ArtifactsRepository,
    release::ReleasesRepository,
};
use crate::{
    domain::{
        application::Application, artifact::Artifact, publication::ReleaseChannel, release::Release,
    },
    services::error::{Result, ServiceError},
};
use sqlx::{SqliteConnection, SqlitePool};

const CHANNEL: &str = "SELECT c.*, r.version AS current_version,r.published_at,(SELECT count(*) FROM channel_releases cr WHERE cr.channel_id=c.id) AS release_count FROM release_channels c LEFT JOIN releases r ON r.id=c.current_release_id";
#[derive(sqlx::FromRow)]
struct StoredChannel {
    id: String,
    application_id: String,
    name: String,
    current_release_id: Option<String>,
    current_version: Option<String>,
    published_at: Option<i64>,
    release_count: i64,
    revision: i64,
    created_at: i64,
    updated_at: i64,
}
impl From<StoredChannel> for ReleaseChannel {
    fn from(c: StoredChannel) -> Self {
        Self {
            id: c.id,
            application_id: c.application_id,
            name: c.name,
            current_release_id: c.current_release_id,
            current_version: c.current_version,
            published_at: c.published_at,
            release_count: c.release_count,
            revision: c.revision,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}
pub struct PublicationSnapshot {
    pub application: Application,
    pub release: Release,
    pub artifacts: Vec<Artifact>,
    pub channel: ReleaseChannel,
    pub default_distribution: Option<crate::domain::application::DistributionSelector>,
}
#[derive(Clone)]
pub struct ChannelsRepository {
    pool: SqlitePool,
}
impl ChannelsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn list(&self, app: &str) -> Result<Vec<ReleaseChannel>> {
        let mut tx = self.pool.begin().await?;
        ApplicationsRepository::get_in(&mut tx, app).await?;
        let rows = sqlx::query_as::<_, StoredChannel>(&format!(
            "{CHANNEL} WHERE c.application_id=? ORDER BY c.name"
        ))
        .bind(app)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
    pub async fn get_in(c: &mut SqliteConnection, app: &str, name: &str) -> Result<ReleaseChannel> {
        Ok(sqlx::query_as::<_, StoredChannel>(&format!(
            "{CHANNEL} WHERE c.application_id=? AND c.name=?"
        ))
        .bind(app)
        .bind(name)
        .fetch_optional(c)
        .await?
        .ok_or(ServiceError::NotFound)?
        .into())
    }
    pub async fn snapshot(
        &self,
        app: &str,
        release: &str,
        channel: &str,
    ) -> Result<PublicationSnapshot> {
        let mut tx = self.pool.begin().await?;
        let snapshot = Self::snapshot_in(&mut tx, app, release, channel).await?;
        tx.commit().await?;
        Ok(snapshot)
    }
    pub async fn snapshot_in(
        c: &mut SqliteConnection,
        app: &str,
        release: &str,
        channel: &str,
    ) -> Result<PublicationSnapshot> {
        Ok(PublicationSnapshot {
            default_distribution: super::distribution::DistributionRepository::selector_in(c, app)
                .await?,
            application: ApplicationsRepository::get_in(c, app).await?,
            release: ReleasesRepository::get_in(c, app, release).await?,
            artifacts: ArtifactsRepository::list_in(c, release).await?,
            channel: Self::get_in(c, app, channel).await?,
        })
    }
    pub async fn assign(
        c: &mut SqliteConnection,
        channel: &ReleaseChannel,
        release: &str,
        now: i64,
    ) -> Result<()> {
        sqlx::query("INSERT INTO channel_releases(channel_id,release_id,application_id,added_at) VALUES(?,?,?,?) ON CONFLICT(channel_id,release_id) DO NOTHING").bind(&channel.id).bind(release).bind(&channel.application_id).bind(now).execute(&mut *c).await?;
        let changed=sqlx::query("UPDATE release_channels SET current_release_id=?,revision=revision+1,updated_at=max(updated_at,?) WHERE id=? AND revision=?").bind(release).bind(now).bind(&channel.id).bind(channel.revision).execute(c).await?;
        if changed.rows_affected() != 1 {
            return Err(ServiceError::PublicationConflict);
        }
        Ok(())
    }
    pub async fn publish(c: &mut SqliteConnection, id: &str, now: i64) -> Result<()> {
        sqlx::query("UPDATE releases SET status='published',published_at=?,updated_at=max(updated_at,?) WHERE id=? AND status='draft'").bind(now).bind(now).bind(id).execute(c).await?;
        Ok(())
    }
    /// The explicit channel pointer is the only source of the current release.
    pub async fn current_snapshot(
        &self,
        key: &str,
        name: &str,
        by_slug: bool,
    ) -> Result<(Application, ReleaseChannel, Option<PublicationSnapshot>)> {
        let mut tx = self.pool.begin().await?;
        let app = if by_slug {
            ApplicationsRepository::by_slug_in(&mut tx, key).await?
        } else {
            ApplicationsRepository::get_in(&mut tx, key).await?
        };
        let channel = Self::get_in(&mut tx, &app.id, name).await?;
        let snapshot = match &channel.current_release_id {
            Some(id) => Some(Self::snapshot_in(&mut tx, &app.id, id, name).await?),
            None => None,
        };
        tx.commit().await?;
        Ok((app, channel, snapshot))
    }
    pub async fn public_active(&self, id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM applications WHERE id=? AND status='active' AND visibility='public'").bind(id).fetch_one(&self.pool).await? ==1)
    }
}
