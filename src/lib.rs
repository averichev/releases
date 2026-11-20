//! Veruna Releases: a modular monolith with an embedded administration UI.
pub mod cli;
pub mod config;
pub mod domain;
pub mod http;
pub mod infrastructure;
pub mod repositories;
pub mod services;

/// Release builds use the calculated Git version; ordinary builds use Cargo's version.
pub const VERSION: &str = env!("APP_VERSION");

use anyhow::Result;
use sqlx::SqlitePool;

use crate::{
    config::AppConfig,
    infrastructure::storage::{ArtifactStorage, LocalArtifactStorage},
};

/// Initialize durable infrastructure before accepting requests.
pub async fn initialize(config: &AppConfig) -> Result<SqlitePool> {
    config.validate()?;
    let storage = LocalArtifactStorage::new(&config.storage);
    storage.initialize().await?;
    let database = infrastructure::database::connect(&config.database).await?;
    let icons = infrastructure::icons::IconStorage::new(config.storage.icons_directory.clone());
    let result = async {
        let keys = repositories::applications::ApplicationsRepository::new(database.clone())
            .icon_keys()
            .await?;
        let references = repositories::artifact::ArtifactsRepository::new(database.clone())
            .references()
            .await?;
        storage.reconcile(references).await?;
        icons
            .initialize(config.storage.directory.clone(), keys)
            .await
    }
    .await;
    if let Err(error) = result {
        database.close().await;
        return Err(error.into());
    }
    Ok(database)
}

/// Composition root for the concrete adapters shared by CLI and HTTP.
pub async fn application_state(
    database: SqlitePool,
    config: &AppConfig,
) -> services::error::Result<http::AppState> {
    let passwords = infrastructure::passwords::Passwords::new().await?;
    let users = repositories::users::UsersRepository::new(database.clone());
    let sessions = repositories::sessions::SessionsRepository::new(database.clone());
    let storage = LocalArtifactStorage::new(&config.storage);
    let releases = repositories::release::ReleasesRepository::new(database.clone());
    let validator = services::publication_validation::PublicationValidator::new(storage.clone());
    let distribution = services::distribution::PublicDistribution::new(
        repositories::distribution::DistributionRepository::new(database.clone()),
        storage.clone(),
        config.download.clone(),
    );
    Ok(http::AppState {
        server_bind: config.server.bind,
        tauri_integration: services::tauri_integration::TauriIntegration::new(
            repositories::applications::ApplicationsRepository::new(database.clone()),
            distribution.clone(),
            validator.clone(),
        ),
        publishing: services::publishing::ReleasePublishing::new(
            releases.clone(),
            repositories::distribution::DistributionRepository::new(database.clone()),
            validator.clone(),
            repositories::publication_history::PublicationHistoryRepository::new(database.clone()),
        ),
        manifests: services::manifest::ManifestGeneration::new(distribution.clone()),
        updater: services::updater::TauriUpdater::new(distribution.clone()),
        standalone_updater: services::standalone::StandaloneUpdater::new(distribution.clone()),
        distribution,
        storage_config: config.storage.clone(),
        releases: services::release::ReleaseManagement::new(releases.clone(), storage.clone()),
        artifacts: services::artifact::ArtifactManagement::new(
            repositories::artifact::ArtifactsRepository::new(database.clone()),
            releases,
            storage,
        ),
        database: database.clone(),
        auth_config: config.auth.clone(),
        authentication: services::auth::Authentication::new(
            users.clone(),
            sessions.clone(),
            passwords.clone(),
            config.auth.clone(),
        ),
        sessions: services::sessions::Sessions::new(sessions),
        users: services::users::UserManagement::new(users, passwords, config.auth.clone()),
        applications: services::applications::ApplicationCatalog::new(
            repositories::applications::ApplicationsRepository::new(database.clone()),
            infrastructure::icons::IconStorage::new(config.storage.icons_directory.clone()),
        ),
    })
}
