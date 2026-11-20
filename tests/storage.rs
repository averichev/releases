use tempfile::tempdir;
use tokio::io::AsyncReadExt;
use veruna_releases::{
    config::AppConfig,
    infrastructure::storage::{ArtifactStorage, LocalArtifactStorage},
};

// Exercise directory synchronization without SQLite or HTTP: on Linux the
// original O_PATH directory handles failed at the first reconciliation fsync.
#[tokio::test]
async fn storage_initialization_and_durable_file_lifecycle() {
    for separate_temporary in [false, true] {
        let directory = tempdir().unwrap();
        let mut config = AppConfig::default().storage;
        config.directory = directory.path().join("artifacts");
        config.icons_directory = directory.path().join("icons");
        if separate_temporary {
            config.temporary_directory = Some(directory.path().join("uploads"));
        }
        let storage = LocalArtifactStorage::new(&config);
        storage.initialize().await.unwrap();
        storage.reconcile(Vec::new()).await.unwrap();

        let mut upload = storage.begin().await.unwrap();
        upload.write(b"durable artifact").await.unwrap();
        storage.promote(&mut upload).await.unwrap();
        let key = upload.storage_key().to_owned();
        let size = upload.size_bytes() as i64;
        let sha256 = upload.sha256();
        upload.preserve_on_commit();
        drop(upload);

        // Simulate remnants of a process interrupted during a different upload.
        let partial = config
            .temporary_directory()
            .join(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
        std::fs::write(&partial, b"partial").unwrap();
        let orphan = config
            .directory
            .join(format!("{}.blob", uuid::Uuid::new_v4().simple()));
        std::fs::write(&orphan, b"uncommitted").unwrap();
        let restarted = LocalArtifactStorage::new(&config);
        restarted.initialize().await.unwrap();
        restarted
            .reconcile(vec![(key.clone(), size)])
            .await
            .unwrap();
        assert!(!partial.exists());
        assert!(!orphan.exists());
        assert!(restarted.verify(&key, size, &sha256).await.unwrap());
        let mut bytes = Vec::new();
        restarted
            .open(&key, size)
            .await
            .unwrap()
            .read_to_end(&mut bytes)
            .await
            .unwrap();
        assert_eq!(bytes, b"durable artifact");

        restarted.remove(&key).await.unwrap();
        assert!(!config.directory.join(&key).exists());
        restarted.reconcile(Vec::new()).await.unwrap();
    }
}
