//! Reading ReplayGain from files, for levelling the volume between tracks.

use std::fs;
use std::path::{Path, PathBuf};

use aria_library::{ReplayGain, read_replay_gain};
use lofty::config::WriteOptions;
use lofty::tag::{ItemKey, Tag, TagExt, TagType};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../aria-audio/tests/fixtures")
        .join(name)
}

/// A copy of fixture `name` in `dir`, with a tag of `items` written into it.
fn tagged(
    dir: &Path,
    name: &str,
    tag_type: TagType,
    items: &[(ItemKey, &str)],
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let copy = dir.join(name);
    fs::copy(fixture(name), &copy)?;
    let mut tag = Tag::new(tag_type);
    for (key, value) in items {
        assert!(tag.insert_text(*key, (*value).to_owned()), "{key:?}");
    }
    tag.save_to_path(&copy, WriteOptions::default())?;
    Ok(copy)
}

const ALL: [(ItemKey, &str); 4] = [
    (ItemKey::ReplayGainTrackGain, "-6.50 dB"),
    (ItemKey::ReplayGainTrackPeak, "0.988525"),
    (ItemKey::ReplayGainAlbumGain, "-7.25 dB"),
    (ItemKey::ReplayGainAlbumPeak, "1.000000"),
];

#[test]
fn replay_gain_is_read_from_id3_and_vorbis_comments() {
    let dir = tempfile::tempdir().unwrap();
    for (name, tag_type) in [
        ("tone.mp3", TagType::Id3v2),
        ("tone.flac", TagType::VorbisComments),
    ] {
        let path = tagged(dir.path(), name, tag_type, &ALL).unwrap();
        let gain = read_replay_gain(&path);
        assert_eq!(
            gain,
            ReplayGain {
                track_gain_db: Some(-6.5),
                track_peak: Some(0.988_525),
                album_gain_db: Some(-7.25),
                album_peak: Some(1.0),
            },
            "{name}"
        );
    }
}

#[test]
fn a_file_without_replay_gain_has_none() {
    assert_eq!(
        read_replay_gain(&fixture("tone.flac")),
        ReplayGain::default()
    );
    // And one that cannot be read at all.
    assert_eq!(
        read_replay_gain(Path::new("no such file.mp3")),
        ReplayGain::default()
    );
}

#[test]
fn odd_spellings_of_a_gain_are_read_and_junk_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let path = tagged(
        dir.path(),
        "tone.flac",
        TagType::VorbisComments,
        &[
            (ItemKey::ReplayGainTrackGain, "+3.1dB"),
            (ItemKey::ReplayGainTrackPeak, "loud"),
        ],
    )
    .unwrap();
    let gain = read_replay_gain(&path);
    assert_eq!(gain.track_gain_db, Some(3.1));
    assert_eq!(gain.track_peak, None);
}

#[test]
fn the_linear_gain_follows_the_mode_and_never_clips() {
    let gain = ReplayGain {
        track_gain_db: Some(-6.0),
        track_peak: Some(0.5),
        album_gain_db: Some(-12.0),
        album_peak: Some(0.5),
    };
    // -6 dB is about half.
    assert!((gain.linear(false) - 0.501).abs() < 0.001);
    assert!((gain.linear(true) - 0.251).abs() < 0.001);
    // +12 dB on a peak of 0.5 would clip; it is held at 2.0, where the peak reaches 1.0.
    let loud = ReplayGain {
        track_gain_db: Some(12.0),
        track_peak: Some(0.5),
        ..ReplayGain::default()
    };
    assert!((loud.linear(false) - 2.0).abs() < 1e-6);
    // Album mode without an album gain uses the track's; without any, nothing changes.
    assert!((loud.linear(true) - 2.0).abs() < 1e-6);
    assert_eq!(ReplayGain::default().linear(true), 1.0);
    // Without a peak a positive gain is applied as it is.
    let no_peak = ReplayGain {
        track_gain_db: Some(6.0),
        ..ReplayGain::default()
    };
    assert!((no_peak.linear(false) - 1.995).abs() < 0.001);
}
