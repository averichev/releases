use std::{collections::HashMap, fs};

use tempfile::tempdir;
use veruna_releases::config::AppConfig;

#[test]
fn defaults_work_without_a_file() {
    let config = AppConfig::from_sources(None, HashMap::new()).unwrap();
    assert_eq!(config.server.bind.to_string(), "127.0.0.1:8080");
    assert_eq!(config.database.path.to_str(), Some("data/veruna.sqlite"));
}

#[test]
fn environment_overrides_file_and_preserves_other_values() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("veruna.toml");
    fs::write(&path, "[server]\nbind = '127.0.0.1:9090'\n[database]\npath = 'custom/db.sqlite'\nmax_connections = 2\n").unwrap();
    let environment = HashMap::from([
        ("VERUNA__SERVER__BIND".into(), "127.0.0.1:8181".into()),
        ("VERUNA__DATABASE__MAX_CONNECTIONS".into(), "3".into()),
        (
            "VERUNA_CONFIG".into(),
            "ignored-by-field-loader.toml".into(),
        ),
    ]);
    let config = AppConfig::from_sources(Some(&path), environment).unwrap();
    assert_eq!(config.server.bind.port(), 8181);
    assert_eq!(config.database.max_connections, 3);
    assert_eq!(config.database.path.to_str(), Some("custom/db.sqlite"));
}

#[test]
fn invalid_or_misspelled_settings_fail_early() {
    for (key, value) in [
        ("VERUNA__SERVER__BIND", "invalid"),
        ("VERUNA__SERVER__SHUTDOWN_TIMEOUT_SECONDS", "0"),
        ("VERUNA__DATABASE__MAX_CONNECTIONS", "0"),
        ("VERUNA__DATABASE__BUSY_TIMEOUT_SECONDS", "0"),
        ("VERUNA__DATABASE__PATH", ""),
        ("VERUNA__STORAGE__DIRECTORY", ""),
        ("VERUNA__LOGGING__FILTER", ""),
        ("VERUNA__DATABASE__PAHT", "typo"),
    ] {
        let environment = HashMap::from([(key.into(), value.into())]);
        assert!(AppConfig::from_sources(None, environment).is_err(), "{key}");
    }
    let dir = tempdir().unwrap();
    assert!(
        AppConfig::from_sources(Some(&dir.path().join("missing.toml")), HashMap::new()).is_err()
    );
    let path = dir.path().join("broken.toml");
    fs::write(&path, "[server").unwrap();
    assert!(AppConfig::from_sources(Some(&path), HashMap::new()).is_err());
}
