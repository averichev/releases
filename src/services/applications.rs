use crate::{
    domain::{
        application::{
            self, Application, ApplicationChanges, ApplicationPage, ListQuery, NewApplication,
            Status,
        },
        user::Role,
    },
    infrastructure::{icons::IconStorage, secrets},
    repositories::{applications::ApplicationsRepository, sessions::SessionsRepository},
    services::{
        error::{Result, ServiceError},
        sessions::Identity,
    },
};

#[derive(Clone)]
pub struct ApplicationCatalog {
    repository: ApplicationsRepository,
    icons: IconStorage,
}
impl ApplicationCatalog {
    pub fn new(repository: ApplicationsRepository, icons: IconStorage) -> Self {
        Self { repository, icons }
    }
    async fn authorize(c: &mut sqlx::SqliteConnection, actor: &Identity) -> Result<()> {
        let user = SessionsRepository::authorize(c, &actor.session_hash, secrets::now()).await?;
        if user.role != Role::Admin {
            return Err(ServiceError::Forbidden);
        }
        Ok(())
    }
    pub async fn list(&self, query: ListQuery) -> Result<ApplicationPage> {
        if query.page == 0
            || !(1..=100).contains(&query.per_page)
            || query.search.chars().count() > 120
        {
            return Err(ServiceError::InvalidApplication);
        }
        self.repository.list(&query).await
    }
    pub async fn get(&self, id: &str) -> Result<Application> {
        self.repository.get(id).await
    }
    pub async fn create(&self, actor: &Identity, input: NewApplication) -> Result<Application> {
        if !application::valid_slug(&input.slug) {
            return Err(ServiceError::InvalidApplication);
        }
        let now = secrets::now();
        let app = Application {
            id: uuid::Uuid::new_v4().to_string(),
            slug: input.slug,
            name: application::name(&input.name).ok_or(ServiceError::InvalidApplication)?,
            description: application::description(&input.description)
                .ok_or(ServiceError::InvalidApplication)?,
            icon_key: None,
            visibility: input.visibility,
            status: Status::Active,
            created_at: now,
            updated_at: now,
        };
        let mut tx = self.repository.write().await?;
        Self::authorize(&mut tx, actor).await?;
        ApplicationsRepository::insert(&mut tx, &app).await?;
        tx.commit().await?;
        Ok(app)
    }
    pub async fn update(
        &self,
        actor: &Identity,
        id: &str,
        changes: ApplicationChanges,
    ) -> Result<Application> {
        if changes.name.is_none() && changes.description.is_none() && changes.visibility.is_none() {
            return Err(ServiceError::InvalidApplication);
        }
        let name = changes
            .name
            .map(|v| application::name(&v).ok_or(ServiceError::InvalidApplication))
            .transpose()?;
        let description = changes
            .description
            .map(|v| application::description(&v).ok_or(ServiceError::InvalidApplication))
            .transpose()?;
        let mut tx = self.repository.write().await?;
        Self::authorize(&mut tx, actor).await?;
        let mut app = ApplicationsRepository::get_in(&mut tx, id).await?;
        if let Some(name) = name {
            app.name = name;
        }
        if let Some(description) = description {
            app.description = description;
        }
        if let Some(visibility) = changes.visibility {
            app.visibility = visibility;
        }
        app.updated_at = secrets::now().max(app.updated_at);
        ApplicationsRepository::update(&mut tx, &app).await?;
        tx.commit().await?;
        Ok(app)
    }
    pub async fn set_status(
        &self,
        actor: &Identity,
        id: &str,
        status: Status,
    ) -> Result<Application> {
        let mut tx = self.repository.write().await?;
        Self::authorize(&mut tx, actor).await?;
        let mut app = ApplicationsRepository::get_in(&mut tx, id).await?;
        if app.status != status {
            app.status = status;
            app.updated_at = secrets::now().max(app.updated_at);
            ApplicationsRepository::update(&mut tx, &app).await?;
        }
        tx.commit().await?;
        Ok(app)
    }
    pub async fn set_icon(
        &self,
        actor: &Identity,
        id: &str,
        bytes: Option<Vec<u8>>,
    ) -> Result<Application> {
        // Do expensive work outside the write lock. Recheck the live session inside it.
        if actor.user.role != Role::Admin {
            return Err(ServiceError::Forbidden);
        }
        self.get(id).await?;
        let key = match bytes {
            Some(bytes) => Some(self.icons.store(bytes).await?),
            None => None,
        };
        let mut tx = self.repository.write().await?;
        Self::authorize(&mut tx, actor).await?;
        let mut app = ApplicationsRepository::get_in(&mut tx, id).await?;
        if app.icon_key != key {
            app.icon_key = key;
            app.updated_at = secrets::now().max(app.updated_at);
            ApplicationsRepository::update(&mut tx, &app).await?;
        }
        // Never delete a new file on an ambiguous commit failure, or an old file
        // still being read. Startup reclaims all unreferenced objects and temporaries.
        tx.commit().await?;
        Ok(app)
    }
    pub async fn set_distribution(
        &self,
        actor: &Identity,
        id: &str,
        selector: Option<application::DistributionSelector>,
    ) -> Result<()> {
        if selector.as_ref().is_some_and(|s| !s.valid()) {
            return Err(ServiceError::InvalidArtifact);
        }
        let mut tx = self.repository.write().await?;
        Self::authorize(&mut tx, actor).await?;
        ApplicationsRepository::get_in(&mut tx, id).await?;
        crate::repositories::distribution::DistributionRepository::set_selector(
            &mut tx,
            id,
            selector.as_ref(),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn icon(&self, id: &str) -> Result<Vec<u8>> {
        let app = self.get(id).await?;
        let key = app.icon_key.ok_or(ServiceError::NotFound)?;
        self.icons.read(&key).await
    }
}
