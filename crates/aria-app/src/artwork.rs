//! Artwork thumbnails as egui textures.
//!
//! The library stores a JPEG thumbnail per cover (`<dir>/<id>.jpg`). Reading and decoding it
//! is disk and CPU work, so it never happens on the UI thread: [`ArtworkTextures::get`] only
//! queues a request for the worker thread `aria-artwork` and answers `None` until the decoded
//! pixels come back. [`ArtworkTextures::poll`], called once per frame, uploads them as
//! textures. A cover that cannot be shown stays the placeholder: nothing here fails loudly.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Cursor, Read as _},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use aria_library::{ArtworkCache, ArtworkId};
use eframe::egui::{
    Color32, ColorImage, Context, Painter, Rect, TextureHandle, TextureOptions, epaint::RectShape,
    pos2,
};
use image::{ImageFormat, ImageReader, Limits};

/// How many textures are kept. Past it, the least recently used are dropped.
pub const TEXTURE_CACHE_LIMIT: usize = 256;
/// How many covers may wait for the worker at once. Covers asked for beyond it are asked for
/// again on a later frame, if they are still in view.
const MAX_PENDING: usize = 64;
/// Thumbnails are a few tens of kilobytes; a larger file is not one of ours.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Thumbnails are at most 256 pixels on a side; a larger image is refused.
const MAX_SIDE: u32 = 1024;
/// Decoder allocation limit, in bytes.
const MAX_ALLOC: u64 = 32 * 1024 * 1024;
/// Name of the decoding thread.
const THREAD_NAME: &str = "aria-artwork";

/// A cover for the worker to decode.
struct Request {
    id: ArtworkId,
    path: PathBuf,
}

/// What the worker made of a [`Request`]: `None` if the file was missing or broken.
struct Decoded {
    id: ArtworkId,
    image: Option<ColorImage>,
}

/// A texture and when it was last asked for.
struct Entry {
    handle: TextureHandle,
    used: u64,
}

/// Cover textures, loaded in the background and kept in a size-limited LRU cache.
pub struct ArtworkTextures {
    cache: ArtworkCache,
    ctx: Context,
    limit: usize,
    /// `None` when the worker could not start, or has died.
    requests: Option<Sender<Request>>,
    results: Receiver<Decoded>,
    /// Covers queued or being decoded. A cover is never queued twice while it is in here.
    pending: HashSet<ArtworkId>,
    /// Covers whose file was missing or broken, so they are not tried every frame.
    failed: HashSet<ArtworkId>,
    textures: HashMap<ArtworkId, Entry>,
    /// Counts calls to [`Self::get`], to order textures by use.
    clock: u64,
}

impl ArtworkTextures {
    /// Textures for the thumbnails in `dir`. The worker wakes the UI through `ctx`.
    pub fn new(dir: PathBuf, ctx: &Context) -> Self {
        Self::with_limit(dir, ctx, TEXTURE_CACHE_LIMIT)
    }

    fn with_limit(dir: PathBuf, ctx: &Context, limit: usize) -> Self {
        let (requests, wanted) = mpsc::channel::<Request>();
        let (done, results) = mpsc::channel::<Decoded>();
        let waker = ctx.clone();
        let worker = thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                // Ends when the last sender is dropped, or when nobody takes the results.
                for Request { id, path } in wanted {
                    let image = decode(&path);
                    if done.send(Decoded { id, image }).is_err() {
                        break;
                    }
                    waker.request_repaint();
                }
            });
        let requests = match worker {
            Ok(_) => Some(requests),
            Err(err) => {
                tracing::warn!(%err, "could not start the artwork thread; covers stay blank");
                None
            }
        };
        Self {
            cache: ArtworkCache::new(dir),
            ctx: ctx.clone(),
            limit: limit.max(1),
            requests,
            results,
            pending: HashSet::new(),
            failed: HashSet::new(),
            textures: HashMap::new(),
            clock: 0,
        }
    }

    /// The texture of cover `id`, if it is loaded. Otherwise starts loading it, unless that
    /// already happened or failed, and answers `None`: draw the placeholder meanwhile.
    pub fn get(&mut self, id: &ArtworkId) -> Option<&TextureHandle> {
        self.clock += 1;
        let now = self.clock;
        if self.textures.contains_key(id) {
            return self.textures.get_mut(id).map(|entry| {
                entry.used = now;
                &entry.handle
            });
        }
        if !self.failed.contains(id)
            && !self.pending.contains(id)
            && self.pending.len() < MAX_PENDING
        {
            self.request(id);
        }
        None
    }

    fn request(&mut self, id: &ArtworkId) {
        let Some(requests) = &self.requests else {
            return;
        };
        let request = Request {
            id: id.clone(),
            path: self.cache.path_of(id),
        };
        if requests.send(request).is_ok() {
            self.pending.insert(id.clone());
        } else {
            tracing::warn!("the artwork thread has stopped; covers stay blank");
            self.requests = None;
        }
    }

    /// Uploads what the worker has decoded since the last call, and drops the least recently
    /// used textures past the limit. Call it once per frame.
    pub fn poll(&mut self) {
        while let Ok(Decoded { id, image }) = self.results.try_recv() {
            self.pending.remove(&id);
            match image {
                Some(image) => {
                    let name = format!("artwork-{}", id.0);
                    let handle = self.ctx.load_texture(name, image, TextureOptions::LINEAR);
                    self.clock += 1;
                    let used = self.clock;
                    self.textures.insert(id, Entry { handle, used });
                }
                None => {
                    self.failed.insert(id);
                }
            }
        }
        while self.textures.len() > self.limit {
            let oldest = self
                .textures
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => self.textures.remove(&id),
                None => break,
            };
        }
    }

    /// Forgets which covers failed, so they are tried again when asked for. Call it when a
    /// scan has finished, which may have written the thumbnails that were missing: not on every
    /// snapshot, or a cover that is really gone would be asked for again with each one.
    pub fn retry_failed(&mut self) {
        self.failed.clear();
    }

    #[cfg(test)]
    pub(crate) fn has_failed(&self, id: &ArtworkId) -> bool {
        self.failed.contains(id)
    }

    #[cfg(test)]
    pub(crate) fn is_pending(&self, id: &ArtworkId) -> bool {
        self.pending.contains(id)
    }
}

/// Paints `texture` filling `rect`, with rounded corners. A cover that is not square is
/// cropped to its middle, never stretched.
pub fn paint_cover(painter: &Painter, rect: Rect, radius: u8, texture: &TextureHandle) {
    let [w, h] = texture.size();
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let (kx, ky) = if w > h { (h / w, 1.0) } else { (1.0, w / h) };
    let uv = Rect::from_min_max(
        pos2((1.0 - kx) / 2.0, (1.0 - ky) / 2.0),
        pos2((1.0 + kx) / 2.0, (1.0 + ky) / 2.0),
    );
    painter.add(RectShape::filled(rect, radius, Color32::WHITE).with_texture(texture.id(), uv));
}

/// Reads and decodes the thumbnail at `path`. A panic in the decoder costs this cover only.
fn decode(path: &Path) -> Option<ColorImage> {
    match catch_unwind(AssertUnwindSafe(|| try_decode(path))) {
        Ok(image) => image,
        Err(_) => {
            tracing::warn!("the artwork decoder panicked; the cover stays blank");
            None
        }
    }
}

fn try_decode(path: &Path) -> Option<ColorImage> {
    let mut bytes = Vec::new();
    let read =
        File::open(path).and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes));
    if let Err(err) = read {
        tracing::debug!(path = %path.display(), %err, "cannot read a thumbnail");
        return None;
    }
    if bytes.len() as u64 > MAX_FILE_BYTES {
        tracing::debug!(path = %path.display(), "a thumbnail is too big");
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let rgba = match reader.decode() {
        Ok(image) => image.into_rgba8(),
        Err(err) => {
            tracing::debug!(path = %path.display(), %err, "cannot decode a thumbnail");
            return None;
        }
    };
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    const DEADLINE: Duration = Duration::from_secs(10);

    fn id(name: &str) -> ArtworkId {
        ArtworkId(name.into())
    }

    /// Writes a small JPEG as the thumbnail of `name`.
    fn write_jpeg(dir: &Path, name: &str) {
        let image = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let path = dir.join(format!("{name}.jpg"));
        image
            .save_with_format(path, ImageFormat::Jpeg)
            .unwrap_or_else(|err| panic!("cannot write a test image: {err}"));
    }

    fn textures(dir: &Path, limit: usize) -> ArtworkTextures {
        ArtworkTextures::with_limit(dir.to_path_buf(), &Context::default(), limit)
    }

    /// Asks for `name` until it is loaded.
    fn load(textures: &mut ArtworkTextures, name: &str) {
        let start = Instant::now();
        while textures.get(&id(name)).is_none() {
            assert!(start.elapsed() < DEADLINE, "{name} was not loaded");
            thread::sleep(Duration::from_millis(2));
            textures.poll();
        }
    }

    /// Polls until nothing is pending.
    fn settle(textures: &mut ArtworkTextures) {
        let start = Instant::now();
        while !textures.pending.is_empty() {
            assert!(start.elapsed() < DEADLINE, "the worker did not answer");
            thread::sleep(Duration::from_millis(2));
            textures.poll();
        }
    }

    #[test]
    fn loads_a_thumbnail_off_thread() {
        let dir = tempfile::tempdir().unwrap();
        write_jpeg(dir.path(), "cover");
        let mut textures = textures(dir.path(), 8);

        // Asking neither blocks nor answers: the work is the worker's.
        assert!(textures.get(&id("cover")).is_none());
        assert!(textures.pending.contains(&id("cover")));
        // Asking again while it waits queues nothing more.
        assert!(textures.get(&id("cover")).is_none());
        assert_eq!(textures.pending.len(), 1);

        load(&mut textures, "cover");
        let texture = textures.get(&id("cover")).map(TextureHandle::size);
        assert_eq!(texture, Some([8, 4]));
        assert!(textures.pending.is_empty());
    }

    #[test]
    fn broken_file_is_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.jpg"), b"not a jpeg").unwrap();
        let mut textures = textures(dir.path(), 8);

        for name in ["broken", "missing"] {
            assert!(textures.get(&id(name)).is_none());
        }
        settle(&mut textures);
        assert!(textures.failed.contains(&id("broken")));
        assert!(textures.failed.contains(&id("missing")));

        // Once failed, asking again queues nothing, even though the file is good now.
        write_jpeg(dir.path(), "broken");
        for _ in 0..20 {
            assert!(textures.get(&id("broken")).is_none());
            assert!(textures.pending.is_empty(), "the failed cover was retried");
            thread::sleep(Duration::from_millis(5));
            textures.poll();
        }
        assert!(textures.textures.is_empty());

        // Until told that the library changed.
        textures.retry_failed();
        load(&mut textures, "broken");
    }

    #[test]
    fn cache_evicts_least_recently_used() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a", "b", "c"] {
            write_jpeg(dir.path(), name);
        }
        let mut textures = textures(dir.path(), 2);
        load(&mut textures, "a");
        load(&mut textures, "b");
        // `a` is used after `b` was loaded, so `b` is the one to go.
        assert!(textures.get(&id("a")).is_some());
        load(&mut textures, "c");

        assert!(textures.textures.contains_key(&id("a")));
        assert!(textures.textures.contains_key(&id("c")));
        assert!(!textures.textures.contains_key(&id("b")));
        assert_eq!(textures.textures.len(), 2);
    }

    #[test]
    fn oversized_images_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let big = image::RgbImage::from_pixel(MAX_SIDE + 1, 8, image::Rgb([1, 2, 3]));
        big.save_with_format(dir.path().join("big.jpg"), ImageFormat::Jpeg)
            .unwrap();
        assert!(try_decode(&dir.path().join("big.jpg")).is_none());
    }
}
