//! Playback model: the pure play queue. No I/O and no threads; the playback service drives it.

pub mod queue;

pub use queue::{HISTORY_LIMIT, PreviousAction, Queue, RESTART_THRESHOLD_MS, RepeatMode, UpNext};
