//! Aria's icon for its window: the title bar, the taskbar and Alt+Tab. The same drawing is built
//! into `aria.exe` on Windows (see `build.rs`); `assets/icon/` holds both, made by
//! `cargo run -p aria-app --example make_icon`.

use eframe::egui::IconData;

/// The icon, 256 pixels square, as a PNG.
const PNG: &[u8] = include_bytes!("../../../assets/icon/aria-256.png");

/// The window's icon, or `None` if it cannot be read, when the window keeps the system's.
pub fn app_icon() -> Option<IconData> {
    let image = match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(image) => image.into_rgba8(),
        Err(error) => {
            tracing::warn!(%error, "Aria's icon could not be read; the window keeps the system's");
            return None;
        }
    };
    let (width, height) = image.dimensions();
    Some(IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_256_pixels_and_clear_around_the_letter() {
        let icon = app_icon().expect("the icon reads");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        let alpha = |x: usize, y: usize| icon.rgba[(y * 256 + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0, "a corner is drawn on");
        assert_eq!(alpha(255, 255), 0, "a corner is drawn on");
        assert_eq!(alpha(128, 110), 0, "the inside of the A is drawn on");
        assert_eq!(alpha(94, 128), 255, "the left leg is missing");
        assert_eq!(alpha(128, 164), 255, "the crossbar is missing");
    }
}
