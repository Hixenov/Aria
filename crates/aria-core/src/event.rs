//! Events sent from the backend to the UI thread.

use crate::{notice::Notice, settings::Settings};

/// Something the backend reports to the UI thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Settings were written to disk.
    SettingsSaved,
    /// The backend replaced the settings (for example, reset them to the
    /// defaults); the UI adopts these.
    SettingsReplaced(Settings),
    /// A notice to show the user.
    Notice(Notice),
}
