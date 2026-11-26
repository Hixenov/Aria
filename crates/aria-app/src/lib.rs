//! Aria application library: UI building blocks shared by the binary and tests.

use std::time::Duration;

pub mod app;
pub mod artwork;
pub mod autostart;
mod folders;
mod hotkeys;
pub mod icon;
pub mod media_controls;
pub mod nav;
mod shortcuts;
pub mod spotify;
pub mod tray;
pub mod ui;
pub mod update;

pub use hotkeys::SystemHotkeys;

/// Application name, shown as the sidebar wordmark.
pub const APP_NAME: &str = "Aria";

/// `elapsed` in whole milliseconds, saturating: how startup timings are logged.
pub fn whole_ms(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_ms_truncates_and_saturates() {
        assert_eq!(whole_ms(Duration::from_micros(1999)), 1);
        assert_eq!(whole_ms(Duration::MAX), u64::MAX);
    }
}
