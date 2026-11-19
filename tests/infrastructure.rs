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
    assert_eq!(migrations, 8);
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
        8
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

#[tokio::test]
async fn append_migration_preserves_published_signed_artifacts_and_channel_links() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use veruna_releases::{
        application_state,
        domain::{
            application::{NewApplication, Visibility},
            artifact::UploadMetadata,
            release::ReleaseInput,
        },
        services::publishing::PublicationCommand,
    };
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = dir.path().join("existing.sqlite");
    config.storage.directory = dir.path().join("artifacts");
    config.storage.icons_directory = dir.path().join("icons");
    let old = dir.path().join("old");
    std::fs::create_dir(&old).unwrap();
    for entry in std::fs::read_dir("migrations").unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_str().unwrap() < "0008" {
            std::fs::copy(entry.path(), old.join(entry.file_name())).unwrap();
        }
    }
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&config.database.path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    sqlx::migrate::Migrator::new(old.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    use veruna_releases::infrastructure::storage::{ArtifactStorage, LocalArtifactStorage};
    LocalArtifactStorage::new(&config.storage)
        .initialize()
        .await
        .unwrap();
    let state = application_state(pool.clone(), &config).await.unwrap();
    state
        .authentication
        .bootstrap("owner", "test-password-123".into())
        .await
        .unwrap();
    let login = state
        .authentication
        .login("owner", "test-password-123".into(), None)
        .await
        .unwrap();
    let actor = state.sessions.authenticate(&login.token).await.unwrap();
    let app = state
        .applications
        .create(
            &actor,
            NewApplication {
                slug: "product".into(),
                name: "Product".into(),
                description: "".into(),
                visibility: Visibility::Public,
            },
        )
        .await
        .unwrap();
    let r = state
        .releases
        .create(
            &actor,
            &app.id,
            ReleaseInput {
                version: "0.5.0+ci.7".into(),
                title: Some("Preserve title".into()),
                release_notes: "Preserve notes".into(),
            },
        )
        .await
        .unwrap();
    let mut packet = [0u8; 74];
    packet[..2].copy_from_slice(b"Ed");
    let sig = STANDARD.encode(format!(
        "untrusted comment: signature\n{}\ntrusted comment: timestamp:1\n{}\n",
        STANDARD.encode(packet),
        STANDARD.encode([0u8; 64])
    ));
    let mut pending = state
        .artifacts
        .begin(
            &actor,
            &app.id,
            &r.id,
            UploadMetadata {
                filename: "updater.exe".into(),
                target: "windows-x86_64".into(),
                file_type: None,
                distribution_role: Some("updater".into()),
            },
        )
        .await
        .unwrap();
    pending.set_signature(sig.as_bytes()).unwrap();
    pending.write(b"package").await.unwrap();
    let a = state.artifacts.complete(&actor, pending).await.unwrap();
    state
        .publishing
        .publish(
            &actor,
            &app.id,
            &r.id,
            PublicationCommand {
                channel: "stable".into(),
                expected_channel_revision: 0,
                expected_release_revision: 1,
            },
        )
        .await
        .unwrap();
    let before = state.releases.get(&app.id, &r.id).await.unwrap();
    let links: Vec<(String, String, i64, String, String, i64)> = sqlx::query_as("SELECT c.id,c.current_release_id,c.revision,cr.release_id,cr.application_id,cr.added_at FROM release_channels c JOIN channel_releases cr ON cr.channel_id=c.id").fetch_all(&pool).await.unwrap();
    let history: Vec<(i64, String, String, String, String, String, i64)> = sqlx::query_as("SELECT id,application_id,channel_id,release_id,action,user_id,created_at FROM publication_history").fetch_all(&pool).await.unwrap();
    drop(state);
    pool.close().await;
    let pool = initialize(&config).await.unwrap();
    let state = application_state(pool.clone(), &config).await.unwrap();
    assert_eq!(state.releases.get(&app.id, &r.id).await.unwrap(), before);
    assert_eq!(
        state.artifacts.list(&app.id, &r.id).await.unwrap(),
        vec![a.clone()]
    );
    assert_eq!(
        std::fs::read(config.storage.directory.join(&a.storage_key)).unwrap(),
        b"package"
    );
    assert_eq!(sqlx::query_as::<_, (String, String, i64, String, String, i64)>("SELECT c.id,c.current_release_id,c.revision,cr.release_id,cr.application_id,cr.added_at FROM release_channels c JOIN channel_releases cr ON cr.channel_id=c.id").fetch_all(&pool).await.unwrap(), links);
    assert_eq!(sqlx::query_as::<_, (i64, String, String, String, String, String, i64)>("SELECT id,application_id,channel_id,release_id,action,user_id,created_at FROM publication_history").fetch_all(&pool).await.unwrap(), history);
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    drop(state);
    pool.close().await;
    let pool = initialize(&config).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        8
    );
    pool.close().await;
}

#[tokio::test]
async fn append_migration_rejects_legacy_updater_duplicates_without_losing_data() {
    use sqlx::Connection;
    let mut c = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    for sql in [
        include_str!("../migrations/0001_baseline.sql"),
        include_str!("../migrations/0002_identity.sql"),
        include_str!("../migrations/0003_applications.sql"),
        include_str!("../migrations/0004_releases_artifacts.sql"),
        include_str!("../migrations/0005_publication.sql"),
        include_str!("../migrations/0006_distribution.sql"),
        include_str!("../migrations/0007_tauri_integration.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&mut c).await.unwrap();
    }
    let app = uuid::Uuid::new_v4().to_string();
    let release = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO applications(id,slug,name,name_search,description,visibility,status,created_at,updated_at) VALUES(?,'product','Product','product','','public','active',1,1)").bind(&app).execute(&mut c).await.unwrap();
    sqlx::query("INSERT INTO releases(id,application_id,version,version_key,created_at,updated_at) VALUES(?,?,'0.5.0','0.5.0',1,1)").bind(&release).bind(&app).execute(&mut c).await.unwrap();
    for format in ["exe", "msi"] {
        sqlx::query("INSERT INTO artifacts(id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at,distribution_role,updater_signature) VALUES(?,?,'update.exe','windows-x86_64',?,7,?,?,1,'updater','preserve-signature')")
            .bind(uuid::Uuid::new_v4().to_string()).bind(&release).bind(format).bind("a".repeat(64)).bind(format!("{}.blob", uuid::Uuid::new_v4().simple())).execute(&mut c).await.unwrap();
    }
    let before: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id,storage_key,updater_signature FROM artifacts ORDER BY id")
            .fetch_all(&mut c)
            .await
            .unwrap();
    let mut tx = c.begin().await.unwrap();
    assert!(
        sqlx::raw_sql(include_str!(
            "../migrations/0008_artifact_roles_and_append.sql"
        ))
        .execute(&mut *tx)
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT id,storage_key,updater_signature FROM artifacts ORDER BY id"
        )
        .fetch_all(&mut c)
        .await
        .unwrap(),
        before
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM sqlite_schema WHERE name='artifacts_next'"
        )
        .fetch_one(&mut c)
        .await
        .unwrap(),
        0
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut c)
            .await
            .unwrap()
            .is_empty()
    );
}
