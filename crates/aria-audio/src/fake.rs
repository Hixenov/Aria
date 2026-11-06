//! An output with no device behind it, for tests. Nothing plays until [`FakeControl::pull`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::device::DeviceId;
use crate::error::AudioError;
use crate::mixer::{InputHandle, Mixer, MixerControl};
use crate::output::{AudioOutput, OutputFactory, OutputFormat};

#[derive(Default)]
struct FactoryState {
    failures_left: usize,
    outputs: Vec<FakeControl>,
}

/// Opens fake outputs. Clones share their state, so a test can keep one and hand one away.
#[derive(Clone)]
pub struct FakeFactory {
    format: OutputFormat,
    state: Arc<Mutex<FactoryState>>,
}

impl FakeFactory {
    /// Create a factory whose outputs all run at `format`.
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

    /// A control for every output opened so far, oldest first.
    pub fn outputs(&self) -> Vec<FakeControl> {
        self.state().outputs.clone()
    }

    fn state(&self) -> MutexGuard<'_, FactoryState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl OutputFactory for FakeFactory {
    fn open(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioOutput>, AudioError> {
        let mut state = self.state();
        if state.failures_left > 0 {
            state.failures_left -= 1;
            return Err(AudioError::Device("fake open failure".into()));
        }
        let (mixer, control) = Mixer::new(self.format);
        let shared = Arc::new(FakeShared {
            mixer: Mutex::new(mixer),
            format: self.format,
            device: device.cloned(),
            failed: AtomicBool::new(false),
        });
        state.outputs.push(FakeControl {
            shared: Arc::clone(&shared),
        });
        Ok(Box::new(FakeOutput { shared, control }))
    }
}

struct FakeShared {
    mixer: Mutex<Mixer>,
    format: OutputFormat,
    device: Option<DeviceId>,
    failed: AtomicBool,
}

/// An output driven by hand through its [`FakeControl`].
pub struct FakeOutput {
    shared: Arc<FakeShared>,
    control: MixerControl,
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

    /// The device the output was opened on. `None` is the system default.
    pub fn device(&self) -> Option<DeviceId> {
        self.shared.device.clone()
    }
}
