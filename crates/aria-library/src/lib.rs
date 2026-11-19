//! Local music library: scanning, tags and storage.

mod artwork;
mod collections;
mod db;
mod error;
mod migrations;
mod model;
mod paths;
mod replay_gain;
mod scan;
mod snapshot;
mod sounds;
mod tags;

pub use artwork::{
    ArtworkCache, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE, THUMB_QUALITY, THUMB_SIZE, find_folder_image,
};
pub use collections::{
    Collections, PLAYLIST_NAME_MAX, PLAYS_KEPT, Playlist, PlaylistId, PlaylistItem, RECENT_LIMIT,
    playlist_name,
};
pub use db::{Library, Opened};
pub use error::LibraryError;
pub use migrations::SCHEMA_VERSION;
pub use model::{ArtworkId, Folder, FolderId, Quality, QueueRecord, Track, TrackId};
pub use replay_gain::{ReplayGain, read_replay_gain};
pub use scan::{SUPPORTED_EXTENSIONS, ScanProgress, ScanSummary};
pub use snapshot::{Album, Artist, LibrarySnapshot};
pub use sounds::{SOUND_NAME_MAX, SOUNDS_MAX, Sound, SoundId, sound_name};
pub use tags::{TrackTags, read_tags};
