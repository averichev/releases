use crate::{
    domain::{
        publication::{PublicationValidation, ReleaseChannel, switch_action},
        release::ReleaseStatus,
    },
    infrastructure::secrets,
    repositories::{
        channel::ChannelsRepository,
        publication_history::{PublicationEvent, PublicationHistoryRepository},
        release::ReleasesRepository,
    },
    services::{
        error::{Result, ServiceError},
        publication_validation::PublicationValidator,
        release::authorize,
        sessions::Identity,
    },
};
#[derive(Clone)]
pub struct ReleasePublishing {
    channels: ChannelsRepository,
    releases: ReleasesRepository,
    validator: PublicationValidator,
    history: PublicationHistoryRepository,
}
pub struct PublicationCommand {
    pub channel: String,
    pub expected_channel_revision: i64,
    pub expected_release_revision: i64,
}
pub struct PublicationCheck {
    pub release_revision: i64,
    pub channel: ReleaseChannel,
    pub validation: PublicationValidation,
}
impl ReleasePublishing {
    pub fn new(
        channels: ChannelsRepository,
        releases: ReleasesRepository,
        validator: PublicationValidator,
        history: PublicationHistoryRepository,
    ) -> Self {
        Self {
            channels,
            releases,
            validator,
            history,
        }
    }
    pub async fn channels(&self, app: &str) -> Result<Vec<ReleaseChannel>> {
        self.channels.list(app).await
    }
    pub async fn history(
        &self,
        app: &str,
        limit: u32,
        before: Option<i64>,
    ) -> Result<Vec<PublicationEvent>> {
        if !(1..=100).contains(&limit) || before.is_some_and(|v| v < 1) {
            return Err(ServiceError::Invalid);
        }
        self.channels.list(app).await?;
        self.history.list(app, limit, before).await
    }
    pub async fn validate(
        &self,
        actor: &Identity,
        app: &str,
        id: &str,
        channel: &str,
    ) -> Result<PublicationCheck> {
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        tx.commit().await?;
        let snapshot = self.channels.snapshot(app, id, channel).await?;
        let validation = self.validator.validate(&snapshot, true).await?;
        Ok(PublicationCheck {
            release_revision: snapshot.release.revision,
            channel: snapshot.channel,
            validation,
        })
    }
    pub async fn publish(
        &self,
        actor: &Identity,
        app: &str,
        id: &str,
        command: PublicationCommand,
    ) -> Result<ReleaseChannel> {
        self.change(actor, app, id, command, true).await
    }
    pub async fn switch(
        &self,
        actor: &Identity,
        app: &str,
        id: &str,
        command: PublicationCommand,
    ) -> Result<ReleaseChannel> {
        self.change(actor, app, id, command, false).await
    }
    async fn change(
        &self,
        actor: &Identity,
        app: &str,
        id: &str,
        command: PublicationCommand,
        publish: bool,
    ) -> Result<ReleaseChannel> {
        // Authorize before expensive hashing, and reauthorize inside the final write transaction.
        let mut tx = self.releases.write().await?;
        authorize(&mut tx, actor).await?;
        tx.commit().await?;
        let snapshot = self.channels.snapshot(app, id, &command.channel).await?;
        if !publish && snapshot.release.status != ReleaseStatus::Published {
            return Err(ServiceError::ReleaseLocked);
        }
        let validation = self.validator.validate(&snapshot, false).await?;
        if !validation.errors.is_empty() {
            return Err(ServiceError::PublicationBlocked(validation));
        }
        let mut tx = self.releases.write().await?;
        let user = authorize(&mut tx, actor).await?;
        let current = ChannelsRepository::snapshot_in(&mut tx, app, id, &command.channel).await?;
        if current.default_distribution != snapshot.default_distribution
            || current.application.status != snapshot.application.status
            || current.release != snapshot.release
            || current.artifacts != snapshot.artifacts
        {
            return Err(ServiceError::PublicationConflict);
        }
        // Retry of a completed request has no side effects, including timestamp/history.
        if current.release.status == ReleaseStatus::Published
            && current.channel.current_release_id.as_deref() == Some(id)
        {
            tx.commit().await?;
            return Ok(current.channel);
        }
        if current.channel.revision != command.expected_channel_revision
            || current.release.revision != command.expected_release_revision
        {
            return Err(ServiceError::PublicationConflict);
        }
        // publish also promotes an already published immutable release to another channel.
        let first_publication = current.release.status == ReleaseStatus::Draft;
        let now = secrets::now().max(current.release.created_at);
        let previous = match &current.channel.current_release_id {
            Some(id) => Some(ReleasesRepository::get_in(&mut tx, app, id).await?),
            None => None,
        };
        if first_publication {
            ChannelsRepository::publish(&mut tx, id, now).await?;
        }
        ChannelsRepository::assign(&mut tx, &current.channel, id, now).await?;
        PublicationHistoryRepository::append(
            &mut tx,
            &current.channel,
            id,
            if first_publication {
                "release_published"
            } else {
                switch_action(previous.as_ref(), &current.release)
            },
            &user.id,
            now,
        )
        .await?;
        let channel = ChannelsRepository::get_in(&mut tx, app, &command.channel).await?;
        tx.commit().await?;
        Ok(channel)
    }
}
