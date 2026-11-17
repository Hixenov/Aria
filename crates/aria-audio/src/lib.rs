//! Audio decoding and playback.

pub mod capture;
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
pub mod sample_format;
mod sounds;
mod system_output;
#[cfg(windows)]
mod wasapi_output;

#[cfg(feature = "test-support")]
pub mod testing;

pub use capture::{CaptureFactory, CaptureFormat, CaptureReader, CpalCapture};
pub use clock::{ClockReading, PlaybackClock};
pub use cpal_output::CpalFactory;
pub use decode::{Clip, StreamInfo, TrackDecoder, decode_clip};
pub use device::{DeviceId, DeviceInfo, DeviceKind, list_devices};
pub use engine::{Engine, EngineCommand, EngineEvent, EngineHandle, IDLE_GRACE, TrackRequest};
pub use error::AudioError;
pub use mixer::{FADE_MS, InputHandle, MAX_INPUTS, Mixer, MixerControl};
pub use output::{AudioOutput, DeviceLister, Opened, OutputFactory, OutputFormat, OutputRequest};
pub use resample::{Resampler, convert_channels};
pub use sample_format::{
    DeviceFormat, OutputPath, OutputState, PathFacts, SampleKind, SourceFormat, choose_format,
    nearest_rate, output_path, write_samples,
};
pub use sounds::{SOUNDS_AT_ONCE, SoundRoute};
pub use system_output::SystemFactory;
