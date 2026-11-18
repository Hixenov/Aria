//! What the player needs from an audio output.

use std::sync::Arc;

use crate::device::{DeviceId, DeviceInfo};
use crate::error::AudioError;
use crate::mixer::InputHandle;
use crate::sample_format::{DeviceFormat, SourceFormat};

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

/// How an output is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutputRequest {
    /// Open the device for this program alone, when it allows that.
    pub exclusive: bool,
    /// The track the output is for, which an exclusive output takes its format from. `None`:
    /// the device's own format.
    pub source: Option<SourceFormat>,
}

/// An output just opened, and how.
pub struct Opened {
    pub output: Box<dyn AudioOutput>,
    /// Whether the device is held exclusively. False when exclusive was asked for and the device
    /// would not allow it.
    pub exclusive: bool,
    /// The format the device runs at. A shared output reports [`SampleKind::I32`] for the float
    /// stream it hands to the system mixer.
    ///
    /// [`SampleKind::I32`]: crate::sample_format::SampleKind::I32
    pub device: DeviceFormat,
}

/// Lists the output devices a factory can open. It can be slow, so it is for a thread that may
/// wait.
pub type DeviceLister = Arc<dyn Fn() -> Result<Vec<DeviceInfo>, AudioError> + Send + Sync>;

/// Opens outputs. Each call opens a new, independent one, and calls may come from more than one
/// thread at once.
pub trait OutputFactory: Send + Sync {
    /// Open `device`, or the system default when it is `None`.
    ///
    /// A factory that cannot open the device exclusively opens it shared and says so in
    /// [`Opened::exclusive`]. It never fails only because exclusive was asked for.
    fn open(&self, device: Option<&DeviceId>, request: OutputRequest)
    -> Result<Opened, AudioError>;

    /// A way to list the devices this factory opens, which can be kept after the factory has been
    /// handed to an engine.
    fn device_lister(&self) -> DeviceLister;

    /// The system default output device, if there is one. Factories that can ask for it alone
    /// should, as this one lists every device to find it.
    fn default_device(&self) -> Option<DeviceId> {
        let devices = (self.device_lister())().ok()?;
        devices
            .into_iter()
            .find(|device| device.is_default)
            .map(|device| device.id)
    }
}
