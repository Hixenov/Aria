//! What the player needs from an audio output.

use std::sync::Arc;

use crate::device::{DeviceId, DeviceInfo};
use crate::error::AudioError;
use crate::mixer::InputHandle;

/// The sample rate and channel count an output runs at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputFormat {
    /// Frames per second.
    pub sample_rate: u32,
    /// Interleaved channels per frame.
    pub channels: u16,
}

/// One stream on one device, with its own mixer. Dropping it closes the stream.
pub trait AudioOutput: Send {
    /// The format every input of this output must be written in.
    fn format(&self) -> OutputFormat;

    /// Add an input whose ring holds `capacity_frames` frames.
    fn add_input(&mut self, capacity_frames: usize) -> Result<InputHandle, AudioError>;

    /// Whether the stream has ended, so the output must be replaced.
    fn failed(&self) -> bool;
}

/// Lists the output devices a factory can open. It can be slow, so it is for a thread that may
/// wait.
pub type DeviceLister = Arc<dyn Fn() -> Result<Vec<DeviceInfo>, AudioError> + Send + Sync>;

/// Opens outputs. Each call opens a new, independent one.
pub trait OutputFactory: Send {
    /// Open `device`, or the system default when it is `None`.
    fn open(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioOutput>, AudioError>;

    /// A way to list the devices this factory opens, which can be kept after the factory has been
    /// handed to an engine.
    fn device_lister(&self) -> DeviceLister;
}
