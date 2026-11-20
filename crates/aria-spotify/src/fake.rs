//! A fake Spotify for tests: a tiny HTTP/1.1 server on `127.0.0.1` that plays scripted answers
//! and records what it was asked. It never talks to the real Spotify.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::Endpoints;

/// One scripted answer. `{base}` in the body becomes the fake's `http://127.0.0.1:{port}`.
#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: body.as_bytes().to_vec(),
        }
    }

    /// `tests/fixtures/{name}.json` of this crate.
    pub fn fixture(status: u16, name: &str) -> Reply {
        let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
        let body = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("fixture {path}: {e}"));
        Reply::json(status, &body)
    }

    /// A status with no body (`204`).
    pub fn empty(status: u16) -> Reply {
        Reply {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// A binary answer, such as an image.
    pub fn bytes(status: u16, content_type: &str, body: Vec<u8>) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".to_owned(), content_type.to_owned())],
            body,
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Reply {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// One request the fake received.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub authorization: Option<String>,
    pub body: String,
}

impl Seen {
    pub fn param(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
struct State {
    replies: HashMap<(String, String), VecDeque<Reply>>,
    offline: bool,
    seen: Vec<Seen>,
}

pub struct FakeSpotify {
    port: u16,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeSpotify {
    pub fn start() -> FakeSpotify {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|e| panic!("{e}"));
        listener
            .set_nonblocking(true)
            .unwrap_or_else(|e| panic!("{e}"));
        let port = listener
            .local_addr()
            .unwrap_or_else(|e| panic!("{e}"))
            .port();
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (state, stop) = (Arc::clone(&state), Arc::clone(&stop));
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let offline =
                                state.lock().unwrap_or_else(PoisonError::into_inner).offline;
                            if !offline {
                                serve(stream, port, &state);
                            }
                        }
                        Err(_) => thread::sleep(Duration::from_millis(2)),
                    }
                }
            })
        };
        FakeSpotify {
            port,
            state,
            stop,
            thread: Some(thread),
        }
    }

    /// `http://127.0.0.1:{port}`.
    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn endpoints(&self) -> Endpoints {
        Endpoints {
            accounts: self.base(),
            api: format!("{}/v1", self.base()),
        }
    }

    /// Queues an answer for `method path` (the path without the query). Answers are used in
    /// order; the last one repeats. Unscripted paths answer `404`.
    pub fn reply(&self, method: &str, path: &str, reply: Reply) {
        self.lock()
            .replies
            .entry((method.to_owned(), path.to_owned()))
            .or_default()
            .push_back(reply);
    }

    /// While offline, every connection is closed without an answer, as when the network is
    /// gone. Nothing is recorded.
    pub fn set_offline(&self, offline: bool) {
        self.lock().offline = offline;
    }

    /// Forgets every scripted answer (what was seen stays).
    pub fn clear(&self) {
        self.lock().replies.clear();
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.lock().seen.clone()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for FakeSpotify {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn next(queue: &mut VecDeque<Reply>) -> Option<Reply> {
    if queue.len() > 1 {
        queue.pop_front()
    } else {
        queue.front().cloned()
    }
}

fn serve(mut stream: TcpStream, port: u16, state: &Mutex<State>) {
    if stream.set_nonblocking(false).is_err()
        || stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .is_err()
    {
        return;
    }
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("").to_owned();
    let target = first.next().unwrap_or("").to_owned();
    let mut length = 0usize;
    let mut authorization = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            if name == "content-length" {
                length = value.trim().parse().unwrap_or(0);
            } else if name == "authorization" {
                authorization = Some(value.trim().to_owned());
            }
        }
    }
    while buffer.len() < head_end + length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
    let body = String::from_utf8_lossy(&buffer[head_end.min(buffer.len())..]).into_owned();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let query: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    let key = (method.clone(), path.to_owned());
    let reply = {
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        state.seen.push(Seen {
            method,
            path: path.to_owned(),
            query,
            authorization,
            body,
        });
        state.replies.get_mut(&key).and_then(next)
    };
    let base = format!("http://127.0.0.1:{port}");
    let reply = reply
        .unwrap_or_else(|| Reply::json(404, r#"{"error":{"status":404,"message":"Not found"}}"#));
    let is_text = reply
        .headers
        .iter()
        .any(|(n, v)| n.eq_ignore_ascii_case("content-type") && v.contains("json"));
    let body = if is_text {
        String::from_utf8_lossy(&reply.body)
            .replace("{base}", &base)
            .into_bytes()
    } else {
        reply.body
    };
    let mut out = format!(
        "HTTP/1.1 {} Fake\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        body.len()
    );
    for (name, value) in reply.headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(&body);
    let _ = stream.write_all(&bytes);
    let _ = stream.flush();
}
