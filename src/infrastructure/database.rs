use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::config::DatabaseConfig;

static MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[cfg(target_os = "linux")]
pub(crate) fn data_compatibility() -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    // Bump the format marker for incompatible persisted changes outside SQLite.
    hash.update(b"veruna-releases/storage-v1\0");
    for migration in MIGRATIONS
        .iter()
        .filter(|m| m.migration_type.is_up_migration())
    {
        hash.update(migration.version.to_be_bytes());
        hash.update(migration.checksum.as_ref());
    }
    format!("{:x}", hash.finalize())
}

pub async fn connect(config: &DatabaseConfig) -> Result<SqlitePool> {
    if let Some(parent) = config.path.parent().filter(|p| !p.as_os_str().is_empty()) {
        tokio::fs::create_dir_all(parent)
            .await
            .context("Cannot create database directory")?;
    }
    let options = SqliteConnectOptions::new()
        .filename(&config.path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(Duration::from_secs(config.busy_timeout_seconds));
    let pool = SqlitePoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(config.busy_timeout_seconds))
        .connect_with(options)
        .await
        .context("Cannot open SQLite database")?;
    if let Err(error) = MIGRATIONS.run(&pool).await {
        pool.close().await;
        return Err(error).context("Cannot apply database migrations");
    }
    Ok(pool)
}
