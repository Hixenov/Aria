//! Local music library: scanning, tags and storage.

mod artwork;
mod db;
mod error;
mod migrations;
mod model;
mod paths;
mod tags;

pub use artwork::{
    ArtworkCache, MAX_IMAGE_BYTES, MAX_IMAGE_SIDE, THUMB_QUALITY, THUMB_SIZE, find_folder_image,
};
pub use db::{Library, Opened};
pub use error::LibraryError;
pub use migrations::SCHEMA_VERSION;
pub use model::{ArtworkId, Folder, FolderId, QueueRecord, Track, TrackId};
pub use tags::{TrackTags, read_tags};
