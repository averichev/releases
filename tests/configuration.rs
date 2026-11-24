use std::{collections::HashMap, fs};

use tempfile::tempdir;
use veruna_releases::config::AppConfig;

#[test]
fn defaults_work_without_a_file() {
    let config = AppConfig::from_sources(None, HashMap::new()).unwrap();
    assert_eq!(config.server.bind.to_string(), "127.0.0.1:8080");
    assert!(config.server.auto_port);
    assert!(config.server.open_browser);
    assert!(config.auth.public_origin.is_empty());
    assert!(config.download.public_base_url.is_empty());
    assert_eq!(config.database.path.to_str(), Some("data/veruna.sqlite"));
}

#[test]
fn legacy_names_fail_instead_of_silently_using_the_wrong_port() {
    for key in ["VERUNA__SERVER__BIND", "VERUNA_CONFIG"] {
        let error =
            AppConfig::from_sources(None, HashMap::from([(key.into(), "old-value".into())]))
                .unwrap_err()
                .to_string();
        assert!(error.contains(key));
        assert!(!error.contains("old-value"));
    }
    let config = AppConfig::from_sources(
        None,
        HashMap::from([
            ("PATH".into(), "/usr/bin".into()),
            ("HOME".into(), "/tmp".into()),
            ("SERVER__BIND".into(), "127.0.0.1:3001".into()),
        ]),
    )
    .unwrap();
    assert_eq!(config.server.bind.port(), 3001);
}

#[test]
fn environment_overrides_file_and_preserves_other_values() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("veruna.toml");
    fs::write(&path, "[server]\nbind = '127.0.0.1:9090'\n[database]\npath = 'custom/db.sqlite'\nmax_connections = 2\n").unwrap();
    let environment = HashMap::from([
        ("SERVER__BIND".into(), "127.0.0.1:8181".into()),
        ("SERVER__AUTO_PORT".into(), "false".into()),
        ("SERVER__OPEN_BROWSER".into(), "false".into()),
        ("DATABASE__MAX_CONNECTIONS".into(), "3".into()),
        ("CONFIG".into(), "ignored-by-field-loader.toml".into()),
    ]);
    let config = AppConfig::from_sources(Some(&path), environment).unwrap();
    assert_eq!(config.server.bind.port(), 8181);
    assert!(!config.server.auto_port);
    assert!(!config.server.open_browser);
    assert_eq!(config.database.max_connections, 3);
    assert_eq!(config.database.path.to_str(), Some("custom/db.sqlite"));
}

#[test]
fn invalid_or_misspelled_settings_fail_early() {
    for (key, value) in [
        ("SERVER__BIND", "invalid"),
        ("SERVER__AUTO_PORT", "invalid"),
        ("SERVER__OPEN_BROWSER", "invalid"),
        ("DOWNLOAD__PUBLIC_BASE_URL", "https://user:pass@evil.test"),
        ("DOWNLOAD__PUBLIC_BASE_URL", "http://evil.test"),
        (
            "DOWNLOAD__PUBLIC_BASE_URL",
            "https://apps.example.test/path",
        ),
        ("DOWNLOAD__PUBLIC_BASE_URL", "https://apps.example.test/"),
        ("DOWNLOAD__PUBLIC_BASE_URL", "https://apps.example.test?x=1"),
        ("DOWNLOAD__DELIVERY_MODE", "invalid"),
        ("DOWNLOAD__MAX_CONCURRENT_DOWNLOADS", "0"),
        ("STORAGE__MAX_CONCURRENT_UPLOADS", "0"),
        ("STORAGE__MAX_CONCURRENT_UPLOADS", "65"),
        ("SERVER__SHUTDOWN_TIMEOUT_SECONDS", "0"),
        ("DATABASE__MAX_CONNECTIONS", "0"),
        ("DATABASE__BUSY_TIMEOUT_SECONDS", "0"),
        ("DATABASE__PATH", ""),
        ("STORAGE__DIRECTORY", ""),
        ("STORAGE__ICONS_DIRECTORY", ""),
        ("STORAGE__TEMPORARY_DIRECTORY", ""),
        ("STORAGE__MAX_FILE_BYTES", "0"),
        ("STORAGE__MAX_FILE_BYTES", "9223372036854775808"),
        ("STORAGE__MAX_REQUEST_BYTES", "100"),
        ("STORAGE__UPLOAD_IDLE_SECONDS", "0"),
        ("STORAGE__UPLOAD_IDLE_SECONDS", "86401"),
        ("LOGGING__FILTER", ""),
        ("DATABASE__PAHT", "typo"),
        ("AUTH__PUBLIC_ORIGIN", "http://example.com"),
        ("AUTH__PUBLIC_ORIGIN", "https://example.com/"),
        ("AUTH__PUBLIC_ORIGIN", "https://example.com/path"),
        ("AUTH__PUBLIC_ORIGIN", "https://user:pass@example.com"),
        ("AUTH__SESSION_TTL_SECONDS", "0"),
        ("AUTH__SESSION_TTL_SECONDS", "2592001"),
        ("AUTH__INVITATION_TTL_SECONDS", "604801"),
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
                ("AUTH__PUBLIC_ORIGIN".into(), origin.into()),
                ("AUTH__SESSION_TTL_SECONDS".into(), "600".into()),
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
            ("STORAGE__DIRECTORY".into(), "/disk/releases".into()),
            ("STORAGE__MAX_FILE_BYTES".into(), "4096".into()),
            ("STORAGE__MAX_REQUEST_BYTES".into(), "8192".into()),
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
