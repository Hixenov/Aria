//! Encoder delay and length from an MP4 edit list.
//!
//! Symphonia reads the `elst` atom but does not turn it into trimming, so AAC in `.m4a`
//! would play with its priming frames and padding. This finds them directly.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Largest `moov` atom read into memory.
const MAX_MOOV_BYTES: u64 = 64 * 1024 * 1024;

/// The part of a track a player should keep, in frames at the track's sample rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EditTrim {
    /// Leading frames to drop.
    pub delay: u64,
    /// Frames to play after the delay.
    pub length: u64,
}

/// The edit-list trim of the audio track in the MP4 at `path`, if it has a simple one.
///
/// Returns `None` for anything that is not an MP4, has no edit list, or has an edit list
/// that is not one plain run of media (empty edits, several segments, a timescale that is
/// not the sample rate).
pub(crate) fn edit_trim(path: &Path, sample_rate: u32) -> Option<EditTrim> {
    let mut file = File::open(path).ok()?;
    let moov = find_moov(&mut file)?;
    let movie_scale = u64::from(movie_timescale(&moov)?);

    let (_, trak) = children(&moov).find(|(kind, body)| {
        kind == b"trak" && handler_type(body).is_some_and(|h| h == *b"soun")
    })?;
    let media_scale = u64::from(media_timescale(trak)?);
    if media_scale != u64::from(sample_rate) || movie_scale == 0 {
        return None;
    }

    let (_, edts) = children(trak).find(|(kind, _)| kind == b"edts")?;
    let (_, elst) = children(edts).find(|(kind, _)| kind == b"elst")?;
    let (segment, media_time) = single_edit(elst)?;
    edit_to_trim(segment, media_time, media_scale, movie_scale)
}

/// Convert an edit entry (segment length in movie units, start in media units) to frames.
fn edit_to_trim(
    segment: u64,
    media_time: i64,
    media_scale: u64,
    movie_scale: u64,
) -> Option<EditTrim> {
    // The scaling is done in u128 so a hostile segment length cannot overflow.
    let length = u128::from(segment) * u128::from(media_scale) / u128::from(movie_scale);
    Some(EditTrim {
        delay: u64::try_from(media_time).ok()?,
        length: u64::try_from(length).ok()?,
    })
}

/// Read the `moov` atom's contents from the top level of the file.
fn find_moov(file: &mut File) -> Option<Vec<u8>> {
    let file_len = file.metadata().ok()?.len();
    let mut pos = 0u64;
    while pos + 8 <= file_len {
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut head = [0u8; 16];
        file.read_exact(&mut head[..8]).ok()?;
        let mut size = u64::from(u32::from_be_bytes(head[..4].try_into().ok()?));
        let mut header_len = 8;
        if size == 1 {
            file.read_exact(&mut head[8..16]).ok()?;
            size = u64::from_be_bytes(head[8..16].try_into().ok()?);
            header_len = 16;
        } else if size == 0 {
            size = file_len - pos;
        }
        if size < header_len {
            return None;
        }
        if pos == 0 && &head[4..8] != b"ftyp" {
            return None;
        }
        if &head[4..8] == b"moov" {
            if pos.checked_add(size)? > file_len {
                return None;
            }
            let body_len = size - header_len;
            if body_len > MAX_MOOV_BYTES {
                return None;
            }
            let mut body = vec![0u8; usize::try_from(body_len).ok()?];
            file.seek(SeekFrom::Start(pos + header_len)).ok()?;
            file.read_exact(&mut body).ok()?;
            return Some(body);
        }
        pos = pos.checked_add(size)?;
    }
    None
}

/// The atoms packed one after another in `data`, as (type, body).
fn children(data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut rest = data;
    std::iter::from_fn(move || {
        let size = u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize;
        let kind: [u8; 4] = rest.get(4..8)?.try_into().ok()?;
        // 64-bit and to-the-end sizes only occur for `mdat`, never inside `moov`.
        if size < 8 || size > rest.len() {
            return None;
        }
        let body = &rest[8..size];
        rest = &rest[size..];
        Some((kind, body))
    })
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn be_u64(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(data.get(at..at + 8)?.try_into().ok()?))
}

fn movie_timescale(moov: &[u8]) -> Option<u32> {
    let (_, mvhd) = children(moov).find(|(kind, _)| kind == b"mvhd")?;
    // Version, flags, then creation and modification times of 4 or 8 bytes each.
    let at = if *mvhd.first()? == 1 { 20 } else { 12 };
    be_u32(mvhd, at)
}

fn media_timescale(trak: &[u8]) -> Option<u32> {
    let (_, mdia) = children(trak).find(|(kind, _)| kind == b"mdia")?;
    let (_, mdhd) = children(mdia).find(|(kind, _)| kind == b"mdhd")?;
    let at = if *mdhd.first()? == 1 { 20 } else { 12 };
    be_u32(mdhd, at)
}

fn handler_type(trak: &[u8]) -> Option<[u8; 4]> {
    let (_, mdia) = children(trak).find(|(kind, _)| kind == b"mdia")?;
    let (_, hdlr) = children(mdia).find(|(kind, _)| kind == b"hdlr")?;
    // Version and flags, a predefined field, then the handler type.
    hdlr.get(8..12)?.try_into().ok()
}

/// (segment duration, media time) of an edit list with exactly one non-empty entry.
fn single_edit(elst: &[u8]) -> Option<(u64, i64)> {
    let version = *elst.first()?;
    if be_u32(elst, 4)? != 1 {
        return None;
    }
    let (segment, media_time) = if version == 1 {
        (be_u64(elst, 8)?, be_u64(elst, 16)? as i64)
    } else {
        (
            u64::from(be_u32(elst, 8)?),
            i64::from(be_u32(elst, 12)? as i32),
        )
    };
    Some((segment, media_time))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_aac_fixture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.m4a");
        let trim = edit_trim(&path, 44_100).unwrap();
        assert_eq!(
            trim,
            EditTrim {
                delay: 1024,
                length: 44_100
            }
        );
    }

    #[test]
    fn hostile_edit_values_do_not_overflow() {
        assert_eq!(edit_to_trim(u64::MAX, 0, u64::MAX, 1), None);
        assert_eq!(edit_to_trim(u64::MAX, 0, 48_000, 1), None);
        assert_eq!(edit_to_trim(10, -1, 48_000, 1), None);
        assert_eq!(
            edit_to_trim(u64::MAX, 5, 1, 1),
            Some(EditTrim {
                delay: 5,
                length: u64::MAX
            })
        );
        // A version 1 edit list carrying the largest segment and media time.
        let mut elst = vec![1, 0, 0, 0, 0, 0, 0, 1];
        elst.extend_from_slice(&u64::MAX.to_be_bytes());
        elst.extend_from_slice(&i64::MAX.to_be_bytes());
        elst.extend_from_slice(&[0, 1, 0, 0]);
        let (segment, media_time) = single_edit(&elst).unwrap();
        assert_eq!(edit_to_trim(segment, media_time, 44_100, 1_000), None);
        // Truncated lists are rejected rather than read past the end.
        assert_eq!(single_edit(&elst[..20]), None);
    }

    #[test]
    fn ignores_other_files() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.wav");
        assert_eq!(edit_trim(&path, 44_100), None);
        assert_eq!(edit_trim(Path::new("does-not-exist.m4a"), 44_100), None);
    }
}
