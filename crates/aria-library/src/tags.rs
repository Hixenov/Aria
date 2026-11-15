//! Reading tags and audio properties from a file with lofty.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;

use lofty::config::{GlobalOptions, ParseOptions, apply_global_options};
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::mp4::{Mp4Codec, Mp4File};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, Tag};

use crate::LibraryError;
use crate::artwork::MAX_IMAGE_BYTES;

/// What the scanner learns about one audio file. Field types match `Track`.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackTags {
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
    /// Bits per sample; 0 when the format has none (MP3, AAC) or lofty does not report one.
    pub bits_per_sample: u16,
    pub codec: Arc<str>,
    /// Raw bytes of the embedded cover; pictures over `MAX_IMAGE_BYTES` are dropped.
    pub picture: Option<Vec<u8>>,
}

/// Reads the tags and properties of the file at `path`.
///
/// A file lofty cannot parse gives `Err(LibraryError::Tags)`; it never panics.
pub fn read_tags(path: &Path) -> Result<TrackTags, LibraryError> {
    // lofty's limit is per thread and defaults to 16 MiB per tag item, which would fail the
    // whole read for a large cover. Allow anything `pick_picture` may keep, plus slack.
    apply_global_options(GlobalOptions::new().allocation_limit(MAX_IMAGE_BYTES + 1024 * 1024));
    let parsed = catch_unwind(AssertUnwindSafe(|| {
        Probe::open(path)
            .map_err(|e| LibraryError::Tags(Box::new(e)))?
            .read()
            .map_err(|e| LibraryError::Tags(Box::new(e)))
    }))
    .unwrap_or_else(|_| {
        tracing::debug!(path = %path.display(), "tag parser panicked");
        Err(LibraryError::Tags("the tag parser failed".into()))
    })?;
    let mp4_codec = if parsed.file_type() == FileType::Mp4 {
        read_mp4_codec(path)
    } else {
        None
    };
    Ok(from_tagged(path, &parsed, mp4_codec))
}

/// The codec inside an MP4 container, read from its sample description. `None` when it cannot
/// be read, which leaves the codec as plain `mp4`.
fn read_mp4_codec(path: &Path) -> Option<Mp4Codec> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
        let options = ParseOptions::new().read_tags(false).read_cover_art(false);
        Mp4File::read_from(&mut file, options)
            .ok()?
            .properties()
            .codec()
    }))
    .unwrap_or(None)
}

fn from_tagged(path: &Path, file: &TaggedFile, mp4_codec: Option<Mp4Codec>) -> TrackTags {
    let tag = file.primary_tag().or_else(|| file.first_tag());
    let props = file.properties();
    let text = |value: Option<std::borrow::Cow<'_, str>>| -> Option<Arc<str>> {
        value
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .map(Arc::from)
    };
    let title = tag
        .and_then(|t| t.title())
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| title_from_path(path));
    TrackTags {
        title,
        artist: tag.and_then(|t| text(t.artist())),
        album: tag.and_then(|t| text(t.album())),
        album_artist: tag.and_then(|t| {
            text(
                t.get_string(lofty::tag::ItemKey::AlbumArtist)
                    .map(Into::into),
            )
        }),
        track_no: tag.and_then(Accessor::track),
        disc_no: tag.and_then(Accessor::disk),
        year: tag
            .and_then(Accessor::date)
            .map(|d| i32::from(d.year))
            .filter(|y| *y > 0),
        genre: tag.and_then(|t| text(t.genre())),
        duration_ms: u64::try_from(props.duration().as_millis()).unwrap_or(u64::MAX),
        sample_rate: props.sample_rate().unwrap_or(0),
        channels: props.channels().map_or(0, u16::from),
        bits_per_sample: props.bit_depth().map_or(0, u16::from),
        codec: Arc::from(codec_name(file.file_type(), mp4_codec)),
        picture: tag.and_then(pick_picture),
    }
}

fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The front cover if there is one, else the first picture; dropped when too large.
fn pick_picture(tag: &Tag) -> Option<Vec<u8>> {
    let pictures = tag.pictures();
    let chosen = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())?;
    let data = chosen.data();
    if data.is_empty() || data.len() > MAX_IMAGE_BYTES {
        tracing::debug!(len = data.len(), "embedded picture dropped");
        return None;
    }
    Some(data.to_vec())
}

fn codec_name(file_type: FileType, mp4_codec: Option<Mp4Codec>) -> &'static str {
    match file_type {
        FileType::Aac => "aac",
        FileType::Aiff => "aiff",
        FileType::Ape => "ape",
        FileType::Flac => "flac",
        FileType::Mpeg => "mp3",
        FileType::Mp4 => match mp4_codec {
            Some(Mp4Codec::ALAC) => "alac",
            Some(Mp4Codec::AAC) => "aac",
            _ => "mp4",
        },
        FileType::Mpc => "mpc",
        FileType::Opus => "opus",
        FileType::Vorbis => "vorbis",
        FileType::Speex => "speex",
        FileType::Wav => "wav",
        FileType::WavPack => "wavpack",
        _ => "unknown",
    }
}
