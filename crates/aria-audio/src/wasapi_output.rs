//! The exclusive output (spec 3.2): the device held by Aria alone through WASAPI, at the track's
//! own rate and an integer format it accepts. Windows only.
//!
//! Like the CPAL output, the stream lives on a thread of its own, which reports the format it
//! opened, or the error, before [`open`] returns. All mixing stays in [`Mixer`]; the thread
//! renders it into a float buffer and converts that with [`write_samples`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};

use wasapi::{
    AudioClient, AudioRenderClient, Device, DeviceEnumerator, Direction, Handle, SampleType,
    StreamMode, WasapiError, WaveFormat,
};

use crate::cpal_output::{CLOSE_WAIT, OPEN_WAIT, join_within};
use crate::device::{DeviceId, Endpoint, device_error, find_endpoint};
use crate::error::AudioError;
use crate::mixer::{InputHandle, Mixer, MixerControl};
use crate::output::{AudioOutput, Opened, OutputFormat};
use crate::sample_format::{DeviceFormat, SampleKind, SourceFormat, choose_format, write_samples};

/// The rates asked about when the device does not take the track's own: the usual ones from
/// 32 kHz to 384 kHz.
const RATES: [u32; 9] = [
    32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
];
/// Devices that follow the Intel High Definition Audio specification need buffers in multiples
/// of 128 bytes.
const ALIGN_BYTES: u32 = 128;
/// `AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED`: the driver wants another period, which it then names.
const BUFFER_SIZE_NOT_ALIGNED: i32 = 0x8889_0019_u32 as i32;
/// How long the render thread waits for the device to ask for more. A device asks every period
/// (about 10 ms); one that stays quiet this long is gone.
const EVENT_WAIT_MS: u32 = 1_000;
/// Device periods the stream runs on after it is told to stop. In event-driven exclusive mode the
/// device asks for the next buffer as it starts playing the last one written, so the last buffer
/// of music has played out once it has asked twice more.
const DRAIN_PERIODS: usize = 2;

/// What the render thread and the output share. Atomics only: the render loop takes no lock.
#[derive(Default)]
struct Flags {
    /// Set by the output as it drops.
    stop: AtomicBool,
    /// Set by the render thread when the device fails.
    failed: AtomicBool,
}

/// Open `device`, or the default output when it is `None`, exclusively, in a format taken from
/// `source`, or from the device's own format when there is no track.
///
/// Fails when the device cannot be held exclusively in any format for the track: another program
/// holds it, the user does not allow exclusive mode, or no format fits. The caller then opens it
/// shared.
pub(crate) fn open(
    device: Option<&DeviceId>,
    source: Option<SourceFormat>,
) -> Result<Opened, AudioError> {
    let device = device.cloned();
    let flags = Arc::new(Flags::default());
    let thread_flags = Arc::clone(&flags);
    let (opened_tx, opened_rx) = mpsc::channel();
    let (closed_tx, closed) = mpsc::channel::<()>();
    let thread = thread::Builder::new()
        .name("aria-audio-exclusive".into())
        .spawn(move || {
            // Nothing is ever sent: this hangs up as the thread ends.
            let _closed = closed_tx;
            run(device.as_ref(), source, &thread_flags, &opened_tx);
        })
        .map_err(device_error)?;
    // On a timeout the thread is left behind. If it ever finishes opening, nobody is listening,
    // and it closes the stream again.
    let (device_format, control) = opened_rx.recv_timeout(OPEN_WAIT).map_err(|e| match e {
        RecvTimeoutError::Timeout => {
            AudioError::Device("the exclusive output took too long to open".into())
        }
        RecvTimeoutError::Disconnected => {
            AudioError::Device("the exclusive output thread stopped".into())
        }
    })??;
    Ok(Opened {
        output: Box::new(WasapiOutput {
            format: OutputFormat {
                sample_rate: device_format.sample_rate,
                channels: device_format.channels,
            },
            control,
            flags,
            closed,
            thread: Some(thread),
        }),
        exclusive: true,
        device: device_format,
    })
}

type OpenResult = Result<(DeviceFormat, MixerControl), AudioError>;

/// The render thread: open the stream, report how, then play until told to stop.
fn run(
    device: Option<&DeviceId>,
    source: Option<SourceFormat>,
    flags: &Flags,
    opened: &Sender<OpenResult>,
) {
    if let Err(error) = wasapi::initialize_mta().ok() {
        let _ = opened.send(Err(device_error(error)));
        return;
    }
    // In a block of its own, so every COM object is released before COM is closed.
    {
        match Stream::open(device, source) {
            Ok((stream, format, control)) => {
                if opened.send(Ok((format, control))).is_ok() {
                    stream.play(flags);
                }
            }
            Err(error) => {
                let _ = opened.send(Err(error));
            }
        }
    }
    wasapi::deinitialize();
}

/// The endpoint `id` names, or the default output.
fn find_device(enumerator: &DeviceEnumerator, id: Option<&DeviceId>) -> Result<Device, AudioError> {
    let Some(id) = id else {
        return enumerator
            .get_default_device(Direction::Render)
            .map_err(|_| AudioError::NoDevice);
    };
    let collection = enumerator
        .get_device_collection(Direction::Render)
        .map_err(device_error)?;
    let count = collection.get_nbr_devices().map_err(device_error)?;
    let mut devices = Vec::new();
    let mut endpoints = Vec::new();
    for index in 0..count {
        // A device that disconnects while it is listed is skipped.
        let Ok(device) = collection.get_device_at_index(index) else {
            continue;
        };
        endpoints.push(Endpoint {
            id: device.get_id().unwrap_or_default(),
            name: device.get_friendlyname().unwrap_or_default(),
        });
        devices.push(device);
    }
    let found = find_endpoint(id, &endpoints)
        .and_then(|found| endpoints.iter().position(|endpoint| endpoint == found));
    match found {
        Some(index) if index < devices.len() => Ok(devices.swap_remove(index)),
        _ => Err(AudioError::DeviceNotFound(id.0.clone())),
    }
}

/// The WASAPI description of `format`.
fn wave_format(format: DeviceFormat) -> WaveFormat {
    let (store, valid) = match format.kind {
        SampleKind::I16 => (16, 16),
        SampleKind::I24In32 => (32, 24),
        SampleKind::I32 => (32, 32),
    };
    WaveFormat::new(
        store,
        valid,
        SampleType::Int,
        format.sample_rate as usize,
        usize::from(format.channels),
        None,
    )
}

/// Asks the device about `format` in exclusive mode (`IsFormatSupported`, with the usual
/// driver quirks tried). The accepted form, which may carry another channel mask, is what the
/// stream must be opened with.
fn accepted(client: &AudioClient, format: DeviceFormat) -> Option<WaveFormat> {
    client
        .is_supported_exclusive_with_quirks(&wave_format(format))
        .ok()
}

/// The format to open with for `source`, in the order of spec 3.2, and its accepted form.
fn pick_format(client: &AudioClient, source: SourceFormat) -> Option<(DeviceFormat, WaveFormat)> {
    let mut chosen = None;
    let mut ask = |format: DeviceFormat| match accepted(client, format) {
        Some(wave) => {
            chosen = Some(wave);
            true
        }
        None => false,
    };
    // The track's own rate first. Only if the device takes nothing there are the other rates
    // asked about, which takes many more questions.
    let format = choose_format(source, &[], &mut ask).or_else(|| {
        let rates = supported_rates(client, source);
        choose_format(source, &rates, &mut ask)
    })?;
    Some((format, chosen?))
}

/// The rates of [`RATES`] at which the device takes `source`'s channels, or stereo for a mono
/// source, in any integer kind.
fn supported_rates(client: &AudioClient, source: SourceFormat) -> Vec<u32> {
    let layouts: &[u16] = if source.channels == 1 {
        &[1, 2]
    } else {
        std::slice::from_ref(&source.channels)
    };
    let kinds = [SampleKind::I16, SampleKind::I24In32, SampleKind::I32];
    RATES
        .into_iter()
        .filter(|rate| *rate != source.sample_rate)
        .filter(|rate| {
            kinds.iter().any(|kind| {
                layouts.iter().any(|channels| {
                    let format = DeviceFormat {
                        sample_rate: *rate,
                        channels: *channels,
                        kind: *kind,
                    };
                    accepted(client, format).is_some()
                })
            })
        })
        .collect()
}

/// The device's own format, as the track to open for when there is none.
fn device_source(device: &Device) -> Result<SourceFormat, AudioError> {
    let format = device.get_device_format().map_err(device_error)?;
    Ok(SourceFormat {
        sample_rate: format.get_samplespersec(),
        channels: format.get_nchannels(),
        bits: None,
        float: false,
    })
}

/// Initialize `client` for exclusive, event-driven rendering of `wave`. Some drivers refuse the
/// period asked for and name the one they want; then a fresh client is opened with that.
fn initialize(
    device: &Device,
    mut client: AudioClient,
    wave: &WaveFormat,
) -> Result<AudioClient, AudioError> {
    let (default_period, _) = client.get_device_period().map_err(device_error)?;
    let period = client
        .calculate_aligned_period_near(default_period, Some(ALIGN_BYTES), wave)
        .map_err(device_error)?;
    let mode = StreamMode::EventsExclusive { period_hns: period };
    match client.initialize_client(wave, Direction::Render, mode, None) {
        Ok(()) => Ok(client),
        Err(WasapiError::Windows(error)) if error.code().0 == BUFFER_SIZE_NOT_ALIGNED => {
            let frames = client.get_buffer_size().map_err(device_error)?;
            let period = wasapi::calculate_period_100ns(
                i64::from(frames),
                i64::from(wave.get_samplespersec()),
            );
            let mut client = device.get_iaudioclient().map_err(device_error)?;
            let mode = StreamMode::EventsExclusive { period_hns: period };
            client
                .initialize_client(wave, Direction::Render, mode, None)
                .map_err(device_error)?;
            Ok(client)
        }
        Err(error) => Err(device_error(error)),
    }
}

/// A running exclusive stream and everything its render loop needs, allocated up front.
struct Stream {
    client: AudioClient,
    render: AudioRenderClient,
    event: Handle,
    mixer: Mixer,
    kind: SampleKind,
    /// One device buffer of mixed audio.
    scratch: Vec<f32>,
    /// The same buffer in the device's format.
    bytes: Vec<u8>,
}

impl Stream {
    /// Open the stream, fill its first buffer and start it.
    fn open(
        device: Option<&DeviceId>,
        source: Option<SourceFormat>,
    ) -> Result<(Self, DeviceFormat, MixerControl), AudioError> {
        let enumerator = DeviceEnumerator::new().map_err(device_error)?;
        let device = find_device(&enumerator, device)?;
        let source = match source {
            Some(source) => source,
            None => device_source(&device)?,
        };
        let client = device.get_iaudioclient().map_err(device_error)?;
        let (format, wave) = pick_format(&client, source).ok_or_else(|| {
            AudioError::Device("the device takes no exclusive format for this track".into())
        })?;
        let client = initialize(&device, client, &wave)?;
        let event = client.set_get_eventhandle().map_err(device_error)?;
        let render = client.get_audiorenderclient().map_err(device_error)?;
        // In event-driven exclusive mode the device asks for one whole buffer at a time.
        let frames = client.get_buffer_size().map_err(device_error)? as usize;
        let samples = frames * usize::from(format.channels);
        let (mixer, control) = Mixer::new(OutputFormat {
            sample_rate: format.sample_rate,
            channels: format.channels,
        });
        let mut stream = Self {
            client,
            render,
            event,
            mixer,
            kind: format.kind,
            scratch: vec![0.0; samples],
            bytes: vec![0; samples * format.kind.bytes()],
        };
        // The first buffer goes in before the start, as WASAPI asks.
        stream.fill().map_err(device_error)?;
        stream.client.start_stream().map_err(device_error)?;
        Ok((stream, format, control))
    }

    /// Render the mixer into the next device buffer. Allocates nothing.
    fn fill(&mut self) -> Result<(), WasapiError> {
        self.mixer.render(&mut self.scratch);
        write_samples(self.kind, &self.scratch, &mut self.bytes);
        self.render.write_to_device(&self.bytes, None)
    }

    /// Write one buffer of silence. Allocates nothing.
    fn fill_silence(&mut self) -> Result<(), WasapiError> {
        self.bytes.fill(0);
        self.render.write_to_device(&self.bytes, None)
    }

    /// Play until `flags.stop` is set or the device fails, then let the last buffer play out and
    /// stop. The loop allocates nothing, takes no lock and logs nothing.
    fn play(mut self, flags: &Flags) {
        while !flags.stop.load(Ordering::Acquire) {
            if self.event.wait_for_event(EVENT_WAIT_MS).is_err() || self.fill().is_err() {
                flags.failed.store(true, Ordering::Release);
                break;
            }
        }
        if !flags.failed.load(Ordering::Acquire) {
            // The last buffer of music is in the device. Stopping now would cut it; let the
            // device ask twice more, with silence in between, so it has played out.
            for period in 0..DRAIN_PERIODS {
                if self.event.wait_for_event(EVENT_WAIT_MS).is_err() {
                    break;
                }
                if period + 1 < DRAIN_PERIODS && self.fill_silence().is_err() {
                    break;
                }
            }
        } else {
            tracing::debug!("the exclusive output stream ended");
        }
        let _ = self.client.stop_stream();
    }
}

/// An exclusive output. Dropping it lets what is already in the device play out, then closes
/// the stream and frees the device.
struct WasapiOutput {
    format: OutputFormat,
    control: MixerControl,
    flags: Arc<Flags>,
    /// Hangs up when the render thread ends.
    closed: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl AudioOutput for WasapiOutput {
    fn format(&self) -> OutputFormat {
        self.format
    }

    fn add_input(&mut self, capacity_frames: usize) -> Result<InputHandle, AudioError> {
        self.control.add_input(capacity_frames)
    }

    fn failed(&self) -> bool {
        self.flags.failed.load(Ordering::Acquire)
    }
}

impl Drop for WasapiOutput {
    fn drop(&mut self) {
        self.flags.stop.store(true, Ordering::Release);
        // Wait for the stream to close, so the device is free for whatever opens it next.
        if let Some(thread) = self.thread.take() {
            join_within(thread, &self.closed, CLOSE_WAIT);
        }
    }
}
