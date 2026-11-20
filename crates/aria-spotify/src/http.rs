//! The HTTP client: blocking `ureq` over rustls with the system's certificate store.

use std::fmt;
use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig};

use crate::SpotifyError;

const TIMEOUT: Duration = Duration::from_secs(10);

/// One HTTP agent for all of Aria's Spotify requests.
#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub fn new() -> Http {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .user_agent(concat!("Aria/", env!("CARGO_PKG_VERSION")))
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build();
        Http {
            agent: config.into(),
        }
    }

    pub(crate) fn agent(&self) -> &ureq::Agent {
        &self.agent
    }
}

impl Default for Http {
    fn default() -> Http {
        Http::new()
    }
}

impl fmt::Debug for Http {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Http")
    }
}

/// A response read to the end.
pub(crate) struct Answer {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: String,
}

/// Reads a response, or names a transport failure by its kind only (never the request).
pub(crate) fn read(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Answer, SpotifyError> {
    let mut response = result.map_err(|e| transport(&e))?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = if status == 204 {
        String::new()
    } else {
        response
            .body_mut()
            .read_to_string()
            .map_err(|e| transport(&e))?
    };
    Ok(Answer {
        status,
        retry_after,
        body,
    })
}

/// A transport failure, named by its kind only: never the URL, a header or the body.
pub(crate) fn transport(error: &ureq::Error) -> SpotifyError {
    let kind = match error {
        ureq::Error::Timeout(_) => "timed out".to_owned(),
        ureq::Error::Io(e) => e.kind().to_string(),
        ureq::Error::HostNotFound => "host not found".to_owned(),
        ureq::Error::ConnectionFailed => "connection failed".to_owned(),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::Pem(_) => "TLS".to_owned(),
        ureq::Error::BodyExceedsLimit(_) => {
            return SpotifyError::BadAnswer("answer too large".to_owned());
        }
        _ => "request failed".to_owned(),
    };
    SpotifyError::Network(kind)
}
