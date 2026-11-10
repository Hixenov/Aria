//! Audio decoding and playback.

pub mod clock;
mod cpal_output;
pub mod decode;
pub mod device;
pub mod engine;
pub mod error;
pub mod fake;
pub mod mixer;
mod mp4;
pub mod output;
pub mod resample;

#[cfg(feature = "test-support")]
pub mod testing;

pub use clock::{ClockReading, PlaybackClock};
pub use cpal_output::CpalFactory;
pub use decode::{Clip, StreamInfo, TrackDecoder, decode_clip};
pub use device::{DeviceId, DeviceInfo, DeviceKind, list_devices};
pub use engine::{Engine, EngineCommand, EngineEvent, EngineHandle, IDLE_GRACE, TrackRequest};
pub use error::AudioError;
pub use mixer::{FADE_MS, InputHandle, MAX_INPUTS, Mixer, MixerControl};
pub use output::{AudioOutput, DeviceLister, OutputFactory, OutputFormat};
pub use resample::{Resampler, convert_channels};
