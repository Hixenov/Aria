//! Commands sent from the UI thread to the backend.

use std::path::PathBuf;

use aria_library::FolderId;

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
    /// Add a folder to the library and scan it.
    AddFolder(PathBuf),
    /// Remove a folder and its tracks from the library. Files are not touched.
    RemoveFolder(FolderId),
    /// Scan the library's folders.
    Rescan,
    /// Delete the artwork thumbnails. The next scan rebuilds them.
    ClearArtwork,
    /// Write any pending settings, then stop the backend.
    Shutdown,
}
