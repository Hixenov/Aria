//! PKCE (RFC 7636) and the authorize URL.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::{CLIENT_ID, Endpoints, SCOPES, Secret, SpotifyError};

/// One sign-in's PKCE values. The verifier never leaves Aria except in the token exchange.
#[derive(Clone, Debug)]
pub struct Pkce {
    pub verifier: Secret,
    pub challenge: String,
    pub state: String,
}

impl Pkce {
    /// A fresh verifier (64 random bytes, 86 base64url characters) and `state` (16 bytes).
    pub fn new() -> Result<Pkce, SpotifyError> {
        let mut verifier = [0u8; 64];
        let mut state = [0u8; 16];
        getrandom::fill(&mut verifier).map_err(|_| SpotifyError::Random)?;
        getrandom::fill(&mut state).map_err(|_| SpotifyError::Random)?;
        Ok(Pkce::from_parts(
            &URL_SAFE_NO_PAD.encode(verifier),
            &URL_SAFE_NO_PAD.encode(state),
        ))
    }

    /// PKCE from a known verifier and state; the challenge is computed (S256).
    pub fn from_parts(verifier: &str, state: &str) -> Pkce {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Pkce {
            verifier: Secret::new(verifier.to_owned()),
            challenge,
            state: state.to_owned(),
        }
    }
}

/// The URL that starts the sign-in in the browser. It carries the challenge, never the verifier.
pub fn authorize_url(endpoints: &Endpoints, pkce: &Pkce, redirect_uri: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("state", &pkce.state)
        .append_pair("scope", SCOPES)
        .finish();
    format!("{}/authorize?{query}", endpoints.accounts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{REDIRECT_PORT, redirect_uri};

    #[test]
    fn the_challenge_is_rfc_7636s_example() {
        // RFC 7636, appendix B.
        let pkce = Pkce::from_parts("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", "s");
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn new_verifiers_are_long_random_and_url_safe() {
        let (a, b) = (Pkce::new().unwrap(), Pkce::new().unwrap());
        assert_eq!(a.verifier.expose().len(), 86);
        assert_ne!(a.verifier.expose(), b.verifier.expose());
        assert_ne!(a.state, b.state);
        assert!(
            a.verifier
                .expose()
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
    }

    #[test]
    fn the_authorize_url_asks_for_s256_with_the_built_in_client() {
        let pkce = Pkce::from_parts("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", "st4te");
        let url = authorize_url(&Endpoints::spotify(), &pkce, &redirect_uri(REDIRECT_PORT));
        assert!(url.starts_with("https://accounts.spotify.com/authorize?"));
        for part in [
            "client_id=250bd9732a644668911a74bdd329e66e",
            "response_type=code",
            "code_challenge_method=S256",
            "state=st4te",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A43819%2Fcallback",
            "code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            "scope=user-read-private+user-library-read",
        ] {
            assert!(url.contains(part), "{part} in {url}");
        }
        assert!(!url.contains("dBjftJeZ"), "the verifier never leaves Aria");
    }
}
