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
    /// Another stream holds the device, for example exclusively, so it cannot be opened now.
    #[error("audio device in use")]
    DeviceBusy,
    /// There is no output device.
    #[error("no audio output device")]
    NoDevice,
    /// The operating system refuses this program the device, as it can for a microphone.
    #[error("access to the audio device denied")]
    PermissionDenied,
    /// The microphone asked for is a virtual cable's recording side. Sending it into the cable
    /// would feed the chat's sound back into itself, so it is never opened.
    #[error("a virtual cable is not a microphone")]
    CableAsMicrophone,
    /// The named output device is not present.
    #[error("audio device not found: {0}")]
    DeviceNotFound(String),
}
