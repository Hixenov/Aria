//! Soundboard voices: sounds mixed into the music's output, into a second output, or both.
//!
//! A voice is one sound playing. It has one decoder and, for each output it plays on, a part: a
//! mixer input on that output and a resampler to that output's format. The engine thread feeds
//! voices on each pass, as it feeds the track, so nothing here runs on a real-time thread.

use crate::decode::TrackDecoder;
use crate::error::AudioError;
use crate::mixer::{InputHandle, MAX_INPUTS};
use crate::output::{AudioOutput, OutputFormat};
use crate::resample::{Resampler, convert_channels};

/// Where a sound is played.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SoundRoute {
    /// The device the music uses.
    #[default]
    Mine,
    /// The second device. Without one, or when it is the music's device, as [`Mine`](Self::Mine).
    Second,
    /// Both devices, or the music's alone as for [`Second`](Self::Second).
    Both,
}

/// The most sounds that play at once. Another one stops the one that started first.
pub const SOUNDS_AT_ONCE: usize = 6;

// The music's output holds one input for the music, one for each sound, and one more for a sound
// that is set up before the one it replaces lets go of its input.
const _: () = assert!(SOUNDS_AT_ONCE + 2 <= MAX_INPUTS);
// The second output holds one input for each sound and one being set up, as the music's does, and
// one each for the music's copy and the microphone. A copy let go of keeps its input until it has
// faded out, so there is room for one more.
const _: () = assert!(SOUNDS_AT_ONCE + 4 <= MAX_INPUTS);

/// One of the outputs a sound can play on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    /// The output the music plays on.
    Music,
    /// The output on the second device.
    Second,
}

/// Inputs on one output that voices have finished with, kept for the next voice there.
///
/// A dropped input stays in the mixer until it has faded out, and counts against
/// [`MAX_INPUTS`] until then. Reusing inputs instead means an output holds no more inputs for
/// sounds than [`SOUNDS_AT_ONCE`] plus the one being set up, however fast sounds are fired: a new
/// input is made only when every input there is in use.
#[derive(Default)]
pub(crate) struct Spares(Vec<InputHandle>);

impl Spares {
    /// A free input on `output`: a spare one that has finished fading out if there is one, as it
    /// takes new audio at once, then any spare one, then a new one holding one second.
    fn take(
        &mut self,
        output: &mut dyn AudioOutput,
        format: OutputFormat,
    ) -> Result<InputHandle, AudioError> {
        let ready = self.0.iter().position(|input| !input.clearing());
        match ready
            .map(|at| self.0.swap_remove(at))
            .or_else(|| self.0.pop())
        {
            Some(input) => Ok(input),
            None => output.add_input(format.sample_rate as usize),
        }
    }

    /// Keep `input` for later. What it still holds fades out and is dropped.
    pub(crate) fn put(&mut self, mut input: InputHandle) {
        input.clear();
        self.0.push(input);
    }
}

/// The output on the second device, opened shared when a sound needs it.
pub(crate) struct SecondOutput {
    pub(crate) output: Box<dyn AudioOutput>,
    pub(crate) format: OutputFormat,
    pub(crate) spares: Spares,
}

/// A voice's way onto one output.
struct Part {
    target: Target,
    input: InputHandle,
    resampler: Resampler,
    /// The output's channels.
    channels: u16,
    /// Audio in the output's format still to go into the ring, from `pending_from` on.
    pending: Vec<f32>,
    pending_from: usize,
    /// The index on the input where the sound ends, once it has all been decoded.
    end: Option<u64>,
}

impl Part {
    /// Move pending audio into the ring. Returns whether there is room for more.
    fn write_pending(&mut self) -> bool {
        // A reused input plays out its old audio first; until then only a sliver would fit.
        if self.input.clearing() {
            return false;
        }
        self.pending_from += self.input.write(&self.pending[self.pending_from..]);
        if self.pending_from < self.pending.len() {
            return false;
        }
        self.pending.clear();
        self.pending_from = 0;
        self.input.free() > 0
    }

    /// The decoder is done: let the resampler's tail out and mark where the sound ends.
    fn finish(&mut self) {
        if let Err(error) = self.resampler.flush(&mut self.pending) {
            tracing::debug!(%error, "lost the end of a sound");
        }
        let queued = (self.pending.len() - self.pending_from) as u64;
        self.end = Some(self.input.written() + queued);
    }

    fn finished(&self) -> bool {
        self.end.is_some_and(|end| self.input.consumed() >= end)
    }
}

/// One sound playing.
pub(crate) struct Voice {
    pub(crate) token: u64,
    /// `None` once the whole file has been decoded.
    decoder: Option<TrackDecoder>,
    gain: f32,
    parts: Vec<Part>,
}

impl Voice {
    /// A voice for `decoder`, on no output yet. Gains outside 0.0 to 1.0 are brought inside.
    pub(crate) fn new(token: u64, decoder: TrackDecoder, gain: f32) -> Self {
        let gain = if gain.is_finite() {
            gain.clamp(0.0, 1.0)
        } else {
            0.0
        };
        Self {
            token,
            decoder: Some(decoder),
            gain,
            parts: Vec::new(),
        }
    }

    /// Play on `output` too, through one of its free inputs.
    pub(crate) fn add_part(
        &mut self,
        target: Target,
        output: &mut dyn AudioOutput,
        format: OutputFormat,
        spares: &mut Spares,
    ) -> Result<(), AudioError> {
        let rate = self
            .decoder
            .as_ref()
            .map_or(format.sample_rate, |decoder| decoder.info().sample_rate);
        let resampler = Resampler::new(rate, format.sample_rate, format.channels)?;
        let input = spares.take(output, format)?;
        input.set_gain(self.gain);
        input.set_paused(false);
        self.parts.push(Part {
            target,
            input,
            resampler,
            channels: format.channels,
            pending: Vec::new(),
            pending_from: 0,
            end: None,
        });
        Ok(())
    }

    pub(crate) fn has_parts(&self) -> bool {
        !self.parts.is_empty()
    }

    pub(crate) fn plays_on(&self, target: Target) -> bool {
        self.parts.iter().any(|part| part.target == target)
    }

    /// Stop playing on `target`, whose output has gone. Its input goes with it.
    pub(crate) fn leave(&mut self, target: Target) {
        self.parts.retain(|part| part.target != target);
    }

    /// The inputs the voice played through, to be kept for other voices.
    pub(crate) fn into_inputs(self) -> impl Iterator<Item = (Target, InputHandle)> {
        self.parts.into_iter().map(|part| (part.target, part.input))
    }

    /// Whether everything decoded has been heard on every output.
    pub(crate) fn finished(&self) -> bool {
        self.decoder.is_none() && self.parts.iter().all(Part::finished)
    }

    /// Decode while every output has room. `mixed` is scratch space for the channel conversion.
    pub(crate) fn feed(&mut self, mixed: &mut Vec<f32>) {
        loop {
            let mut room = true;
            for part in &mut self.parts {
                room &= part.write_pending();
            }
            if !room {
                return;
            }
            let Some(decoder) = &mut self.decoder else {
                return;
            };
            let channels = decoder.info().channels;
            let ended = match decoder.next_block() {
                Ok(Some(block)) => {
                    let mut failed = false;
                    for part in &mut self.parts {
                        mixed.clear();
                        convert_channels(block, channels, part.channels, mixed);
                        if let Err(error) = part.resampler.process(mixed, &mut part.pending) {
                            tracing::debug!(%error, "a sound could not be resampled");
                            failed = true;
                        }
                    }
                    failed
                }
                Ok(None) => true,
                Err(error) => {
                    // What was decoded still plays.
                    tracing::debug!(%error, "a sound stopped decoding");
                    true
                }
            };
            if ended {
                self.decoder = None;
                for part in &mut self.parts {
                    part.finish();
                }
            }
        }
    }
}
