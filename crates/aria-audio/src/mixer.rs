//! Sums inputs into one output buffer.
//!
//! [`Mixer::render`] is what the real-time callback runs, so it takes no locks, allocates
//! nothing and frees nothing. Inputs are built on the control side and reach the mixer through
//! a ring of prepared inputs. Removed inputs go back through a second ring, so their buffers are
//! freed on the control side too.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rtrb::{Consumer, Producer, PushError, RingBuffer};

use crate::error::AudioError;
use crate::output::OutputFormat;

/// The most inputs one output can hold.
pub const MAX_INPUTS: usize = 8;
/// How long a fade lasts when clearing, pausing and resuming, in milliseconds.
pub const FADE_MS: u32 = 5;

/// What an input's handle and the mixer share.
struct InputShared {
    /// Samples read from the ring, or discarded from it.
    consumed: AtomicU64,
    /// The index the latest `clear` discards up to.
    clear_to: AtomicU64,
    /// The target gain, as `f32` bits.
    gain: AtomicU32,
    paused: AtomicBool,
}

/// The writing end of one mixer input. Dropping it fades the input out and removes it.
pub struct InputHandle {
    writer: Producer<f32>,
    shared: Arc<InputShared>,
    written: u64,
}

impl InputHandle {
    /// Queue interleaved samples without blocking. Returns how many fitted.
    pub fn write(&mut self, samples: &[f32]) -> usize {
        let (pushed, _) = self.writer.push_partial_slice(samples);
        self.written += pushed.len() as u64;
        pushed.len()
    }

    /// How many samples the next [`write`](Self::write) can take.
    pub fn free(&self) -> usize {
        self.writer.slots()
    }

    /// Samples accepted so far.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Samples played or discarded so far, in the same index space as [`written`](Self::written).
    pub fn consumed(&self) -> u64 {
        self.shared.consumed.load(Ordering::Acquire)
    }

    /// Drop everything written so far. The mixer fades out over what is buffered, discards the
    /// rest, and fades in on whatever is written next.
    pub fn clear(&mut self) {
        self.shared.clear_to.store(self.written, Ordering::Release);
    }

    /// Set the gain, which ramps there within one buffer. Anything but a finite gain mutes.
    pub fn set_gain(&self, gain: f32) {
        let gain = if gain.is_finite() { gain.max(0.0) } else { 0.0 };
        self.shared.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// Fade out and hold the buffered samples, or fade back in.
    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }
}

impl Drop for InputHandle {
    fn drop(&mut self) {
        // Published before the ring is abandoned, which is what the mixer checks first.
        self.clear();
    }
}

/// The mixer's end of one input.
struct MixInput {
    reader: Consumer<f32>,
    shared: Arc<InputShared>,
    consumed: u64,
    /// The last `clear_to` that has been acted on.
    cleared: u64,
    /// The gain at the end of the last buffer.
    gain: f32,
    /// Position in the fade, from 0 (silent) to the mixer's `fade_frames` (full level).
    fade: usize,
}

impl MixInput {
    /// Set the starting state. Nothing has been heard yet, so nothing needs fading or ramping.
    fn start(&mut self, fade_frames: usize) {
        let clearing = self.shared.clear_to.load(Ordering::Acquire) != self.cleared;
        let paused = self.shared.paused.load(Ordering::Relaxed);
        self.fade = if clearing || paused { 0 } else { fade_frames };
        self.gain = f32::from_bits(self.shared.gain.load(Ordering::Relaxed));
    }

    /// Add this input to `out`. Returns whether it has finished and can leave the mixer.
    fn mix(&mut self, out: &mut [f32], channels: usize, fade_frames: usize) -> bool {
        // In this order: seeing the handle gone guarantees seeing its last clear.
        let abandoned = self.reader.is_abandoned();
        let clear_to = self.shared.clear_to.load(Ordering::Acquire);
        let paused = self.shared.paused.load(Ordering::Relaxed);
        let target_gain = f32::from_bits(self.shared.gain.load(Ordering::Relaxed));

        let total = out.len() / channels;
        let gain_step = (target_gain - self.gain) / total as f32;
        let mut done = 0;
        while done < total {
            let clearing = clear_to != self.cleared;
            let stopping = clearing || paused;
            if stopping && self.fade == 0 {
                if !clearing {
                    break;
                }
                self.discard_to(clear_to);
                continue;
            }

            // Whole frames only, so a half-written frame cannot shift the channels. Audio
            // written after a clear must wait for the clear to finish.
            let limit = if clearing {
                clear_to.saturating_sub(self.consumed)
            } else {
                u64::MAX
            };
            let buffered = (self.reader.slots() as u64).min(limit) as usize / channels;
            let mut frames = buffered.min(total - done);
            if stopping {
                frames = frames.min(self.fade);
            }
            if frames == 0 {
                if !stopping {
                    break;
                }
                // The buffered audio ran out before the fade did.
                self.fade = 0;
                continue;
            }

            let Ok(chunk) = self.reader.read_chunk(frames * channels) else {
                break;
            };
            let (first, second) = chunk.as_slices();
            let mut samples = first.iter().chain(second);
            let block = &mut out[done * channels..(done + frames) * channels];
            for (index, frame) in block.chunks_exact_mut(channels).enumerate() {
                self.fade = if stopping {
                    self.fade - 1
                } else {
                    (self.fade + 1).min(fade_frames)
                };
                let gain = self.gain + gain_step * (done + index + 1) as f32;
                let scale = gain * (self.fade as f32 / fade_frames as f32);
                for (mixed, sample) in frame.iter_mut().zip(&mut samples) {
                    *mixed += sample * scale;
                }
            }
            chunk.commit_all();
            self.consumed += (frames * channels) as u64;
            done += frames;
        }

        self.gain = target_gain;
        self.shared.consumed.store(self.consumed, Ordering::Release);
        abandoned && clear_to == self.cleared
    }

    /// Skip the samples before `clear_to`, counting them as consumed.
    fn discard_to(&mut self, clear_to: u64) {
        let count = clear_to
            .saturating_sub(self.consumed)
            .min(self.reader.slots() as u64) as usize;
        if let Ok(chunk) = self.reader.read_chunk(count) {
            chunk.commit_all();
            self.consumed += count as u64;
        }
        self.cleared = clear_to;
    }
}

/// Sums up to [`MAX_INPUTS`] inputs for one output. One mixer belongs to one audio stream.
pub struct Mixer {
    inputs: [Option<MixInput>; MAX_INPUTS],
    arrivals: Consumer<MixInput>,
    returns: Producer<MixInput>,
    channels: usize,
    fade_frames: usize,
}

impl Mixer {
    /// Create a mixer, and the control half that adds inputs to it from another thread.
    pub fn new(format: OutputFormat) -> (Self, MixerControl) {
        let (arrivals_in, arrivals) = RingBuffer::new(MAX_INPUTS);
        let (returns, returns_out) = RingBuffer::new(MAX_INPUTS);
        let channels = usize::from(format.channels).max(1);
        let fade_frames = u64::from(format.sample_rate) * u64::from(FADE_MS) / 1_000;
        let mixer = Self {
            inputs: std::array::from_fn(|_| None),
            arrivals,
            returns,
            channels,
            fade_frames: (fade_frames as usize).max(1),
        };
        let control = MixerControl {
            arrivals: arrivals_in,
            returns: returns_out,
            channels,
            live: 0,
        };
        (mixer, control)
    }

    /// Fill `out` with the sum of the inputs, as interleaved samples clipped to full scale.
    ///
    /// This runs on the real-time thread. Inputs only ever move: from the arrivals ring into a
    /// free place, and from a place into the returns ring. [`MixerControl`] never lets more than
    /// [`MAX_INPUTS`] exist, so there is always a free place and always room in the returns ring.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        for place in self.inputs.iter_mut().filter(|place| place.is_none()) {
            let Ok(mut input) = self.arrivals.pop() else {
                break;
            };
            input.start(self.fade_frames);
            *place = Some(input);
        }
        if out.len() < self.channels {
            return;
        }
        for place in &mut self.inputs {
            if let Some(input) = place
                && input.mix(out, self.channels, self.fade_frames)
                && let Some(input) = place.take()
                && let Err(PushError::Full(input)) = self.returns.push(input)
            {
                // Dropping it here would free memory on the real-time thread.
                *place = Some(input);
            }
        }
        for sample in out.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }
}

/// The control half of a [`Mixer`], kept by the output that owns the stream.
pub struct MixerControl {
    arrivals: Producer<MixInput>,
    returns: Consumer<MixInput>,
    channels: usize,
    /// Inputs that exist anywhere: on their way in, in the mixer, or on their way back.
    live: usize,
}

impl MixerControl {
    /// Add an input whose ring holds `capacity_frames` frames.
    pub fn add_input(&mut self, capacity_frames: usize) -> Result<InputHandle, AudioError> {
        self.reclaim();
        if self.live >= MAX_INPUTS {
            return Err(AudioError::Device("too many inputs on one output".into()));
        }
        let capacity = capacity_frames
            .checked_mul(self.channels)
            .filter(|capacity| *capacity > 0)
            .ok_or_else(|| AudioError::Device("unusable input capacity".into()))?;

        let (writer, reader) = RingBuffer::new(capacity);
        let shared = Arc::new(InputShared {
            consumed: AtomicU64::new(0),
            clear_to: AtomicU64::new(0),
            gain: AtomicU32::new(1.0_f32.to_bits()),
            paused: AtomicBool::new(false),
        });
        let input = MixInput {
            reader,
            shared: Arc::clone(&shared),
            consumed: 0,
            cleared: 0,
            gain: 1.0,
            fade: 0,
        };
        self.arrivals
            .push(input)
            .map_err(|_| AudioError::Device("the mixer is not taking inputs".into()))?;
        self.live += 1;
        Ok(InputHandle {
            writer,
            shared,
            written: 0,
        })
    }

    /// Free the inputs the mixer has handed back.
    fn reclaim(&mut self) {
        while self.returns.pop().is_ok() {
            self.live -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const FADE_FRAMES: usize = 240;
    const SECOND: usize = RATE as usize;

    fn mixer(channels: u16) -> (Mixer, MixerControl) {
        Mixer::new(OutputFormat {
            sample_rate: RATE,
            channels,
        })
    }

    fn render(mixer: &mut Mixer, samples: usize) -> Vec<f32> {
        let mut out = vec![1.0; samples];
        mixer.render(&mut out);
        out
    }

    fn write_constant(input: &mut InputHandle, value: f32, samples: usize) {
        assert_eq!(input.write(&vec![value; samples]), samples);
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "{actual} is not {expected}"
        );
    }

    fn largest_step(samples: &[f32]) -> f32 {
        samples
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn sums_inputs_with_gain() {
        let (mut mixer, mut control) = mixer(1);
        let mut first = control.add_input(SECOND).unwrap();
        let mut second = control.add_input(SECOND).unwrap();
        write_constant(&mut first, 0.25, 1_024);
        write_constant(&mut second, 0.25, 1_024);

        let unity = render(&mut mixer, 128);
        assert_close(unity[127], 0.5);

        first.set_gain(1.0);
        second.set_gain(0.5);
        render(&mut mixer, 128);

        for sample in render(&mut mixer, 128) {
            assert_close(sample, 0.375);
        }
    }

    #[test]
    fn loud_inputs_clip_at_full_scale() {
        let (mut mixer, mut control) = mixer(1);
        let mut first = control.add_input(SECOND).unwrap();
        let mut second = control.add_input(SECOND).unwrap();
        first.write(&[0.75, -0.75, 0.25]);
        second.write(&[0.75, -0.75, 0.25]);

        assert_eq!(render(&mut mixer, 3), [1.0, -1.0, 0.5]);
    }

    #[test]
    fn gain_ramps_across_one_buffer() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 1_024);
        let mut heard = render(&mut mixer, 128);

        input.set_gain(0.0);
        heard.extend(render(&mut mixer, 128));

        assert!(largest_step(&heard) < 0.01, "gain jumped");
        assert_close(heard[255], 0.0);
        assert_close(render(&mut mixer, 128)[0], 0.0);
    }

    #[test]
    fn gain_set_before_the_first_render_needs_no_ramp() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        input.set_gain(0.5);
        write_constant(&mut input, 1.0, 16);

        assert_close(render(&mut mixer, 16)[0], 0.5);
    }

    #[test]
    fn unusable_gain_mutes_rather_than_poisoning_the_mix() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 256);
        render(&mut mixer, 16);

        input.set_gain(f32::NAN);

        assert!(render(&mut mixer, 16).iter().all(|s| s.is_finite()));
        assert_close(render(&mut mixer, 16)[0], 0.0);
    }

    #[test]
    fn underrun_is_silence() {
        let (mut mixer, mut control) = mixer(2);
        let input = control.add_input(SECOND).unwrap();

        let out = render(&mut mixer, 256);

        assert!(out.iter().all(|sample| *sample == 0.0));
        assert_eq!(input.consumed(), 0);
    }

    #[test]
    fn audio_comes_out_in_order_across_the_ring_wrap() {
        let (mut mixer, mut control) = mixer(2);
        let mut input = control.add_input(5).unwrap();
        let samples: Vec<f32> = (0..64).map(|n| n as f32 / 64.0).collect();

        let mut heard = Vec::new();
        for block in samples.chunks(6) {
            assert_eq!(input.write(block), block.len());
            heard.extend(render(&mut mixer, block.len()));
        }

        assert_eq!(heard, samples);
    }

    #[test]
    fn write_stops_at_the_ring_capacity() {
        let (_mixer, mut control) = mixer(2);
        let mut input = control.add_input(4).unwrap();
        assert_eq!(input.free(), 8);

        assert_eq!(input.write(&[0.5; 12]), 8);

        assert_eq!(input.free(), 0);
        assert_eq!(input.written(), 8);
    }

    #[test]
    fn half_a_frame_waits_for_its_other_channel() {
        let (mut mixer, mut control) = mixer(2);
        let mut input = control.add_input(16).unwrap();

        input.write(&[0.25]);
        assert_eq!(render(&mut mixer, 2), [0.0, 0.0]);
        assert_eq!(input.consumed(), 0);

        input.write(&[0.75]);
        assert_eq!(render(&mut mixer, 2), [0.25, 0.75]);
    }

    #[test]
    fn clear_drops_buffered_audio() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 0.5, 1_000);
        render(&mut mixer, 16);

        input.clear();
        write_constant(&mut input, 0.9, 1_000);
        let out = render(&mut mixer, 4_000);

        let heard = out.iter().filter(|sample| **sample != 0.0).count();
        assert!(heard <= FADE_FRAMES + 1_000, "old audio played: {heard}");
        assert_close(out[FADE_FRAMES + 999], 0.9);
        assert_eq!(input.consumed(), input.written());
    }

    #[test]
    fn clear_fades_without_a_jump() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 4_800);
        let mut heard = render(&mut mixer, 480);

        input.clear();
        write_constant(&mut input, 1.0, 4_800);
        heard.extend(render(&mut mixer, 2_400));

        assert!(largest_step(&heard) <= 0.05, "the clear clicked");
        let quietest = heard.iter().copied().fold(1.0, f32::min);
        assert!(quietest < 0.01, "no fade ran: {quietest}");
        assert_close(heard[heard.len() - 1], 1.0);
    }

    #[test]
    fn clear_never_plays_or_drops_audio_written_after_it() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        // Less old audio than the fade needs, so the fade must stop short of the new audio.
        write_constant(&mut input, 0.5, 100);
        render(&mut mixer, 16);

        input.clear();
        write_constant(&mut input, -0.5, 2_000);
        let out = render(&mut mixer, 4_000);

        let turn = out
            .iter()
            .position(|sample| *sample < 0.0)
            .expect("new audio plays");
        assert!(out[..turn].iter().all(|sample| *sample >= 0.0));
        assert!(out[turn..].iter().all(|sample| *sample <= 0.0));
        let new_audio = out.iter().filter(|sample| **sample < 0.0).count();
        assert_eq!(new_audio, 2_000);
    }

    #[test]
    fn clear_fades_in_only_when_new_audio_arrives() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 4_800);
        render(&mut mixer, 480);

        input.clear();
        render(&mut mixer, 4_800);
        assert!(render(&mut mixer, 256).iter().all(|s| *s == 0.0));

        write_constant(&mut input, 1.0, 1_000);
        let out = render(&mut mixer, 480);

        assert!(out[0] < 0.05, "new audio started at {}", out[0]);
        assert!(largest_step(&out) <= 0.05);
    }

    #[test]
    fn clear_of_an_empty_ring_still_fades_in_new_audio() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 16);
        render(&mut mixer, 32);

        input.clear();
        render(&mut mixer, 32);
        write_constant(&mut input, 1.0, 1_000);

        assert!(render(&mut mixer, 16)[0] < 0.05);
    }

    #[test]
    fn clear_keeps_channels_aligned_after_half_a_frame() {
        let (mut mixer, mut control) = mixer(2);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 0.0, 1);

        input.clear();
        let frames: Vec<f32> = [0.25, -0.25].repeat(1_000);
        assert_eq!(input.write(&frames), frames.len());
        let out = render(&mut mixer, 2_000);

        let settled = &out[2 * FADE_FRAMES..];
        assert!(settled[0] > 0.0, "left channel holds {}", settled[0]);
        assert!(settled[1] < 0.0, "right channel holds {}", settled[1]);
    }

    #[test]
    fn clear_before_the_first_render_is_never_heard() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 1_000);

        input.clear();

        assert!(render(&mut mixer, 512).iter().all(|s| *s == 0.0));
        assert_eq!(input.consumed(), 1_000);
    }

    #[test]
    fn paused_input_keeps_its_samples() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 4_800);
        render(&mut mixer, 480);

        input.set_paused(true);
        render(&mut mixer, 480);
        let held = input.consumed();
        assert_eq!(held, 480 + FADE_FRAMES as u64);

        let silent = render(&mut mixer, 480);
        assert!(silent.iter().all(|sample| *sample == 0.0));
        assert_eq!(input.consumed(), held);

        input.set_paused(false);
        render(&mut mixer, 480);
        assert_eq!(input.consumed(), held + 480);
    }

    #[test]
    fn pause_and_resume_fade_without_a_jump() {
        let (mut mixer, mut control) = mixer(2);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 9_600);
        let mut heard = render(&mut mixer, 960);

        input.set_paused(true);
        heard.extend(render(&mut mixer, 960));
        input.set_paused(false);
        heard.extend(render(&mut mixer, 960));

        assert!(largest_step(&heard) <= 0.05, "pausing clicked");
        assert!(heard.iter().copied().fold(1.0, f32::min) < 0.01);
        assert_close(heard[heard.len() - 1], 1.0);
    }

    #[test]
    fn input_paused_before_the_first_render_is_never_heard() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        input.set_paused(true);
        write_constant(&mut input, 1.0, 1_000);

        assert!(render(&mut mixer, 512).iter().all(|s| *s == 0.0));
        assert_eq!(input.consumed(), 0);
    }

    #[test]
    fn inputs_join_a_running_mixer() {
        let (mut mixer, mut control) = mixer(1);
        render(&mut mixer, 64);

        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 0.5, 64);

        assert_close(render(&mut mixer, 64)[0], 0.5);
    }

    #[test]
    fn dropped_input_fades_out() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 4_800);
        let mut heard = render(&mut mixer, 480);

        drop(input);
        heard.extend(render(&mut mixer, 480));

        assert!(largest_step(&heard) <= 0.05, "the removal clicked");
        assert_eq!(heard[heard.len() - 1], 0.0);
    }

    #[test]
    fn dropped_input_is_handed_back_and_never_freed_by_the_mixer() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 4_800);
        render(&mut mixer, 16);
        let shared = Arc::downgrade(&input.shared);

        drop(input);
        render(&mut mixer, 2 * FADE_FRAMES);
        render(&mut mixer, 16);

        assert!(!control.returns.is_empty(), "the input never left");
        assert!(shared.upgrade().is_some(), "the mixer freed the input");

        control.reclaim();
        assert!(shared.upgrade().is_none(), "reclaim kept the input");
    }

    #[test]
    fn paused_input_can_still_be_dropped() {
        let (mut mixer, mut control) = mixer(1);
        let mut input = control.add_input(SECOND).unwrap();
        write_constant(&mut input, 1.0, 4_800);
        input.set_paused(true);
        render(&mut mixer, 16);

        drop(input);
        render(&mut mixer, 16);

        assert!(!control.returns.is_empty(), "the input never left");
    }

    #[test]
    fn inputs_are_limited_until_one_leaves() {
        let (mut mixer, mut control) = mixer(1);
        let mut inputs: Vec<_> = (0..MAX_INPUTS)
            .map(|_| control.add_input(16).unwrap())
            .collect();
        assert!(control.add_input(16).is_err());

        inputs.pop();
        render(&mut mixer, 16);

        assert!(control.add_input(16).is_ok());
    }

    #[test]
    fn places_are_reused_as_inputs_come_and_go() {
        let (mut mixer, mut control) = mixer(1);
        for _ in 0..4 * MAX_INPUTS {
            let mut input = control.add_input(16).unwrap();
            write_constant(&mut input, 0.5, 8);
            assert_close(render(&mut mixer, 8)[0], 0.5);
            drop(input);
            render(&mut mixer, 8);
        }
    }

    #[test]
    fn empty_input_ring_is_refused() {
        let (_mixer, mut control) = mixer(2);

        assert!(control.add_input(0).is_err());
    }

    #[test]
    fn empty_buffer_is_harmless() {
        let (mut mixer, mut control) = mixer(2);
        let mut input = control.add_input(16).unwrap();
        write_constant(&mut input, 1.0, 8);
        input.set_gain(0.5);

        mixer.render(&mut []);

        assert_close(render(&mut mixer, 2)[0], 0.5);
    }
}
