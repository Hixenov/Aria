//! Whether a saved place for the mini-player is still on a screen.

use eframe::egui::{Rect, pos2, vec2};

/// How much of the window must be on one monitor for its saved place to be used.
const ON_SCREEN: f32 = 48.0;

/// Whether the window at `place`, of `size`, has at least a 48 × 48 piece on one of
/// `monitors`. With no monitors known it is taken as on screen.
pub fn place_on_screen(place: [f32; 2], size: [f32; 2], monitors: &[Rect]) -> bool {
    if monitors.is_empty() {
        return true;
    }
    let window = Rect::from_min_size(pos2(place[0], place[1]), vec2(size[0], size[1]));
    monitors.iter().any(|monitor| {
        let on = window.intersect(*monitor);
        on.width() >= ON_SCREEN && on.height() >= ON_SCREEN
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitors() -> [Rect; 1] {
        [Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0))]
    }

    #[test]
    fn a_place_on_a_monitor_is_kept() {
        assert!(place_on_screen([100.0, 100.0], [300.0, 300.0], &monitors()));
        // Only 40 × 40 of it on the screen.
        assert!(!place_on_screen(
            [1880.0, 1040.0],
            [300.0, 300.0],
            &monitors()
        ));
    }

    #[test]
    fn a_place_on_a_gone_monitor_is_dropped() {
        assert!(!place_on_screen(
            [2200.0, 100.0],
            [300.0, 300.0],
            &monitors()
        ));
        assert!(!place_on_screen(
            [-900.0, 100.0],
            [300.0, 300.0],
            &monitors()
        ));
    }

    #[test]
    fn a_place_on_a_second_monitor_is_kept() {
        let two = [
            monitors()[0],
            Rect::from_min_size(pos2(1920.0, 0.0), vec2(2560.0, 1440.0)),
        ];
        assert!(place_on_screen([2200.0, 100.0], [300.0, 300.0], &two));
    }

    #[test]
    fn unknown_monitors_keep_the_place() {
        assert!(place_on_screen([5000.0, 5000.0], [300.0, 300.0], &[]));
    }
}
