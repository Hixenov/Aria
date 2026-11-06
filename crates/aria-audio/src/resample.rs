//! Sample-rate and channel-count conversion for interleaved `f32` audio.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Indexing, Resampler as _, SincInterpolationParameters, WindowFunction,
};

use crate::error::AudioError;

/// Input frames per resampler chunk.
const CHUNK_FRAMES: usize = 1024;
/// Headroom for the resampler's ratio; unused because the ratio never changes.
const MAX_RELATIVE_RATIO: f64 = 1.1;

fn resample_err(e: impl std::fmt::Display) -> AudioError {
    AudioError::Resample(e.to_string())
}

/// A streaming sample-rate converter. Equal rates make it a passthrough.
pub struct Resampler {
    state: Option<Sinc>,
}

struct Sinc {
    inner: Async<f32>,
    channels: usize,
    ratio: f64,
    /// Input not yet big enough for a whole chunk.
    pending: Vec<f32>,
    /// Scratch space for one chunk of output.
    scratch: Vec<f32>,
    /// Startup delay still to drop from the output, in frames.
    to_skip: usize,
    /// Frames fed in since the last reset.
    frames_in: u64,
    /// Frames handed out since the last reset.
    frames_out: u64,
}

impl Resampler {
    /// Create a converter from `from` Hz to `to` Hz for `channels` interleaved channels.
    pub fn new(from: u32, to: u32, channels: u16) -> Result<Self, AudioError> {
        if from == 0 || to == 0 || channels == 0 {
            return Err(AudioError::Resample(
                "sample rates and channel count must be non-zero".into(),
            ));
        }
        if from == to {
            return Ok(Self { state: None });
        }
        let channels = usize::from(channels);
        let ratio = f64::from(to) / f64::from(from);
        let params = SincInterpolationParameters::new(256, WindowFunction::BlackmanHarris2);
        let inner = Async::<f32>::new_sinc(
            ratio,
            MAX_RELATIVE_RATIO,
            &params,
            CHUNK_FRAMES,
            channels,
            FixedAsync::Input,
        )
        .map_err(resample_err)?;
        let to_skip = inner.output_delay();
        let scratch = vec![0.0; inner.output_frames_max() * channels];
        Ok(Self {
            state: Some(Sinc {
                inner,
                channels,
                ratio,
                pending: Vec::new(),
                scratch,
                to_skip,
                frames_in: 0,
                frames_out: 0,
            }),
        })
    }

    /// Convert `input` (whole interleaved frames) and append what is ready to `out`.
    ///
    /// Some input is held back until a whole chunk is available; [`flush`](Self::flush)
    /// releases it at the end of a stream.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), AudioError> {
        match &mut self.state {
            None => {
                out.extend_from_slice(input);
                Ok(())
            }
            Some(s) => s.process(input, out),
        }
    }

    /// Append the tail of the stream to `out` and get ready for a new stream.
    pub fn flush(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
        match &mut self.state {
            None => Ok(()),
            Some(s) => {
                let result = s.flush(out);
                s.reset();
                result
            }
        }
    }

    /// Drop all buffered audio and filter history, as at the start of a stream.
    pub fn reset(&mut self) {
        if let Some(s) = &mut self.state {
            s.reset();
        }
    }
}

impl Sinc {
    fn reset(&mut self) {
        self.inner.reset();
        self.pending.clear();
        self.to_skip = self.inner.output_delay();
        self.frames_in = 0;
        self.frames_out = 0;
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), AudioError> {
        let whole = input.len() / self.channels * self.channels;
        self.frames_in += (whole / self.channels) as u64;
        self.pending.extend_from_slice(&input[..whole]);

        let mut start = 0;
        loop {
            let need = self.inner.input_frames_next();
            let end = start + need * self.channels;
            if end > self.pending.len() {
                break;
            }
            self.run_chunk(start, need, None, out)?;
            start = end;
        }
        self.pending.drain(..start);
        Ok(())
    }

    fn flush(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
        let expected = (self.ratio * self.frames_in as f64).ceil() as u64;
        let before = out.len();

        // The leftover input, padded with silence to a whole chunk.
        let leftover = self.pending.len() / self.channels;
        if leftover > 0 {
            let need = self.inner.input_frames_next();
            self.pending.resize(need * self.channels, 0.0);
            self.run_chunk(0, need, Some(leftover), out)?;
            self.pending.clear();
        }
        // Push silence through until the whole tail has come out.
        while self.frames_out < expected {
            let need = self.inner.input_frames_next();
            self.pending.clear();
            self.pending.resize(need * self.channels, 0.0);
            self.run_chunk(0, need, Some(0), out)?;
        }
        if self.frames_out > expected {
            let excess = usize::try_from(self.frames_out - expected).unwrap_or(usize::MAX);
            let keep = out.len().saturating_sub(excess * self.channels).max(before);
            out.truncate(keep);
            self.frames_out = expected;
        }
        Ok(())
    }

    /// Resample `need` frames of `pending` starting at sample offset `start`.
    fn run_chunk(
        &mut self,
        start: usize,
        need: usize,
        partial_len: Option<usize>,
        out: &mut Vec<f32>,
    ) -> Result<(), AudioError> {
        let channels = self.channels;
        let input =
            InterleavedSlice::new(&self.pending[start..], channels, need).map_err(resample_err)?;
        let max_out = self.scratch.len() / channels;
        let mut output = InterleavedSlice::new_mut(&mut self.scratch, channels, max_out)
            .map_err(resample_err)?;
        let indexing = Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len,
            active_channels_mask: None,
        };
        let (_, produced) = self
            .inner
            .process_into_buffer(&input, &mut output, Some(&indexing))
            .map_err(resample_err)?;

        let skip = self.to_skip.min(produced);
        self.to_skip -= skip;
        out.extend_from_slice(&self.scratch[skip * channels..produced * channels]);
        self.frames_out += (produced - skip) as u64;
        Ok(())
    }
}

/// Append `input` (interleaved, `from` channels) to `out` as `to` channels.
///
/// Mono spreads to every output channel; many channels to mono are averaged; otherwise the
/// first `min(from, to)` channels are copied and any extra output channels are silent.
pub fn convert_channels(input: &[f32], from: u16, to: u16, out: &mut Vec<f32>) {
    let (from, to) = (usize::from(from), usize::from(to));
    if from == 0 || to == 0 {
        return;
    }
    out.reserve(input.len() / from * to);
    if from == to {
        out.extend_from_slice(&input[..input.len() / from * from]);
    } else if from == 1 {
        for &s in input {
            out.extend(std::iter::repeat_n(s, to));
        }
    } else if to == 1 {
        for frame in input.chunks_exact(from) {
            out.push(frame.iter().sum::<f32>() / from as f32);
        }
    } else {
        let shared = from.min(to);
        for frame in input.chunks_exact(from) {
            out.extend_from_slice(&frame[..shared]);
            out.extend(std::iter::repeat_n(0.0, to - shared));
        }
    }
}
