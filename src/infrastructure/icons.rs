//! Bounded image processing and a separate capability-scoped icon directory.
use crate::services::error::{Result, ServiceError};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use image::{ImageFormat, ImageReader, Limits};
use std::{
    collections::HashSet,
    io::{Cursor, Read, Write},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::Semaphore;

pub const MAX_ICON_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ICON_DIMENSION: u32 = 1024;
const MAX_STORED_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Clone)]
pub struct IconStorage {
    directory: PathBuf,
    workers: Arc<Semaphore>,
}
impl IconStorage {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            workers: Arc::new(Semaphore::new(2)),
        }
    }
    pub async fn initialize(
        &self,
        artifact_directory: PathBuf,
        referenced: Vec<String>,
    ) -> Result<()> {
        let directory = self.directory.clone();
        blocking(move || {
            std::fs::create_dir_all(&directory)?;
            let icons = directory.canonicalize()?;
            let artifacts = artifact_directory.canonicalize()?;
            if icons.starts_with(&artifacts) || artifacts.starts_with(&icons) {
                return Err(
                    anyhow::anyhow!("Icon and artifact directories must be separate").into(),
                );
            }
            let root = Dir::open_ambient_dir(&icons, ambient_authority())?;
            let referenced: HashSet<_> = referenced.into_iter().collect();
            // Only server-generated objects are reclaimed. Run before the listener opens.
            for entry in root.entries()? {
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if (valid_key(name) && !referenced.contains(name)) || valid_temporary(name) {
                    root.remove_file(name)?;
                }
            }
            Ok(())
        })
        .await
    }
    pub async fn store(&self, bytes: Vec<u8>) -> Result<String> {
        if bytes.is_empty() || bytes.len() > MAX_ICON_BYTES {
            return Err(ServiceError::InvalidIcon);
        }
        let permit = self
            .workers
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| ServiceError::Internal(e.into()))?;
        let directory = self.directory.clone();
        blocking(move || {
            let _permit = permit;
            let format = image::guess_format(&bytes).map_err(|_| ServiceError::InvalidIcon)?;
            if !matches!(
                format,
                ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
            ) {
                return Err(ServiceError::InvalidIcon);
            }
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            let mut limits = Limits::default();
            limits.max_image_width = Some(MAX_ICON_DIMENSION);
            limits.max_image_height = Some(MAX_ICON_DIMENSION);
            limits.max_alloc = Some(32 * 1024 * 1024);
            reader.limits(limits);
            let decoded = reader.decode().map_err(|_| ServiceError::InvalidIcon)?;
            if decoded.width() == 0 || decoded.height() == 0 {
                return Err(ServiceError::InvalidIcon);
            }
            // Re-encoding strips untrusted metadata and trailing non-image content.
            let mut png = Cursor::new(Vec::new());
            decoded
                .to_rgba8()
                .write_to(&mut png, ImageFormat::Png)
                .map_err(|e| ServiceError::Internal(e.into()))?;
            let id = uuid::Uuid::new_v4().simple().to_string();
            let key = format!("{id}.png");
            let temporary = format!("{id}.tmp");
            let root = Dir::open_ambient_dir(directory, ambient_authority())?;
            let mut file =
                root.open_with(&temporary, OpenOptions::new().write(true).create_new(true))?;
            file.write_all(png.get_ref())?;
            file.sync_all()?;
            drop(file);
            root.rename(&temporary, &root, &key)?;
            root.into_std_file().sync_all()?;
            Ok(key)
        })
        .await
    }
    pub async fn read(&self, key: &str) -> Result<Vec<u8>> {
        if !valid_key(key) {
            return Err(ServiceError::NotFound);
        }
        let directory = self.directory.clone();
        let key = key.to_owned();
        blocking(move || {
            let root = Dir::open_ambient_dir(directory, ambient_authority())?;
            // Capability lookup prevents traversal and symlinks escaping the root.
            if root.symlink_metadata(&key)?.file_type().is_symlink() {
                return Err(ServiceError::NotFound);
            }
            let file = root.open(&key)?;
            if !file.metadata()?.is_file() {
                return Err(ServiceError::NotFound);
            }
            let mut bytes = Vec::new();
            file.take(MAX_STORED_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_STORED_BYTES {
                return Err(ServiceError::InvalidIcon);
            }
            Ok(bytes)
        })
        .await
    }
}
fn valid_key(key: &str) -> bool {
    key.len() == 36
        && key.ends_with(".png")
        && key
            .bytes()
            .take(32)
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_temporary(key: &str) -> bool {
    key.strip_suffix(".tmp")
        .is_some_and(|stem| valid_key(&format!("{stem}.png")))
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ServiceError::Internal(e.into()))?
}
