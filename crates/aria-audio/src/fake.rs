//! An output with no device behind it, for tests. Nothing plays until [`FakeControl::pull`].
//! And a capture with no device behind it, which captures what a test pushes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::capture::{CaptureFactory, CaptureFormat, CaptureReader, CaptureWriter, capture_ring};
use crate::device::{DeviceId, DeviceInfo};
use crate::error::AudioError;
use crate::mixer::{InputHandle, Mixer, MixerControl};
use crate::output::{
    AudioOutput, DeviceLister, Opened, OutputFactory, OutputFormat, OutputRequest,
};
use crate::sample_format::{DeviceFormat, SampleKind, SourceFormat, choose_format};

/// The rates a fake device takes exclusively until told otherwise.
const EXCLUSIVE_RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
/// The sample kinds a fake device takes exclusively until told otherwise.
const EXCLUSIVE_KINDS: [SampleKind; 3] = [SampleKind::I16, SampleKind::I24In32, SampleKind::I32];

struct FactoryState {
    failures_left: usize,
    outputs: Vec<FakeControl>,
    devices: Vec<DeviceInfo>,
    exclusive_rates: Vec<u32>,
    exclusive_kinds: Vec<SampleKind>,
    /// `None`: the factory's own channel count only.
    exclusive_channels: Option<Vec<u16>>,
    refuse_exclusive: bool,
    /// Devices that never open, as unplugged ones would not.
    missing: Vec<DeviceId>,
    /// Devices whose shared outputs run at another format than the factory's.
    shared_formats: Vec<(DeviceId, OutputFormat)>,
}

impl Default for FactoryState {
    fn default() -> Self {
        Self {
            failures_left: 0,
            outputs: Vec::new(),
            devices: Vec::new(),
            exclusive_rates: EXCLUSIVE_RATES.to_vec(),
            exclusive_kinds: EXCLUSIVE_KINDS.to_vec(),
            exclusive_channels: None,
            refuse_exclusive: false,
            missing: Vec::new(),
            shared_formats: Vec::new(),
        }
    }
}

/// Opens fake outputs. Clones share their state, so a test can keep one and hand one away.
///
/// A shared open runs at the factory's format. An exclusive one takes the track's format where
/// the device allows it: by default the factory's channel count only, every rate from 44.1 kHz
/// to 192 kHz and every [`SampleKind`].
#[derive(Clone)]
pub struct FakeFactory {
    format: OutputFormat,
    state: Arc<Mutex<FactoryState>>,
}

impl FakeFactory {
    /// Create a factory whose shared outputs all run at `format`.
    pub fn new(format: OutputFormat) -> Self {
        Self {
            format,
            state: Arc::default(),
        }
    }

    /// Make the next `count` opens fail.
    pub fn fail_open(&self, count: usize) {
        self.state().failures_left = count;
    }

    /// Make the devices the factory lists `devices`. It lists none until then.
    pub fn set_devices(&self, devices: Vec<DeviceInfo>) {
        self.state().devices = devices;
    }

    /// Make the rates the device takes exclusively `rates`.
    pub fn set_exclusive_rates(&self, rates: Vec<u32>) {
        self.state().exclusive_rates = rates;
    }

    /// Make the sample kinds the device takes exclusively `kinds`.
    pub fn set_exclusive_kinds(&self, kinds: Vec<SampleKind>) {
        self.state().exclusive_kinds = kinds;
    }

    /// Make the channel counts the device takes exclusively `channels`. Until then it takes only
    /// the factory's own.
    pub fn set_exclusive_channels(&self, channels: Vec<u16>) {
        self.state().exclusive_channels = Some(channels);
    }

    /// Make exclusive opens land on a shared output, as a device another program holds would.
    pub fn refuse_exclusive(&self, refuse: bool) {
        self.state().refuse_exclusive = refuse;
    }

    /// Make every open of `device` fail from now on, as for a device that is not plugged in.
    pub fn remove_device(&self, device: DeviceId) {
        self.state().missing.push(device);
    }

    /// Make shared outputs on `device` run at `format`, instead of the factory's.
    pub fn set_shared_format(&self, device: DeviceId, format: OutputFormat) {
        self.state().shared_formats.push((device, format));
    }

    /// A control for every output opened so far, oldest first.
    pub fn outputs(&self) -> Vec<FakeControl> {
        self.state().outputs.clone()
    }

    /// A control for every output opened so far on `device`, oldest first. `None` is the system
    /// default.
    pub fn outputs_on(&self, device: Option<&DeviceId>) -> Vec<FakeControl> {
        self.state()
            .outputs
            .iter()
            .filter(|output| output.shared.device.as_ref() == device)
            .cloned()
            .collect()
    }

    fn state(&self) -> MutexGuard<'_, FactoryState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The format an exclusive open of `source` gets, if the device takes one.
    fn exclusive_format(&self, state: &FactoryState, source: SourceFormat) -> Option<DeviceFormat> {
        if state.refuse_exclusive {
            return None;
        }
        let own = [self.format.channels];
        let channels = state.exclusive_channels.as_deref().unwrap_or(&own);
        choose_format(source, &state.exclusive_rates, &mut |format| {
            channels.contains(&format.channels)
                && state.exclusive_rates.contains(&format.sample_rate)
                && state.exclusive_kinds.contains(&format.kind)
        })
    }
}

impl OutputFactory for FakeFactory {
    fn device_lister(&self) -> DeviceLister {
        let state = Arc::clone(&self.state);
        Arc::new(move || {
            let state = state.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(state.devices.clone())
        })
    }

    fn open(
        &self,
        device: Option<&DeviceId>,
        request: OutputRequest,
    ) -> Result<Opened, AudioError> {
        let mut state = self.state();
        if state.failures_left > 0 {
            state.failures_left -= 1;
            return Err(AudioError::Device("fake open failure".into()));
        }
        if let Some(device) = device.filter(|device| state.missing.contains(device)) {
            return Err(AudioError::DeviceNotFound(device.0.clone()));
        }
        let own = device
            .and_then(|device| {
                state
                    .shared_formats
                    .iter()
                    .find(|(id, _)| id == device)
                    .map(|(_, format)| *format)
            })
            .unwrap_or(self.format);
        let shared_format = DeviceFormat {
            sample_rate: own.sample_rate,
            channels: own.channels,
            kind: SampleKind::I32,
        };
        let exclusive_format = request.exclusive.then(|| {
            let source = request.source.unwrap_or(SourceFormat {
                sample_rate: self.format.sample_rate,
                channels: self.format.channels,
                bits: None,
                float: false,
            });
            self.exclusive_format(&state, source)
        });
        let (exclusive, device_format) = match exclusive_format.flatten() {
            Some(format) => (true, format),
            None => (false, shared_format),
        };
        let format = OutputFormat {
            sample_rate: device_format.sample_rate,
            channels: device_format.channels,
        };
        let (mixer, control) = Mixer::new(format);
        let shared = Arc::new(FakeShared {
            mixer: Mutex::new(mixer),
            format,
            device: device.cloned(),
            opened_as: (exclusive, device_format),
            failed: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        state.outputs.push(FakeControl {
            shared: Arc::clone(&shared),
        });
        Ok(Opened {
            output: Box::new(FakeOutput { shared, control }),
            exclusive,
            device: device_format,
        })
    }
}

struct FakeShared {
    mixer: Mutex<Mixer>,
    format: OutputFormat,
    device: Option<DeviceId>,
    opened_as: (bool, DeviceFormat),
    failed: AtomicBool,
    closed: AtomicBool,
}

/// An output driven by hand through its [`FakeControl`].
pub struct FakeOutput {
    shared: Arc<FakeShared>,
    control: MixerControl,
}

impl Drop for FakeOutput {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
    }
}

impl AudioOutput for FakeOutput {
    fn format(&self) -> OutputFormat {
        self.shared.format
    }

    fn add_input(&mut self, capacity_frames: usize) -> Result<InputHandle, AudioError> {
        self.control.add_input(capacity_frames)
    }

    fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Acquire)
    }
}

/// The test's side of a [`FakeOutput`].
#[derive(Clone)]
pub struct FakeControl {
    shared: Arc<FakeShared>,
}

impl FakeControl {
    /// Run the mixer for `frames` frames and return what it rendered.
    pub fn pull(&self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * usize::from(self.shared.format.channels)];
        self.shared
            .mixer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .render(&mut out);
        out
    }

    /// Mark the output failed, as a lost device would.
    pub fn fail(&self) {
        self.shared.failed.store(true, Ordering::Release);
    }

    /// Whether the output has been dropped, as a closed stream would be.
    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }

    /// The device the output was opened on. `None` is the system default.
    pub fn device(&self) -> Option<DeviceId> {
        self.shared.device.clone()
    }

    /// Whether the output was opened exclusively, and at what format.
    pub fn opened_as(&self) -> (bool, DeviceFormat) {
        self.shared.opened_as
    }
}

/// What a [`FakeCapture`] and its control share.
#[derive(Default)]
struct CaptureState {
    /// The open capture's writer, if one is open.
    writer: Option<CaptureWriter>,
    /// Set when the open capture's reader drops.
    open: Option<Arc<AtomicBool>>,
    opened: usize,
    fail_open: bool,
    deny: bool,
    devices: Vec<DeviceInfo>,
}

/// A capture with no device behind it, for tests. It captures what [`FakeCaptureControl::push`]
/// gives it.
pub struct FakeCapture {
    format: CaptureFormat,
    state: Arc<Mutex<CaptureState>>,
}

/// The test's side of a [`FakeCapture`].
#[derive(Clone)]
pub struct FakeCaptureControl {
    state: Arc<Mutex<CaptureState>>,
}

fn capture_state(state: &Mutex<CaptureState>) -> MutexGuard<'_, CaptureState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Marks the fake capture closed when its reader drops.
struct FakeCaptureOpen(Arc<AtomicBool>);

impl Drop for FakeCaptureOpen {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl FakeCapture {
    /// A fake capture whose every open runs at `format`.
    pub fn new(format: CaptureFormat) -> (Self, FakeCaptureControl) {
        let state = Arc::new(Mutex::new(CaptureState::default()));
        (
            Self {
                format,
                state: Arc::clone(&state),
            },
            FakeCaptureControl { state },
        )
    }
}

impl CaptureFactory for FakeCapture {
    fn device_lister(&self) -> DeviceLister {
        let state = Arc::clone(&self.state);
        Arc::new(move || Ok(capture_state(&state).devices.clone()))
    }

    fn open(&self, _device: Option<&DeviceId>) -> Result<CaptureReader, AudioError> {
        let mut state = capture_state(&self.state);
        if state.deny {
            return Err(AudioError::PermissionDenied);
        }
        if state.fail_open {
            return Err(AudioError::Device("fake capture failure".into()));
        }
        let open = Arc::new(AtomicBool::new(true));
        let (writer, reader) =
            capture_ring(self.format, Box::new(FakeCaptureOpen(Arc::clone(&open))));
        state.writer = Some(writer);
        state.open = Some(open);
        state.opened += 1;
        Ok(reader)
    }
}

impl FakeCaptureControl {
    /// Capture `samples` into the open capture, as much as its ring holds. Returns how many went
    /// in: none when nothing is open.
    pub fn push(&self, samples: &[f32]) -> usize {
        let mut state = capture_state(&self.state);
        if !state
            .open
            .as_ref()
            .is_some_and(|open| open.load(Ordering::Acquire))
        {
            return 0;
        }
        state
            .writer
            .as_mut()
            .map_or(0, |writer| writer.write(samples))
    }

    /// Make the open capture fail, as a lost device does.
    pub fn fail(&self) {
        if let Some(writer) = &capture_state(&self.state).writer {
            writer.fail();
        }
    }

    /// Make the input devices the fake lists `devices`. It lists none until then.
    pub fn set_devices(&self, devices: Vec<DeviceInfo>) {
        capture_state(&self.state).devices = devices;
    }

    /// Make opens fail from now on, or work again.
    pub fn fail_open(&self, fail: bool) {
        capture_state(&self.state).fail_open = fail;
    }

    /// Make opens be refused by the "operating system" from now on, or allowed again.
    pub fn deny(&self, deny: bool) {
        capture_state(&self.state).deny = deny;
    }

    /// How many captures have been opened.
    pub fn opened(&self) -> usize {
        capture_state(&self.state).opened
    }

    /// Whether a capture is open now.
    pub fn open_now(&self) -> bool {
        capture_state(&self.state)
            .open
            .as_ref()
            .is_some_and(|open| open.load(Ordering::Acquire))
    }
}
