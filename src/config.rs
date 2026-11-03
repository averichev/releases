use std::{
    collections::HashMap,
    ffi::OsString,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use config::{Config, Environment, File};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub storage: StorageConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub shutdown_timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    pub path: PathBuf,
    pub max_connections: u32,
    pub busy_timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub directory: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub filter: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: ([127, 0, 0, 1], 8080).into(),
            shutdown_timeout_seconds: 15,
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: "data/veruna.sqlite".into(),
            max_connections: 4,
            busy_timeout_seconds: 5,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            directory: "data/artifacts".into(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            filter: "info,tower_http=info".into(),
        }
    }
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let explicit = std::env::var_os("VERUNA_CONFIG").map(PathBuf::from);
        let default_path = Path::new("veruna.toml");
        let path = explicit
            .as_deref()
            .or_else(|| default_path.exists().then_some(default_path));
        Self::from_sources(path, configuration_environment(std::env::vars_os())?)
    }

    /// Explicit sources keep tests independent of process-global environment.
    pub fn from_sources(path: Option<&Path>, environment: HashMap<String, String>) -> Result<Self> {
        let mut builder = Config::builder();
        if let Some(path) = path {
            builder = builder.add_source(File::from(path).required(true));
        }
        let settings: Self = builder
            .add_source(
                Environment::with_prefix("VERUNA")
                    .prefix_separator("__")
                    .separator("__")
                    .try_parsing(true)
                    .source(Some(environment)),
            )
            .build()
            .context("Cannot load configuration")?
            .try_deserialize()
            .context("Invalid configuration")?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.database.path.as_os_str().is_empty(),
            "database.path must not be empty"
        );
        ensure!(
            !self.storage.directory.as_os_str().is_empty(),
            "storage.directory must not be empty"
        );
        ensure!(
            (1..=32).contains(&self.database.max_connections),
            "database.max_connections must be between 1 and 32"
        );
        ensure!(
            (1..=300).contains(&self.database.busy_timeout_seconds),
            "database.busy_timeout_seconds must be between 1 and 300"
        );
        ensure!(
            (1..=300).contains(&self.server.shutdown_timeout_seconds),
            "server.shutdown_timeout_seconds must be between 1 and 300"
        );
        ensure!(
            !self.logging.filter.trim().is_empty(),
            "logging.filter must not be empty"
        );
        tracing_subscriber::EnvFilter::try_new(&self.logging.filter)
            .context("Invalid logging.filter")?;
        Ok(())
    }
}

fn configuration_environment(
    environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<HashMap<String, String>> {
    let mut settings = HashMap::new();
    for (name, value) in environment {
        // Unrelated environment values may contain arbitrary platform bytes.
        let Some(name) = name.to_str().filter(|name| name.starts_with("VERUNA__")) else {
            continue;
        };
        let value = value
            .into_string()
            .map_err(|_| anyhow::anyhow!("{name} must contain valid UTF-8"))?;
        settings.insert(name.to_owned(), value);
    }
    Ok(settings)
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    #[test]
    fn ignores_non_unicode_values_outside_configuration() {
        let environment = [
            (OsString::from("_"), OsString::from_vec(vec![0xff])),
            (
                OsString::from("VERUNA__SERVER__BIND"),
                OsString::from("127.0.0.1:8081"),
            ),
        ];
        let values = configuration_environment(environment).unwrap();
        let config = AppConfig::from_sources(None, values).unwrap();
        assert_eq!(config.server.bind.port(), 8081);
    }

    #[test]
    fn rejects_non_unicode_configuration_without_panicking_or_echoing_value() {
        let environment = [(
            OsString::from("VERUNA__STORAGE__DIRECTORY"),
            OsString::from_vec(vec![0xff]),
        )];
        assert_eq!(
            configuration_environment(environment)
                .unwrap_err()
                .to_string(),
            "VERUNA__STORAGE__DIRECTORY must contain valid UTF-8"
        );
    }
}
