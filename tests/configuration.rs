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
        ("VERUNA__STORAGE__ICONS_DIRECTORY", ""),
        ("VERUNA__STORAGE__TEMPORARY_DIRECTORY", ""),
        ("VERUNA__STORAGE__MAX_FILE_BYTES", "0"),
        ("VERUNA__STORAGE__MAX_FILE_BYTES", "9223372036854775808"),
        ("VERUNA__STORAGE__MAX_REQUEST_BYTES", "100"),
        ("VERUNA__STORAGE__UPLOAD_IDLE_SECONDS", "0"),
        ("VERUNA__STORAGE__UPLOAD_IDLE_SECONDS", "86401"),
        ("VERUNA__LOGGING__FILTER", ""),
        ("VERUNA__DATABASE__PAHT", "typo"),
        ("VERUNA__AUTH__PUBLIC_ORIGIN", "http://example.com"),
        ("VERUNA__AUTH__PUBLIC_ORIGIN", "https://example.com/"),
        ("VERUNA__AUTH__PUBLIC_ORIGIN", "https://example.com/path"),
        (
            "VERUNA__AUTH__PUBLIC_ORIGIN",
            "https://user:pass@example.com",
        ),
        ("VERUNA__AUTH__SESSION_TTL_SECONDS", "0"),
        ("VERUNA__AUTH__SESSION_TTL_SECONDS", "2592001"),
        ("VERUNA__AUTH__INVITATION_TTL_SECONDS", "604801"),
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

#[test]
fn validates_https_and_loopback_origins_and_session_configuration() {
    for origin in [
        "https://apps.example.com",
        "http://127.0.0.1:5173",
        "http://localhost:8080",
        "http://[::1]:8080",
    ] {
        let config = AppConfig::from_sources(
            None,
            HashMap::from([
                ("VERUNA__AUTH__PUBLIC_ORIGIN".into(), origin.into()),
                ("VERUNA__AUTH__SESSION_TTL_SECONDS".into(), "600".into()),
            ]),
        )
        .unwrap();
        assert_eq!(config.auth.session_ttl_seconds, 600);
        assert_eq!(config.auth.secure_cookie(), origin.starts_with("https://"));
    }
}

#[test]
fn artifact_defaults_and_overrides_keep_temporary_data_with_storage() {
    let config = AppConfig::from_sources(
        None,
        HashMap::from([
            ("VERUNA__STORAGE__DIRECTORY".into(), "/disk/releases".into()),
            ("VERUNA__STORAGE__MAX_FILE_BYTES".into(), "4096".into()),
            ("VERUNA__STORAGE__MAX_REQUEST_BYTES".into(), "8192".into()),
        ]),
    )
    .unwrap();
    assert_eq!(
        config.storage.temporary_directory(),
        std::path::Path::new("/disk/releases/.tmp")
    );
    assert_eq!(config.storage.max_file_bytes, 4096);
    assert_eq!(
        AppConfig::default().storage.max_file_bytes,
        2 * 1024 * 1024 * 1024
    );
}
