//! Audio decoding and playback.

pub mod decode;
pub mod error;
mod mp4;
pub mod resample;

#[cfg(feature = "test-support")]
pub mod testing;

pub use decode::{Clip, StreamInfo, TrackDecoder, decode_clip};
pub use error::AudioError;
pub use resample::{Resampler, convert_channels};
