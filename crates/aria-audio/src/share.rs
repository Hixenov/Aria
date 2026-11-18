//! The music copied into the second output, and the microphone sent there, for a chat to hear.
//!
//! The engine thread hands the copy every stretch of music it writes into the listener's output,
//! in that output's format. The copy converts it to the second output's format and writes what
//! fits. The listener's output always comes first: when the second output does not keep up, the
//! copy drops audio rather than hold the music back.

use std::time::{Duration, Instant};

use crate::capture::{CaptureFormat, CaptureReader};
use crate::device::DeviceId;
use crate::error::AudioError;
use crate::mixer::InputHandle;
use crate::output::OutputFormat;
use crate::resample::{Resampler, convert_channels};

/// How much audio the copy keeps waiting while its input cannot take it, as just after a clear,
/// when the listener's output may refill its whole ring before the second output has played out
/// the cleared audio. More than that ring, so that only a second output that has stopped drops
/// any.
const MAX_WAITING: Duration = Duration::from_millis(1_500);
/// How often dropped audio is noted in the debug log, at most.
const DROP_REPORT: Duration = Duration::from_secs(1);

/// The copy of the music on the second output.
pub(crate) struct MusicCopy {
    input: InputHandle,
    /// The listener's output format, which the music comes in.
    from: OutputFormat,
    to: OutputFormat,
    resampler: Resampler,
    mixed: Vec<f32>,
    /// Converted audio still to go into the input.
    waiting: Vec<f32>,
    max_waiting: usize,
    /// Samples dropped since the last report.
    dropped: u64,
    reported: Option<Instant>,
}

impl MusicCopy {
    /// A copy of music in `from` onto `input`, an input of an output running at `to`, at `gain`.
    pub(crate) fn new(
        input: InputHandle,
        from: OutputFormat,
        to: OutputFormat,
        gain: f32,
        paused: bool,
    ) -> Result<Self, AudioError> {
        let resampler = Resampler::new(from.sample_rate, to.sample_rate, to.channels)?;
        input.set_gain(gain);
        input.set_paused(paused);
        let max_waiting = (u128::from(to.sample_rate) * MAX_WAITING.as_millis() / 1_000) as usize
            * usize::from(to.channels.max(1));
        Ok(Self {
            input,
            from,
            to,
            resampler,
            mixed: Vec::new(),
            waiting: Vec::new(),
            max_waiting,
            dropped: 0,
            reported: None,
        })
    }

    /// The format the music must come in.
    pub(crate) fn from(&self) -> OutputFormat {
        self.from
    }

    /// Take `music`, whole frames in the listener's format, and write what fits.
    pub(crate) fn push(&mut self, music: &[f32]) {
        if music.is_empty() {
            return;
        }
        self.mixed.clear();
        convert_channels(music, self.from.channels, self.to.channels, &mut self.mixed);
        if let Err(error) = self.resampler.process(&self.mixed, &mut self.waiting) {
            tracing::debug!(%error, "the music copy could not be resampled");
        }
        self.flush();
    }

    /// Write what is waiting, as far as the input takes it, and drop what has waited too long.
    pub(crate) fn flush(&mut self) {
        if !self.input.clearing() && !self.waiting.is_empty() {
            let written = self.input.write(&self.waiting);
            self.waiting.drain(..written);
        }
        if self.waiting.len() > self.max_waiting {
            let channels = usize::from(self.to.channels.max(1));
            let excess = (self.waiting.len() - self.max_waiting).div_ceil(channels) * channels;
            let excess = excess.min(self.waiting.len());
            self.waiting.drain(..excess);
            self.dropped += excess as u64;
        }
        self.report_drops();
    }

    fn report_drops(&mut self) {
        if self.dropped == 0 {
            return;
        }
        let due = self.reported.is_none_or(|at| at.elapsed() >= DROP_REPORT);
        if due {
            tracing::debug!(
                samples = self.dropped,
                "the second output fell behind; music copy dropped"
            );
            self.dropped = 0;
            self.reported = Some(Instant::now());
        }
    }

    /// Start with the silence of `samples` samples in the listener's format, the music that
    /// output already holds, so that what follows plays in step with it.
    pub(crate) fn lead_in(&mut self, samples: u64) {
        let from_channels = u64::from(self.from.channels.max(1));
        let frames = samples / from_channels;
        let frames = u128::from(frames) * u128::from(self.to.sample_rate)
            / u128::from(self.from.sample_rate.max(1));
        let silence = frames as usize * usize::from(self.to.channels.max(1));
        self.waiting.resize(self.waiting.len() + silence, 0.0);
        self.flush();
    }

    /// The music has run out: let the end of it out of the resampler.
    pub(crate) fn finish(&mut self) {
        if let Err(error) = self.resampler.flush(&mut self.waiting) {
            tracing::debug!(%error, "lost the end of the music copy");
        }
        self.flush();
    }

    /// Throw away all of the copy not yet heard, as the listener's output does at a seek.
    pub(crate) fn clear(&mut self) {
        self.input.clear();
        self.resampler.reset();
        self.waiting.clear();
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        self.input.set_paused(paused);
    }

    pub(crate) fn set_gain(&self, gain: f32) {
        self.input.set_gain(gain);
    }
}

/// The most microphone audio the second output's input holds. Older captured audio is dropped
/// rather than queued, so the delay to the chat cannot grow.
pub(crate) const MIC_RING: Duration = Duration::from_millis(40);
/// The microphone's gain can amplify up to this.
const MAX_MIC_GAIN: f32 = 2.0;

/// The microphone on the second output. The engine never logs what it captures, nor its level.
pub(crate) struct Microphone {
    reader: CaptureReader,
    input: InputHandle,
    from: CaptureFormat,
    to: OutputFormat,
    resampler: Resampler,
    /// The device asked for. `None` is the system default.
    device: Option<DeviceId>,
    read: Vec<f32>,
    mixed: Vec<f32>,
    out: Vec<f32>,
}

impl Microphone {
    /// The frames of `to` that the microphone's input holds.
    pub(crate) fn ring_frames(to: OutputFormat) -> usize {
        (u128::from(to.sample_rate) * MIC_RING.as_millis() / 1_000) as usize
    }

    /// The microphone `reader`, captured from `device`, onto `input` of an output running at
    /// `to`, at `gain`.
    pub(crate) fn new(
        reader: CaptureReader,
        input: InputHandle,
        to: OutputFormat,
        gain: f32,
        device: Option<DeviceId>,
    ) -> Result<Self, AudioError> {
        let from = reader.format();
        // Live audio: short chunks, so the converter holds back milliseconds, not tens of them.
        let resampler = Resampler::live(from.sample_rate, to.sample_rate, to.channels)?;
        input.set_gain(gain);
        Ok(Self {
            reader,
            input,
            from,
            to,
            resampler,
            device,
            read: Vec::new(),
            mixed: Vec::new(),
            out: Vec::new(),
        })
    }

    pub(crate) fn device(&self) -> Option<&DeviceId> {
        self.device.as_ref()
    }

    /// Whether the capture has ended, as when the device is lost.
    pub(crate) fn failed(&self) -> bool {
        self.reader.failed()
    }

    pub(crate) fn set_gain(&self, gain: f32) {
        self.input.set_gain(gain);
    }

    /// Move what has been captured into the input: only as much as the input has room for, and
    /// the newest of it.
    pub(crate) fn feed(&mut self) {
        let channels = usize::from(self.to.channels.max(1));
        let room = self.input.free() / channels;
        let keep = (room as u128 * u128::from(self.from.sample_rate)
            / u128::from(self.to.sample_rate.max(1))) as usize;
        self.reader.skip_to_newest(keep);
        let wanted = keep * usize::from(self.from.channels.max(1));
        if self.read.len() < wanted {
            self.read.resize(wanted, 0.0);
        }
        let read = self.reader.read(&mut self.read[..wanted]);
        if read == 0 {
            return;
        }
        self.mixed.clear();
        convert_channels(
            &self.read[..read],
            self.from.channels,
            self.to.channels,
            &mut self.mixed,
        );
        self.out.clear();
        if let Err(error) = self.resampler.process(&self.mixed, &mut self.out) {
            tracing::debug!(%error, "the microphone could not be resampled");
        }
        // Rounding in the resampler can leave a frame or two more than there is room for; that
        // is dropped, as older audio is.
        self.input.write(&self.out);
    }
}

/// A gain for the microphone: 0.0 to 2.0, anything else brought inside, and silence for one that
/// is not a number.
pub(crate) fn mic_gain(gain: f32) -> f32 {
    if gain.is_finite() {
        gain.clamp(0.0, MAX_MIC_GAIN)
    } else {
        0.0
    }
}

/// A gain for the copy: 0.0 to 1.0, anything else brought inside, and silence for one that is
/// not a number.
pub(crate) fn copy_gain(gain: f32) -> f32 {
    if gain.is_finite() {
        gain.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gains_are_brought_inside() {
        assert_eq!(copy_gain(0.5), 0.5);
        assert_eq!(copy_gain(2.0), 1.0);
        assert_eq!(copy_gain(-1.0), 0.0);
        assert_eq!(copy_gain(f32::NAN), 0.0);
        assert_eq!(mic_gain(1.5), 1.5);
        assert_eq!(mic_gain(3.0), 2.0);
        assert_eq!(mic_gain(-1.0), 0.0);
        assert_eq!(mic_gain(f32::INFINITY), 0.0);
    }

    #[test]
    fn the_microphone_ring_is_40_ms() {
        let format = OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        };
        assert_eq!(Microphone::ring_frames(format), 1_920);
    }
}
