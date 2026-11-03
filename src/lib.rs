//! Veruna Releases: a modular monolith with an embedded administration UI.
pub mod config;
pub mod domain;
pub mod http;
pub mod infrastructure;
pub mod repositories;
pub mod services;

use anyhow::Result;
use sqlx::SqlitePool;

use crate::{
    config::AppConfig,
    infrastructure::storage::{ArtifactStorage, LocalArtifactStorage},
};

/// Initialize durable infrastructure before accepting requests.
pub async fn initialize(config: &AppConfig) -> Result<SqlitePool> {
    config.validate()?;
    let storage = LocalArtifactStorage::new(config.storage.directory.clone());
    storage.initialize().await?;
    infrastructure::database::connect(&config.database).await
}
