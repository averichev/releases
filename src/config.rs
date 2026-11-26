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
    pub auth: AuthConfig,
    pub download: DownloadConfig,
    pub self_update: SelfUpdateConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SelfUpdateConfig {
    pub manifest_url: String,
    pub mode: SelfUpdateMode,
    pub directory: Option<PathBuf>,
}

impl Default for SelfUpdateConfig {
    fn default() -> Self {
        Self {
            manifest_url: "https://app.releases.veruna.systems/releases/manifest.json".into(),
            mode: SelfUpdateMode::External,
            directory: None,
        }
    }
}

impl SelfUpdateConfig {
    pub(crate) fn parsed_manifest_url(&self) -> Result<Option<url::Url>> {
        if self.manifest_url.is_empty() {
            ensure!(
                self.mode == SelfUpdateMode::External,
                "Self-installation requires a non-empty self_update.manifest_url"
            );
            return Ok(None);
        }
        let url =
            url::Url::parse(&self.manifest_url).context("Invalid self_update.manifest_url")?;
        let loopback = match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        ensure!(
            url.has_host()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
                && url.as_str().len() <= 8192
                && (url.scheme() == "https" || url.scheme() == "http" && loopback),
            "self_update.manifest_url requires HTTPS or numeric loopback HTTP without credentials or fragment"
        );
        Ok(Some(url))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelfUpdateMode {
    #[default]
    External,
    Supervisor,
    Standalone,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryMode {
    #[default]
    Direct,
    Nginx,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DownloadConfig {
    pub public_base_url: String,
    pub delivery_mode: DeliveryMode,
    pub max_concurrent_downloads: usize,
}
impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            public_base_url: String::new(),
            delivery_mode: DeliveryMode::Direct,
            max_concurrent_downloads: 64,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub auto_port: bool,
    pub open_browser: bool,
    pub shutdown_timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    pub path: PathBuf,
    pub max_connections: u32,
    pub busy_timeout_seconds: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub directory: PathBuf,
    pub icons_directory: PathBuf,
    pub temporary_directory: Option<PathBuf>,
    pub max_file_bytes: u64,
    pub max_request_bytes: usize,
    pub upload_idle_seconds: u64,
    pub max_concurrent_uploads: usize,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub filter: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    pub public_origin: String,
    pub session_ttl_seconds: i64,
    pub invitation_ttl_seconds: i64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            public_origin: String::new(),
            session_ttl_seconds: 43_200,
            invitation_ttl_seconds: 86_400,
        }
    }
}

impl AuthConfig {
    pub fn secure_cookie(&self) -> bool {
        self.public_origin.starts_with("https://")
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: ([127, 0, 0, 1], 8080).into(),
            auto_port: true,
            open_browser: true,
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
            icons_directory: "data/icons".into(),
            temporary_directory: None,
            max_file_bytes: 2 * 1024 * 1024 * 1024,
            max_request_bytes: 2 * 1024 * 1024 * 1024 + 1024 * 1024,
            upload_idle_seconds: 120,
            max_concurrent_uploads: 4,
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
        let explicit = std::env::var_os("CONFIG").map(PathBuf::from);
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
                Environment::default()
                    .separator("__")
                    .try_parsing(true)
                    .source(Some(configuration_environment(
                        environment.into_iter().map(|(k, v)| (k.into(), v.into())),
                    )?)),
            )
            .build()
            .context("Cannot load configuration")?
            .try_deserialize()
            .context("Invalid configuration")?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<()> {
        let update = &self.self_update;
        if update.parsed_manifest_url()?.is_some() && update.mode != SelfUpdateMode::External {
            ensure!(
                cfg!(target_os = "linux"),
                "Self-installation requires Linux; use self_update.mode=external on this platform"
            );
            ensure!(
                update
                    .directory
                    .as_ref()
                    .is_some_and(|p| !p.as_os_str().is_empty()),
                "Set a private self_update.directory for installation"
            );
            let address = self.server.bind;
            ensure!(
                address.port() != 0
                    && (address.ip().is_loopback() || address.ip().is_unspecified()),
                "Self-installation requires a fixed server port reachable on loopback"
            );
        }
        ensure!(
            !self.database.path.as_os_str().is_empty(),
            "database.path must not be empty"
        );
        ensure!(
            !self.storage.directory.as_os_str().is_empty(),
            "storage.directory must not be empty"
        );
        ensure!(
            !self.storage.icons_directory.as_os_str().is_empty(),
            "storage.icons_directory must not be empty"
        );
        ensure!(
            self.storage.max_file_bytes > 0 && self.storage.max_file_bytes <= i64::MAX as u64,
            "storage.max_file_bytes must fit a positive SQLite integer"
        );
        ensure!(
            self.storage.max_request_bytes as u64 > self.storage.max_file_bytes,
            "storage.max_request_bytes must exceed max_file_bytes for multipart overhead"
        );
        ensure!(
            (1..=86400).contains(&self.storage.upload_idle_seconds),
            "storage.upload_idle_seconds must be between 1 and 86400"
        );
        ensure!(
            (1..=64).contains(&self.storage.max_concurrent_uploads),
            "storage.max_concurrent_uploads must be between 1 and 64"
        );
        ensure!(
            self.storage
                .temporary_directory
                .as_ref()
                .is_none_or(|p| !p.as_os_str().is_empty()),
            "storage.temporary_directory must not be empty"
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
        if !self.auth.public_origin.is_empty() {
            let origin =
                url::Url::parse(&self.auth.public_origin).context("Invalid auth.public_origin")?;
            ensure!(
                origin.origin().ascii_serialization() == self.auth.public_origin
                    && origin.username().is_empty()
                    && origin.password().is_none()
                    && (origin.scheme() == "https"
                        || (origin.scheme() == "http"
                            && matches!(
                                origin.host_str(),
                                Some("localhost" | "127.0.0.1" | "[::1]")
                            ))),
                "auth.public_origin must be an HTTPS origin, or HTTP on loopback, without path or trailing slash"
            );
        }
        ensure!(
            (60..=2_592_000).contains(&self.auth.session_ttl_seconds),
            "auth.session_ttl_seconds must be between 60 and 2592000"
        );
        ensure!(
            (60..=604_800).contains(&self.auth.invitation_ttl_seconds),
            "auth.invitation_ttl_seconds must be between 60 and 604800"
        );
        if !self.download.public_base_url.is_empty() {
            let base = url::Url::parse(&self.download.public_base_url)
                .context("Invalid download.public_base_url")?;
            ensure!(
                base.origin().ascii_serialization() == self.download.public_base_url
                    && base.username().is_empty()
                    && base.password().is_none()
                    && (base.scheme() == "https"
                        || (base.scheme() == "http"
                            && matches!(
                                base.host_str(),
                                Some("localhost" | "127.0.0.1" | "[::1]")
                            ))),
                "download.public_base_url must be an HTTPS origin, or HTTP on loopback, without path or trailing slash"
            );
        }
        ensure!(
            (1..=4096).contains(&self.download.max_concurrent_downloads),
            "download.max_concurrent_downloads must be between 1 and 4096"
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
        let Some(name) = name.to_str() else {
            continue;
        };
        ensure!(
            !name.starts_with("VERUNA__") && name != "VERUNA_CONFIG",
            "Remove the VERUNA prefix from configuration variable {name}"
        );
        if ![
            "SERVER__",
            "DATABASE__",
            "STORAGE__",
            "LOGGING__",
            "AUTH__",
            "DOWNLOAD__",
            "SELF_UPDATE__",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        {
            continue;
        }
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
                OsString::from("SERVER__BIND"),
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
            OsString::from("STORAGE__DIRECTORY"),
            OsString::from_vec(vec![0xff]),
        )];
        assert_eq!(
            configuration_environment(environment)
                .unwrap_err()
                .to_string(),
            "STORAGE__DIRECTORY must contain valid UTF-8"
        );
    }
}

impl StorageConfig {
    pub fn temporary_directory(&self) -> PathBuf {
        self.temporary_directory
            .clone()
            .unwrap_or_else(|| self.directory.join(".tmp"))
    }
}
