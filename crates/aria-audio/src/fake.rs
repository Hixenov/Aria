//! An output with no device behind it, for tests. Nothing plays until [`FakeControl::pull`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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
    refuse_exclusive: bool,
}

impl Default for FactoryState {
    fn default() -> Self {
        Self {
            failures_left: 0,
            outputs: Vec::new(),
            devices: Vec::new(),
            exclusive_rates: EXCLUSIVE_RATES.to_vec(),
            exclusive_kinds: EXCLUSIVE_KINDS.to_vec(),
            refuse_exclusive: false,
        }
    }
}

/// Opens fake outputs. Clones share their state, so a test can keep one and hand one away.
///
/// A shared open runs at the factory's format. An exclusive one takes the track's format where
/// the device allows it, at the factory's channel count only: by default every rate from
/// 44.1 kHz to 192 kHz and every [`SampleKind`].
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

    /// Make exclusive opens land on a shared output, as a device another program holds would.
    pub fn refuse_exclusive(&self, refuse: bool) {
        self.state().refuse_exclusive = refuse;
    }

    /// A control for every output opened so far, oldest first.
    pub fn outputs(&self) -> Vec<FakeControl> {
        self.state().outputs.clone()
    }

    fn state(&self) -> MutexGuard<'_, FactoryState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The format an exclusive open of `source` gets, if the device takes one.
    fn exclusive_format(&self, state: &FactoryState, source: SourceFormat) -> Option<DeviceFormat> {
        if state.refuse_exclusive {
            return None;
        }
        let channels = self.format.channels;
        choose_format(source, &state.exclusive_rates, &mut |format| {
            format.channels == channels
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
        let shared_format = DeviceFormat {
            sample_rate: self.format.sample_rate,
            channels: self.format.channels,
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
