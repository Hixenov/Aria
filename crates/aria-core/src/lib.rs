//! Core domain logic for Aria.

pub mod command;
pub mod error;
pub mod event;
pub mod hotkey;
pub mod i18n;
pub mod library_service;
pub mod logging;
pub mod notice;
pub mod paths;
pub mod playback_service;
pub mod player;
pub mod redact;
pub mod runtime;
pub mod settings;
pub mod soundboard;

// What `aria-app` needs from the crates below it, which it reaches only through this one.
pub use aria_audio::{
    ClockReading, DeviceId, DeviceInfo, OutputFactory, OutputFormat, PlaybackClock, SystemFactory,
    fake::FakeFactory,
};
pub use aria_library::{
    Collections, Folder, FolderId, LibrarySnapshot, PLAYLIST_NAME_MAX, Playlist, PlaylistId,
    PlaylistItem, SOUND_NAME_MAX, SOUNDS_MAX, ScanProgress, SoundId, TrackId, playlist_name,
    sound_name,
};
