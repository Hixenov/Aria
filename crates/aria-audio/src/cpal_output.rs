//! The real output. All mixing lives in [`Mixer`]; this only moves it into a CPAL stream.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample};

use crate::device::{DeviceId, DeviceKind, device_error, find_output_device, list_devices};
use crate::error::AudioError;
use crate::mixer::{InputHandle, Mixer, MixerControl};
use crate::output::{AudioOutput, DeviceLister, OutputFactory, OutputFormat};

/// How much the callback renders at a time, before converting to the device's sample type.
const SCRATCH_FRAMES: usize = 1024;
/// How long a backend may take to set up a stream.
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
/// How long opening may take in all, finding the device included. A driver can stick anywhere, and
/// whoever opens an output has other work to get back to.
const OPEN_WAIT: Duration = Duration::from_secs(7);
/// How long dropping an output waits for the stream to close.
const CLOSE_WAIT: Duration = Duration::from_secs(2);

/// Opens outputs through CPAL on the default host.
pub struct CpalFactory;

impl OutputFactory for CpalFactory {
    fn device_lister(&self) -> DeviceLister {
        Arc::new(|| list_devices(DeviceKind::Output))
    }

    fn open(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioOutput>, AudioError> {
        let device = device.cloned();
        let health = Arc::new(StreamHealth::default());
        let stream_health = Arc::clone(&health);
        let (opened_tx, opened_rx) = mpsc::channel();
        let (stop, stopped) = mpsc::channel::<()>();
        let (closed_tx, closed) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("aria-audio-output".into())
            .spawn(move || {
                // Nothing is ever sent: this hangs up as the thread ends.
                let _closed = closed_tx;
                hold_stream(device, stream_health, opened_tx, stopped);
            })
            .map_err(device_error)?;
        // On a timeout the thread is left behind. If it ever finishes opening, nobody is
        // listening, and it closes the stream again.
        let (format, control) = opened_rx.recv_timeout(OPEN_WAIT).map_err(|e| match e {
            RecvTimeoutError::Timeout => {
                AudioError::Device("the output took too long to open".into())
            }
            RecvTimeoutError::Disconnected => {
                AudioError::Device("the output thread stopped".into())
            }
        })??;
        Ok(Box::new(CpalOutput {
            format,
            control,
            health,
            stop: Some(stop),
            closed,
            thread: Some(thread),
        }))
    }
}

/// What the stream's error callback reports back.
#[derive(Default)]
struct StreamHealth {
    failed: AtomicBool,
    /// Errors the stream carried on from, such as xruns.
    survived: AtomicU64,
}

impl StreamHealth {
    /// Note a stream error. Hosts call this on audio threads, some of them real-time, so errors
    /// that can repeat while audio plays are only counted. Logging would lock and allocate.
    fn record(&self, error: &cpal::Error) {
        if ends_stream(error.kind()) {
            self.failed.store(true, Ordering::Release);
            tracing::debug!(%error, "audio stream ended");
        } else {
            self.survived.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A `Stream` is not `Send` on every host, so it is built and dropped on this one thread, which
/// sleeps until `stopped` hangs up.
fn hold_stream(
    device: Option<DeviceId>,
    health: Arc<StreamHealth>,
    opened: mpsc::Sender<Result<(OutputFormat, MixerControl), AudioError>>,
    stopped: mpsc::Receiver<()>,
) {
    match open_stream(device.as_ref(), Arc::clone(&health)) {
        Ok((stream, format, control)) => {
            if opened.send(Ok((format, control))).is_ok() {
                // Nothing is ever sent, so this only returns when the output drops.
                let _ = stopped.recv();
            }
            drop(stream);
            let survived = health.survived.load(Ordering::Relaxed);
            if survived > 0 {
                tracing::debug!(survived, "audio stream errors survived");
            }
        }
        Err(error) => {
            let _ = opened.send(Err(error));
        }
    }
}

fn open_stream(
    device: Option<&DeviceId>,
    health: Arc<StreamHealth>,
) -> Result<(cpal::Stream, OutputFormat, MixerControl), AudioError> {
    let host = cpal::default_host();
    let device = find_output_device(&host, device)?;
    let config = device.default_output_config().map_err(device_error)?;
    let format = OutputFormat {
        sample_rate: config.sample_rate(),
        channels: config.channels(),
    };
    let (mixer, control) = Mixer::new(format);
    let stream_config = config.config();
    let stream = match config.sample_format() {
        SampleFormat::F32 => build_stream::<f32>(&device, stream_config, mixer, health),
        SampleFormat::I16 => build_stream::<i16>(&device, stream_config, mixer, health),
        SampleFormat::U16 => build_stream::<u16>(&device, stream_config, mixer, health),
        SampleFormat::I32 => build_stream::<i32>(&device, stream_config, mixer, health),
        other => Err(AudioError::Device(format!(
            "unsupported sample format: {other}"
        ))),
    }?;
    stream.play().map_err(device_error)?;
    Ok((stream, format, control))
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut mixer: Mixer,
    health: Arc<StreamHealth>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let mut scratch = vec![0.0; SCRATCH_FRAMES * usize::from(config.channels).max(1)];
    let on_data = move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
        for block in out.chunks_mut(scratch.len()) {
            let rendered = &mut scratch[..block.len()];
            mixer.render(rendered);
            for (sample, value) in block.iter_mut().zip(rendered.iter()) {
                *sample = T::from_sample(*value);
            }
        }
    };
    let on_error = move |error: cpal::Error| health.record(&error);
    device
        .build_output_stream(config, on_data, on_error, Some(OPEN_TIMEOUT))
        .map_err(device_error)
}

/// Whether an error leaves the stream dead, so the output must be replaced.
///
/// WASAPI stops its stream on any error during a render pass, whatever the kind, so this lists
/// what a stream survives rather than what ends it. A needless rebuild is a short gap in the
/// sound; a dead stream nobody notices is silence with no way out.
fn ends_stream(kind: ErrorKind) -> bool {
    !matches!(
        kind,
        ErrorKind::DeviceChanged | ErrorKind::Xrun | ErrorKind::RealtimeDenied
    )
}

struct CpalOutput {
    format: OutputFormat,
    control: MixerControl,
    health: Arc<StreamHealth>,
    stop: Option<mpsc::Sender<()>>,
    /// Hangs up when the output thread ends.
    closed: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl AudioOutput for CpalOutput {
    fn format(&self) -> OutputFormat {
        self.format
    }

    fn add_input(&mut self, capacity_frames: usize) -> Result<InputHandle, AudioError> {
        self.control.add_input(capacity_frames)
    }

    fn failed(&self) -> bool {
        self.health.failed.load(Ordering::Acquire)
    }
}

impl Drop for CpalOutput {
    fn drop(&mut self) {
        self.stop.take();
        // Wait for the stream to close, so the device is free for whatever opens it next.
        if let Some(thread) = self.thread.take() {
            join_within(thread, &self.closed, CLOSE_WAIT);
        }
    }
}

/// Join `thread` once `closed` hangs up. Returns false if that takes longer than `limit`, which
/// leaves the thread to finish on its own, or if it panicked.
fn join_within(thread: JoinHandle<()>, closed: &Receiver<()>, limit: Duration) -> bool {
    if let Err(RecvTimeoutError::Timeout) = closed.recv_timeout(limit) {
        tracing::debug!("the audio stream did not close in time; leaving it");
        return false;
    }
    let joined = thread.join().is_ok();
    if !joined {
        tracing::debug!("the audio output thread panicked");
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closing_thread_is_joined() {
        let (hang_up, closed) = mpsc::channel::<()>();
        let thread = thread::spawn(move || drop(hang_up));

        assert!(join_within(thread, &closed, Duration::from_secs(20)));
    }

    #[test]
    fn a_stuck_thread_is_left_behind() {
        let (release, stuck) = mpsc::channel::<()>();
        let (hang_up, closed) = mpsc::channel::<()>();
        let thread = thread::spawn(move || {
            let _hang_up = hang_up;
            let _ = stuck.recv();
        });

        assert!(!join_within(thread, &closed, Duration::from_millis(20)));

        drop(release);
    }

    #[test]
    fn errors_that_end_the_stream_fail_the_output() {
        for kind in [
            ErrorKind::BackendError,
            ErrorKind::DeviceBusy,
            ErrorKind::DeviceNotAvailable,
            ErrorKind::HostUnavailable,
            ErrorKind::InvalidInput,
            ErrorKind::Other,
            ErrorKind::PermissionDenied,
            ErrorKind::ResourceExhausted,
            ErrorKind::StreamInvalidated,
            ErrorKind::UnsupportedConfig,
            ErrorKind::UnsupportedOperation,
        ] {
            assert!(ends_stream(kind), "{kind:?} leaves the stream dead");
        }
    }

    #[test]
    fn errors_the_stream_survives_leave_the_output_running() {
        for kind in [
            ErrorKind::DeviceChanged,
            ErrorKind::Xrun,
            ErrorKind::RealtimeDenied,
        ] {
            assert!(!ends_stream(kind), "the stream survives {kind:?}");
        }
    }

    #[test]
    fn survived_errors_are_counted_without_failing_the_output() {
        let health = StreamHealth::default();

        health.record(&ErrorKind::Xrun.into());
        health.record(&ErrorKind::DeviceChanged.into());

        assert_eq!(health.survived.load(Ordering::Relaxed), 2);
        assert!(!health.failed.load(Ordering::Acquire));
    }

    #[test]
    fn fatal_errors_fail_the_output_without_being_counted() {
        let health = StreamHealth::default();

        health.record(&ErrorKind::StreamInvalidated.into());

        assert_eq!(health.survived.load(Ordering::Relaxed), 0);
        assert!(health.failed.load(Ordering::Acquire));
    }
}
