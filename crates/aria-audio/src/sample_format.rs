//! Integer sample formats for an exclusive output, and how the sound reaches the device.
//!
//! Symphonia turns integer samples into `f32` by dividing by `2^(bits - 1)`: 16-bit audio by
//! 32 768 and 24-bit audio by 8 388 608 (`symphonia-core` 0.6.1, `audio/conv.rs`). FLAC hands
//! over `i32` samples shifted up to fill 32 bits, divided by `2^31`, which gives the same
//! values. [`write_samples`] multiplies by the same powers of two, so a source of 24 bits or
//! fewer comes back exactly.

/// The integer format an exclusive output writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleKind {
    /// 16-bit signed integer.
    I16,
    /// 24-bit signed integer, left-justified in 32 bits (the low byte is zero).
    I24In32,
    /// 32-bit signed integer.
    I32,
}

impl SampleKind {
    /// Bytes one sample takes in the device's buffer.
    pub const fn bytes(self) -> usize {
        match self {
            Self::I16 => 2,
            Self::I24In32 | Self::I32 => 4,
        }
    }

    /// Bits of the sample that carry audio.
    pub const fn bits(self) -> u16 {
        match self {
            Self::I16 => 16,
            Self::I24In32 => 24,
            Self::I32 => 32,
        }
    }
}

const SCALE_16: f64 = 32_768.0; // 2^15
const SCALE_24: f64 = 8_388_608.0; // 2^23
const SCALE_32: f64 = 2_147_483_648.0; // 2^31

/// `sample` scaled by `scale`, rounded to nearest and clamped to `[-scale, scale - 1]`.
/// NaN is silence.
fn to_int(sample: f32, scale: f64) -> i32 {
    if sample.is_nan() {
        return 0;
    }
    let scaled = (f64::from(sample) * scale)
        .round()
        .clamp(-scale, scale - 1.0);
    // In range after the clamp, so the cast is exact.
    scaled as i32
}

/// Writes `from` into `to` as little-endian `kind` samples, without dither.
///
/// Writes `min(from.len(), to.len() / kind.bytes())` samples; the rest of `to` is left as it
/// was. Does not allocate.
pub fn write_samples(kind: SampleKind, from: &[f32], to: &mut [u8]) {
    let chunks = to.chunks_exact_mut(kind.bytes());
    match kind {
        SampleKind::I16 => {
            for (sample, out) in from.iter().zip(chunks) {
                // Within i16 after the clamp in `to_int`.
                let value = to_int(*sample, SCALE_16) as i16;
                out.copy_from_slice(&value.to_le_bytes());
            }
        }
        SampleKind::I24In32 => {
            for (sample, out) in from.iter().zip(chunks) {
                let value = to_int(*sample, SCALE_24) << 8;
                out.copy_from_slice(&value.to_le_bytes());
            }
        }
        SampleKind::I32 => {
            for (sample, out) in from.iter().zip(chunks) {
                let value = to_int(*sample, SCALE_32);
                out.copy_from_slice(&value.to_le_bytes());
            }
        }
    }
}

/// What the decoded track is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFormat {
    pub sample_rate: u32,
    pub channels: u16,
    /// Bits per sample, when the file says.
    pub bits: Option<u16>,
    /// The decoder produces floating-point samples.
    pub float: bool,
}

/// A format an output device is opened with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub kind: SampleKind,
}

/// The integer kinds to try for `source`, best first, in the order of spec 3.2: the source's
/// own depth, 24 bits in 32, 32-bit, 16-bit.
fn kinds_for(source: SourceFormat) -> impl Iterator<Item = SampleKind> {
    let own = match source.bits {
        _ if source.float => Some(SampleKind::I32),
        Some(1..=16) => Some(SampleKind::I16),
        Some(17..=24) => Some(SampleKind::I24In32),
        Some(25..) => Some(SampleKind::I32),
        Some(0) | None => None,
    };
    let rest = [SampleKind::I24In32, SampleKind::I32, SampleKind::I16]
        .into_iter()
        .filter(move |kind| Some(*kind) != own);
    own.into_iter().chain(rest)
}

/// Picks the format to open a device with for `source`.
///
/// Tries the order of spec 3.2: the source's own depth, 24 in 32, 32-bit, 16-bit, at the
/// source's rate; then the same at the nearest supported rate. `rates` are the rates the
/// device supports; `accepts` asks the device about one format. `None` when it takes none.
///
/// A mono source is played to both channels where the device allows it (spec 3.1 counts that as
/// unchanged), so that a stereo track after it can follow on the same output. For each depth it
/// is asked for with two channels first and one channel second, before the next depth; so a
/// mono 16-bit source at 44.1 kHz asks, in order: I16 stereo, I16 mono, I24In32 stereo, I24In32
/// mono, I32 stereo, I32 mono, then the same at the nearest rate. Any other channel count is
/// asked for only as it is.
pub fn choose_format(
    source: SourceFormat,
    rates: &[u32],
    accepts: &mut dyn FnMut(DeviceFormat) -> bool,
) -> Option<DeviceFormat> {
    let layouts: &[u16] = if source.channels == 1 {
        &[2, 1]
    } else {
        std::slice::from_ref(&source.channels)
    };
    let mut try_rate = |sample_rate: u32| {
        kinds_for(source)
            .flat_map(|kind| {
                layouts.iter().map(move |channels| DeviceFormat {
                    sample_rate,
                    channels: *channels,
                    kind,
                })
            })
            .find(|format| accepts(*format))
    };
    if let Some(format) = try_rate(source.sample_rate) {
        return Some(format);
    }
    let others: Vec<u32> = rates
        .iter()
        .copied()
        .filter(|rate| *rate != source.sample_rate)
        .collect();
    nearest_rate(source.sample_rate, &others).and_then(try_rate)
}

/// The supported rate closest in spirit to `wanted`: the smallest whole multiple of it, else
/// the next higher, else the highest. Zero rates are ignored; `None` when nothing is left.
pub fn nearest_rate(wanted: u32, supported: &[u32]) -> Option<u32> {
    let rates = || supported.iter().copied().filter(|rate| *rate > 0);
    let multiple = rates()
        .filter(|rate| wanted > 0 && rate % wanted == 0)
        .min();
    let higher = || rates().filter(|rate| *rate > wanted).min();
    multiple.or_else(higher).or_else(|| rates().max())
}

/// How the sound reaches the device (spec 3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputPath {
    /// Shared output, through the system mixer.
    SystemMixer,
    /// Exclusive output, and every condition of spec 3.1 holds.
    BitPerfect,
    /// Exclusive output at a rate other than the track's.
    Resampled,
    /// Exclusive output, but the samples are not the source's.
    Converted,
    /// Exclusive output with a soundboard sound mixed in.
    MixedWithSound,
}

/// What the engine reports about the current output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputState {
    pub path: OutputPath,
    pub device_rate: u32,
    pub device_bits: u16,
}

/// Everything [`output_path`] decides from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathFacts {
    pub exclusive: bool,
    pub source: SourceFormat,
    pub device: DeviceFormat,
    /// The music gain is exactly 1.0.
    pub gain_is_unity: bool,
    /// The caller passes the track's channels on as they are (mono to both counts as unchanged).
    /// [`output_path`] also checks the channel counts, so a remix is never taken for unchanged.
    pub channels_unchanged: bool,
    pub sound_playing: bool,
}

/// The path the sound takes, in order of precedence: shared output, a sound in the stream,
/// a resample, any conversion; bit-perfect only when none of them applies.
pub fn output_path(facts: PathFacts) -> OutputPath {
    if !facts.exclusive {
        return OutputPath::SystemMixer;
    }
    if facts.sound_playing {
        return OutputPath::MixedWithSound;
    }
    if facts.source.sample_rate != facts.device.sample_rate {
        return OutputPath::Resampled;
    }
    let exact_depth = match facts.source.bits {
        Some(bits @ 1..=24) => bits <= facts.device.kind.bits(),
        // Unknown, or wider than 24 bits: `f32` cannot carry it exactly.
        _ => false,
    };
    let (from, to) = (facts.source.channels, facts.device.channels);
    let channels_unchanged = facts.channels_unchanged && (from == to || (from == 1 && to == 2));
    if !exact_depth || facts.source.float || !channels_unchanged || !facts.gain_is_unity {
        return OutputPath::Converted;
    }
    OutputPath::BitPerfect
}

#[cfg(test)]
mod tests {
    use symphonia::core::audio::conv::FromSample;
    use symphonia::core::audio::sample::i24;

    use super::*;

    fn write_one(kind: SampleKind, sample: f32) -> Vec<u8> {
        let mut out = vec![0xAA; kind.bytes()];
        write_samples(kind, &[sample], &mut out);
        out
    }

    fn as_i32(bytes: &[u8]) -> i32 {
        i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    #[test]
    fn sixteen_bit_values_round_trip_exactly() {
        for v in i16::MIN..=i16::MAX {
            // Symphonia's two paths for 16-bit audio: WAV decodes to i16, FLAC to i32 shifted
            // up by 16. Both must land on `v / 32768.0`.
            let wav = f32::from_sample(v);
            let flac = f32::from_sample(i32::from(v) << 16);
            assert_eq!(wav, f32::from(v) / 32_768.0, "{v}");
            assert_eq!(flac, wav, "{v}");

            assert_eq!(write_one(SampleKind::I16, wav), v.to_le_bytes(), "{v}");
            // Widening is exact too.
            assert_eq!(
                as_i32(&write_one(SampleKind::I24In32, wav)),
                i32::from(v) << 16,
                "{v}"
            );
            assert_eq!(
                as_i32(&write_one(SampleKind::I32, wav)),
                i32::from(v) << 16,
                "{v}"
            );
        }
    }

    #[test]
    fn twenty_four_bit_values_round_trip_exactly() {
        let min = -(1 << 23);
        let max = (1 << 23) - 1;
        let values = (min..=max)
            .step_by(251)
            .chain([min, min + 1, -1, 0, 1, max - 1, max]);
        for v in values {
            // WAV decodes 24-bit audio to i24, FLAC to i32 shifted up by 8.
            let wav = f32::from_sample(i24::from(v));
            let flac = f32::from_sample(v << 8);
            assert_eq!(wav, v as f32 / 8_388_608.0, "{v}");
            assert_eq!(flac, wav, "{v}");

            for kind in [SampleKind::I24In32, SampleKind::I32] {
                let out = write_one(kind, wav);
                let word = as_i32(&out);
                assert_eq!(word >> 8, v, "{kind:?} {v}");
                assert_eq!(word & 0xFF, 0, "{kind:?} {v}");
            }
        }
    }

    #[test]
    fn full_scale_and_beyond_saturates() {
        let cases = [
            (SampleKind::I16, i32::from(i16::MAX), i32::from(i16::MIN)),
            (SampleKind::I24In32, ((1 << 23) - 1) << 8, i32::MIN),
            (SampleKind::I32, i32::MAX, i32::MIN),
        ];
        for (kind, max, min) in cases {
            let read = |bytes: Vec<u8>| match kind {
                SampleKind::I16 => i32::from(i16::from_le_bytes([bytes[0], bytes[1]])),
                _ => as_i32(&bytes),
            };
            for loud in [1.0, 2.0, f32::INFINITY] {
                assert_eq!(read(write_one(kind, loud)), max, "{kind:?} {loud}");
            }
            for quiet in [-1.0, -2.0, f32::NEG_INFINITY] {
                assert_eq!(read(write_one(kind, quiet)), min, "{kind:?} {quiet}");
            }
        }
    }

    #[test]
    fn nan_is_silence() {
        for kind in [SampleKind::I16, SampleKind::I24In32, SampleKind::I32] {
            assert!(
                write_one(kind, f32::NAN).iter().all(|b| *b == 0),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_block_is_written_in_order() {
        let mut out = [0u8; 6];
        write_samples(SampleKind::I16, &[0.5, -0.5, 0.0], &mut out);
        assert_eq!(out, [0x00, 0x40, 0x00, 0xC0, 0x00, 0x00]);

        // A short buffer takes only the samples that fit, and nothing past its end.
        let mut short = [0xAAu8; 5];
        write_samples(SampleKind::I16, &[0.5, -0.5, 0.25], &mut short);
        assert_eq!(short, [0x00, 0x40, 0x00, 0xC0, 0xAA]);
    }

    fn source(sample_rate: u32, bits: Option<u16>) -> SourceFormat {
        SourceFormat {
            sample_rate,
            channels: 2,
            bits,
            float: false,
        }
    }

    /// Runs `choose_format` against a device that accepts `kinds` at `rates`, returning the
    /// result and every format asked about, in order.
    fn choose(
        source: SourceFormat,
        rates: &[u32],
        kinds: &[SampleKind],
    ) -> (Option<DeviceFormat>, Vec<DeviceFormat>) {
        let mut asked = Vec::new();
        let chosen = choose_format(source, rates, &mut |format| {
            asked.push(format);
            rates.contains(&format.sample_rate) && kinds.contains(&format.kind)
        });
        (chosen, asked)
    }

    const ALL_KINDS: [SampleKind; 3] = [SampleKind::I16, SampleKind::I24In32, SampleKind::I32];

    #[test]
    fn the_source_depth_is_tried_first() {
        let (chosen, asked) = choose(source(44_100, Some(16)), &[44_100], &ALL_KINDS);
        assert_eq!(
            chosen,
            Some(DeviceFormat {
                sample_rate: 44_100,
                channels: 2,
                kind: SampleKind::I16
            })
        );
        assert_eq!(asked.len(), 1);

        let (chosen, _) = choose(source(96_000, Some(24)), &[96_000], &ALL_KINDS);
        assert_eq!(chosen.map(|f| f.kind), Some(SampleKind::I24In32));

        let (chosen, _) = choose(source(44_100, Some(32)), &[44_100], &ALL_KINDS);
        assert_eq!(chosen.map(|f| f.kind), Some(SampleKind::I32));

        // A 16-bit source on a device without 16-bit goes on in the order of spec 3.2.
        let (_, asked) = choose(source(44_100, Some(16)), &[44_100], &[]);
        let kinds: Vec<_> = asked.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            [SampleKind::I16, SampleKind::I24In32, SampleKind::I32]
        );
    }

    #[test]
    fn twenty_four_bit_falls_back_in_order() {
        let (chosen, _) = choose(source(96_000, Some(24)), &[96_000], &[SampleKind::I32]);
        assert_eq!(chosen.map(|f| f.kind), Some(SampleKind::I32));

        let (chosen, asked) = choose(source(96_000, Some(24)), &[96_000], &[SampleKind::I16]);
        assert_eq!(chosen.map(|f| f.kind), Some(SampleKind::I16));
        let kinds: Vec<_> = asked.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            [SampleKind::I24In32, SampleKind::I32, SampleKind::I16]
        );
        assert!(asked.iter().all(|f| f.sample_rate == 96_000));
    }

    #[test]
    fn an_unsupported_rate_takes_the_nearest() {
        let (chosen, _) = choose(source(44_100, Some(16)), &[48_000, 88_200], &ALL_KINDS);
        assert_eq!(
            chosen,
            Some(DeviceFormat {
                sample_rate: 88_200,
                channels: 2,
                kind: SampleKind::I16
            })
        );

        let (chosen, asked) = choose(source(96_000, Some(24)), &[44_100, 48_000], &ALL_KINDS);
        assert_eq!(
            chosen,
            Some(DeviceFormat {
                sample_rate: 48_000,
                channels: 2,
                kind: SampleKind::I24In32
            })
        );
        // Every depth is tried at the source's own rate before the rate gives way.
        let rates: Vec<_> = asked.iter().map(|f| f.sample_rate).collect();
        assert_eq!(rates, [96_000, 96_000, 96_000, 48_000]);
    }

    #[test]
    fn nothing_accepted_is_none() {
        let (chosen, asked) = choose(source(44_100, Some(16)), &[44_100, 48_000], &[]);
        assert_eq!(chosen, None);
        assert!(!asked.is_empty());

        let (chosen, _) = choose(source(44_100, Some(16)), &[], &ALL_KINDS);
        assert_eq!(chosen, None);
    }

    #[test]
    fn a_mono_source_asks_for_stereo_first() {
        let mono = SourceFormat {
            sample_rate: 44_100,
            channels: 1,
            bits: Some(16),
            float: false,
        };
        let stereo_cd = DeviceFormat {
            sample_rate: 44_100,
            channels: 2,
            kind: SampleKind::I16,
        };

        // A device that takes both gets stereo; nothing else is asked.
        let (chosen, asked) = choose(mono, &[44_100], &ALL_KINDS);
        assert_eq!(chosen, Some(stereo_cd));
        assert_eq!(asked.len(), 1);

        // A device that takes only mono gets mono, after stereo was asked.
        let mut asked = Vec::new();
        let chosen = choose_format(mono, &[44_100], &mut |format| {
            asked.push(format);
            format.channels == 1
        });
        assert_eq!(chosen.map(|f| f.channels), Some(1));
        let order: Vec<_> = asked.iter().map(|f| (f.kind, f.channels)).collect();
        assert_eq!(order, [(SampleKind::I16, 2), (SampleKind::I16, 1)]);

        // Every depth tries both layouts before the rate gives way.
        let mut asked = Vec::new();
        let chosen = choose_format(mono, &[48_000], &mut |format| {
            asked.push(format);
            format.sample_rate == 48_000 && format.channels == 2
        });
        assert_eq!(
            chosen,
            Some(DeviceFormat {
                sample_rate: 48_000,
                ..stereo_cd
            })
        );
        let order: Vec<_> = asked
            .iter()
            .map(|f| (f.sample_rate, f.kind, f.channels))
            .collect();
        assert_eq!(
            order,
            [
                (44_100, SampleKind::I16, 2),
                (44_100, SampleKind::I16, 1),
                (44_100, SampleKind::I24In32, 2),
                (44_100, SampleKind::I24In32, 1),
                (44_100, SampleKind::I32, 2),
                (44_100, SampleKind::I32, 1),
                (48_000, SampleKind::I16, 2),
            ]
        );

        // Stereo and wider sources are asked for only as they are.
        let (_, asked) = choose(source(44_100, Some(16)), &[44_100], &[]);
        assert!(asked.iter().all(|f| f.channels == 2));
    }

    #[test]
    fn the_nearest_rate_prefers_a_multiple_then_higher_then_highest() {
        assert_eq!(nearest_rate(44_100, &[48_000, 88_200]), Some(88_200));
        assert_eq!(nearest_rate(44_100, &[176_400, 88_200]), Some(88_200));
        assert_eq!(nearest_rate(44_100, &[44_100, 48_000]), Some(44_100));
        assert_eq!(
            nearest_rate(44_100, &[96_000, 48_000, 32_000]),
            Some(48_000)
        );
        assert_eq!(nearest_rate(96_000, &[44_100, 48_000]), Some(48_000));
        assert_eq!(nearest_rate(44_100, &[]), None);
        // Zero has no multiples; every rate is higher.
        assert_eq!(nearest_rate(0, &[48_000, 44_100]), Some(44_100));
        assert_eq!(nearest_rate(44_100, &[0]), None);
    }

    fn bit_perfect_facts() -> PathFacts {
        PathFacts {
            exclusive: true,
            source: SourceFormat {
                sample_rate: 96_000,
                channels: 2,
                bits: Some(24),
                float: false,
            },
            device: DeviceFormat {
                sample_rate: 96_000,
                channels: 2,
                kind: SampleKind::I32,
            },
            gain_is_unity: true,
            channels_unchanged: true,
            sound_playing: false,
        }
    }

    #[test]
    fn the_state_is_bit_perfect_only_when_every_condition_holds() {
        let base = bit_perfect_facts();
        assert_eq!(output_path(base), OutputPath::BitPerfect);

        use OutputPath::{BitPerfect, Converted, MixedWithSound, Resampled, SystemMixer};
        type Flip = fn(&mut PathFacts);
        let rows: [(&str, Flip, OutputPath); 11] = [
            ("shared output", |f| f.exclusive = false, SystemMixer),
            (
                "a sound playing",
                |f| f.sound_playing = true,
                MixedWithSound,
            ),
            ("another rate", |f| f.device.sample_rate = 48_000, Resampled),
            (
                "depth reduced",
                |f| f.device.kind = SampleKind::I16,
                Converted,
            ),
            (
                "channels changed",
                |f| f.channels_unchanged = false,
                Converted,
            ),
            ("float source", |f| f.source.float = true, Converted),
            (
                "wider than 24 bits",
                |f| f.source.bits = Some(32),
                Converted,
            ),
            ("25 bits", |f| f.source.bits = Some(25), Converted),
            ("24 bits", |f| f.source.bits = Some(24), BitPerfect),
            ("unknown depth", |f| f.source.bits = None, Converted),
            ("gain not unity", |f| f.gain_is_unity = false, Converted),
        ];
        for (name, flip, expected) in rows {
            let mut facts = base;
            flip(&mut facts);
            assert_eq!(output_path(facts), expected, "{name}");
        }
    }

    #[test]
    fn the_channel_counts_are_checked_whatever_the_caller_says() {
        let mut facts = bit_perfect_facts();
        // A 5.1 source on a stereo device is a remix, even if the caller calls it unchanged.
        facts.source.channels = 6;
        assert_eq!(output_path(facts), OutputPath::Converted, "6 to 2");

        // Mono to both channels is unchanged, when the caller says so.
        facts.source.channels = 1;
        assert_eq!(output_path(facts), OutputPath::BitPerfect, "1 to 2");
        facts.channels_unchanged = false;
        assert_eq!(output_path(facts), OutputPath::Converted, "1 to 2, remixed");

        // Mono to anything wider than stereo is not.
        facts.channels_unchanged = true;
        facts.device.channels = 6;
        assert_eq!(output_path(facts), OutputPath::Converted, "1 to 6");
    }

    #[test]
    fn samples_round_to_the_nearest_step() {
        let i16_of = |sample: f32| {
            let bytes = write_one(SampleKind::I16, sample);
            i16::from_le_bytes([bytes[0], bytes[1]])
        };
        // Truncation would give 1, 1 and -1 here.
        assert_eq!(i16_of(1.5 / 32_768.0), 2);
        assert_eq!(i16_of(1.6 / 32_768.0), 2);
        assert_eq!(i16_of(1.4 / 32_768.0), 1);
        assert_eq!(i16_of(-1.5 / 32_768.0), -2);
        assert_eq!(i16_of(-1.6 / 32_768.0), -2);
        assert_eq!(i16_of(-1.4 / 32_768.0), -1);

        let i24_of = |sample: f32| as_i32(&write_one(SampleKind::I24In32, sample)) >> 8;
        assert_eq!(i24_of(1.5 / 8_388_608.0), 2);
        assert_eq!(i24_of(-1.6 / 8_388_608.0), -2);
    }

    #[test]
    fn widening_keeps_bit_perfect() {
        for (bits, kind) in [
            (16, SampleKind::I16),
            (16, SampleKind::I24In32),
            (16, SampleKind::I32),
            (24, SampleKind::I24In32),
            (24, SampleKind::I32),
        ] {
            let mut facts = bit_perfect_facts();
            facts.source.bits = Some(bits);
            facts.device.kind = kind;
            assert_eq!(
                output_path(facts),
                OutputPath::BitPerfect,
                "{bits} {kind:?}"
            );
        }
        let mut facts = bit_perfect_facts();
        facts.source.bits = Some(0);
        assert_eq!(output_path(facts), OutputPath::Converted, "zero bits");
    }

    #[test]
    fn the_path_follows_the_order_of_precedence() {
        let mut facts = bit_perfect_facts();
        facts.gain_is_unity = false;
        facts.device.sample_rate = 48_000;
        assert_eq!(output_path(facts), OutputPath::Resampled);
        facts.sound_playing = true;
        assert_eq!(output_path(facts), OutputPath::MixedWithSound);
        facts.exclusive = false;
        assert_eq!(output_path(facts), OutputPath::SystemMixer);
    }
}
