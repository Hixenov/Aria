//! Plain data types shared by the library, scanner and backend.

use std::path::PathBuf;
use std::sync::Arc;

/// Row id of a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackId(pub i64);

/// Row id of a library folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FolderId(pub i64);

/// Hash text naming a thumbnail file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArtworkId(pub Arc<str>);

/// One audio file in the library.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub folder: FolderId,
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<Arc<str>>,
    pub album: Option<Arc<str>>,
    pub album_artist: Option<Arc<str>>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<Arc<str>>,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub codec: Arc<str>,
    pub artwork: Option<ArtworkId>,
    pub added_at: i64,
}

/// A folder the user added to the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: FolderId,
    pub path: PathBuf,
    /// False while the folder's root cannot be read; its tracks are kept.
    pub available: bool,
}

/// The saved play queue.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueueRecord {
    pub context: Vec<TrackId>,
    /// The context in its pre-shuffle order; empty when it was not shuffled.
    pub original: Vec<TrackId>,
    pub user: Vec<TrackId>,
    pub current: Option<TrackId>,
    pub position_ms: u64,
    pub shuffle_seed: u64,
    pub context_index: Option<usize>,
}
