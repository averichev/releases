use std::{future::Future, io, path::PathBuf};

/// The storage boundary only exposes operations required by current use cases.
/// Upload, streaming, deletion and opaque artifact keys arrive with those features.
pub trait ArtifactStorage: Send + Sync {
    fn initialize(&self) -> impl Future<Output = io::Result<()>> + Send;
}

pub struct LocalArtifactStorage {
    directory: PathBuf,
}

impl LocalArtifactStorage {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
}

impl ArtifactStorage for LocalArtifactStorage {
    async fn initialize(&self) -> io::Result<()> {
        tokio::fs::create_dir_all(&self.directory).await
    }
}
