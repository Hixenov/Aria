//! Decoder and resampler behaviour against the checked-in fixtures.

use std::path::{Path, PathBuf};

use aria_audio::AudioError;
use aria_audio::decode::{TrackDecoder, decode_clip};
use aria_audio::resample::{Resampler, convert_channels};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Decode a whole file, returning (interleaved samples, channels).
fn decode_all(path: &Path) -> Result<(Vec<f32>, usize), AudioError> {
    let mut dec = TrackDecoder::open(path)?;
    let channels = usize::from(dec.info().channels);
    let mut all = Vec::new();
    while let Some(block) = dec.next_block()? {
        all.extend_from_slice(block);
    }
    Ok((all, channels))
}

fn rms(samples: &[f32]) -> f32 {
    let sum: f64 = samples.iter().map(|s| f64::from(*s).powi(2)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

#[test]
fn every_format_decodes() {
    let cases = [
        ("tone.mp3", 44_100.0),
        ("tone.flac", 44_100.0),
        ("tone.wav", 44_100.0),
        ("tone.m4a", 44_100.0),
        ("tone.ogg", 44_100.0),
        ("tone.opus", 48_000.0),
        ("tone-alac.m4a", 44_100.0),
        ("tone.aiff", 44_100.0),
        ("tone.caf", 44_100.0),
    ];
    for (name, expected) in cases {
        let (samples, channels) = decode_all(&fixture(name)).unwrap();
        assert_eq!(channels, 2, "{name}");
        let frames = (samples.len() / channels) as f64;
        assert!(
            (frames - expected).abs() <= expected * 0.02,
            "{name}: {frames} frames, expected about {expected}"
        );
        assert!(rms(&samples) > 0.05, "{name}: rms {}", rms(&samples));
    }
}

#[test]
fn gapless_trim_is_exact_for_flac() {
    let mut dec = TrackDecoder::open(&fixture("tone.flac")).unwrap();
    let total = dec.info().total_frames.unwrap();
    let channels = usize::from(dec.info().channels);
    let mut frames = 0u64;
    while let Some(block) = dec.next_block().unwrap() {
        frames += (block.len() / channels) as u64;
    }
    assert_eq!(frames, total);
}

#[test]
fn delay_and_padding_are_trimmed_exactly_once() {
    // Every fixture is 1 s long; if a format were trimmed twice or not at all, its decoded
    // length would drift from the length the container declares.
    for name in [
        "tone.mp3",
        "tone.m4a",
        "tone.ogg",
        "tone.opus",
        "tone-alac.m4a",
    ] {
        let mut dec = TrackDecoder::open(&fixture(name)).unwrap();
        let declared = dec.info().total_frames.unwrap();
        let channels = usize::from(dec.info().channels);
        let mut frames = 0u64;
        while let Some(block) = dec.next_block().unwrap() {
            frames += (block.len() / channels) as u64;
        }
        assert_eq!(frames, declared, "{name}");
    }
}

#[test]
fn seek_is_accurate() {
    let exact = ["tone.flac", "tone.wav", "tone.aiff", "tone.caf"];
    let loose = [
        "tone.mp3",
        "tone.m4a",
        "tone.ogg",
        "tone.opus",
        "tone-alac.m4a",
    ];
    for name in exact.iter().chain(loose.iter()) {
        let mut dec = TrackDecoder::open(&fixture(name)).unwrap();
        // The target is half a second in, in this file's own rate.
        let target = u64::from(dec.info().sample_rate) / 2;
        let reached = dec.seek(target).unwrap();
        let tolerance = if exact.contains(name) { 0 } else { 2_048 };
        assert!(
            reached.abs_diff(target) <= tolerance,
            "{name}: reached {reached}, wanted {target}"
        );
        // Decoding continues from there.
        let block = dec.next_block().unwrap();
        assert!(block.is_some_and(|b| !b.is_empty()), "{name}");
    }
    // The brief's literal case: frame 22,050 on the 44.1 kHz exact formats.
    for name in exact {
        let mut dec = TrackDecoder::open(&fixture(name)).unwrap();
        assert_eq!(dec.seek(22_050).unwrap(), 22_050, "{name}");
    }
}

#[test]
fn seek_lands_on_the_right_samples() {
    // A block read after a seek must match the same samples from a straight decode.
    for name in ["tone.wav", "tone.flac"] {
        let (all, channels) = decode_all(&fixture(name)).unwrap();
        let mut dec = TrackDecoder::open(&fixture(name)).unwrap();
        dec.seek(10_000).unwrap();
        let block = dec.next_block().unwrap().unwrap();
        assert_eq!(
            block[..16],
            all[10_000 * channels..10_000 * channels + 16],
            "{name}"
        );
    }
}

/// Lag, in frames, at which the first block after `seek(target)` best matches a straight
/// decode of the same file. Zero means the seek landed exactly where it said.
fn seek_lag(name: &str, target: u64) -> Result<i64, AudioError> {
    let (all, channels) = decode_all(&fixture(name))?;
    // Left channel only; white noise has no repeating period, so the match is unambiguous.
    let straight: Vec<f32> = all.chunks_exact(channels).map(|f| f[0]).collect();

    let mut dec = TrackDecoder::open(&fixture(name))?;
    // Play a little first, so the decoder is not in its fresh state.
    for _ in 0..3 {
        dec.next_block()?;
    }
    let reached = dec.seek(target)?;
    assert_eq!(reached, target, "{name}");
    let mut after = Vec::new();
    while after.len() < 1_024 * channels {
        let block = dec
            .next_block()?
            .ok_or_else(|| AudioError::Decode("file ended too soon".into()))?;
        after.extend_from_slice(block);
    }
    let after: Vec<f32> = after
        .chunks_exact(channels)
        .map(|f| f[0])
        .take(1_024)
        .collect();

    let mut best = (f64::MIN, 0i64);
    for lag in -600i64..=600 {
        let start = (target as i64 + lag) as usize;
        let window = &straight[start..start + after.len()];
        let dot: f64 = window
            .iter()
            .zip(&after)
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum();
        if dot > best.0 {
            best = (dot, lag);
        }
    }
    Ok(best.1)
}

#[test]
fn seek_lands_on_the_right_samples_in_lossy_formats() {
    let cases: [(&str, [u64; 6]); 4] = [
        ("noise.mp3", [3_000, 5_000, 10_000, 15_000, 22_050, 30_000]),
        ("noise.m4a", [3_000, 5_000, 10_000, 15_000, 22_050, 30_000]),
        ("noise.ogg", [3_000, 5_000, 10_000, 15_000, 22_050, 30_000]),
        // Opus is 48 kHz.
        ("noise.opus", [3_000, 5_000, 10_000, 15_000, 24_000, 30_000]),
    ];
    for (name, targets) in cases {
        for target in targets {
            let lag = seek_lag(name, target).unwrap();
            assert!(
                lag.abs() <= 2,
                "{name}: seek({target}) landed {lag} frames off"
            );
        }
    }
}

#[test]
fn garbage_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.mp3");
    std::fs::write(&empty, b"").unwrap();
    assert!(TrackDecoder::open(&empty).is_err());

    let text = dir.path().join("text.flac");
    std::fs::write(&text, "this is not audio, it is a sentence.\n".repeat(200)).unwrap();
    assert!(TrackDecoder::open(&text).is_err());

    assert!(TrackDecoder::open(&dir.path().join("missing.wav")).is_err());
}

#[test]
fn truncated_file_ends_cleanly() {
    let bytes = std::fs::read(fixture("tone.flac")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("half.flac");
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();

    let mut dec = TrackDecoder::open(&path).unwrap();
    let mut blocks = 0;
    while let Ok(Some(_)) = dec.next_block() {
        blocks += 1;
        assert!(blocks < 10_000, "decoder never ended");
    }
    assert!(blocks > 0);
}

#[test]
fn unicode_path_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("тест-թեստ-テスト.wav");
    std::fs::copy(fixture("tone.wav"), &path).unwrap();
    let mut dec = TrackDecoder::open(&path).unwrap();
    assert!(dec.next_block().unwrap().is_some());
}

#[test]
fn resampler_keeps_duration_and_pitch() {
    let rate_in = 44_100usize;
    let input: Vec<f32> = (0..rate_in)
        .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / rate_in as f32).sin() * 0.5)
        .collect();

    let mut resampler = Resampler::new(44_100, 48_000, 1).unwrap();
    let mut out = Vec::new();
    // Feed in uneven pieces, as a decoder would.
    for piece in input.chunks(1_000) {
        resampler.process(piece, &mut out).unwrap();
    }
    resampler.flush(&mut out).unwrap();

    assert!(
        out.len().abs_diff(48_000) <= 64,
        "{} frames, expected 48000",
        out.len()
    );
    let crossings = out
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    // 440 Hz crosses zero twice a cycle: 880 a second.
    let seconds = out.len() as f64 / 48_000.0;
    let per_second = crossings as f64 / seconds;
    assert!(
        (per_second - 880.0).abs() <= 8.8,
        "{per_second} crossings a second"
    );
}

#[test]
fn resampler_passes_through_and_resets() {
    let mut passthrough = Resampler::new(48_000, 48_000, 2).unwrap();
    let mut out = Vec::new();
    passthrough
        .process(&[0.1, 0.2, 0.3, 0.4], &mut out)
        .unwrap();
    passthrough.flush(&mut out).unwrap();
    assert_eq!(out, [0.1, 0.2, 0.3, 0.4]);

    let tone: Vec<f32> = (0..4_410).map(|i| (i as f32 * 0.06).sin() * 0.5).collect();
    let mut resampler = Resampler::new(44_100, 22_050, 1).unwrap();
    let mut first = Vec::new();
    resampler.process(&tone, &mut first).unwrap();
    resampler.flush(&mut first).unwrap();
    resampler.reset();
    let mut second = Vec::new();
    resampler.process(&tone, &mut second).unwrap();
    resampler.flush(&mut second).unwrap();
    assert_eq!(first.len(), 2_205);
    assert_eq!(first, second);
}

#[test]
fn resampler_rejects_nonsense() {
    assert!(Resampler::new(0, 48_000, 2).is_err());
    assert!(Resampler::new(44_100, 0, 2).is_err());
    assert!(Resampler::new(44_100, 48_000, 0).is_err());
}

#[test]
fn channels_convert() {
    let mut out = Vec::new();
    convert_channels(&[1.0, 2.0], 1, 2, &mut out);
    assert_eq!(out, [1.0, 1.0, 2.0, 2.0]);

    out.clear();
    convert_channels(&[1.0, 3.0, 2.0, 6.0], 2, 1, &mut out);
    assert_eq!(out, [2.0, 4.0]);

    out.clear();
    convert_channels(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 6, 2, &mut out);
    assert_eq!(out, [1.0, 2.0]);

    out.clear();
    convert_channels(&[1.0, 2.0], 2, 4, &mut out);
    assert_eq!(out, [1.0, 2.0, 0.0, 0.0]);

    out.clear();
    convert_channels(&[1.0, 2.0], 2, 2, &mut out);
    assert_eq!(out, [1.0, 2.0]);
}

#[test]
fn decode_clip_limits_length() {
    // 44.1 kHz stereo source to 48 kHz mono.
    let clip = decode_clip(&fixture("tone.wav"), 1_000, 48_000, 1).unwrap();
    assert_eq!(clip.sample_rate, 48_000);
    assert_eq!(clip.channels, 1);
    assert_eq!(clip.samples.len(), 1_000);

    // No resampling or remixing needed.
    let clip = decode_clip(&fixture("tone.wav"), 1_000, 44_100, 2).unwrap();
    assert_eq!(clip.samples.len(), 2_000);

    // A limit past the end gives the whole file.
    let clip = decode_clip(&fixture("tone.wav"), 10_000_000, 44_100, 2).unwrap();
    assert_eq!(clip.samples.len(), 44_100 * 2);
}
