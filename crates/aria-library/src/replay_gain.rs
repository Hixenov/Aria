//! ReplayGain: the loudness tags that let tracks play at an even level.
//!
//! A file may carry a gain for itself and one for its album, in decibels, with the loudest
//! sample of each. Aria reads them when a track is played, from the file itself, and stores
//! nothing. Opus's own R128 gains are not read.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::ItemKey;

/// What a file's ReplayGain tags say. A value missing or unreadable is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ReplayGain {
    /// The track's gain, in decibels.
    pub track_gain_db: Option<f32>,
    /// The track's loudest sample, 1.0 being full scale.
    pub track_peak: Option<f32>,
    /// The album's gain, in decibels.
    pub album_gain_db: Option<f32>,
    /// The album's loudest sample.
    pub album_peak: Option<f32>,
}

/// The most a gain may lift, whatever a tag says: 20 dB.
const MAX_GAIN: f32 = 10.0;

impl ReplayGain {
    /// The gain to play at, as a factor: the album's if `album` and the file has one, else the
    /// track's, and 1.0 without either. It is held where the matching peak reaches full scale,
    /// so levelling never clips.
    pub fn linear(&self, album: bool) -> f32 {
        let (gain, peak) = match (album, self.album_gain_db) {
            (true, Some(gain)) => (Some(gain), self.album_peak),
            _ => (self.track_gain_db, self.track_peak),
        };
        let Some(gain) = gain else {
            return 1.0;
        };
        let mut linear = 10f32.powf(gain / 20.0).min(MAX_GAIN);
        if let Some(peak) = peak.filter(|peak| *peak > 0.0)
            && linear * peak > 1.0
        {
            linear = 1.0 / peak;
        }
        if linear.is_finite() { linear } else { 1.0 }
    }
}

/// The ReplayGain tags of the file at `path`, from its first tag that has them. A file that
/// cannot be read, or has none, gives all `None`.
pub fn read_replay_gain(path: &Path) -> ReplayGain {
    let options = ParseOptions::new()
        .read_properties(false)
        .read_cover_art(false);
    let read = catch_unwind(AssertUnwindSafe(|| {
        Probe::open(path).ok()?.options(options).read().ok()
    }));
    let Ok(Some(file)) = read else {
        return ReplayGain::default();
    };
    let value = |key: ItemKey| {
        file.tags()
            .iter()
            .find_map(|tag| tag.get_string(key).map(str::to_owned))
    };
    ReplayGain {
        track_gain_db: value(ItemKey::ReplayGainTrackGain)
            .as_deref()
            .and_then(parse_number),
        track_peak: value(ItemKey::ReplayGainTrackPeak)
            .as_deref()
            .and_then(parse_number),
        album_gain_db: value(ItemKey::ReplayGainAlbumGain)
            .as_deref()
            .and_then(parse_number),
        album_peak: value(ItemKey::ReplayGainAlbumPeak)
            .as_deref()
            .and_then(parse_number),
    }
}

/// The number at the start of a tag's text: "-6.50 dB", "+3.1dB" and "0.98" all read.
fn parse_number(text: &str) -> Option<f32> {
    let text = text.trim();
    let number = text
        .strip_suffix("dB")
        .or_else(|| text.strip_suffix("db"))
        .or_else(|| text.strip_suffix("DB"))
        .unwrap_or(text)
        .trim();
    number.parse::<f32>().ok().filter(|value| value.is_finite())
}
