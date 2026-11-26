//! A small HTTP client for GitHub: HTTPS through rustls with the system's certificates, a plain
//! `Aria` User-Agent (no version), and a cap on every body it reads.
//!
//! It reads GitHub's web pages and release downloads, never its API: the API allows 60 questions
//! an hour per IP address, which a shared address uses up for everyone behind it.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use ureq::http::{StatusCode, header};
use ureq::tls::{RootCerts, TlsConfig};

use crate::{Asset, Endpoints, Release, UpdateError, release_for_tag};

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
            // Every hop, redirects included, so no file arrives in plain text.
            .https_only(https_only(&endpoints))
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

    /// The latest full release: where GitHub's "latest release" page redirects to. The redirect
    /// is read, not followed, and must lead to a release page of Aria's own repository.
    pub fn latest(&self) -> Result<Release, UpdateError> {
        let response = self
            .agent
            .get(&self.endpoints.latest)
            .config()
            .max_redirects(0)
            .build()
            .call()
            .map_err(|e| transport(&e))?;
        let status = response.status();
        if !status.is_redirection() {
            return Err(match refusal(status) {
                UpdateError::Missing => UpdateError::Answer,
                other => other,
            });
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(UpdateError::Answer)?;
        let page = absolute(location, &self.endpoints.latest).ok_or(UpdateError::Answer)?;
        let tag = page
            .strip_prefix(&self.endpoints.tag_prefix)
            .ok_or(UpdateError::Answer)?;
        release_for_tag(tag, page.clone(), &self.endpoints)
    }

    /// A small text file of a release, at most `cap` bytes. `Missing` if it is not there.
    pub fn text(&self, asset: &Asset, cap: u64) -> Result<String, UpdateError> {
        self.check_url(asset)?;
        let mut response = self.get(&asset.url)?;
        response
            .body_mut()
            .with_config()
            .limit(cap)
            .read_to_string()
            .map_err(|e| transport(&e))
    }

    /// Downloads `asset` to `dest`, reporting `(done, total)` bytes as it goes; `total` is the
    /// size GitHub gives, or 0 when it gives none. Refuses a file over `cap` before reading
    /// any of it, never reads more than `cap`, and removes `dest` unless the whole file arrived:
    /// exactly the size given, when one was.
    pub fn download(
        &self,
        asset: &Asset,
        dest: &Path,
        cap: u64,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), UpdateError> {
        self.check_url(asset)?;
        let mut response = self.get(&asset.url)?;
        let size = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        if size.is_some_and(|size| size > cap) {
            return Err(UpdateError::TooBig);
        }
        let most = size.unwrap_or(cap);
        // More than this means a wrong size given, or a file over the cap.
        let over = if size.is_some() {
            UpdateError::Answer
        } else {
            UpdateError::TooBig
        };
        let result = (|| {
            let mut file = File::create(dest).map_err(|e| UpdateError::Disk(e.kind()))?;
            // One byte over is enough to know it is wrong.
            let mut reader = response
                .body_mut()
                .with_config()
                .limit(most.saturating_add(1))
                .reader();
            let mut chunk = vec![0u8; CHUNK];
            let mut done = 0u64;
            loop {
                let n = match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) if done >= most => return Err(over.clone()),
                    Err(_) => return Err(UpdateError::Offline),
                };
                file.write_all(&chunk[..n])
                    .map_err(|e| UpdateError::Disk(e.kind()))?;
                done += n as u64;
                if done > most {
                    return Err(over.clone());
                }
                progress(done, size.unwrap_or(0));
            }
            if size.is_some_and(|size| done != size) {
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

    fn get(&self, url: &str) -> Result<ureq::http::Response<ureq::Body>, UpdateError> {
        let response = self.agent.get(url).call().map_err(|e| transport(&e))?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(refusal(response.status()))
        }
    }
}

/// Whether every request, and every redirect it follows, must be HTTPS: whenever all of Aria's
/// addresses are. (Only the local fakes in tests are not.)
fn https_only(endpoints: &Endpoints) -> bool {
    [
        &endpoints.latest,
        &endpoints.tag_prefix,
        &endpoints.download_prefix,
    ]
    .iter()
    .all(|url| url.starts_with("https://"))
}

/// What a status other than success means.
fn refusal(status: StatusCode) -> UpdateError {
    match status {
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => UpdateError::Busy,
        StatusCode::NOT_FOUND => UpdateError::Missing,
        _ => UpdateError::Answer,
    }
}

/// `location` as a full URL: as it is, or, when it is a path (`/…`), on `base`'s scheme and
/// host. Anything else, a scheme-relative `//host/…` included, is not taken.
fn absolute(location: &str, base: &str) -> Option<String> {
    if location.starts_with("https://") || location.starts_with("http://") {
        return Some(location.to_owned());
    }
    if !location.starts_with('/') || location.starts_with("//") {
        return None;
    }
    let host_start = base.find("://")? + 3;
    let origin_end = base[host_start..]
        .find('/')
        .map_or(base.len(), |at| host_start + at);
    Some(format!("{}{location}", &base[..origin_end]))
}

/// A failed request, by its kind only.
fn transport(error: &ureq::Error) -> UpdateError {
    match error {
        ureq::Error::BodyExceedsLimit(_) => UpdateError::TooBig,
        ureq::Error::Json(_)
        | ureq::Error::BadUri(_)
        | ureq::Error::Http(_)
        | ureq::Error::RequireHttpsOnly(_) => UpdateError::Answer,
        _ => UpdateError::Offline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_spotify::fake::{FakeSpotify, Reply};

    const LATEST: &str = "/Hixenov/Aria/releases/latest";
    const TAG: &str = "/Hixenov/Aria/releases/tag/";
    const SETUP: &str = "/Hixenov/Aria/releases/download/v0.3.6/Aria-0.3.6-setup.exe";
    const SUMS: &str = "/Hixenov/Aria/releases/download/v0.3.6/SHA256SUMS.txt";

    fn endpoints(base: &str) -> Endpoints {
        Endpoints {
            latest: format!("{base}{LATEST}"),
            tag_prefix: format!("{base}{TAG}"),
            download_prefix: format!("{base}/Hixenov/Aria/releases/download/"),
        }
    }

    fn client(fake: &FakeSpotify) -> Client {
        Client::new(endpoints(&fake.base()))
    }

    fn redirect(location: &str) -> Reply {
        Reply::empty(302).header("Location", location)
    }

    fn asset(fake: &FakeSpotify, path: &str) -> Asset {
        Asset {
            name: path.rsplit('/').next().unwrap_or_default().to_owned(),
            url: format!("{}{path}", fake.base()),
        }
    }

    fn header<'a>(seen: &'a aria_spotify::fake::Seen, name: &str) -> Option<&'a str> {
        seen.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn github_is_only_ever_reached_over_https() {
        // Every hop, redirects included: a file sent in plain text could be swapped together
        // with its checksum.
        assert!(https_only(&Endpoints::github()));
        // The local fakes the tests use are plain HTTP.
        assert!(!https_only(&endpoints("http://127.0.0.1:1")));
    }

    #[test]
    fn a_plain_http_hop_is_an_answer_that_makes_no_sense() {
        let error = ureq::Error::RequireHttpsOnly("http://x".to_owned());
        assert_eq!(transport(&error), UpdateError::Answer);
    }

    #[test]
    fn the_latest_release_is_read_from_the_redirect() {
        let fake = FakeSpotify::start();
        let page = format!("{}{TAG}v0.3.6", fake.base());
        fake.reply("GET", LATEST, redirect(&page));
        let release = client(&fake).latest().unwrap();
        assert_eq!(release.version, semver::Version::new(0, 3, 6));
        assert_eq!(release.page, page);
        assert_eq!(release.installer, asset(&fake, SETUP));
        assert_eq!(release.sums, asset(&fake, SUMS));
        // The redirect is read, not followed.
        let seen = fake.requests();
        assert_eq!(seen.len(), 1);
        assert_eq!(header(&seen[0], "user-agent"), Some("Aria"));
        assert_eq!(seen[0].authorization, None);
    }

    #[test]
    fn a_relative_location_is_read_against_github() {
        let fake = FakeSpotify::start();
        fake.reply("GET", LATEST, redirect(&format!("{TAG}v0.3.6")));
        let release = client(&fake).latest().unwrap();
        assert_eq!(release.page, format!("{}{TAG}v0.3.6", fake.base()));
        assert_eq!(release.installer, asset(&fake, SETUP));
    }

    #[test]
    fn a_location_elsewhere_is_refused() {
        for location in [
            "https://evil.example/Hixenov/Aria/releases/tag/v0.3.6",
            "//evil.example/Hixenov/Aria/releases/tag/v0.3.6",
            "{base}/Other/Repo/releases/tag/v0.3.6",
            "{base}/Hixenov/Aria/releases",
            "releases/tag/v0.3.6",
        ] {
            let fake = FakeSpotify::start();
            let location = location.replace("{base}", &fake.base());
            fake.reply("GET", LATEST, redirect(&location));
            assert_eq!(
                client(&fake).latest(),
                Err(UpdateError::Answer),
                "{location}"
            );
            // Never followed.
            assert_eq!(fake.requests().len(), 1);
        }
    }

    #[test]
    fn no_redirect_is_an_answer_that_makes_no_sense() {
        for reply in [
            Reply::bytes(200, "text/html", b"<html>".to_vec()),
            Reply::empty(302),
            Reply::empty(500),
        ] {
            let fake = FakeSpotify::start();
            fake.reply("GET", LATEST, reply);
            assert_eq!(client(&fake).latest(), Err(UpdateError::Answer));
        }
    }

    #[test]
    fn an_odd_tag_is_not_a_release() {
        let fake = FakeSpotify::start();
        let page = format!("{}{TAG}v0.3.6-beta", fake.base());
        fake.reply("GET", LATEST, redirect(&page));
        assert_eq!(client(&fake).latest(), Err(UpdateError::NotARelease));
    }

    #[test]
    fn a_refusal_is_busy() {
        for status in [403, 429] {
            let fake = FakeSpotify::start();
            fake.reply(
                "GET",
                LATEST,
                Reply::bytes(status, "text/plain", b"slow down".to_vec()),
            );
            assert_eq!(client(&fake).latest(), Err(UpdateError::Busy), "{status}");
        }
        let fake = FakeSpotify::start();
        fake.reply("GET", SUMS, Reply::empty(429));
        assert_eq!(
            client(&fake).text(&asset(&fake, SUMS), 100),
            Err(UpdateError::Busy)
        );
    }

    #[test]
    fn an_unreachable_github_is_offline() {
        let fake = FakeSpotify::start();
        fake.set_offline(true);
        assert_eq!(client(&fake).latest(), Err(UpdateError::Offline));
    }

    #[test]
    fn missing_sums_are_missing() {
        let fake = FakeSpotify::start();
        fake.reply("GET", SUMS, Reply::empty(404));
        assert_eq!(
            client(&fake).text(&asset(&fake, SUMS), 100),
            Err(UpdateError::Missing)
        );
        let fake = FakeSpotify::start();
        fake.reply("GET", SUMS, Reply::empty(500));
        assert_eq!(
            client(&fake).text(&asset(&fake, SUMS), 100),
            Err(UpdateError::Answer)
        );
    }

    #[test]
    fn a_text_over_its_cap_is_too_big() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            SUMS,
            Reply::bytes(200, "text/plain", vec![b'a'; 100]),
        );
        let sums = asset(&fake, SUMS);
        assert_eq!(client(&fake).text(&sums, 10), Err(UpdateError::TooBig));
        assert_eq!(client(&fake).text(&sums, 100).map(|t| t.len()), Ok(100));
    }

    #[test]
    fn a_text_from_elsewhere_is_refused() {
        let fake = FakeSpotify::start();
        let elsewhere = Asset {
            name: "SHA256SUMS.txt".to_owned(),
            url: "https://evil.example/SHA256SUMS.txt".to_owned(),
        };
        assert_eq!(
            client(&fake).text(&elsewhere, 100),
            Err(UpdateError::Answer)
        );
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn a_download_takes_its_size_from_the_answer() {
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
            .download(&asset(&fake, SETUP), &dest, 100, &mut |done, total| {
                last = (done, total)
            })
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello world");
        assert_eq!(last, (11, 11));
    }

    #[test]
    fn a_download_over_the_cap_is_refused_before_reading() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            SETUP,
            Reply::bytes(200, "application/octet-stream", b"hello world".to_vec()),
        );
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.part");
        let mut reported = false;
        let result = client(&fake).download(&asset(&fake, SETUP), &dest, 10, &mut |_, _| {
            reported = true;
        });
        assert_eq!(result, Err(UpdateError::TooBig));
        assert!(!reported);
        assert!(!dest.exists());
    }

    #[test]
    fn a_refused_download_is_busy() {
        let fake = FakeSpotify::start();
        fake.reply("GET", SETUP, Reply::empty(429));
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.part");
        let result = client(&fake).download(&asset(&fake, SETUP), &dest, 100, &mut |_, _| {});
        assert_eq!(result, Err(UpdateError::Busy));
        assert!(!dest.exists());
    }

    /// A server that answers one request with `head` (status line and headers, without the
    /// blank line) and then `parts` of the body, `gap` apart, closing the connection at the end.
    fn trickle(
        head: &'static str,
        parts: &'static [&'static [u8]],
        gap: std::time::Duration,
    ) -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
            }
            let _ = write!(stream, "{head}\r\nConnection: close\r\n\r\n");
            for part in parts {
                std::thread::sleep(gap);
                let _ = stream.write_all(part);
                let _ = stream.flush();
            }
        });
        base
    }

    fn trickle_client(base: &str, stall: std::time::Duration) -> (Client, Asset) {
        let client = Client::build(endpoints(base), stall);
        let asset = Asset {
            name: "Aria-0.3.6-setup.exe".to_owned(),
            url: format!("{base}{SETUP}"),
        };
        (client, asset)
    }

    #[test]
    fn a_short_download_is_not_kept() {
        let base = trickle(
            "HTTP/1.1 200 OK\r\nContent-Length: 11",
            &[b"hello"],
            std::time::Duration::ZERO,
        );
        let (client, asset) = trickle_client(&base, std::time::Duration::from_secs(5));
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.part");
        let result = client.download(&asset, &dest, 100, &mut |_, _| {});
        assert!(
            matches!(result, Err(UpdateError::Answer | UpdateError::Offline)),
            "{result:?}"
        );
        assert!(!dest.exists());
    }

    #[test]
    fn a_download_without_a_length_stops_at_the_cap() {
        let base = trickle(
            "HTTP/1.1 200 OK",
            &[b"hello world"],
            std::time::Duration::ZERO,
        );
        let (client, asset) = trickle_client(&base, std::time::Duration::from_secs(5));
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.part");
        let result = client.download(&asset, &dest, 5, &mut |_, _| {});
        assert_eq!(result, Err(UpdateError::TooBig));
        assert!(!dest.exists());
    }

    #[test]
    fn a_download_without_a_length_within_the_cap_is_kept() {
        let base = trickle(
            "HTTP/1.1 200 OK",
            &[b"hello ", b"world"],
            std::time::Duration::ZERO,
        );
        let (client, asset) = trickle_client(&base, std::time::Duration::from_secs(5));
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.part");
        let mut last = (0, 1);
        client
            .download(&asset, &dest, 100, &mut |done, total| last = (done, total))
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello world");
        // No size was given: the total is unknown.
        assert_eq!(last, (11, 0));
    }

    #[test]
    fn a_slow_but_steady_download_is_not_cut_off() {
        // Each part arrives well within the stall limit; all of them together take longer.
        let base = trickle(
            "HTTP/1.1 200 OK\r\nContent-Length: 11",
            &[b"hel", b"lo ", b"wor", b"ld"],
            std::time::Duration::from_millis(150),
        );
        let (client, asset) = trickle_client(&base, std::time::Duration::from_millis(400));
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.part");
        client.download(&asset, &dest, 100, &mut |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello world");
    }

    #[test]
    fn a_download_from_elsewhere_is_refused() {
        let fake = FakeSpotify::start();
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = Asset {
            url: "https://evil.example/Aria-0.3.6-setup.exe".to_owned(),
            ..asset(&fake, SETUP)
        };
        let result = client(&fake).download(&elsewhere, &dir.path().join("f"), 100, &mut |_, _| {});
        assert_eq!(result, Err(UpdateError::Answer));
        assert!(fake.requests().is_empty());
    }
}
