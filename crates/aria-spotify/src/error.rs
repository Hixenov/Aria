//! What can go wrong talking to Spotify, in terms the UI can name. No variant carries a token,
//! a code, a verifier or a request header.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpotifyError {
    #[error("network: {0}")]
    Network(String),
    #[error("the Spotify session has ended")]
    SessionEnded,
    #[error("sign-in was denied")]
    SignInDenied,
    #[error("sign-in timed out")]
    SignInTimedOut,
    #[error("sign-in was cancelled")]
    SignInCancelled,
    #[error("the sign-in port is in use")]
    PortBusy,
    #[error("this account is not on the app's allow-list")]
    NotAllowListed,
    #[error("rate limited for {retry_after:?}")]
    RateLimited { retry_after: Duration },
    #[error("the app's quota is used up for {retry_after:?}")]
    QuotaExceeded { retry_after: Duration },
    #[error("Spotify Premium is required")]
    PremiumRequired,
    #[error("no active Spotify device")]
    NoActiveDevice,
    #[error("Spotify answered {status}: {message}")]
    Api { status: u16, message: String },
    #[error("token store: {0}")]
    Store(String),
    #[error("no random numbers from the system")]
    Random,
    #[error("unexpected answer: {0}")]
    BadAnswer(String),
}
