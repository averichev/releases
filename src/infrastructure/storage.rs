//! Local, capability-scoped artifact I/O. No user filename is ever a disk path.
use crate::{
    config::StorageConfig,
    services::error::{Result, ServiceError},
};
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, future::Future, io, path::PathBuf, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, Semaphore},
};

pub trait ArtifactStorage: Send + Sync {
    fn initialize(&self) -> impl Future<Output = Result<()>> + Send;
    fn reconcile(&self, referenced: Vec<(String, i64)>) -> impl Future<Output = Result<()>> + Send;
    fn begin(&self) -> impl Future<Output = Result<PendingUpload>> + Send;
    fn promote(&self, upload: &mut PendingUpload) -> impl Future<Output = Result<()>> + Send;
    fn open(&self, key: &str, size: i64) -> impl Future<Output = Result<tokio::fs::File>> + Send;
    fn remove(&self, key: &str) -> impl Future<Output = Result<()>> + Send;
    fn verify(
        &self,
        key: &str,
        size: i64,
        sha256: &str,
    ) -> impl Future<Output = Result<bool>> + Send;
}

#[derive(Clone)]
pub struct LocalArtifactStorage {
    directory: PathBuf,
    temporary: PathBuf,
    icons: PathBuf,
    max_bytes: u64,
    workers: Arc<Semaphore>,
}
impl LocalArtifactStorage {
    pub fn new(config: &StorageConfig) -> Self {
        Self {
            directory: config.directory.clone(),
            temporary: config.temporary_directory(),
            icons: config.icons_directory.clone(),
            max_bytes: config.max_file_bytes,
            workers: Arc::new(Semaphore::new(4)),
        }
    }
}

// Owns compensation even if the request is cancelled. Process crashes are
// handled by reconciliation before the listener opens.
struct Cleanup {
    temporary: Arc<Dir>,
    root: Arc<Dir>,
    temporary_key: String,
    permanent_key: Option<String>,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let temp = self.temporary.clone();
        let root = self.root.clone();
        let temporary_key = self.temporary_key.clone();
        let permanent = self.permanent_key.take();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(move || {
                for (dir, key) in
                    std::iter::once((temp, temporary_key)).chain(permanent.map(|key| (root, key)))
                {
                    if let Err(error) = dir.remove_file(key)
                        && error.kind() != io::ErrorKind::NotFound
                    {
                        tracing::warn!(error=?error,"Artifact compensation deferred until startup");
                    }
                }
            });
        }
    }
}
pub struct PendingUpload {
    file: Option<tokio::fs::File>,
    cleanup: Cleanup,
    _permit: OwnedSemaphorePermit,
    max_bytes: u64,
    hasher: Sha256,
    pub(crate) size_bytes: u64,
    pub(crate) key: String,
}
impl PendingUpload {
    pub async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let size = self
            .size_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(ServiceError::PayloadTooLarge)?;
        if size > self.max_bytes {
            return Err(ServiceError::PayloadTooLarge);
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| ServiceError::Internal(anyhow::anyhow!("Upload already sealed")))?;
        for chunk in bytes.chunks(64 * 1024) {
            file.write_all(chunk).await.map_err(storage_error)?;
            self.hasher.update(chunk);
        }
        self.size_bytes = size;
        Ok(())
    }
    pub fn storage_key(&self) -> &str {
        &self.key
    }
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
    pub fn sha256(&self) -> String {
        format!("{:x}", self.hasher.clone().finalize())
    }
    /// Call BEFORE awaiting commit: commit/cancellation may have an unknown result.
    pub fn preserve_on_commit(&mut self) {
        self.cleanup.permanent_key = None;
    }
}
impl ArtifactStorage for LocalArtifactStorage {
    async fn initialize(&self) -> Result<()> {
        let directory = self.directory.clone();
        let temporary = self.temporary.clone();
        let icons = self.icons.clone();
        blocking(move || {
            std::fs::create_dir_all(&directory)?; std::fs::create_dir_all(&temporary)?; std::fs::create_dir_all(&icons)?;
            let root=directory.canonicalize()?; let temp=temporary.canonicalize()?; let icons=icons.canonicalize()?;
            if root==temp || root.starts_with(&temp) || icons.starts_with(&root) || root.starts_with(&icons) || icons.starts_with(&temp) || temp.starts_with(&icons) {
                return Err(ServiceError::Internal(anyhow::anyhow!("Artifact, temporary and icon directories overlap incompatibly")));
            }
            let root=Dir::open_ambient_dir(root,ambient_authority())?; let temp=Dir::open_ambient_dir(temp,ambient_authority())?;
            // A no-overwrite atomic link requires one filesystem. Verify at startup.
            let probe=format!("{}.tmp",uuid::Uuid::new_v4().simple());
            let file=temp.open_with(&probe,OpenOptions::new().write(true).create_new(true))?;
            drop(file);
            let result=temp.hard_link(&probe,&root,&probe);
            let _=temp.remove_file(&probe); let _=root.remove_file(&probe);
            result.map_err(|e| ServiceError::Internal(anyhow::anyhow!("Artifact temporary and permanent storage must support hard links on one filesystem: {e}")))?;
            Ok(())
        }).await
    }
    async fn reconcile(&self, referenced: Vec<(String, i64)>) -> Result<()> {
        let directory = self.directory.clone();
        let temporary = self.temporary.clone();
        blocking(move || {
            let root = Dir::open_ambient_dir(directory, ambient_authority())?;
            let temp = Dir::open_ambient_dir(temporary, ambient_authority())?;
            let referenced: HashMap<_, _> = referenced.into_iter().collect();
            // Validate every reference BEFORE reclaiming anything. Missing/truncated
            // files stop startup and require restoring a consistent backup.
            for (key, size) in &referenced {
                let file = open_object(&root, key).map_err(|_| ServiceError::StorageUnavailable)?;
                if file.metadata()?.len() != *size as u64 {
                    return Err(ServiceError::StorageUnavailable);
                }
            }
            for entry in temp.entries()? {
                let entry = entry?;
                if let Some(key) = entry.file_name().to_str()
                    && valid_temporary(key)
                {
                    temp.remove_file(key)?;
                }
            }
            for entry in root.entries()? {
                let entry = entry?;
                if let Some(key) = entry.file_name().to_str()
                    && ((valid_key(key) && !referenced.contains_key(key)) || valid_temporary(key))
                {
                    root.remove_file(key)?;
                }
            }
            root.into_std_file().sync_all()?;
            temp.into_std_file().sync_all()?;
            Ok(())
        })
        .await
    }
    async fn begin(&self) -> Result<PendingUpload> {
        let permit = self
            .workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| ServiceError::StorageBusy)?;
        let directory = self.directory.clone();
        let temporary = self.temporary.clone();
        let max_bytes = self.max_bytes;
        blocking(move || {
            let root = Arc::new(Dir::open_ambient_dir(directory, ambient_authority())?);
            let temporary = Arc::new(Dir::open_ambient_dir(temporary, ambient_authority())?);
            let id = uuid::Uuid::new_v4().simple().to_string();
            let key = format!("{id}.blob");
            let temporary_key = format!("{id}.tmp");
            let file = temporary.open_with(
                &temporary_key,
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .follow(FollowSymlinks::No),
            )?;
            // Compensation owns only a file whose create_new succeeded.
            let cleanup = Cleanup {
                temporary: temporary.clone(),
                root,
                temporary_key: temporary_key.clone(),
                permanent_key: None,
            };
            Ok(PendingUpload {
                file: Some(tokio::fs::File::from_std(file.into_std())),
                cleanup,
                _permit: permit,
                max_bytes,
                hasher: Sha256::new(),
                size_bytes: 0,
                key,
            })
        })
        .await
    }
    async fn promote(&self, upload: &mut PendingUpload) -> Result<()> {
        if upload.size_bytes == 0 {
            return Err(ServiceError::InvalidArtifact);
        }
        let mut file = upload
            .file
            .take()
            .ok_or_else(|| ServiceError::Internal(anyhow::anyhow!("Upload already promoted")))?;
        file.flush().await.map_err(storage_error)?;
        file.sync_all().await.map_err(storage_error)?;
        drop(file);
        let temp = upload.cleanup.temporary.clone();
        let root = upload.cleanup.root.clone();
        let from = upload.cleanup.temporary_key.clone();
        let key = upload.key.clone();
        // Install without replacing an existing name. Hard-link + unlink is an
        // atomic no-clobber alternative to rename; startup handles either name.
        let cleanup = blocking(move || {
            temp.hard_link(&from, &root, &key).map_err(storage_error)?;
            // Return a guard in the blocking result, so cancellation cleans only
            // after the operation completes, never racing a late link.
            let guard = Cleanup {
                temporary: temp.clone(),
                root: root.clone(),
                temporary_key: from.clone(),
                permanent_key: Some(key),
            };
            temp.remove_file(from)?;
            root.try_clone()?.into_std_file().sync_all()?;
            temp.try_clone()?.into_std_file().sync_all()?;
            Ok(guard)
        })
        .await?;
        upload.cleanup = cleanup;
        Ok(())
    }
    async fn open(&self, key: &str, size: i64) -> Result<tokio::fs::File> {
        let directory = self.directory.clone();
        let key = key.to_owned();
        blocking(move || {
            let root = Dir::open_ambient_dir(directory, ambient_authority())?;
            let file = open_object(&root, &key).map_err(|_| ServiceError::StorageUnavailable)?;
            if file.metadata()?.len() != size as u64 {
                return Err(ServiceError::StorageUnavailable);
            }
            Ok(tokio::fs::File::from_std(file.into_std()))
        })
        .await
    }
    async fn remove(&self, key: &str) -> Result<()> {
        if !valid_key(key) {
            return Err(ServiceError::InvalidArtifact);
        }
        let directory = self.directory.clone();
        let key = key.to_owned();
        blocking(move || {
            let root = Dir::open_ambient_dir(directory, ambient_authority())?;
            match root.remove_file(key) {
                Ok(()) => root.into_std_file().sync_all()?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            Ok(())
        })
        .await
    }
    async fn verify(&self, key: &str, size: i64, sha256: &str) -> Result<bool> {
        let _permit = self
            .workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| ServiceError::StorageBusy)?;
        let mut file = self.open(key, size).await?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 64 * 1024];
        let mut total = 0u64;
        loop {
            let count = file.read(&mut buffer).await.map_err(storage_error)?;
            if count == 0 {
                break;
            }
            total += count as u64;
            hash.update(&buffer[..count]);
        }
        Ok(total == size as u64 && format!("{:x}", hash.finalize()) == sha256)
    }
}
fn open_object(root: &Dir, key: &str) -> io::Result<cap_std::fs::File> {
    if !valid_key(key) {
        return Err(io::Error::other("Invalid artifact key"));
    }
    if !root.symlink_metadata(key)?.is_file() {
        return Err(io::Error::other("Artifact is not a regular file"));
    }
    let file = root.open_with(
        key,
        OpenOptions::new().read(true).follow(FollowSymlinks::No),
    )?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("Artifact is not a regular file"));
    }
    Ok(file)
}
fn valid_key(key: &str) -> bool {
    key.len() == 37
        && key.ends_with(".blob")
        && key
            .bytes()
            .take(32)
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_temporary(key: &str) -> bool {
    key.strip_suffix(".tmp")
        .is_some_and(|v| valid_key(&format!("{v}.blob")))
}
fn storage_error(error: io::Error) -> ServiceError {
    if error.kind() == io::ErrorKind::StorageFull {
        ServiceError::StorageFull
    } else {
        error.into()
    }
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ServiceError::Internal(e.into()))?
}
