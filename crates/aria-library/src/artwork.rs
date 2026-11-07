//! The artwork thumbnail cache and folder-image lookup.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use sha2::{Digest, Sha256};

use crate::{ArtworkId, LibraryError};

/// Longest side of a stored thumbnail, in pixels.
pub const THUMB_SIZE: u32 = 256;
/// Images wider or taller than this are refused.
pub const MAX_IMAGE_SIDE: u32 = 8192;
/// Source images larger than this are refused before any parsing.
pub const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
/// JPEG quality of stored thumbnails.
pub const THUMB_QUALITY: u8 = 85;

/// Ceiling for decoder allocations, so a lying header cannot exhaust memory.
const MAX_DECODE_ALLOC: u64 = 256 * 1024 * 1024;

/// A directory of `<id>.jpg` thumbnails.
#[derive(Debug, Clone)]
pub struct ArtworkCache {
    dir: PathBuf,
}

impl ArtworkCache {
    /// A cache in `dir`; the directory is created on the first store.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Makes a thumbnail of `bytes` and returns its id.
    ///
    /// Images that are too big, damaged or in an unsupported format give `Ok(None)`;
    /// only disk failures are errors.
    pub fn store(&self, bytes: &[u8]) -> Result<Option<ArtworkId>, LibraryError> {
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            tracing::debug!(len = bytes.len(), "image refused: size");
            return Ok(None);
        }
        let id = id_of(bytes);
        let target = self.path_of(&id);
        if target.is_file() {
            return Ok(Some(id));
        }
        let Some(jpeg) = make_thumbnail(bytes) else {
            return Ok(None);
        };
        fs::create_dir_all(&self.dir)?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".thumb-")
            .suffix(".tmp")
            .tempfile_in(&self.dir)?;
        tmp.write_all(&jpeg)?;
        tmp.persist(&target)
            .map_err(|e| LibraryError::Io(e.error))?;
        Ok(Some(id))
    }

    /// Where the thumbnail with this id lives (it may not exist).
    pub fn path_of(&self, id: &ArtworkId) -> PathBuf {
        self.dir.join(format!("{}.jpg", id.0))
    }

    /// Removes every `.jpg` file and stray `.thumb-*.tmp` file in the cache directory. A missing directory is fine.
    pub fn clear(&self) -> Result<(), LibraryError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_jpg = path.extension().is_some_and(|e| e == "jpg");
            let is_stray_temp = name.starts_with(".thumb-") && name.ends_with(".tmp");
            if (is_jpg || is_stray_temp) && entry.file_type()?.is_file() {
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(())
    }
}

fn id_of(bytes: &[u8]) -> ArtworkId {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(32);
    for b in &digest[..16] {
        // Writing to a String cannot fail.
        let _ = write!(hex, "{b:02x}");
    }
    ArtworkId(Arc::from(hex))
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    limits
}

fn reader(bytes: &[u8]) -> Option<ImageReader<Cursor<&[u8]>>> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.limits(limits());
    match reader.format() {
        Some(ImageFormat::Jpeg | ImageFormat::Png) => Some(reader),
        other => {
            tracing::debug!(format = ?other, "image refused: unsupported format");
            None
        }
    }
}

/// Decodes and shrinks the image to a JPEG, or `None` when it cannot be used.
fn make_thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = reader(bytes)?
        .into_dimensions()
        .inspect_err(|e| tracing::debug!(error = %e, "image refused: header"))
        .ok()?;
    if w == 0 || h == 0 || w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE {
        tracing::debug!(w, h, "image refused: dimensions");
        return None;
    }
    let decoded = reader(bytes)?
        .decode()
        .inspect_err(|e| tracing::debug!(error = %e, "image refused: decode"))
        .ok()?;
    let shrunk = if decoded.width() > THUMB_SIZE || decoded.height() > THUMB_SIZE {
        decoded.thumbnail(THUMB_SIZE, THUMB_SIZE)
    } else {
        decoded
    };
    // The JPEG encoder refuses alpha, so flatten to RGB.
    let rgb = DynamicImage::ImageRgb8(shrunk.to_rgb8());
    let mut out = Vec::new();
    rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut out, THUMB_QUALITY))
        .inspect_err(|e| tracing::debug!(error = %e, "image refused: encode"))
        .ok()?;
    Some(out)
}

/// Finds `cover`, `folder` or `front` (in that order) with a `.jpg`, `.jpeg` or `.png`
/// extension, ignoring case, directly inside `dir`.
pub fn find_folder_image(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .filter_map(|e| Some((e.file_name().to_str()?.to_lowercase(), e.path())))
        .collect();
    files.sort();
    for stem in ["cover", "folder", "front"] {
        for ext in ["jpg", "jpeg", "png"] {
            let wanted = format!("{stem}.{ext}");
            if let Some((_, path)) = files.iter().find(|(name, _)| *name == wanted) {
                return Some(path.clone());
            }
        }
    }
    None
}
