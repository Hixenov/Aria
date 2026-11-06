//! What the player needs from an audio output.

use crate::device::DeviceId;
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

/// Opens outputs. Each call opens a new, independent one.
pub trait OutputFactory: Send {
    /// Open `device`, or the system default when it is `None`.
    fn open(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioOutput>, AudioError>;
}
