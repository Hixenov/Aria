//! Commands sent from the UI thread to the backend.

use crate::settings::Settings;

/// A request from the UI thread to the backend.
#[derive(Debug, Clone)]
pub enum Command {
    /// Persist these settings once updates have been quiet for the debounce
    /// delay. A newer update replaces one that is still pending.
    UpdateSettings(Settings),
    /// Write the default settings now and announce them with
    /// [`Event::SettingsReplaced`](crate::event::Event::SettingsReplaced).
    ResetSettings,
    /// Write any pending settings, then stop the backend.
    Shutdown,
}
