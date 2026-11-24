//! The token exchange and refresh at Spotify's accounts service (PKCE: no client secret).

use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::http::{self, Http};
use crate::{CLIENT_ID, Endpoints, Pkce, Secret, SpotifyError};

/// The access token is treated as expired this long before Spotify says it is.
const EARLY: Duration = Duration::from_secs(60);

/// The tokens of a signed-in session. `Debug` prints `***` for both tokens.
#[derive(Clone, Debug)]
pub struct Tokens {
    pub access: Secret,
    pub refresh: Secret,
    pub expires_at: Instant,
    /// The scopes Spotify granted, space-separated, from the last answer that named them.
    pub scopes: Option<String>,
}

impl Tokens {
    /// Whether Spotify granted `streaming`, which the Web Playback SDK needs.
    pub fn can_stream(&self) -> bool {
        self.scopes
            .as_deref()
            .is_some_and(|s| s.split_whitespace().any(|scope| scope == "streaming"))
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
    error_description: Option<String>,
}

/// Exchanges the authorization code for tokens, proving the sign-in with the PKCE verifier.
pub fn exchange_code(
    http: &Http,
    endpoints: &Endpoints,
    code: &Secret,
    pkce: &Pkce,
    redirect_uri: &str,
) -> Result<Tokens, SpotifyError> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.expose()),
        ("redirect_uri", redirect_uri),
        ("client_id", CLIENT_ID),
        ("code_verifier", pkce.verifier.expose()),
    ];
    request(http, endpoints, &form, None)
}

/// A new access token. Spotify may answer without a new refresh token: the given one is kept.
/// `invalid_grant` (the refresh token has ended or was revoked) is [`SpotifyError::SessionEnded`].
pub fn refresh(
    http: &Http,
    endpoints: &Endpoints,
    refresh: &Secret,
) -> Result<Tokens, SpotifyError> {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh.expose()),
        ("client_id", CLIENT_ID),
    ];
    request(http, endpoints, &form, Some(refresh))
}

fn request(
    http: &Http,
    endpoints: &Endpoints,
    form: &[(&str, &str)],
    old_refresh: Option<&Secret>,
) -> Result<Tokens, SpotifyError> {
    let url = format!("{}/api/token", endpoints.accounts);
    let answer = http::read(http.agent().post(&url).send_form(form.iter().copied()))?;
    match answer.status {
        200 => {
            let parsed: TokenAnswer = serde_json::from_str(&answer.body)
                .map_err(|_| SpotifyError::BadAnswer("token answer".to_owned()))?;
            let refresh = match (parsed.refresh_token, old_refresh) {
                (Some(new), _) => Secret::new(new),
                (None, Some(old)) => old.clone(),
                (None, None) => {
                    return Err(SpotifyError::BadAnswer("no refresh token".to_owned()));
                }
            };
            let lifetime = Duration::from_secs(parsed.expires_in).saturating_sub(EARLY);
            Ok(Tokens {
                access: Secret::new(parsed.access_token),
                refresh,
                expires_at: Instant::now() + lifetime,
                scopes: parsed.scope,
            })
        }
        429 => Err(SpotifyError::RateLimited {
            retry_after: answer.retry_after.unwrap_or(Duration::from_secs(30)),
        }),
        status => {
            let error: Option<TokenError> = serde_json::from_str(&answer.body).ok();
            match error {
                Some(e) if e.error == "invalid_grant" && old_refresh.is_some() => {
                    Err(SpotifyError::SessionEnded)
                }
                Some(e) => Err(SpotifyError::Api {
                    status,
                    message: e.error_description.unwrap_or(e.error),
                }),
                None => Err(SpotifyError::Api {
                    status,
                    message: String::new(),
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeSpotify, Reply};
    use crate::redirect_uri;

    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

    fn pkce() -> Pkce {
        Pkce::from_parts(VERIFIER, "st4te")
    }

    fn form_of(body: &str) -> Vec<(String, String)> {
        form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect()
    }

    #[test]
    fn the_code_is_exchanged_with_the_verifier_and_no_secret() {
        let fake = FakeSpotify::start();
        fake.reply("POST", "/api/token", Reply::fixture(200, "token"));
        let tokens = exchange_code(
            &Http::new(),
            &fake.endpoints(),
            &Secret::new("the-code".to_owned()),
            &pkce(),
            &redirect_uri(43819),
        )
        .unwrap();
        assert_eq!(tokens.access.expose(), "fixture-access-token-1");
        assert_eq!(tokens.refresh.expose(), "fixture-refresh-token-1");
        let seen = fake.requests();
        assert_eq!(seen.len(), 1);
        let form = form_of(&seen[0].body);
        for (key, value) in [
            ("grant_type", "authorization_code"),
            ("code", "the-code"),
            ("client_id", "250bd9732a644668911a74bdd329e66e"),
            ("code_verifier", VERIFIER),
            ("redirect_uri", "http://127.0.0.1:43819/callback"),
        ] {
            assert!(
                form.contains(&(key.to_owned(), value.to_owned())),
                "{key} in {form:?}"
            );
        }
        assert!(!form.iter().any(|(k, _)| k == "client_secret"));
        assert_eq!(seen[0].authorization, None, "PKCE sends no Basic header");
    }

    #[test]
    fn a_refresh_without_a_new_refresh_token_keeps_the_old() {
        let fake = FakeSpotify::start();
        fake.reply("POST", "/api/token", Reply::fixture(200, "token-refreshed"));
        let old = Secret::new("old-refresh".to_owned());
        let tokens = refresh(&Http::new(), &fake.endpoints(), &old).unwrap();
        assert_eq!(tokens.access.expose(), "fixture-access-token-2");
        assert_eq!(tokens.refresh, old);
        let form = form_of(&fake.requests()[0].body);
        assert!(form.contains(&("grant_type".to_owned(), "refresh_token".to_owned())));
        assert!(form.contains(&("refresh_token".to_owned(), "old-refresh".to_owned())));
    }

    #[test]
    fn granted_scopes_are_kept() {
        let fake = FakeSpotify::start();
        fake.reply(
            "POST",
            "/api/token",
            Reply::json(
                200,
                r#"{"access_token":"a","refresh_token":"r","expires_in":3600,"scope":"streaming user-read-private"}"#,
            ),
        );
        let tokens = exchange_code(
            &Http::new(),
            &fake.endpoints(),
            &Secret::new("c".to_owned()),
            &pkce(),
            &redirect_uri(43819),
        )
        .unwrap();
        assert!(tokens.can_stream());
    }

    #[test]
    fn no_streaming_scope_cannot_stream() {
        let mut tokens = Tokens {
            access: Secret::new("a".to_owned()),
            refresh: Secret::new("r".to_owned()),
            expires_at: Instant::now(),
            scopes: Some("user-read-private".to_owned()),
        };
        assert!(!tokens.can_stream());
        tokens.scopes = None;
        assert!(!tokens.can_stream());
        tokens.scopes = Some("streamingx user-read-private".to_owned());
        assert!(!tokens.can_stream());
    }

    #[test]
    fn invalid_grant_means_the_session_ended() {
        let fake = FakeSpotify::start();
        fake.reply(
            "POST",
            "/api/token",
            Reply::json(
                400,
                r#"{"error":"invalid_grant","error_description":"Refresh token revoked"}"#,
            ),
        );
        let old = Secret::new("old-refresh".to_owned());
        assert_eq!(
            refresh(&Http::new(), &fake.endpoints(), &old).err(),
            Some(SpotifyError::SessionEnded)
        );
    }

    #[test]
    fn the_access_token_expires_a_minute_early() {
        let fake = FakeSpotify::start();
        fake.reply("POST", "/api/token", Reply::fixture(200, "token"));
        let r = Secret::new("r".to_owned());
        let tokens = refresh(&Http::new(), &fake.endpoints(), &r).unwrap();
        let left = tokens.expires_at.saturating_duration_since(Instant::now());
        assert!(left <= Duration::from_secs(3540), "{left:?}");
        assert!(left >= Duration::from_secs(3538), "{left:?}");
    }

    #[test]
    fn nothing_secret_is_ever_printed() {
        let fake = FakeSpotify::start();
        fake.reply("POST", "/api/token", Reply::fixture(200, "token"));
        let http = Http::new();
        let code = Secret::new("the-code".to_owned());
        let endpoints = fake.endpoints();
        let tokens = exchange_code(&http, &endpoints, &code, &pkce(), &redirect_uri(1)).unwrap();
        fake.clear();
        fake.reply(
            "POST",
            "/api/token",
            Reply::json(
                400,
                r#"{"error":"invalid_grant","error_description":"Invalid authorization code"}"#,
            ),
        );
        let refused = exchange_code(&http, &endpoints, &code, &pkce(), &redirect_uri(1))
            .err()
            .unwrap();
        let closed = Endpoints {
            accounts: "http://127.0.0.1:9".to_owned(),
            api: "http://127.0.0.1:9/v1".to_owned(),
        };
        let offline = refresh(&http, &closed, &tokens.refresh).err().unwrap();
        assert!(matches!(offline, SpotifyError::Network(_)), "{offline:?}");
        let printed = format!(
            "{tokens:?} {:?} {refused:?} {refused} {offline:?} {offline} {http:?} {code:?}",
            pkce()
        );
        for secret in [
            "fixture-access-token-1",
            "fixture-refresh-token-1",
            "the-code",
            VERIFIER,
        ] {
            assert!(!printed.contains(secret), "{secret} in {printed}");
        }
    }
}
