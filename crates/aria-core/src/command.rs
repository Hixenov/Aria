//! Commands sent from the UI thread to the backend.

use std::path::PathBuf;

use aria_library::{FolderId, PlaylistId, TrackId};

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
    /// Make `ids` the list being played and start the entry at `start`. Ids not in the library are
    /// dropped.
    PlayContext {
        /// The list, in the order shown.
        ids: Vec<TrackId>,
        /// The index in `ids` of the track to start.
        start: usize,
    },
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
    /// List the output devices, answered with
    /// [`Event::OutputDevices`](crate::event::Event::OutputDevices).
    ListDevices,
    /// Save the queue and any pending settings, then stop the backend.
    Shutdown,
}
