//! Test helpers that write small 16-bit PCM WAV files without extra dependencies.

use std::f32::consts::TAU;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

const AMPLITUDE: f32 = 0.5;

/// Sample `index` of a sine at `freq` Hz, as a 16-bit value.
fn sample(index: u64, rate: u32, freq: f32) -> i16 {
    // Reduce the phase in f64 so long tones stay accurate.
    let phase = (index as f64 * f64::from(freq) / f64::from(rate)).fract() as f32;
    (AMPLITUDE * (TAU * phase).sin() * f32::from(i16::MAX)).round() as i16
}

fn write_wav<W: Write>(
    out: &mut W,
    rate: u32,
    channels: u16,
    first_frame: u64,
    frames: u64,
    freq: f32,
) -> io::Result<()> {
    let too_big = || io::Error::new(io::ErrorKind::InvalidInput, "WAV data too large");
    let data_len = frames
        .checked_mul(u64::from(channels))
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| n.checked_add(36).is_some())
        .ok_or_else(too_big)?;
    let block_align = channels.checked_mul(2).ok_or_else(too_big)?;
    let byte_rate = rate
        .checked_mul(u32::from(block_align))
        .ok_or_else(too_big)?;

    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_len).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?; // PCM
    out.write_all(&channels.to_le_bytes())?;
    out.write_all(&rate.to_le_bytes())?;
    out.write_all(&byte_rate.to_le_bytes())?;
    out.write_all(&block_align.to_le_bytes())?;
    out.write_all(&16u16.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    for frame in 0..frames {
        let bytes = sample(first_frame + frame, rate, freq).to_le_bytes();
        for _ in 0..channels {
            out.write_all(&bytes)?;
        }
    }
    Ok(())
}

fn write_file(
    path: &Path,
    rate: u32,
    channels: u16,
    first_frame: u64,
    frames: u64,
    freq: f32,
) -> io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    write_wav(&mut out, rate, channels, first_frame, frames, freq)?;
    out.flush()
}

/// Write a 16-bit PCM WAV holding a sine tone of `frames` frames.
pub fn write_tone_wav(
    path: &Path,
    rate: u32,
    channels: u16,
    frames: u64,
    freq: f32,
) -> io::Result<()> {
    write_file(path, rate, channels, 0, frames, freq)
}

/// Write two mono WAVs that are consecutive halves of one continuous tone.
pub fn write_gapless_pair(
    a: &Path,
    b: &Path,
    rate: u32,
    freq: f32,
    frames_each: u64,
) -> io::Result<()> {
    write_file(a, rate, 1, 0, frames_each, freq)?;
    write_file(b, rate, 1, frames_each, frames_each, freq)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(b: &[u8], i: usize) -> u16 {
        u16::from_le_bytes([b[i], b[i + 1]])
    }
    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
    }
    fn samples(b: &[u8]) -> Vec<i16> {
        b[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect()
    }

    #[test]
    fn tone_header_and_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wav");
        write_tone_wav(&path, 44_100, 2, 1_000, 440.0).unwrap();
        let b = std::fs::read(&path).unwrap();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(u16_at(&b, 20), 1);
        assert_eq!(u16_at(&b, 22), 2);
        assert_eq!(u32_at(&b, 24), 44_100);
        assert_eq!(u32_at(&b, 28), 44_100 * 4);
        assert_eq!(u16_at(&b, 32), 4);
        assert_eq!(u16_at(&b, 34), 16);
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32_at(&b, 40), 4_000);
        assert_eq!(u32_at(&b, 4), 36 + 4_000);
        assert_eq!(b.len(), 44 + 4_000);
        let s = samples(&b);
        assert_eq!(s[0], 0);
        assert_eq!(s[0], s[1], "channels carry the same sample");
        assert!(s.iter().any(|v| v.abs() > 10_000));
    }

    #[test]
    fn pair_continues_the_sine() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.wav"), dir.path().join("b.wav"));
        let (rate, freq, n) = (48_000, 440.0, 4_800u64);
        write_gapless_pair(&a, &b, rate, freq, n).unwrap();
        let sa = samples(&std::fs::read(&a).unwrap());
        let sb = samples(&std::fs::read(&b).unwrap());
        assert_eq!(sa.len() as u64, n);
        assert_eq!(sb.len() as u64, n);
        assert_eq!(sa[sa.len() - 1], sample(n - 1, rate, freq));
        assert_eq!(sb[0], sample(n, rate, freq));
        // One continuous tone: the step across the cut is an ordinary sample step.
        let step = |x: i16, y: i16| (i32::from(x) - i32::from(y)).abs();
        let across = step(sb[0], sa[sa.len() - 1]);
        let within = step(sa[2], sa[1]);
        assert!(across <= within + 200, "across {across}, within {within}");
    }
}
