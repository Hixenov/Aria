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
    /// Bits per sample; 0 when the format has none (MP3, AAC) or it is not known.
    pub bits_per_sample: u16,
    pub codec: Arc<str>,
    pub artwork: Option<ArtworkId>,
    pub added_at: i64,
}

/// How good a track's file is, for the badge and the "Lossless only" filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Lossy,
    Lossless,
    /// Lossless with a rate above 48 000 Hz or a depth above 16 bits.
    HiRes,
}

impl Track {
    /// Lossless codecs are FLAC, ALAC, WAV and AIFF. The scan names a WAV or AIFF file `wav` or
    /// `aiff` only when its data is integer or float PCM; A-law, µ-law and other codings get
    /// names of their own, which are lossy here. CAF is listed too, but lofty cannot read CAF
    /// files, so the scan never stores that codec and no CAF track is ever in the library.
    pub fn quality(&self) -> Quality {
        if !matches!(&*self.codec, "flac" | "alac" | "wav" | "aiff" | "caf") {
            Quality::Lossy
        } else if self.sample_rate > 48_000 || self.bits_per_sample > 16 {
            Quality::HiRes
        } else {
            Quality::Lossless
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn track(codec: &str, bits: u16, rate: u32) -> Track {
        Track {
            id: TrackId(1),
            folder: FolderId(1),
            path: PathBuf::new(),
            title: String::new(),
            artist: None,
            album: None,
            album_artist: None,
            track_no: None,
            disc_no: None,
            year: None,
            genre: None,
            duration_ms: 0,
            sample_rate: rate,
            channels: 2,
            bits_per_sample: bits,
            codec: Arc::from(codec),
            artwork: None,
            added_at: 0,
        }
    }

    #[test]
    fn quality_follows_codec_rate_and_depth() {
        let table = [
            ("flac", 16, 44_100, Quality::Lossless),
            ("flac", 24, 44_100, Quality::HiRes),
            ("flac", 16, 96_000, Quality::HiRes),
            ("flac", 16, 48_000, Quality::Lossless),
            ("alac", 24, 48_000, Quality::HiRes),
            ("mp3", 0, 44_100, Quality::Lossy),
            ("aac", 0, 44_100, Quality::Lossy),
            ("wav", 16, 44_100, Quality::Lossless),
            ("unknown", 0, 44_100, Quality::Lossy),
            ("mp4", 0, 44_100, Quality::Lossy),
            ("flac", 24, 96_000, Quality::HiRes),
            ("aiff", 16, 44_100, Quality::Lossless),
            // Float PCM is lossless; at 32 bits it is above 16.
            ("wav", 32, 44_100, Quality::HiRes),
            // WAV and AIFF data that is not PCM is named for its coding, and is lossy.
            ("alaw", 8, 44_100, Quality::Lossy),
            ("ulaw", 8, 44_100, Quality::Lossy),
            ("adpcm", 4, 44_100, Quality::Lossy),
            ("wav-compressed", 16, 44_100, Quality::Lossy),
            ("aiff-compressed", 16, 44_100, Quality::Lossy),
        ];
        for (codec, bits, rate, want) in table {
            assert_eq!(
                track(codec, bits, rate).quality(),
                want,
                "{codec} {bits}/{rate}"
            );
        }
    }
}
