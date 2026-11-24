//! Everything in Aria that talks to Spotify: PKCE sign-in, tokens, the Web API client and the
//! refresh token's store. Nothing of the UI is here. Aria plays no Spotify audio: it only
//! controls Spotify's own devices through the public Web API.

pub mod auth;
pub mod browser;
pub mod callback;
pub mod client;
pub mod error;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
pub mod http;
pub mod model;
pub mod secret;
pub mod store;
pub mod token;

pub use auth::{Pkce, authorize_url};
pub use callback::Callback;
pub use client::{Client, fetch_image};
pub use error::SpotifyError;
pub use http::Http;
pub use model::{
    Account, NowItem, PlayRequest, PlaybackState, RepeatState, SearchResults, SpAlbum, SpArtist,
    SpDevice, SpPlaylist, SpTrack,
};
pub use secret::Secret;
pub use store::{KeyringStore, MemoryStore, TokenStore, system_store};
pub use token::{Tokens, exchange_code, refresh};

/// Aria's Spotify app (Web API only). Built in: PKCE needs no client secret.
pub const CLIENT_ID: &str = "250bd9732a644668911a74bdd329e66e";

/// The port of the redirect URI registered for [`CLIENT_ID`].
pub const REDIRECT_PORT: u16 = 43819;

/// The scopes Aria asks for, space-separated.
pub const SCOPES: &str = "user-read-private user-library-read user-follow-read \
playlist-read-private playlist-read-collaborative user-read-playback-state \
user-modify-playback-state user-read-currently-playing streaming user-read-email";

/// The redirect URI for a loopback port: `http://127.0.0.1:{port}/callback`.
pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

/// Where Spotify's accounts service and Web API are. Tests point these at a fake server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub accounts: String,
    pub api: String,
}

impl Endpoints {
    pub fn spotify() -> Endpoints {
        Endpoints {
            accounts: "https://accounts.spotify.com".to_owned(),
            api: "https://api.spotify.com/v1".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scopes_include_streaming_and_email() {
        let scopes: Vec<&str> = SCOPES.split_whitespace().collect();
        assert!(scopes.contains(&"streaming"));
        assert!(scopes.contains(&"user-read-email"));
        assert!(scopes.contains(&"user-read-private"));
    }
}
