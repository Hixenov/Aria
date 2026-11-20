//! The one-shot loopback listener that receives Spotify's redirect after sign-in.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Secret, SpotifyError};

const POLL: Duration = Duration::from_millis(50);
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_REQUEST: usize = 8 * 1024;

/// A listener on `127.0.0.1` for the sign-in redirect. It never listens on other interfaces.
pub struct Callback {
    listener: TcpListener,
    port: u16,
}

impl Callback {
    /// Binds `127.0.0.1:{port}` (`0` picks a free port, for tests).
    pub fn bind(port: u16) -> Result<Callback, SpotifyError> {
        let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| bind_error(e.kind()))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| SpotifyError::Network(e.kind().to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| SpotifyError::Network(e.kind().to_string()))?
            .port();
        Ok(Callback { listener, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Waits for `GET /callback` with this `state` and returns its authorization code. Requests
    /// with another state or path are answered and ignored, so a stray request cannot end the
    /// sign-in. Gives up at `timeout`, or as soon as `cancel` is set.
    pub fn wait(
        self,
        state: &str,
        timeout: Duration,
        cancel: &AtomicBool,
        done_page: &str,
        failed_page: &str,
    ) -> Result<Secret, SpotifyError> {
        let deadline = Instant::now() + timeout;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(SpotifyError::SignInCancelled);
            }
            if Instant::now() >= deadline {
                return Err(SpotifyError::SignInTimedOut);
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(result) = answer(stream, state, done_page, failed_page) {
                        return result;
                    }
                }
                Err(_) => thread::sleep(POLL),
            }
        }
    }
}

/// Why the port could not be had. Windows answers `PermissionDenied` for a port another program
/// holds alone or one in a range the system keeps for itself: busy, to the user, as well.
pub(crate) fn bind_error(kind: io::ErrorKind) -> SpotifyError {
    match kind {
        io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied => SpotifyError::PortBusy,
        kind => SpotifyError::Network(kind.to_string()),
    }
}

/// Answers one connection. `Some` ends the wait; `None` goes on waiting.
fn answer(
    mut stream: TcpStream,
    state: &str,
    done_page: &str,
    failed_page: &str,
) -> Option<Result<Secret, SpotifyError>> {
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(READ_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(READ_TIMEOUT)).is_err()
    {
        return None;
    }
    let head = read_head(&mut stream)?;
    let target = head
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" {
        respond(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "Not found.",
        );
        return None;
    }
    let (mut got_state, mut code, mut error) = (None, None, None);
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "state" => got_state = Some(value.into_owned()),
            "code" => code = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }
    if got_state.as_deref() != Some(state) {
        respond(
            &mut stream,
            "400 Bad Request",
            "text/html; charset=utf-8",
            failed_page,
        );
        return None;
    }
    if let Some(code) = code {
        respond(&mut stream, "200 OK", "text/html; charset=utf-8", done_page);
        return Some(Ok(Secret::new(code)));
    }
    respond(
        &mut stream,
        "200 OK",
        "text/html; charset=utf-8",
        failed_page,
    );
    match error.as_deref() {
        Some("access_denied") => Some(Err(SpotifyError::SignInDenied)),
        Some(other) => Some(Err(SpotifyError::Api {
            status: 400,
            message: other.to_owned(),
        })),
        None => Some(Err(SpotifyError::BadAnswer(
            "a callback without a code".to_owned(),
        ))),
    }
}

/// Reads the request head (up to the blank line), at most [`MAX_REQUEST`] bytes.
fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while buffer.len() < MAX_REQUEST && !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    }
    buffer.truncate(MAX_REQUEST);
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // The browser may already have gone; nothing to do about it.
    let _ = stream.write_all(reply.as_bytes());
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const PAGE: &str = "<p>Connected</p>";

    fn browse(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(stream, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    }

    fn browse_all(port: u16, targets: &[&str]) -> thread::JoinHandle<Vec<String>> {
        let targets: Vec<String> = targets.iter().map(|t| (*t).to_owned()).collect();
        thread::spawn(move || targets.iter().map(|t| browse(port, t)).collect())
    }

    fn wait(callback: Callback, timeout: Duration) -> Result<Secret, SpotifyError> {
        callback.wait("good", timeout, &AtomicBool::new(false), PAGE, "<p>no</p>")
    }

    #[test]
    fn the_code_comes_back_and_the_page_is_shown() {
        let callback = Callback::bind(0).unwrap();
        let browser = browse_all(callback.port(), &["/callback?code=c0de&state=good"]);
        let code = wait(callback, Duration::from_secs(5)).unwrap();
        assert_eq!(code.expose(), "c0de");
        let answers = browser.join().unwrap();
        assert!(answers[0].starts_with("HTTP/1.1 200"), "{}", answers[0]);
        assert!(answers[0].contains(PAGE));
        assert!(answers[0].contains("text/html; charset=utf-8"));
    }

    #[test]
    fn a_wrong_state_is_refused_and_waiting_goes_on() {
        let callback = Callback::bind(0).unwrap();
        let browser = browse_all(
            callback.port(),
            &[
                "/callback?code=evil&state=bad",
                "/callback?code=c0de&state=good",
            ],
        );
        let code = wait(callback, Duration::from_secs(5)).unwrap();
        assert_eq!(code.expose(), "c0de");
        let answers = browser.join().unwrap();
        assert!(answers[0].starts_with("HTTP/1.1 400"), "{}", answers[0]);
        assert!(answers[1].starts_with("HTTP/1.1 200"), "{}", answers[1]);
    }

    #[test]
    fn a_denied_sign_in_is_reported() {
        let callback = Callback::bind(0).unwrap();
        let browser = browse_all(
            callback.port(),
            &["/callback?error=access_denied&state=good"],
        );
        assert_eq!(
            wait(callback, Duration::from_secs(5)),
            Err(SpotifyError::SignInDenied)
        );
        browser.join().unwrap();
    }

    #[test]
    fn other_paths_are_not_found() {
        let callback = Callback::bind(0).unwrap();
        let browser = browse_all(
            callback.port(),
            &["/favicon.ico", "/callback?code=c0de&state=good"],
        );
        let code = wait(callback, Duration::from_secs(5)).unwrap();
        assert_eq!(code.expose(), "c0de");
        let answers = browser.join().unwrap();
        assert!(answers[0].starts_with("HTTP/1.1 404"), "{}", answers[0]);
    }

    #[test]
    fn it_gives_up_after_the_timeout() {
        let callback = Callback::bind(0).unwrap();
        let start = Instant::now();
        assert_eq!(
            wait(callback, Duration::from_millis(200)),
            Err(SpotifyError::SignInTimedOut)
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn cancelling_stops_it() {
        let callback = Callback::bind(0).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::SeqCst);
        });
        let start = Instant::now();
        assert_eq!(
            callback.wait("good", Duration::from_secs(10), &cancel, PAGE, ""),
            Err(SpotifyError::SignInCancelled)
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_refused_port_is_reported_as_busy() {
        assert_eq!(
            bind_error(std::io::ErrorKind::PermissionDenied),
            SpotifyError::PortBusy
        );
        assert_eq!(
            bind_error(std::io::ErrorKind::AddrInUse),
            SpotifyError::PortBusy
        );
    }

    #[test]
    fn a_denied_sign_in_shows_the_failure_page() {
        let callback = Callback::bind(0).unwrap();
        let browser = browse_all(
            callback.port(),
            &["/callback?error=access_denied&state=good"],
        );
        let result = callback.wait(
            "good",
            Duration::from_secs(5),
            &AtomicBool::new(false),
            PAGE,
            "<p>Not connected</p>",
        );
        assert_eq!(result, Err(SpotifyError::SignInDenied));
        let answers = browser.join().unwrap();
        assert!(
            answers[0].contains("<p>Not connected</p>"),
            "{}",
            answers[0]
        );
    }

    #[test]
    fn a_taken_port_is_reported() {
        let taken = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        assert_eq!(Callback::bind(port).err(), Some(SpotifyError::PortBusy));
    }
}
