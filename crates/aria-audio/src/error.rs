//! Errors shared by the audio modules.

/// What can go wrong while opening, decoding, resampling or playing audio.
///
/// Messages never include file paths; callers that want them log at debug level.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// The file could not be opened or read.
    #[error("could not open audio: {0}")]
    Open(String),
    /// The file is not in a format or codec this build can decode.
    #[error("unsupported audio: {0}")]
    Unsupported(String),
    /// The stream is damaged or the decoder gave up.
    #[error("could not decode audio: {0}")]
    Decode(String),
    /// The resampler could not be built or failed while running.
    #[error("could not resample audio: {0}")]
    Resample(String),
    /// The output device failed.
    #[error("audio device error: {0}")]
    Device(String),
    /// There is no output device.
    #[error("no audio output device")]
    NoDevice,
    /// The named output device is not present.
    #[error("audio device not found: {0}")]
    DeviceNotFound(String),
}
