use tempfile::tempdir;
use veruna_releases::{config::AppConfig, initialize};

#[tokio::test]
async fn svg_icon_migration_preserves_existing_icons_relations_and_identity_guards() {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = dir.path().join("existing.sqlite");
    config.storage.directory = dir.path().join("artifacts");
    config.storage.icons_directory = dir.path().join("icons");
    let mut old = sqlx::migrate!("./migrations");
    old.migrations = old
        .iter()
        .filter(|m| m.version < 14)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&config.database.path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    old.run(&pool).await.unwrap();
    let app = uuid::Uuid::new_v4().to_string();
    let release = uuid::Uuid::new_v4().to_string();
    let key = format!("{}.png", uuid::Uuid::new_v4().simple());
    sqlx::query("INSERT INTO applications(id,slug,name,name_search,icon_key,created_at,updated_at) VALUES(?,'existing','Existing','existing',?,1,1)")
        .bind(&app).bind(&key).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO releases(id,application_id,version,version_key,created_at,updated_at) VALUES(?,?,'1.0.0','1.0.0',1,1)")
        .bind(&release).bind(&app).execute(&pool).await.unwrap();
    let guards: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL ORDER BY name")
        .fetch_all(&pool).await.unwrap();
    pool.close().await;
    let pool = initialize(&config).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT icon_key FROM applications WHERE id=?")
            .bind(&app)
            .fetch_one(&pool)
            .await
            .unwrap(),
        key
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT application_id FROM releases WHERE id=?")
            .bind(&release)
            .fetch_one(&pool)
            .await
            .unwrap(),
        app
    );
    let after: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL ORDER BY name").fetch_all(&pool).await.unwrap();
    assert!(guards.iter().all(|guard| after.contains(guard)));
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    for statement in [
        "UPDATE applications SET slug='changed'",
        "UPDATE applications SET id='changed'",
        "UPDATE applications SET created_at=0",
    ] {
        assert!(sqlx::query(statement).execute(&pool).await.is_err());
    }
    for key in [
        "../escape.svg",
        "bad.svg",
        "00000000000000000000000000000000.gif",
    ] {
        assert!(
            sqlx::query("UPDATE applications SET icon_key=?")
                .bind(key)
                .execute(&pool)
                .await
                .is_err()
        );
    }
    let svg_key = format!("{}.svg", uuid::Uuid::new_v4().simple());
    sqlx::query("UPDATE applications SET icon_key=?")
        .bind(&svg_key)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let pool = initialize(&config).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT icon_key FROM applications")
            .fetch_one(&pool)
            .await
            .unwrap(),
        svg_key
    );
}
