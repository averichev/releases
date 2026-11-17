use tempfile::tempdir;
use veruna_releases::{config::AppConfig, initialize};

#[tokio::test]
async fn creates_configured_paths_and_applies_baseline_once() {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = dir.path().join("database/nested/releases.sqlite");
    config.storage.directory = dir.path().join("storage/nested/artifacts");
    config.storage.icons_directory = dir.path().join("icons");
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
    assert_eq!(
        tables,
        vec![
            "_sqlx_migrations",
            "application_distribution",
            "applications",
            "artifacts",
            "channel_releases",
            "login_attempts",
            "publication_history",
            "release_channels",
            "releases",
            "sessions",
            "user_tokens",
            "users"
        ]
    );
    pool.close().await;

    let pool = initialize(&config).await.unwrap();
    let migrations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(migrations, 7);
    pool.close().await;
}

#[tokio::test]
async fn storage_path_cannot_be_an_existing_file() {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = dir.path().join("database.sqlite");
    config.storage.directory = dir.path().join("not-a-directory");
    config.storage.icons_directory = dir.path().join("icons");
    std::fs::write(&config.storage.directory, "existing data").unwrap();
    assert!(initialize(&config).await.is_err());
    assert_eq!(
        std::fs::read_to_string(&config.storage.directory).unwrap(),
        "existing data"
    );
}

#[tokio::test]
async fn catalog_migration_preserves_an_existing_identity_database() {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = dir.path().join("existing.sqlite");
    config.storage.directory = dir.path().join("artifacts");
    config.storage.icons_directory = dir.path().join("icons");
    let migrations = dir.path().join("old-migrations");
    std::fs::create_dir(&migrations).unwrap();
    for filename in ["0001_baseline.sql", "0002_identity.sql"] {
        std::fs::copy(
            std::path::Path::new("migrations").join(filename),
            migrations.join(filename),
        )
        .unwrap();
    }
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&config.database.path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::migrate::Migrator::new(migrations.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(id,username,password_hash,role,status,created_at,updated_at) VALUES('existing-id','existing',NULL,'operator','invited',1,1)").execute(&pool).await.unwrap();
    pool.close().await;
    let pool = initialize(&config).await.unwrap();
    let existing: String = sqlx::query_scalar("SELECT username FROM users WHERE id='existing-id'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(existing, "existing");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM applications")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _sqlx_migrations WHERE success=1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        7
    );
}

#[tokio::test]
async fn publication_migration_preserves_stage_four_objects_and_backfills_channels() {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = dir.path().join("existing.sqlite");
    config.storage.directory = dir.path().join("artifacts");
    config.storage.icons_directory = dir.path().join("icons");
    let migrations = dir.path().join("stage-four");
    std::fs::create_dir(&migrations).unwrap();
    for filename in [
        "0001_baseline.sql",
        "0002_identity.sql",
        "0003_applications.sql",
        "0004_releases_artifacts.sql",
    ] {
        std::fs::copy(
            std::path::Path::new("migrations").join(filename),
            migrations.join(filename),
        )
        .unwrap();
    }
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&config.database.path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    sqlx::migrate::Migrator::new(migrations.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    let app = uuid::Uuid::new_v4().to_string();
    let release = uuid::Uuid::new_v4().to_string();
    let artifact = uuid::Uuid::new_v4().to_string();
    let key = format!("{}.blob", uuid::Uuid::new_v4().simple());
    sqlx::query("INSERT INTO applications(id,slug,name,name_search,description,visibility,status,created_at,updated_at) VALUES(?,'existing','Existing','existing','','public','active',1,1)").bind(&app).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO releases(id,application_id,version,version_key,release_notes,created_at,updated_at) VALUES(?,?,'1.2.0+ci.1','1.2.0','Preserve notes',1,1)").bind(&release).bind(&app).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO artifacts(id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at) VALUES(?,?,'setup.bin','any','bin',3,'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',?,1)").bind(&artifact).bind(&release).bind(&key).execute(&pool).await.unwrap();
    std::fs::create_dir_all(&config.storage.directory).unwrap();
    std::fs::write(config.storage.directory.join(&key), b"abc").unwrap();
    pool.close().await;
    let pool = initialize(&config).await.unwrap();
    let r = veruna_releases::repositories::release::ReleasesRepository::new(pool.clone())
        .get(&app, &release)
        .await
        .unwrap();
    assert_eq!(r.version.to_string(), "1.2.0+ci.1");
    assert_eq!(r.release_notes, "Preserve notes");
    assert_eq!(r.artifact_count, 1);
    let a = veruna_releases::repositories::artifact::ArtifactsRepository::new(pool.clone())
        .get(&app, &release, &artifact)
        .await
        .unwrap();
    assert_eq!(a.storage_key, key);
    assert_eq!(a.distribution_role, "installer");
    assert!(a.updater_signature.is_none());
    assert_eq!(
        std::fs::read(config.storage.directory.join(&key)).unwrap(),
        b"abc"
    );
    let channels = veruna_releases::repositories::channel::ChannelsRepository::new(pool.clone())
        .list(&app)
        .await
        .unwrap();
    assert_eq!(channels.len(), 2);
    let public_key: Option<String> =
        sqlx::query_scalar("SELECT tauri_public_key FROM applications WHERE id=?")
            .bind(&app)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(public_key.is_none());
    for c in channels {
        assert!(uuid::Uuid::parse_str(&c.id).is_ok());
        assert_eq!(c.release_count, 0);
    }
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    pool.close().await;
    let pool = initialize(&config).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM release_channels")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    pool.close().await;
}
