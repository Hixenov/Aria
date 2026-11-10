//! Decoding audio files to interleaved `f32`.

use std::fs::File;
use std::path::Path;
use std::sync::OnceLock;

use symphonia::core::audio::sample::Sample as _;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase, Timestamp};
use symphonia_adapter_libopus::OpusDecoder;

use crate::error::AudioError;
use crate::mp4;
use crate::resample::{Resampler, convert_channels};

/// Packets in a row that may fail to decode before the stream is given up on.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
const NANOS_PER_SEC: i128 = 1_000_000_000;
/// How far before a seek target to start decoding, in frames. Must exceed one packet.
const SEEK_PREROLL_FRAMES: u64 = 4096;

fn codecs() -> &'static CodecRegistry {
    static REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_audio_decoder::<OpusDecoder>();
        registry
    })
}

/// What a decoder reports about the stream it will produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    pub sample_rate: u32,
    pub channels: u16,
    /// Playable frames, excluding encoder delay and padding, when the container says.
    pub total_frames: Option<u64>,
    /// Short codec name, such as `flac`.
    pub codec: String,
}

/// Decodes one audio file, block by block.
///
/// Encoder delay and padding are trimmed by Symphonia itself for MP3, Vorbis and Opus: the
/// demuxers mark them on each packet and the decoders drop those frames. Trimming again
/// here would cut real audio, so this type does not. MP4 is the exception: Symphonia
/// ignores its edit list, so the delay and length come from [`mp4::edit_trim`] and are
/// applied here.
pub struct TrackDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    params: AudioCodecParameters,
    /// Frames by which packet timestamps run ahead of the decoded audio (Ogg Opus pre-skip).
    pts_offset: u64,
    time_base: Option<TimeBase>,
    info: StreamInfo,
    block: Vec<f32>,
    /// Frames to drop from the front of the container's timeline (MP4 priming).
    lead_trim: u64,
    /// Frames still to drop from the front of the decoded audio.
    skip_frames: u64,
    /// Frames left to play before the trimmed end, when the container says.
    remaining: Option<u64>,
    /// After a seek: the container frame the next audio should start at.
    align_to: Option<u64>,
    /// After a seek: the next packet only primes the decoder and is not played.
    priming: bool,
    failures: u32,
    ended: bool,
}

impl TrackDecoder {
    /// Open `path` and prepare to decode its first audio track.
    pub fn open(path: &Path) -> Result<Self, AudioError> {
        tracing::debug!(path = %path.display(), "opening audio file");
        let file = File::open(path).map_err(|e| AudioError::Open(e.to_string()))?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| match e {
                SymphoniaError::IoError(e) => AudioError::Open(e.to_string()),
                other => AudioError::Unsupported(other.to_string()),
            })?;

        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| AudioError::Unsupported("no audio track".into()))?;
        let Some(CodecParameters::Audio(params)) = track.codec_params.as_ref() else {
            return Err(AudioError::Unsupported("no audio parameters".into()));
        };
        let sample_rate = params
            .sample_rate
            .ok_or_else(|| AudioError::Unsupported("unknown sample rate".into()))?;
        let channels = params
            .channels
            .as_ref()
            .map(|c| c.count())
            .and_then(|c| u16::try_from(c).ok())
            .filter(|c| *c > 0)
            .ok_or_else(|| AudioError::Unsupported("unknown channel layout".into()))?;
        tracing::debug!(
            path = %path.display(),
            delay = ?track.delay,
            padding = ?track.padding,
            frames = ?track.num_frames,
            "found audio track"
        );

        let decoder = codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|e| AudioError::Unsupported(e.to_string()))?;
        let edit = mp4::edit_trim(path, sample_rate);
        // Symphonia's Ogg reader counts Opus pre-skip in the track length, but not the
        // delay of the other codecs; the decoder drops it either way.
        let declared = if params.codec == CODEC_ID_OPUS {
            track
                .num_frames
                .map(|n| n.saturating_sub(u64::from(track.delay.unwrap_or(0))))
        } else {
            track.num_frames
        };
        let info = StreamInfo {
            sample_rate,
            channels,
            total_frames: edit.map(|e| e.length).or(declared),
            codec: decoder.codec_info().short_name.to_string(),
        };
        let (track_id, time_base) = (track.id, track.time_base);
        let pts_offset = if params.codec == CODEC_ID_OPUS {
            u64::from(track.delay.unwrap_or(0))
        } else {
            0
        };
        let params = params.clone();

        Ok(Self {
            format,
            decoder,
            track_id,
            params,
            pts_offset,
            time_base,
            info,
            block: Vec::new(),
            lead_trim: edit.map_or(0, |e| e.delay),
            skip_frames: edit.map_or(0, |e| e.delay),
            remaining: edit.map(|e| e.length),
            align_to: None,
            priming: false,
            failures: 0,
            ended: false,
        })
    }

    pub fn info(&self) -> &StreamInfo {
        &self.info
    }

    /// The next block of interleaved samples, or `None` at the end of the stream.
    ///
    /// A damaged packet is skipped; three in a row end the stream with an error. A file cut
    /// short ends cleanly.
    pub fn next_block(&mut self) -> Result<Option<&[f32]>, AudioError> {
        if self.fill_block()? {
            Ok(Some(&self.block))
        } else {
            Ok(None)
        }
    }

    /// Decode into `self.block`; false at the end of the stream.
    fn fill_block(&mut self) -> Result<bool, AudioError> {
        while !self.ended && self.remaining != Some(0) {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => {
                    self.ended = true;
                    break;
                }
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    self.ended = true;
                    break;
                }
                Err(e) => {
                    self.ended = true;
                    return Err(AudioError::Decode(e.to_string()));
                }
            };
            if packet.track_id != self.track_id {
                continue;
            }
            if let Some(align) = self.align_to {
                if self.priming {
                    // The first packet after a reset has no previous window to overlap
                    // with, and codecs differ in what they return for it. Its position is
                    // the unreliable part of a seek, so decode it for state and drop it;
                    // its timestamp then says exactly where audio resumes.
                    let _ = self.decoder.decode(&packet);
                    self.priming = false;
                    continue;
                }
                self.skip_frames = align
                    .saturating_sub(self.frames_at(packet.pts).saturating_sub(self.pts_offset));
                self.align_to = None;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    self.failures = 0;
                    self.block.resize(decoded.samples_interleaved(), f32::MID);
                    decoded.copy_to_slice_interleaved(&mut self.block[..]);
                }
                Err(SymphoniaError::DecodeError(msg)) => {
                    self.note_failure(msg)?;
                    continue;
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    self.note_failure("decoder reset required")?;
                    continue;
                }
                Err(e) => {
                    self.ended = true;
                    return Err(AudioError::Decode(e.to_string()));
                }
            }

            let channels = usize::from(self.info.channels);
            if self.skip_frames > 0 {
                let frames = self.block.len() / channels;
                let drop = usize::try_from(self.skip_frames).map_or(frames, |s| s.min(frames));
                self.skip_frames -= drop as u64;
                self.block.drain(..drop * channels);
            }
            if let Some(left) = self.remaining {
                let frames = self.block.len() / channels;
                let keep = usize::try_from(left).map_or(frames, |l| l.min(frames));
                self.block.truncate(keep * channels);
                self.remaining = Some(left - keep as u64);
            }
            if !self.block.is_empty() {
                return Ok(true);
            }
        }
        self.block.clear();
        Ok(false)
    }

    fn note_failure(&mut self, reason: &str) -> Result<(), AudioError> {
        self.failures += 1;
        tracing::debug!(
            failures = self.failures,
            reason,
            "skipping undecodable packet"
        );
        if self.failures > MAX_CONSECUTIVE_FAILURES {
            self.ended = true;
            Err(AudioError::Decode(format!(
                "{} packets in a row failed to decode: {reason}",
                self.failures
            )))
        } else {
            Ok(())
        }
    }

    /// A container timestamp as frames at the stream's sample rate (negative is zero).
    fn frames_at(&self, ts: Timestamp) -> u64 {
        match self.time_base {
            Some(base) => {
                let rate = i128::from(self.info.sample_rate);
                let nanos = base.calc_time_saturating(ts).as_nanos();
                u64::try_from((nanos * rate + NANOS_PER_SEC / 2) / NANOS_PER_SEC).unwrap_or(0)
            }
            None => u64::try_from(ts.get()).unwrap_or(0),
        }
    }

    /// Move to `frame` (in the stream's own sample rate) and return the frame reached.
    ///
    /// Decoding restarts a little before `frame`, and the audio up to it is dropped, so the
    /// result is `frame` unless the container can only land later.
    pub fn seek(&mut self, frame: u64) -> Result<u64, AudioError> {
        // The container counts from before any priming frames.
        let target = frame.saturating_add(self.lead_trim);
        let start = target.saturating_sub(SEEK_PREROLL_FRAMES);
        let rate = i128::from(self.info.sample_rate);
        let to = match self.time_base {
            Some(base) => {
                let nanos = i128::from(start) * NANOS_PER_SEC / rate;
                let time = Time::try_from_nanos_i128(nanos)
                    .ok_or_else(|| AudioError::Decode("seek position out of range".into()))?;
                let ts = base
                    .calc_timestamp(time)
                    .ok_or_else(|| AudioError::Decode("seek position out of range".into()))?;
                SeekTo::Timestamp {
                    ts,
                    track_id: self.track_id,
                }
            }
            None => SeekTo::Timestamp {
                ts: Timestamp::new(i64::try_from(start).unwrap_or(i64::MAX)),
                track_id: self.track_id,
            },
        };
        let seeked = self
            .format
            .seek(SeekMode::Accurate, to)
            .map_err(|e| AudioError::Decode(e.to_string()))?;
        // A fresh decoder rather than `reset`: some keep one-off state, such as Opus pre-skip,
        // that a reset does not restore.
        self.decoder = codecs()
            .make_audio_decoder(&self.params, &AudioDecoderOptions::default())
            .map_err(|e| AudioError::Unsupported(e.to_string()))?;
        self.failures = 0;
        self.ended = false;
        self.skip_frames = 0;
        self.align_to = None;
        self.priming = false;

        let landed = self.frames_at(seeked.actual_ts);
        if landed > target {
            // The container could only land after the target.
        } else if start > 0 {
            self.align_to = Some(target);
            self.priming = true;
        } else {
            self.skip_frames = target - landed;
        }
        let reached = landed.max(target).saturating_sub(self.lead_trim);
        if let Some(remaining) = &mut self.remaining {
            let total = self.info.total_frames.unwrap_or(0);
            *remaining = total.saturating_sub(reached);
        }
        Ok(reached)
    }
}

/// A short piece of audio held in memory.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    /// Interleaved samples.
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Decode up to `max_frames` frames of `path`, converted to `rate` Hz and `channels` channels.
///
/// A file shorter than the limit gives the whole file.
pub fn decode_clip(
    path: &Path,
    max_frames: u64,
    rate: u32,
    channels: u16,
) -> Result<Clip, AudioError> {
    let mut decoder = TrackDecoder::open(path)?;
    let info = decoder.info().clone();
    let mut resampler = Resampler::new(info.sample_rate, rate, channels)?;
    let limit = usize::try_from(max_frames)
        .ok()
        .and_then(|f| f.checked_mul(usize::from(channels)))
        .ok_or_else(|| AudioError::Decode("clip length too large".into()))?;

    let mut samples = Vec::new();
    let mut mixed = Vec::new();
    while samples.len() < limit {
        let Some(block) = decoder.next_block()? else {
            resampler.flush(&mut samples)?;
            break;
        };
        mixed.clear();
        convert_channels(block, info.channels, channels, &mut mixed);
        resampler.process(&mixed, &mut samples)?;
    }
    samples.truncate(limit);

    Ok(Clip {
        samples,
        sample_rate: rate,
        channels,
    })
}
