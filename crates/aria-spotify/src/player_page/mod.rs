//! The loopback server for Aria's Spotify player page.
//!
//! The page runs Spotify's Web Playback SDK in Aria's own browser window. Everything is under
//! `/p/<key>/`, where the key is new for each launch. The key is on the browser's command line,
//! which other local users can read, so it only loads the page: the page then takes a session
//! once (`POST session`), keeps it in memory, and sends it with every request that matters. No
//! token, key or session is ever logged.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use crossbeam_channel::{Sender, bounded};
use serde::Deserialize;

use crate::callback::bind_error;
use crate::{Secret, SpotifyError};

const PAGE: &str = include_str!("page.html");
const SCRIPT: &str = include_str!("player.js");
const CSP: &str = "script-src 'self' https://sdk.scdn.co";
/// A request's head and body together, at most.
const MAX_REQUEST: usize = 8 * 1024;
/// A request's body, at most.
const MAX_BODY: usize = 4 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
/// How long `wait` is held before it answers `{}`.
const WAIT_FOR: Duration = Duration::from_secs(20);
/// How long a token request waits for the Spotify thread.
const TOKEN_WAIT: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 8;
const POLL: Duration = Duration::from_millis(50);
const THREAD_NAME: &str = "aria-spotify-page";

/// What went wrong in the page, as the SDK names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageError {
    Initialization,
    Authentication,
    Account,
    Playback,
}

/// What the page tells Aria.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PageEvent {
    Ready {
        device_id: String,
    },
    NotReady {
        device_id: String,
    },
    Error {
        kind: PageError,
        #[serde(default)]
        message: String,
    },
    AutoplayFailed,
    /// The page is still there; also sent for each token or close request it makes.
    Alive,
}

/// What the server hands the Spotify thread.
#[derive(Debug)]
pub enum PageMsg {
    /// The page wants an access token; `None` when none can be had.
    Token(Sender<Option<Secret>>),
    Event(PageEvent),
}

struct Shared {
    key: String,
    port: u16,
    strings: String,
    session: Mutex<Option<String>>,
    close: AtomicBool,
    stop: AtomicBool,
    open: AtomicUsize,
    to_service: Sender<PageMsg>,
}

/// A running page server on `127.0.0.1`. Dropping it stops it.
pub struct PageServer {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl PageServer {
    /// Starts the server on a free loopback port; the page's texts are `strings_json`.
    pub fn start(
        strings_json: String,
        to_service: Sender<PageMsg>,
    ) -> Result<PageServer, SpotifyError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| bind_error(e.kind()))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| SpotifyError::Network(e.kind().to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| SpotifyError::Network(e.kind().to_string()))?
            .port();
        let shared = Arc::new(Shared {
            key: random_text()?,
            port,
            strings: strings_json,
            session: Mutex::new(None),
            close: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            open: AtomicUsize::new(0),
            to_service,
        });
        let thread = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name(THREAD_NAME.to_owned())
                .spawn(move || accept(&listener, &shared))
                .map_err(|e| SpotifyError::Network(e.kind().to_string()))?
        };
        Ok(PageServer {
            shared,
            thread: Some(thread),
        })
    }

    /// The page's address, ending in `/`.
    pub fn url(&self) -> String {
        format!(
            "http://127.0.0.1:{}/p/{}/",
            self.shared.port, self.shared.key
        )
    }

    pub fn port(&self) -> u16 {
        self.shared.port
    }

    /// Asks the page to close its window: `wait` and every later `event` answer so.
    pub fn close_page(&self) {
        self.shared.close.store(true, Ordering::SeqCst);
    }

    /// Stops listening. Requests already being answered finish on their own.
    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.close.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            // The accept loop wakes every 50 ms; a panic there has nothing left to clean up.
            let _ = thread.join();
        }
    }
}

impl Drop for PageServer {
    fn drop(&mut self) {
        self.halt();
    }
}

impl std::fmt::Debug for PageServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PageServer(127.0.0.1:{})", self.shared.port)
    }
}

/// 32 random bytes from the system, as base64url.
fn random_text() -> Result<String, SpotifyError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| SpotifyError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn accept(listener: &TcpListener, shared: &Arc<Shared>) {
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                if shared.open.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    let mut stream = stream;
                    if prepare(&stream) {
                        respond(&mut stream, Answer::status(503));
                    }
                    continue;
                }
                shared.open.fetch_add(1, Ordering::SeqCst);
                let for_thread = Arc::clone(shared);
                let spawned = thread::Builder::new()
                    .name(format!("{THREAD_NAME}-conn"))
                    .spawn(move || {
                        let mut stream = stream;
                        if prepare(&stream) {
                            let answer = answer(&mut stream, &for_thread);
                            respond(&mut stream, answer);
                        }
                        for_thread.open.fetch_sub(1, Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    shared.open.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(_) => thread::sleep(POLL),
        }
    }
}

fn prepare(stream: &TcpStream) -> bool {
    stream.set_nonblocking(false).is_ok()
        && stream.set_read_timeout(Some(IO_TIMEOUT)).is_ok()
        && stream.set_write_timeout(Some(IO_TIMEOUT)).is_ok()
}

/// One request: its method, path (without the query), lowercased headers and body.
struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

/// Reads one request, or the status to refuse it with.
fn read_request(stream: &mut TcpStream) -> Result<Request, u16> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if buffer.len() >= MAX_REQUEST {
            return Err(413);
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("").to_owned();
    let target = first.next().unwrap_or("");
    let path = target.split('?').next().unwrap_or("").to_owned();
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = match headers.get("content-length") {
        Some(value) => value.parse().map_err(|_| 400u16)?,
        None => 0,
    };
    if length > MAX_BODY || head_end + 4 + length > MAX_REQUEST {
        return Err(413);
    }
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body.truncate(length);
    Ok(Request {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

struct Answer {
    status: u16,
    content_type: &'static str,
    body: String,
    csp: bool,
}

impl Answer {
    fn status(status: u16) -> Answer {
        Answer {
            status,
            content_type: "text/plain; charset=utf-8",
            body: String::new(),
            csp: false,
        }
    }

    fn json(body: String) -> Answer {
        Answer {
            status: 200,
            content_type: "application/json",
            body,
            csp: false,
        }
    }

    fn text(content_type: &'static str, body: &str, csp: bool) -> Answer {
        Answer {
            status: 200,
            content_type,
            body: body.to_owned(),
            csp,
        }
    }
}

fn answer(stream: &mut TcpStream, shared: &Shared) -> Answer {
    let request = match read_request(stream) {
        Ok(request) => request,
        Err(status) => return Answer::status(status),
    };
    let origin = format!("127.0.0.1:{}", shared.port);
    if request.headers.get("host") != Some(&origin) {
        return Answer::status(403);
    }
    if let Some(from) = request.headers.get("origin")
        && *from != format!("http://{origin}")
    {
        return Answer::status(403);
    }
    let prefix = format!("/p/{}/", shared.key);
    let Some(route) = request.path.strip_prefix(&prefix) else {
        return Answer::status(404);
    };
    match (request.method.as_str(), route) {
        ("GET", "") => Answer::text("text/html; charset=utf-8", PAGE, true),
        ("GET", "player.js") => Answer::text("text/javascript; charset=utf-8", SCRIPT, false),
        ("GET", "strings") => Answer::json(shared.strings.clone()),
        ("POST", "session") => give_session(shared),
        ("GET", "token") | ("POST", "event") | ("GET", "wait") => {
            if !has_session(&request, shared) {
                return Answer::status(403);
            }
            match route {
                "token" => token(shared),
                "event" => event(&request, shared),
                _ => wait(shared),
            }
        }
        _ => Answer::status(404),
    }
}

fn give_session(shared: &Shared) -> Answer {
    let mut session = shared
        .session
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if session.is_some() {
        return Answer::status(403);
    }
    let Ok(new) = random_text() else {
        return Answer::status(503);
    };
    *session = Some(new.clone());
    Answer::json(serde_json::json!({ "session": new }).to_string())
}

fn has_session(request: &Request, shared: &Shared) -> bool {
    let session = shared
        .session
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    session.is_some() && request.headers.get("x-aria-session") == session.as_ref()
}

fn send(shared: &Shared, msg: PageMsg) {
    // Fails only once the Spotify thread has gone, when nothing is listening.
    let _ = shared.to_service.send(msg);
}

fn token(shared: &Shared) -> Answer {
    send(shared, PageMsg::Event(PageEvent::Alive));
    let (reply, answer) = bounded(1);
    send(shared, PageMsg::Token(reply));
    match answer.recv_timeout(TOKEN_WAIT) {
        Ok(Some(token)) => {
            Answer::json(serde_json::json!({ "access_token": token.expose() }).to_string())
        }
        _ => Answer::status(503),
    }
}

fn closing(shared: &Shared) -> String {
    if shared.close.load(Ordering::SeqCst) {
        r#"{"close":true}"#.to_owned()
    } else {
        "{}".to_owned()
    }
}

fn event(request: &Request, shared: &Shared) -> Answer {
    let Ok(event) = serde_json::from_str::<PageEvent>(&request.body) else {
        return Answer::status(400);
    };
    send(shared, PageMsg::Event(event));
    Answer::json(closing(shared))
}

fn wait(shared: &Shared) -> Answer {
    send(shared, PageMsg::Event(PageEvent::Alive));
    let until = Instant::now() + WAIT_FOR;
    while !shared.close.load(Ordering::SeqCst) && Instant::now() < until {
        thread::sleep(POLL);
    }
    Answer::json(closing(shared))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Service Unavailable",
    }
}

fn respond(stream: &mut TcpStream, answer: Answer) {
    let csp = if answer.csp {
        format!("Content-Security-Policy: {CSP}\r\n")
    } else {
        String::new()
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\n{csp}Connection: close\r\n\r\n",
        answer.status,
        reason(answer.status),
        answer.content_type,
        answer.body.len()
    );
    // The page may already have gone; nothing to do about it.
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(answer.body.as_bytes());
    let _ = stream.flush();
}

/// Plays the page in tests: talks to a [`PageServer`] as the page's script does.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug)]
pub struct PageClient {
    port: u16,
    base: String,
    session: Option<String>,
}

#[cfg(any(test, feature = "test-support"))]
impl PageClient {
    /// `url` is [`PageServer::url`].
    pub fn new(url: &str) -> PageClient {
        let rest = url.trim_start_matches("http://127.0.0.1:");
        let (port, path) = rest.split_once('/').unwrap_or((rest, ""));
        PageClient {
            port: port.parse().unwrap_or(0),
            base: format!("/{path}"),
            session: None,
        }
    }

    /// Takes the session, as the page does once on load.
    pub fn session(&mut self) -> Option<String> {
        let (status, body) = self.request("POST", "session", Some(""));
        if status != 200 {
            return None;
        }
        let value: serde_json::Value = serde_json::from_str(&body).ok()?;
        let session = value.get("session")?.as_str()?.to_owned();
        self.session = Some(session.clone());
        Some(session)
    }

    pub fn set_session(&mut self, session: &str) {
        self.session = Some(session.to_owned());
    }

    pub fn token(&self) -> (u16, String) {
        self.request("GET", "token", None)
    }

    pub fn event(&self, json: &str) -> (u16, String) {
        self.request("POST", "event", Some(json))
    }

    pub fn wait(&self) -> (u16, String) {
        self.request("GET", "wait", None)
    }

    fn request(&self, method: &str, route: &str, body: Option<&str>) -> (u16, String) {
        let Ok(mut stream) = TcpStream::connect(("127.0.0.1", self.port)) else {
            return (0, String::new());
        };
        let session = self
            .session
            .as_ref()
            .map(|s| format!("X-Aria-Session: {s}\r\n"))
            .unwrap_or_default();
        let body = body.unwrap_or("");
        let request = format!(
            "{method} {}{route} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{session}Content-Length: {}\r\n\r\n{body}",
            self.base,
            self.port,
            body.len()
        );
        if stream.write_all(request.as_bytes()).is_err() {
            return (0, String::new());
        }
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        let status = answer
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let body = answer
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_owned())
            .unwrap_or_default();
        (status, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    use crossbeam_channel::{Receiver, unbounded};

    use crate::Secret;

    /// A server whose token requests are answered with `tok` by a thread of the test.
    fn server() -> (PageServer, Receiver<PageEvent>) {
        let (to_service, from_page) = unbounded();
        let (events_tx, events) = unbounded();
        std::thread::spawn(move || {
            for msg in from_page {
                match msg {
                    PageMsg::Token(reply) => {
                        let _ = reply.send(Some(Secret::new("tok".to_owned())));
                    }
                    PageMsg::Event(event) => {
                        let _ = events_tx.send(event);
                    }
                }
            }
        });
        let strings = r#"{"player-page-ready":"Ready"}"#.to_owned();
        let server = PageServer::start(strings, to_service).unwrap();
        (server, events)
    }

    fn raw(port: u16, request: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        let status = answer
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        (status, answer)
    }

    fn get(port: u16, target: &str) -> (u16, String) {
        raw(
            port,
            &format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        )
    }

    fn path(server: &PageServer) -> String {
        let origin = format!("http://127.0.0.1:{}", server.port());
        server.url().trim_start_matches(&origin).to_owned()
    }

    fn events_of(events: &Receiver<PageEvent>) -> Vec<PageEvent> {
        std::thread::sleep(Duration::from_millis(100));
        events
            .try_iter()
            .filter(|e| *e != PageEvent::Alive)
            .collect()
    }

    #[test]
    fn it_listens_on_loopback_only() {
        let (server, _) = server();
        assert!(server.url().starts_with("http://127.0.0.1:"));
        assert!(server.url().ends_with('/'));
    }

    #[test]
    fn the_page_and_script_are_served_with_the_headers() {
        let (server, _) = server();
        let (port, p) = (server.port(), path(&server));
        let script = format!("{p}player.js");
        for (target, kind) in [
            (p.as_str(), "text/html"),
            (script.as_str(), "text/javascript"),
        ] {
            let (status, answer) = get(port, target);
            assert_eq!(status, 200, "{target}");
            assert!(
                answer.contains(&format!("Content-Type: {kind}")),
                "{answer}"
            );
            assert!(answer.contains("Cache-Control: no-store"));
            assert!(answer.contains("Referrer-Policy: no-referrer"));
            assert!(!answer.contains("Access-Control"));
        }
        let (_, page) = get(port, &p);
        let csp = "Content-Security-Policy: script-src 'self' https://sdk.scdn.co\r\n";
        assert!(page.contains(csp));
        assert!(page.contains("https://sdk.scdn.co/spotify-player.js"));
        let (status, strings) = get(port, &format!("{p}strings"));
        assert_eq!(status, 200);
        assert!(strings.ends_with(r#"{"player-page-ready":"Ready"}"#));
    }

    #[test]
    fn a_wrong_key_is_404() {
        let (server, _) = server();
        let port = server.port();
        assert_eq!(get(port, "/p/nope/").0, 404);
        assert_eq!(get(port, "/").0, 404);
    }

    #[test]
    fn a_host_by_another_name_is_refused() {
        let (server, _) = server();
        let (port, p) = (server.port(), path(&server));
        for host in [
            format!("localhost:{port}"),
            "127.0.0.1".to_owned(),
            format!("127.0.0.1:{}", port.wrapping_add(1)),
            format!("evil.example:{port}"),
        ] {
            let request = format!("GET {p} HTTP/1.1\r\nHost: {host}\r\n\r\n");
            assert_eq!(raw(port, &request).0, 403, "{host}");
        }
        let request = format!("GET {p} HTTP/1.1\r\n\r\n");
        assert_eq!(raw(port, &request).0, 403, "no Host");
    }

    #[test]
    fn a_foreign_origin_is_refused() {
        let (server, _) = server();
        let (port, p) = (server.port(), path(&server));
        let with = |origin: &str| {
            let request =
                format!("GET {p} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\n\r\n");
            raw(port, &request).0
        };
        assert_eq!(with("https://evil.example"), 403);
        assert_eq!(with(&format!("http://127.0.0.1:{port}")), 200);
    }

    #[test]
    fn the_session_is_given_once() {
        let (server, _) = server();
        let mut page = PageClient::new(&server.url());
        assert!(page.session().is_some());
        let mut thief = PageClient::new(&server.url());
        assert_eq!(thief.session(), None);
    }

    #[test]
    fn token_needs_the_session() {
        let (server, _) = server();
        let mut thief = PageClient::new(&server.url());
        let (status, body) = thief.token();
        assert_eq!(status, 403);
        assert!(!body.contains("tok"));
        let mut page = PageClient::new(&server.url());
        page.session().unwrap();
        thief.set_session("wrong");
        let (status, body) = thief.token();
        assert_eq!(status, 403);
        assert!(!body.contains("tok"));
        let (status, body) = page.token();
        assert_eq!(status, 200);
        assert_eq!(body, r#"{"access_token":"tok"}"#);
    }

    #[test]
    fn events_are_parsed_and_sent_on() {
        let (server, events) = server();
        let mut page = PageClient::new(&server.url());
        page.session().unwrap();
        assert_eq!(page.event(r#"{"type":"ready","device_id":"d1"}"#).0, 200);
        let error = r#"{"type":"error","kind":"account","message":"m"}"#;
        assert_eq!(page.event(error).0, 200);
        assert_eq!(page.event(r#"{"type":"autoplay_failed"}"#).0, 200);
        assert_eq!(
            page.event(r#"{"type":"not_ready","device_id":"d1"}"#).0,
            200
        );
        assert_eq!(
            events_of(&events),
            [
                PageEvent::Ready {
                    device_id: "d1".to_owned()
                },
                PageEvent::Error {
                    kind: PageError::Account,
                    message: "m".to_owned()
                },
                PageEvent::AutoplayFailed,
                PageEvent::NotReady {
                    device_id: "d1".to_owned()
                },
            ]
        );
        assert_eq!(page.event("{not json").0, 400);
        assert_eq!(page.event(r#"{"type":"error","kind":"other"}"#).0, 400);
        assert!(events_of(&events).is_empty());
    }

    #[test]
    fn page_requests_are_signs_of_life() {
        let (server, events) = server();
        let mut page = PageClient::new(&server.url());
        page.session().unwrap();
        page.token();
        page.event(r#"{"type":"alive"}"#);
        std::thread::sleep(Duration::from_millis(100));
        let alive = events.try_iter().filter(|e| *e == PageEvent::Alive).count();
        assert_eq!(alive, 2);
    }

    #[test]
    fn an_oversized_request_is_413() {
        let (server, _) = server();
        let (port, p) = (server.port(), path(&server));
        let body = "x".repeat(5000);
        let request = format!(
            "POST {p}event HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        assert_eq!(raw(port, &request).0, 413);
        let header = "a".repeat(9000);
        let request =
            format!("GET {p} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Long: {header}\r\n\r\n");
        assert_eq!(raw(port, &request).0, 413);
    }

    #[test]
    fn close_page_answers_wait_and_event() {
        let (server, _) = server();
        let mut page = PageClient::new(&server.url());
        page.session().unwrap();
        assert_eq!(page.event(r#"{"type":"alive"}"#).1, "{}");
        let waiting = {
            let page = page.clone();
            std::thread::spawn(move || page.wait())
        };
        std::thread::sleep(Duration::from_millis(200));
        let closed_at = Instant::now();
        server.close_page();
        let (status, body) = waiting.join().unwrap();
        assert!(closed_at.elapsed() < Duration::from_millis(500));
        assert_eq!(status, 200);
        assert_eq!(body, r#"{"close":true}"#);
        assert_eq!(page.event(r#"{"type":"alive"}"#).1, r#"{"close":true}"#);
    }

    #[test]
    fn a_stopped_server_answers_nothing() {
        let (server, _) = server();
        let port = server.port();
        server.stop();
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }

    #[test]
    fn the_page_script_never_logs_or_stores() {
        for word in [
            "console.",
            "localStorage",
            "sessionStorage",
            "indexedDB",
            "document.cookie",
        ] {
            assert!(!SCRIPT.contains(word), "{word}");
        }
        assert!(SCRIPT.contains("onSpotifyWebPlaybackSDKReady"));
        assert!(SCRIPT.contains(r#"name: "Aria""#));
    }
}
