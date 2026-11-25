//! Commands sent from the UI thread to the backend.

use std::path::PathBuf;

use aria_library::{FolderId, PlaylistId, SoundId, TrackId};

use crate::{
    hotkey::Hotkey, settings::Settings, spotify_service::SpotifyRequest,
    update_service::UpdateRequest,
};

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
    /// Make `ids` the list being played and start the entry at `start`. Ids not in the library are
    /// dropped.
    PlayContext {
        /// The list, in the order shown.
        ids: Vec<TrackId>,
        /// The index in `ids` of the track to start.
        start: usize,
    },
    /// Play these files, in this order, as the list being played: each from the library if it is
    /// there, else read from the file and played without being added. Folders among them are
    /// added to the library. Files Aria does not play are skipped with a notice. As a
    /// double-click, "Open with", the command line or a drop asks.
    OpenPaths(Vec<PathBuf>),
    /// Make a playlist of the M3U file at this path, named after the file.
    ImportPlaylist(PathBuf),
    /// Save a playlist as an M3U8 file at `to`.
    ExportPlaylist {
        /// The playlist.
        id: PlaylistId,
        /// The file to write.
        to: PathBuf,
    },
    /// Back the library and the settings up into a new folder inside this one.
    BackUp(PathBuf),
    /// Restore the backup in this folder when Aria starts again.
    Restore(PathBuf),
    /// Set the sleep timer, or turn it off.
    SetSleepTimer(SleepTimer),
    /// Put a track at the front of the user queue.
    PlayNext(TrackId),
    /// Put a track at the end of the user queue.
    AddToQueue(TrackId),
    /// Remove the entry at this index from the user queue.
    RemoveFromQueue(usize),
    /// Move an entry of the user queue so that it ends up at `to`.
    MoveInQueue {
        /// Its index now.
        from: usize,
        /// Its index afterwards.
        to: usize,
    },
    /// Empty the user queue.
    ClearQueue,
    /// Mark or unmark a track as a favorite.
    SetFavorite {
        /// The track.
        track: TrackId,
        /// Whether it is now a favorite.
        favorite: bool,
    },
    /// Make a playlist, with one track in it if given. A name that is empty or longer than 100
    /// characters gives a notice.
    CreatePlaylist {
        /// The name, trimmed by the library.
        name: String,
        /// The track to start the playlist with.
        first: Option<TrackId>,
    },
    /// Give a playlist a new name.
    RenamePlaylist {
        /// The playlist.
        id: PlaylistId,
        /// The new name, under the rules of [`Command::CreatePlaylist`].
        name: String,
    },
    /// Delete a playlist and its entries.
    DeletePlaylist(PlaylistId),
    /// Append a track to a playlist.
    AddToPlaylist {
        /// The playlist.
        playlist: PlaylistId,
        /// The track.
        track: TrackId,
    },
    /// Remove the entry at this stored position of a playlist.
    RemoveFromPlaylist {
        /// The playlist.
        playlist: PlaylistId,
        /// The entry's stored position, as in `PlaylistItem::position`.
        position: u32,
    },
    /// Move an entry of a playlist so that it ends up at `to`.
    MoveInPlaylist {
        /// The playlist.
        playlist: PlaylistId,
        /// The entry's stored position now.
        from: u32,
        /// Its stored position afterwards.
        to: u32,
    },
    /// Delete every recorded play. Favorites and playlists stay.
    ClearHistory,
    /// Pause or play. With nothing loaded, this starts what would play next.
    TogglePlay,
    /// Skip to the next track.
    Next,
    /// Go to the previous track, or to the start of this one once it has played for a while.
    Previous,
    /// Move within the current track, to this many milliseconds.
    Seek(u64),
    /// Add sounds to the soundboard, named after their files, in this order. Past the limit of
    /// 100, the rest are left out with a notice.
    AddSounds(Vec<PathBuf>),
    /// Give a sound a new name: 1 to 50 characters after trimming, or a notice.
    RenameSound {
        /// The sound.
        id: SoundId,
        /// The new name, trimmed by the library.
        name: String,
    },
    /// Set a sound's volume, 0.0 to 1.0.
    SetSoundVolume {
        /// The sound.
        id: SoundId,
        /// The volume; out of range, it is clamped.
        volume: f32,
    },
    /// Set or clear a sound's hotkey. A hotkey that takes a typing key, or one used by another
    /// sound or by "Stop all", is refused with a notice.
    SetSoundHotkey {
        /// The sound.
        id: SoundId,
        /// The hotkey, or `None` to clear it.
        hotkey: Option<Hotkey>,
    },
    /// Remove a sound from the soundboard. Its file is not touched.
    RemoveSound(SoundId),
    /// Play a sound from its start, or start it again if it is playing. A missing file gives a
    /// notice and plays nothing.
    PlaySound(SoundId),
    /// Stop every sound. The music plays on.
    StopSounds,
    /// List the output devices, answered with
    /// [`Event::OutputDevices`](crate::event::Event::OutputDevices).
    ListDevices,
    /// List the input devices, answered with
    /// [`Event::InputDevices`](crate::event::Event::InputDevices). Listing opens none of them.
    ListInputDevices,
    /// Turn "Send my microphone" on or off, save that, and say which with a notice. The settings
    /// as they now are come back in
    /// [`Event::SettingsReplaced`](crate::event::Event::SettingsReplaced).
    ToggleMicrophone,
    /// A request for the Spotify service. The backend routes the player's ones and keeps
    /// [`SpotifyRequest::Cadence`] and [`SpotifyRequest::Shutdown`] to itself.
    Spotify(SpotifyRequest),
    /// For the update checker; its `Settings` and `Shutdown` are the backend's own.
    Update(UpdateRequest),
    /// The window is shown (`true`), or minimized or hidden (`false`).
    WindowShown(bool),
    /// Save the queue and any pending settings, then stop the backend.
    Shutdown,
}

/// When the sleep timer stops the music.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepTimer {
    /// No timer.
    Off,
    /// After this long: the music fades out over its last seconds, and pauses.
    After(std::time::Duration),
    /// When the track playing ends: the next one is loaded, paused.
    EndOfTrack,
}
