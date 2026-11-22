//! Bounded image processing and a separate capability-scoped icon directory.
use super::directory::sync_directory;
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

pub enum IconFormat {
    Png,
    Svg,
}

pub struct StoredIcon {
    pub bytes: Vec<u8>,
    pub format: IconFormat,
}

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
            let (content, extension) = match image::guess_format(&bytes) {
                Ok(format @ (ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)) => {
                    (normalize_raster(bytes, format)?, "png")
                }
                Ok(_) => return Err(ServiceError::InvalidIcon),
                Err(_) => (sanitize_svg(&bytes)?, "svg"),
            };
            let id = uuid::Uuid::new_v4().simple().to_string();
            let key = format!("{id}.{extension}");
            let temporary = format!("{id}.tmp");
            let root = Dir::open_ambient_dir(directory, ambient_authority())?;
            let mut file =
                root.open_with(&temporary, OpenOptions::new().write(true).create_new(true))?;
            file.write_all(&content)?;
            file.sync_all()?;
            drop(file);
            root.rename(&temporary, &root, &key)?;
            sync_directory(&root)?;
            Ok(key)
        })
        .await
    }
    pub async fn read(&self, key: &str) -> Result<StoredIcon> {
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
            Ok(StoredIcon {
                bytes,
                format: if key.ends_with(".svg") {
                    IconFormat::Svg
                } else {
                    IconFormat::Png
                },
            })
        })
        .await
    }
}
fn valid_key(key: &str) -> bool {
    key.len() == 36
        && (key.ends_with(".png") || key.ends_with(".svg"))
        && key
            .bytes()
            .take(32)
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn normalize_raster(bytes: Vec<u8>, format: ImageFormat) -> Result<Vec<u8>> {
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
    Ok(png.into_inner())
}

fn sanitize_svg(bytes: &[u8]) -> Result<Vec<u8>> {
    use xml::{ParserConfig, reader::XmlEvent};

    // Bound XML complexity before the sanitizer, including discarded elements.
    let parser = ParserConfig::new()
        .allow_multiple_root_elements(false)
        .max_attributes(200)
        .max_name_length(1000)
        .max_attribute_length(MAX_ICON_BYTES)
        .max_data_length(MAX_ICON_BYTES)
        .max_entity_expansion_depth(0)
        .max_entity_expansion_length(MAX_ICON_BYTES)
        .create_reader(bytes);
    let mut depth = 0;
    for (count, event) in parser.into_iter().enumerate() {
        if count >= 100_000 {
            return Err(ServiceError::InvalidIcon);
        }
        match event.map_err(|_| ServiceError::InvalidIcon)? {
            XmlEvent::StartElement { name, .. } => {
                if depth == 0
                    && (name.local_name != "svg"
                        || name.namespace.as_deref() != Some("http://www.w3.org/2000/svg"))
                {
                    return Err(ServiceError::InvalidIcon);
                }
                depth += 1;
                if depth > 64 {
                    return Err(ServiceError::InvalidIcon);
                }
            }
            XmlEvent::EndElement { .. } => depth -= 1,
            XmlEvent::Doctype { .. } => return Err(ServiceError::InvalidIcon),
            _ => {}
        }
    }
    // A fixed-size writer also bounds expansion caused by escaping/indentation.
    let mut output = vec![0; MAX_STORED_BYTES as usize];
    let mut destination = Cursor::new(output.as_mut_slice());
    let mut filter = svg_hush::Filter::new();
    filter.set_data_url_filter(svg_hush::data_url_filter::allow_standard_images);
    filter
        .filter(bytes, &mut destination)
        .map_err(|_| ServiceError::InvalidIcon)?;
    let length = destination.position() as usize;
    output.truncate(length);
    Ok(output)
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
