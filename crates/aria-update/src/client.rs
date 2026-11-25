//! A small HTTP client for GitHub: HTTPS through rustls with the system's certificates, a plain
//! `Aria` User-Agent (no version), and a cap on every body it reads.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig};

use crate::{Asset, Endpoints, Release, UpdateError, parse_release};

/// The largest answer about a release Aria reads.
const ANSWER_CAP: u64 = 1024 * 1024;
/// How long a question to GitHub may take.
const ASK_TIMEOUT: Duration = Duration::from_secs(30);
/// How long any read may wait for its next bytes. A slow download that keeps coming is never
/// cut off.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How much of a download is read at a time.
const CHUNK: usize = 64 * 1024;

/// Reads GitHub's answers and downloads release files.
pub struct Client {
    agent: ureq::Agent,
    endpoints: Endpoints,
}

impl Client {
    pub fn new(endpoints: Endpoints) -> Client {
        Client::build(endpoints, STALL_TIMEOUT)
    }

    /// A client whose reads may each wait up to `stall`.
    fn build(endpoints: Endpoints, stall: Duration) -> Client {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // No version: GitHub needs a User-Agent, not to know which Aria asks.
            .user_agent("Aria")
            .timeout_connect(Some(ASK_TIMEOUT))
            .timeout_recv_response(Some(ASK_TIMEOUT))
            .timeout_per_read(Some(stall))
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build();
        Client {
            agent: config.into(),
            endpoints,
        }
    }

    /// The latest full release.
    pub fn latest(&self) -> Result<Release, UpdateError> {
        let mut response = self.get(&self.endpoints.latest, Some("application/vnd.github+json"))?;
        let body = response
            .body_mut()
            .with_config()
            .limit(ANSWER_CAP)
            .read_to_string()
            .map_err(|e| transport(&e))?;
        parse_release(&body, &self.endpoints)
    }

    /// A small text file of a release, at most `cap` bytes.
    pub fn text(&self, asset: &Asset, cap: u64) -> Result<String, UpdateError> {
        self.check_url(asset)?;
        let mut response = self.get(&asset.url, None)?;
        response
            .body_mut()
            .with_config()
            .limit(cap)
            .read_to_string()
            .map_err(|e| transport(&e))
    }

    /// Downloads `asset` to `dest`, reporting `(done, total)` bytes as it goes. Refuses a file
    /// over `cap` before asking, and removes `dest` unless exactly `asset.size` bytes arrived.
    pub fn download(
        &self,
        asset: &Asset,
        dest: &Path,
        cap: u64,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), UpdateError> {
        if asset.size > cap {
            return Err(UpdateError::TooBig);
        }
        self.check_url(asset)?;
        let mut response = self.get(&asset.url, None)?;
        let result = (|| {
            let mut file = File::create(dest).map_err(|e| UpdateError::Disk(e.kind()))?;
            // One byte over the expected size is enough to know it is wrong.
            let mut reader = response
                .body_mut()
                .with_config()
                .limit(asset.size.saturating_add(1))
                .reader();
            let mut chunk = vec![0u8; CHUNK];
            let mut done = 0u64;
            loop {
                let n = match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) if done >= asset.size => return Err(UpdateError::Answer),
                    Err(_) => return Err(UpdateError::Offline),
                };
                file.write_all(&chunk[..n])
                    .map_err(|e| UpdateError::Disk(e.kind()))?;
                done += n as u64;
                if done > asset.size {
                    return Err(UpdateError::Answer);
                }
                progress(done, asset.size);
            }
            if done != asset.size {
                return Err(UpdateError::Answer);
            }
            file.sync_all().map_err(|e| UpdateError::Disk(e.kind()))
        })();
        if result.is_err() {
            // Nothing half-downloaded is left behind; it may not exist at all.
            let _ = fs::remove_file(dest);
        }
        result
    }

    fn check_url(&self, asset: &Asset) -> Result<(), UpdateError> {
        if asset.url.starts_with(&self.endpoints.download_prefix) {
            Ok(())
        } else {
            Err(UpdateError::Answer)
        }
    }

    fn get(
        &self,
        url: &str,
        accept: Option<&str>,
    ) -> Result<ureq::http::Response<ureq::Body>, UpdateError> {
        let mut request = self.agent.get(url);
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }
        let response = request.call().map_err(|e| transport(&e))?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(UpdateError::Answer)
        }
    }
}

/// A failed request, by its kind only.
fn transport(error: &ureq::Error) -> UpdateError {
    match error {
        ureq::Error::BodyExceedsLimit(_) => UpdateError::TooBig,
        ureq::Error::Json(_) | ureq::Error::BadUri(_) | ureq::Error::Http(_) => UpdateError::Answer,
        _ => UpdateError::Offline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_spotify::fake::{FakeSpotify, Reply};

    const LATEST: &str = "/repos/Hixenov/Aria/releases/latest";
    const SETUP: &str = "/Hixenov/Aria/releases/download/v0.3.6/Aria-0.3.6-setup.exe";
    const FIXTURE: &str = include_str!("../tests/fixtures/release.json");

    fn client(fake: &FakeSpotify) -> Client {
        Client::new(Endpoints {
            latest: format!("{}{LATEST}", fake.base()),
            download_prefix: format!("{}/Hixenov/Aria/releases/download/", fake.base()),
        })
    }

    fn setup_asset(fake: &FakeSpotify, size: u64) -> Asset {
        Asset {
            name: "Aria-0.3.6-setup.exe".to_owned(),
            size,
            url: format!("{}{SETUP}", fake.base()),
        }
    }

    fn header<'a>(seen: &'a aria_spotify::fake::Seen, name: &str) -> Option<&'a str> {
        seen.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn the_latest_release_is_asked_for_and_read() {
        let fake = FakeSpotify::start();
        fake.reply("GET", LATEST, Reply::json(200, FIXTURE));
        let release = client(&fake).latest().unwrap();
        assert_eq!(release.version, semver::Version::new(0, 3, 6));
        assert!(release.installer.is_some());
    }

    #[test]
    fn the_client_sends_a_bare_user_agent() {
        let fake = FakeSpotify::start();
        fake.reply("GET", LATEST, Reply::json(200, FIXTURE));
        client(&fake).latest().unwrap();
        let seen = fake.requests();
        assert_eq!(seen.len(), 1);
        assert_eq!(header(&seen[0], "user-agent"), Some("Aria"));
        assert_eq!(
            header(&seen[0], "accept"),
            Some("application/vnd.github+json")
        );
        assert_eq!(seen[0].authorization, None);
    }

    #[test]
    fn an_error_status_is_an_odd_answer() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            LATEST,
            Reply::json(403, r#"{"message":"rate limited"}"#),
        );
        assert_eq!(client(&fake).latest(), Err(UpdateError::Answer));
    }

    #[test]
    fn an_unreachable_github_is_offline() {
        let fake = FakeSpotify::start();
        fake.set_offline(true);
        assert_eq!(client(&fake).latest(), Err(UpdateError::Offline));
    }

    #[test]
    fn a_text_over_its_cap_is_too_big() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            SETUP,
            Reply::bytes(200, "text/plain", vec![b'a'; 100]),
        );
        let asset = setup_asset(&fake, 100);
        assert_eq!(client(&fake).text(&asset, 10), Err(UpdateError::TooBig));
        assert_eq!(client(&fake).text(&asset, 100).map(|t| t.len()), Ok(100));
    }

    #[test]
    fn a_download_too_big_is_refused_before_asking() {
        let fake = FakeSpotify::start();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.part");
        let result = client(&fake).download(&setup_asset(&fake, 11), &dest, 10, &mut |_, _| {});
        assert_eq!(result, Err(UpdateError::TooBig));
        assert!(fake.requests().is_empty());
        assert!(!dest.exists());
    }

    #[test]
    fn a_download_is_written_and_reported() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            SETUP,
            Reply::bytes(200, "application/octet-stream", b"hello world".to_vec()),
        );
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.part");
        let mut last = (0, 0);
        client(&fake)
            .download(&setup_asset(&fake, 11), &dest, 100, &mut |done, total| {
                last = (done, total)
            })
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello world");
        assert_eq!(last, (11, 11));
    }

    #[test]
    fn a_download_of_the_wrong_size_is_removed() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            SETUP,
            Reply::bytes(200, "application/octet-stream", b"hello".to_vec()),
        );
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.part");
        let result = client(&fake).download(&setup_asset(&fake, 11), &dest, 100, &mut |_, _| {});
        assert_eq!(result, Err(UpdateError::Answer));
        assert!(!dest.exists());
    }

    /// A server that sends `body` in `parts`, `gap` apart, to one request.
    fn trickle(parts: &'static [&'static [u8]], gap: std::time::Duration) -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let total: usize = parts.iter().map(|part| part.len()).sum();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
            );
            for part in parts {
                std::thread::sleep(gap);
                let _ = stream.write_all(part);
                let _ = stream.flush();
            }
        });
        base
    }

    #[test]
    fn a_slow_but_steady_download_is_not_cut_off() {
        // Each part arrives well within the stall limit; all of them together take longer.
        let base = trickle(
            &[b"hel", b"lo ", b"wor", b"ld"],
            std::time::Duration::from_millis(150),
        );
        let client = Client::build(
            Endpoints {
                latest: format!("{base}{LATEST}"),
                download_prefix: format!("{base}/"),
            },
            std::time::Duration::from_millis(400),
        );
        let asset = Asset {
            name: "Aria-0.3.6-setup.exe".to_owned(),
            size: 11,
            url: format!("{base}{SETUP}"),
        };
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.part");
        client.download(&asset, &dest, 100, &mut |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello world");
    }

    #[test]
    fn a_download_from_elsewhere_is_refused() {
        let fake = FakeSpotify::start();
        let dir = tempfile::tempdir().unwrap();
        let asset = Asset {
            url: "https://evil.example/Aria-0.3.6-setup.exe".to_owned(),
            ..setup_asset(&fake, 11)
        };
        let result = client(&fake).download(&asset, &dir.path().join("f"), 100, &mut |_, _| {});
        assert_eq!(result, Err(UpdateError::Answer));
    }
}
