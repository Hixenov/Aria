//! Core domain logic for Aria.

pub mod backup;
pub mod chat;
pub mod command;
pub mod error;
pub mod event;
pub mod hotkey;
pub mod i18n;
pub mod instance;
pub mod library_service;
pub mod logging;
pub mod notice;
pub mod opened;
pub mod paths;
pub mod playback_service;
pub mod player;
pub mod redact;
pub mod runtime;
pub mod settings;
pub mod soundboard;
pub mod spotify_player;
pub mod spotify_service;
pub mod watch;

// What `aria-app` needs from the crates below it, which it reaches only through this one.
pub use aria_audio::{
    CaptureFactory, CaptureFormat, ClockReading, CpalCapture, DeviceId, DeviceInfo, OutputFactory,
    OutputFormat, OutputPath, OutputState, PlaybackClock, SystemFactory,
    fake::{FakeCapture, FakeFactory},
};
pub use aria_library::{
    Collections, Folder, FolderId, LibrarySnapshot, PLAYLIST_NAME_MAX, Playlist, PlaylistId,
    PlaylistItem, SOUND_NAME_MAX, SOUNDS_MAX, ScanProgress, SoundId, TrackId, playlist_name,
    sound_name,
};
pub use aria_spotify::{
    NowItem, PlayRequest, PlaybackState, RepeatState, SearchResults, SpAlbum, SpArtist, SpDevice,
    SpPlaylist, SpTrack,
};
