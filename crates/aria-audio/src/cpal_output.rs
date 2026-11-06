//! The real output. All mixing lives in [`Mixer`]; this only moves it into a CPAL stream.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample};

use crate::device::{DeviceId, device_error, find_output_device};
use crate::error::AudioError;
use crate::mixer::{InputHandle, Mixer, MixerControl};
use crate::output::{AudioOutput, OutputFactory, OutputFormat};

/// How much the callback renders at a time, before converting to the device's sample type.
const SCRATCH_FRAMES: usize = 1024;
/// How long a backend may take to set up a stream.
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens outputs through CPAL on the default host.
pub struct CpalFactory;

impl OutputFactory for CpalFactory {
    fn open(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioOutput>, AudioError> {
        let device = device.cloned();
        let failed = Arc::new(AtomicBool::new(false));
        let stream_failed = Arc::clone(&failed);
        let (opened_tx, opened_rx) = mpsc::channel();
        let (stop, stopped) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("aria-audio-output".into())
            .spawn(move || hold_stream(device, stream_failed, opened_tx, stopped))
            .map_err(device_error)?;
        let (format, control) = opened_rx
            .recv()
            .map_err(|_| AudioError::Device("the output thread stopped".into()))??;
        Ok(Box::new(CpalOutput {
            format,
            control,
            failed,
            stop: Some(stop),
            thread: Some(thread),
        }))
    }
}

/// A `Stream` is not `Send` on every host, so it is built and dropped on this one thread, which
/// sleeps until `stopped` hangs up.
fn hold_stream(
    device: Option<DeviceId>,
    failed: Arc<AtomicBool>,
    opened: mpsc::Sender<Result<(OutputFormat, MixerControl), AudioError>>,
    stopped: mpsc::Receiver<()>,
) {
    match open_stream(device.as_ref(), failed) {
        Ok((stream, format, control)) => {
            if opened.send(Ok((format, control))).is_ok() {
                // Nothing is ever sent, so this only returns when the output drops.
                let _ = stopped.recv();
            }
            drop(stream);
        }
        Err(error) => {
            let _ = opened.send(Err(error));
        }
    }
}

fn open_stream(
    device: Option<&DeviceId>,
    failed: Arc<AtomicBool>,
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
        SampleFormat::F32 => build_stream::<f32>(&device, stream_config, mixer, failed),
        SampleFormat::I16 => build_stream::<i16>(&device, stream_config, mixer, failed),
        SampleFormat::U16 => build_stream::<u16>(&device, stream_config, mixer, failed),
        SampleFormat::I32 => build_stream::<i32>(&device, stream_config, mixer, failed),
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
    failed: Arc<AtomicBool>,
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
    let on_error = move |error: cpal::Error| {
        if error.kind() == ErrorKind::DeviceNotAvailable {
            failed.store(true, Ordering::Release);
        } else {
            tracing::debug!(%error, "audio stream error");
        }
    };
    device
        .build_output_stream(config, on_data, on_error, Some(OPEN_TIMEOUT))
        .map_err(device_error)
}

struct CpalOutput {
    format: OutputFormat,
    control: MixerControl,
    failed: Arc<AtomicBool>,
    stop: Option<mpsc::Sender<()>>,
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
        self.failed.load(Ordering::Acquire)
    }
}

impl Drop for CpalOutput {
    fn drop(&mut self) {
        self.stop.take();
        // Wait for the stream to close, so the device is free for whatever opens it next.
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::debug!("the audio output thread panicked");
        }
    }
}
