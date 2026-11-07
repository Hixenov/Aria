//! Local music library: scanning, tags and storage.

mod db;
mod error;
mod migrations;
mod model;
mod paths;

pub use db::{Library, Opened};
pub use error::LibraryError;
pub use migrations::SCHEMA_VERSION;
pub use model::{ArtworkId, Folder, FolderId, QueueRecord, Track, TrackId};
