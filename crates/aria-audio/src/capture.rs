//! Capturing an input device, such as a microphone, for the second output.
//!
//! The capture callback runs on a real-time thread. It converts what the device hands it to
//! interleaved `f32` and pushes that into a lock-free ring; the engine thread reads the ring. The
//! callback allocates nothing, takes no lock and logs nothing. Nothing captured is kept anywhere
//! but that ring.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample};

use crate::cpal_output::{CLOSE_WAIT, OPEN_WAIT, join_within};
use crate::device::{DeviceId, DeviceKind, device_error, find_device};
use crate::error::AudioError;

/// How much the callback converts at a time.
const SCRATCH_FRAMES: usize = 1024;
/// How long a backend may take to set up a stream.
const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The rate and channel count a capture runs at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureFormat {
    /// Frames per second.
    pub sample_rate: u32,
    /// Interleaved channels per frame.
    pub channels: u16,
}

impl CaptureFormat {
    fn channels(self) -> usize {
        usize::from(self.channels.max(1))
    }
}

/// What both ends of a capture ring can see.
#[derive(Default)]
struct CaptureHealth {
    failed: AtomicBool,
    /// Samples that did not fit in the ring and were dropped.
    dropped: AtomicU64,
}

/// The capture callback's end of the ring.
pub(crate) struct CaptureWriter {
    producer: rtrb::Producer<f32>,
    health: Arc<CaptureHealth>,
    channels: usize,
}

impl CaptureWriter {
    /// Push the whole frames of `samples` that fit and drop the rest. Returns how many samples went
    /// in. Never blocks, allocates or logs.
    pub(crate) fn write(&mut self, samples: &[f32]) -> usize {
        let room = self.producer.slots() / self.channels * self.channels;
        let whole = samples.len() / self.channels * self.channels;
        let take = room.min(whole);
        let (pushed, _) = self.producer.push_partial_slice(&samples[..take]);
        let written = pushed.len();
        let dropped = samples.len() - written;
        if dropped > 0 {
            self.health
                .dropped
                .fetch_add(dropped as u64, Ordering::Relaxed);
        }
        written
    }

    /// Mark the capture dead, as a lost device does.
    pub(crate) fn fail(&self) {
        self.health.failed.store(true, Ordering::Release);
    }
}

/// The engine's end of a capture: interleaved `f32` frames. Dropping it closes the stream.
pub struct CaptureReader {
    format: CaptureFormat,
    consumer: rtrb::Consumer<f32>,
    health: Arc<CaptureHealth>,
    /// Whatever keeps the stream open; dropped with the reader, after the ring.
    _stream: Box<dyn Any + Send>,
}

impl CaptureReader {
    /// The format the samples are in.
    pub fn format(&self) -> CaptureFormat {
        self.format
    }

    /// Moves up to `out.len()` samples, in whole frames, out of the ring. Returns how many. Never
    /// blocks.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        let channels = self.format.channels();
        let available = self.consumer.slots() / channels * channels;
        let take = available.min(out.len() / channels * channels);
        let (read, _) = self.consumer.pop_partial_slice(&mut out[..take]);
        read.len()
    }

    /// Drops everything waiting but the newest `keep` frames.
    pub fn skip_to_newest(&mut self, keep: usize) {
        let channels = self.format.channels();
        let waiting = self.consumer.slots() / channels;
        let skip = waiting.saturating_sub(keep) * channels;
        if skip > 0
            && let Ok(chunk) = self.consumer.read_chunk(skip)
        {
            chunk.commit_all();
        }
    }

    /// Frames waiting to be read.
    pub fn waiting(&self) -> usize {
        self.consumer.slots() / self.format.channels()
    }

    /// Whether the stream has ended, as when the device is lost, so it must be reopened.
    pub fn failed(&self) -> bool {
        self.health.failed.load(Ordering::Acquire) || self.consumer.is_abandoned()
    }

    /// Samples dropped so far because the ring was full.
    pub fn dropped(&self) -> u64 {
        self.health.dropped.load(Ordering::Relaxed)
    }
}

/// A ring holding one second of `format`, and its two ends. `stream` is dropped with the reader.
pub(crate) fn capture_ring(
    format: CaptureFormat,
    stream: Box<dyn Any + Send>,
) -> (CaptureWriter, CaptureReader) {
    let channels = format.channels();
    let capacity = (format.sample_rate.max(1) as usize) * channels;
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    let health = Arc::new(CaptureHealth::default());
    (
        CaptureWriter {
            producer,
            health: Arc::clone(&health),
            channels,
        },
        CaptureReader {
            format,
            consumer,
            health,
            _stream: stream,
        },
    )
}

/// Opens captures. Each call opens a new, independent one.
pub trait CaptureFactory: Send {
    /// Open `device`, or the system default input when it is `None`, in shared mode. Dropping
    /// the reader closes the stream.
    fn open(&self, device: Option<&DeviceId>) -> Result<CaptureReader, AudioError>;
}

/// Captures through CPAL on the default host, always shared.
pub struct CpalCapture;

impl CaptureFactory for CpalCapture {
    fn open(&self, device: Option<&DeviceId>) -> Result<CaptureReader, AudioError> {
        let device = device.cloned();
        let (opened_tx, opened_rx) = mpsc::channel();
        let (stop, stopped) = mpsc::channel::<()>();
        let (closed_tx, closed) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("aria-audio-capture".into())
            .spawn(move || {
                // Nothing is ever sent: this hangs up as the thread ends.
                let _closed = closed_tx;
                hold_capture(device, opened_tx, stopped);
            })
            .map_err(device_error)?;
        let mut reader = opened_rx.recv_timeout(OPEN_WAIT).map_err(|e| match e {
            RecvTimeoutError::Timeout => {
                AudioError::Device("the microphone took too long to open".into())
            }
            RecvTimeoutError::Disconnected => {
                AudioError::Device("the capture thread stopped".into())
            }
        })??;
        // On a timeout the thread is left behind. If it ever finishes opening, nobody is
        // listening, and it closes the stream again.
        reader._stream = Box::new(StreamCloser {
            stop: Some(stop),
            closed: Some(closed),
            thread: Some(thread),
        });
        Ok(reader)
    }
}

/// Ends the capture thread when dropped, and waits for the stream to close.
struct StreamCloser {
    stop: Option<mpsc::Sender<()>>,
    closed: Option<mpsc::Receiver<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for StreamCloser {
    fn drop(&mut self) {
        self.stop.take();
        if let (Some(thread), Some(closed)) = (self.thread.take(), self.closed.take()) {
            join_within(thread, &closed, CLOSE_WAIT);
        }
    }
}

/// A `Stream` is not `Send` on every host, so it is built and dropped on this one thread, which
/// sleeps until `stopped` hangs up.
fn hold_capture(
    device: Option<DeviceId>,
    opened: mpsc::Sender<Result<CaptureReader, AudioError>>,
    stopped: mpsc::Receiver<()>,
) {
    match open_capture(device.as_ref()) {
        Ok((stream, reader)) => {
            if opened.send(Ok(reader)).is_ok() {
                // Nothing is ever sent, so this only returns when the reader drops.
                let _ = stopped.recv();
            }
            drop(stream);
        }
        Err(error) => {
            let _ = opened.send(Err(error));
        }
    }
}

fn open_capture(device: Option<&DeviceId>) -> Result<(cpal::Stream, CaptureReader), AudioError> {
    let host = cpal::default_host();
    let device = find_device(&host, DeviceKind::Input, device)?;
    let config = device.default_input_config().map_err(capture_error)?;
    let format = CaptureFormat {
        sample_rate: config.sample_rate(),
        channels: config.channels(),
    };
    let (writer, reader) = capture_ring(format, Box::new(()));
    let stream_config = config.config();
    let stream = match config.sample_format() {
        SampleFormat::F32 => build_capture::<f32>(&device, stream_config, writer),
        SampleFormat::I16 => build_capture::<i16>(&device, stream_config, writer),
        SampleFormat::U16 => build_capture::<u16>(&device, stream_config, writer),
        SampleFormat::I32 => build_capture::<i32>(&device, stream_config, writer),
        other => Err(AudioError::Device(format!(
            "unsupported sample format: {other}"
        ))),
    }?;
    stream.play().map_err(capture_error)?;
    Ok((stream, reader))
}

/// A capture that could not be built or started. A refusal by the operating system is told apart,
/// so the user can be told where to allow it.
pub(crate) fn capture_error(error: cpal::Error) -> AudioError {
    match error.kind() {
        ErrorKind::PermissionDenied => AudioError::PermissionDenied,
        ErrorKind::DeviceBusy => AudioError::DeviceBusy,
        _ => device_error(error),
    }
}

fn build_capture<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut writer: CaptureWriter,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let failed = Arc::clone(&writer.health);
    let mut scratch = vec![0.0_f32; SCRATCH_FRAMES * usize::from(config.channels.max(1))];
    let on_data = move |input: &[T], _: &cpal::InputCallbackInfo| {
        for block in input.chunks(scratch.len()) {
            let converted = &mut scratch[..block.len()];
            for (out, sample) in converted.iter_mut().zip(block) {
                *out = f32::from_sample_(*sample);
            }
            writer.write(converted);
        }
    };
    let on_error = move |error: cpal::Error| {
        if ends_capture(error.kind()) {
            failed.failed.store(true, Ordering::Release);
        }
    };
    device
        .build_input_stream(config, on_data, on_error, Some(OPEN_TIMEOUT))
        .map_err(capture_error)
}

/// Whether an error leaves the capture dead, so it must be reopened. As for outputs, this lists
/// what a stream survives.
fn ends_capture(kind: ErrorKind) -> bool {
    !matches!(
        kind,
        ErrorKind::DeviceChanged | ErrorKind::Xrun | ErrorKind::RealtimeDenied
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEREO_100: CaptureFormat = CaptureFormat {
        sample_rate: 100,
        channels: 2,
    };

    #[test]
    fn the_ring_holds_a_second_and_drops_on_overflow() {
        let (mut writer, mut reader) = capture_ring(STEREO_100, Box::new(()));

        // One second is 100 stereo frames, 200 samples; 50 more do not fit.
        let samples: Vec<f32> = (0..250).map(|i| i as f32).collect();
        assert_eq!(writer.write(&samples), 200);
        assert_eq!(reader.dropped(), 50);
        assert_eq!(reader.waiting(), 100);

        // A full ring takes nothing more and does not block.
        assert_eq!(writer.write(&[1.0, 2.0]), 0);
        assert_eq!(reader.dropped(), 52);

        // What fitted is the oldest audio, in order.
        let mut out = vec![0.0; 300];
        assert_eq!(reader.read(&mut out), 200);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[199], 199.0);
    }

    #[test]
    fn only_whole_frames_go_in_and_come_out() {
        let (mut writer, mut reader) = capture_ring(STEREO_100, Box::new(()));

        assert_eq!(writer.write(&[1.0, 2.0, 3.0]), 2);
        let mut out = [0.0; 3];
        assert_eq!(reader.read(&mut out), 2);
    }

    #[test]
    fn skip_to_newest_keeps_only_the_newest_frames() {
        let (mut writer, mut reader) = capture_ring(STEREO_100, Box::new(()));
        let samples: Vec<f32> = (0..20).map(|i| i as f32).collect();
        writer.write(&samples);

        reader.skip_to_newest(3);

        assert_eq!(reader.waiting(), 3);
        let mut out = [0.0; 6];
        assert_eq!(reader.read(&mut out), 6);
        assert_eq!(out, [14.0, 15.0, 16.0, 17.0, 18.0, 19.0]);

        // Keeping more than is waiting keeps it all.
        writer.write(&[1.0, 2.0]);
        reader.skip_to_newest(10);
        assert_eq!(reader.waiting(), 1);
    }

    #[test]
    fn a_closed_stream_reports_failed() {
        let (writer, reader) = capture_ring(STEREO_100, Box::new(()));
        assert!(!reader.failed());
        drop(writer);
        assert!(reader.failed());

        let (writer, reader) = capture_ring(STEREO_100, Box::new(()));
        writer.fail();
        assert!(reader.failed());
    }

    #[test]
    fn a_refusal_is_told_apart() {
        assert!(matches!(
            capture_error(ErrorKind::PermissionDenied.into()),
            AudioError::PermissionDenied
        ));
        assert!(matches!(
            capture_error(ErrorKind::BackendError.into()),
            AudioError::Device(_)
        ));
    }

    #[test]
    fn a_lost_device_ends_the_capture_and_an_xrun_does_not() {
        assert!(ends_capture(ErrorKind::DeviceNotAvailable));
        assert!(!ends_capture(ErrorKind::Xrun));
    }
}
