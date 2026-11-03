use tempfile::tempdir;
use veruna_releases::{config::AppConfig, initialize};

#[tokio::test]
async fn creates_configured_paths_and_applies_baseline_once() {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = dir.path().join("database/nested/releases.sqlite");
    config.storage.directory = dir.path().join("storage/nested/artifacts");
    let pool = initialize(&config).await.unwrap();
    assert!(config.database.path.is_file());
    assert!(config.storage.directory.is_dir());

    let mut first = pool.acquire().await.unwrap();
    let mut second = pool.acquire().await.unwrap();
    for connection in [&mut first, &mut second] {
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut **connection)
            .await
            .unwrap();
        let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut **connection)
            .await
            .unwrap();
        let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&mut **connection)
            .await
            .unwrap();
        assert_eq!(foreign_keys, 1);
        assert_eq!(journal, "wal");
        assert_eq!(busy_timeout, 5000);
    }
    drop(first);
    drop(second);
    let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(tables, vec!["_sqlx_migrations"]);
    pool.close().await;

    let pool = initialize(&config).await.unwrap();
    let migrations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(migrations, 1);
    pool.close().await;
}

#[tokio::test]
async fn storage_path_cannot_be_an_existing_file() {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = dir.path().join("database.sqlite");
    config.storage.directory = dir.path().join("not-a-directory");
    std::fs::write(&config.storage.directory, "existing data").unwrap();
    assert!(initialize(&config).await.is_err());
    assert_eq!(
        std::fs::read_to_string(&config.storage.directory).unwrap(),
        "existing data"
    );
}
