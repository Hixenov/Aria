//! Events sent from the backend to the UI thread.

use std::sync::Arc;

use aria_audio::DeviceInfo;
use aria_library::{Collections, LibrarySnapshot, ScanProgress};

use crate::{notice::Notice, playback_service::PlayerView, settings::Settings};

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
    /// The library changed; this is how it now is.
    LibraryUpdated(Arc<LibrarySnapshot>),
    /// Favorites, playlists and recent plays; this is how they now are. It follows every
    /// [`Event::LibraryUpdated`] and every change to them, and may name tracks the UI's snapshot
    /// does not hold yet.
    Collections(Arc<Collections>),
    /// A scan is running, or `None` when it has ended. Arrives at most 4 times
    /// a second.
    ScanProgress(Option<ScanProgress>),
    /// The player changed; this is how it now is.
    Player(PlayerView),
    /// The output devices, as asked for with
    /// [`Command::ListDevices`](crate::command::Command::ListDevices). Empty if they could not be
    /// listed.
    OutputDevices(Vec<DeviceInfo>),
}
