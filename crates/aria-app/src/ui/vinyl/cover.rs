//! The current local track's large cover, read from its file on a thread of its own. Only the
//! latest track's cover is kept; one that arrives for a track no longer current is dropped.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use aria_core::TrackId;
use eframe::egui;
use image::RgbaImage;

/// Large covers fit in this many pixels.
pub const LARGE: u32 = 1024;

pub struct LocalCover {
    wanted: Option<TrackId>,
    texture: Option<(TrackId, egui::TextureHandle)>,
    sender: Sender<(TrackId, Option<RgbaImage>)>,
    arrived: Receiver<(TrackId, Option<RgbaImage>)>,
}

impl Default for LocalCover {
    fn default() -> LocalCover {
        let (sender, arrived) = channel();
        LocalCover {
            wanted: None,
            texture: None,
            sender,
            arrived,
        }
    }
}

impl LocalCover {
    /// Asks for the cover of `key`, read from `path`, unless it is the one already asked for.
    pub fn request(&mut self, ctx: &egui::Context, key: TrackId, path: PathBuf) {
        if self.wanted == Some(key) {
            return;
        }
        self.wanted = Some(key);
        let (sender, ctx) = (self.sender.clone(), ctx.clone());
        let spawned = thread::Builder::new()
            .name("aria-cover".to_owned())
            .spawn(move || {
                let cover = aria_library::large_cover(&path, LARGE);
                // Fails only once the mode has gone, when nobody wants it.
                let _ = sender.send((key, cover));
                ctx.request_repaint();
            });
        if spawned.is_err() {
            tracing::debug!("no thread for a large cover");
        }
    }

    /// The large cover of `key`, once it has arrived.
    pub fn texture(&mut self, ctx: &egui::Context, key: TrackId) -> Option<&egui::TextureHandle> {
        while let Ok((arrived, cover)) = self.arrived.try_recv() {
            if Some(arrived) != self.wanted {
                continue;
            }
            self.texture = cover.map(|image| {
                let size = [image.width() as usize, image.height() as usize];
                let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                let texture =
                    ctx.load_texture("aria-large-cover", pixels, egui::TextureOptions::LINEAR);
                (arrived, texture)
            });
        }
        self.texture
            .as_ref()
            .filter(|(held, _)| *held == key)
            .map(|(_, texture)| texture)
    }

    /// Forgets the cover (when the mode closes).
    pub fn clear(&mut self) {
        self.wanted = None;
        self.texture = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::{Duration, Instant};

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../aria-audio/tests/fixtures/cover.flac")
    }

    #[test]
    fn a_stale_cover_is_dropped() {
        let ctx = egui::Context::default();
        let mut cover = LocalCover::default();
        cover.request(&ctx, TrackId(1), fixture());
        cover.request(&ctx, TrackId(2), fixture());
        let deadline = Instant::now() + Duration::from_secs(10);
        while cover.texture(&ctx, TrackId(2)).is_none() {
            assert!(Instant::now() < deadline, "the cover never came");
            thread::sleep(Duration::from_millis(20));
        }
        // Give the first thread time to answer too: its cover must not replace the second.
        thread::sleep(Duration::from_millis(300));
        assert!(cover.texture(&ctx, TrackId(1)).is_none());
        assert!(cover.texture(&ctx, TrackId(2)).is_some());
    }

    #[test]
    fn a_file_without_a_cover_gives_none() {
        let ctx = egui::Context::default();
        let mut cover = LocalCover::default();
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such.flac");
        cover.request(&ctx, TrackId(3), missing);
        thread::sleep(Duration::from_millis(300));
        assert!(cover.texture(&ctx, TrackId(3)).is_none());
    }
}
