//! Events sent from the backend to the UI thread.

use std::sync::Arc;

use aria_audio::DeviceInfo;
use aria_library::{Collections, LibrarySnapshot, ScanProgress, Track};

use crate::{
    notice::Notice, playback_service::PlayerView, settings::Settings, soundboard::SoundboardState,
    spotify_service::SpotifyEvent, update_service::UpdateEvent,
};

/// Which player the player bar shows and controls: the one last started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActivePlayer {
    /// Aria's own player.
    #[default]
    Local,
    /// Spotify, on one of its devices.
    Spotify,
}

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
    /// The tracks opened from outside the library this session, which the snapshot does not
    /// hold, under their negative ids. Sent when one is added.
    Opened(Arc<[Track]>),
    /// The soundboard: its sounds, which are playing and which files are missing. Sent once
    /// after the library opens and after every change, a sound starting or ending included.
    Soundboard(Arc<SoundboardState>),
    /// The output devices, as asked for with
    /// [`Command::ListDevices`](crate::command::Command::ListDevices). Empty if they could not be
    /// listed.
    OutputDevices(Vec<DeviceInfo>),
    /// The input devices, as asked for with
    /// [`Command::ListInputDevices`](crate::command::Command::ListInputDevices). Empty if they
    /// could not be listed.
    InputDevices(Vec<DeviceInfo>),
    /// The player bar now shows this player. Sent when it changes, and once at start when
    /// Spotify is there.
    ActivePlayer(ActivePlayer),
    /// What the Spotify service reports.
    Spotify(SpotifyEvent),
    /// From the update checker.
    Update(UpdateEvent),
}
