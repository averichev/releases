use anyhow::{Context, Result};
use tracing_subscriber::{EnvFilter, fmt::writer::BoxMakeWriter};

pub fn initialize(filter: &str, reserved_stdout: bool) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(filter).context("Invalid logging filter")?)
        .json()
        .with_writer(if reserved_stdout {
            BoxMakeWriter::new(std::io::stderr)
        } else {
            BoxMakeWriter::new(std::io::stdout)
        })
        .with_target(true)
        .try_init()
        .map_err(|error| anyhow::anyhow!("Cannot initialize logging: {error}"))
}
