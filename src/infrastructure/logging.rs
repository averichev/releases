use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

pub fn initialize(filter: &str) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(filter).context("Invalid logging filter")?)
        .json()
        .with_target(true)
        .try_init()
        .map_err(|error| anyhow::anyhow!("Cannot initialize logging: {error}"))
}
