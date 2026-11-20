//! The Spotify thread: the only owner of the Spotify [`Client`].
//!
//! It signs in (PKCE in the browser, the redirect taken by a one-shot loopback listener),
//! restores a session from the refresh token in the system store, loads the library, debounces
//! search, polls what Spotify is playing at the [`Cadence`] the backend asks for, and sends
//! playback commands to the chosen device. Covers are fetched and decoded on a second thread,
//! `aria-spotify-covers`, and handed back here.
//!
//! Before sign-in and after sign-out it makes no request at all. No token, code or verifier is
//! ever logged: errors are shown by kind, and [`SpotifyError`]'s text carries none.

use std::collections::HashSet;
use std::io::{self, Cursor};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use aria_spotify::{
    Callback, Client, Endpoints, Http, Pkce, PlayRequest, PlaybackState, RepeatState,
    SearchResults, Secret, SpAlbum, SpArtist, SpDevice, SpPlaylist, SpTrack, SpotifyError,
    TokenStore, Tokens, authorize_url, exchange_code, fetch_image, redirect_uri, refresh,
    system_store,
};
use crossbeam_channel::{Receiver, Sender, bounded, never, select, unbounded};

use crate::notice::{Notice, NoticeLevel};
use crate::runtime::{DoneGuard, join_bounded};

const THREAD_NAME: &str = "aria-spotify";
const COVERS_THREAD: &str = "aria-spotify-covers";
const SIGN_IN_THREAD: &str = "aria-spotify-sign-in";
/// Cover requests waiting to be fetched; more are dropped (the UI asks again).
const COVERS_QUEUE: usize = 64;
/// Covers are scaled down to fit in this many pixels.
const COVER_SIZE: u32 = 128;
/// A cover may claim no more than this many pixels on a side before it is decoded.
const COVER_MAX_SIDE: u32 = 4096;
/// A change in `progress_ms` smaller than this is drift, not news.
const DRIFT_MS: u64 = 1500;
/// How long the loop sleeps when nothing is due.
const IDLE: Duration = Duration::from_secs(3600);

/// Opens a URL in the user's browser.
pub type OpenBrowser = Box<dyn Fn(&str) -> io::Result<()> + Send>;

/// What the service needs from the outside, so tests can point it at a fake Spotify.
pub struct SpotifyDeps {
    pub endpoints: Endpoints,
    pub store: Box<dyn TokenStore>,
    pub open_browser: OpenBrowser,
    pub redirect_port: u16,
    pub sign_in_timeout: Duration,
    pub search_quiet: Duration,
    pub fast: Duration,
    pub slow: Duration,
    pub after_command: Duration,
}

impl SpotifyDeps {
    /// Spotify itself, the system keyring and the default browser.
    pub fn system() -> SpotifyDeps {
        SpotifyDeps {
            endpoints: Endpoints::spotify(),
            store: system_store(),
            open_browser: Box::new(|url: &str| open::that(url)),
            redirect_port: aria_spotify::REDIRECT_PORT,
            sign_in_timeout: Duration::from_secs(5 * 60),
            search_quiet: Duration::from_millis(300),
            fast: Duration::from_secs(2),
            slow: Duration::from_secs(15),
            after_command: Duration::from_millis(400),
        }
    }
}

/// How often to ask Spotify what is playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cadence {
    /// The bar shows Spotify.
    Fast,
    /// The bar shows Aria's own player.
    Slow,
    /// The window is minimized or hidden: no polling at all.
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpotifyRequest {
    /// Opens the browser to sign in; `done_page` is the HTML the browser shows afterwards.
    SignIn {
        done_page: String,
    },
    CancelSignIn,
    /// Deletes the stored token and forgets everything.
    SignOut,
    /// Loads the library again.
    Refresh,
    Search(String),
    Play(PlayRequest),
    TogglePlay,
    Pause,
    Next,
    Previous,
    Seek(u64),
    SetVolume(u8),
    SetShuffle(bool),
    SetRepeat(RepeatState),
    /// The device to play on; `None` is Spotify's active one.
    Device(Option<String>),
    ListDevices,
    Cadence(Cadence),
    /// Asks for a cover image by URL.
    Cover(String),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpotifyAccount {
    /// `keyring` says whether a sign-in would be kept across runs.
    SignedOut {
        keyring: bool,
    },
    SigningIn,
    SignedIn {
        name: String,
    },
}

/// The Spotify library, kept in memory only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpotifyLibrary {
    pub tracks: Vec<SpTrack>,
    pub albums: Vec<SpAlbum>,
    pub artists: Vec<SpArtist>,
    pub playlists: Vec<SpPlaylist>,
    pub loading: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpotifySearch {
    pub query: String,
    pub results: SearchResults,
}

/// What Spotify was playing at `at`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpotifyPlayback {
    pub state: PlaybackState,
    pub at: Instant,
}

/// A decoded cover, at most [`COVER_SIZE`] pixels on its longer side.
#[derive(Clone, PartialEq, Eq)]
pub struct Cover {
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Cover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cover({}x{} {})", self.width, self.height, self.url)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SpotifyEvent {
    Account(SpotifyAccount),
    Library(Arc<SpotifyLibrary>),
    Search(Arc<SpotifySearch>),
    Playback(Option<Arc<SpotifyPlayback>>),
    Devices(Vec<SpDevice>),
    Cover(Arc<Cover>),
    Notice(Notice),
}

pub struct SpotifyHandle {
    requests: Sender<SpotifyRequest>,
    done: Receiver<()>,
    thread: thread::JoinHandle<()>,
}

impl SpotifyHandle {
    /// Queues `request` without blocking. If the thread has exited, it is dropped.
    pub fn send(&self, request: SpotifyRequest) {
        // Fails only once the thread has exited, when nothing is listening.
        let _ = self.requests.send(request);
    }

    /// Sends [`SpotifyRequest::Shutdown`] and waits up to `timeout` for the thread to end.
    pub fn join(self, timeout: Duration) -> bool {
        self.send(SpotifyRequest::Shutdown);
        join_bounded(&self.done, self.thread, timeout, THREAD_NAME)
    }
}

pub struct SpotifyService;

impl SpotifyService {
    pub fn start(
        deps: SpotifyDeps,
        on_event: Box<dyn Fn(SpotifyEvent) + Send>,
    ) -> io::Result<SpotifyHandle> {
        let (requests_tx, requests_rx) = unbounded();
        let (done_tx, done_rx) = bounded(1);
        let (covers_tx, covers_rx) = bounded(COVERS_QUEUE);
        let (decoded_tx, decoded_rx) = unbounded();
        let signed_in = Arc::new(AtomicBool::new(false));
        let http = Http::new();
        {
            let (http, endpoints, signed_in) =
                (http.clone(), deps.endpoints.clone(), Arc::clone(&signed_in));
            thread::Builder::new()
                .name(COVERS_THREAD.to_owned())
                .spawn(move || covers(&http, &endpoints, &covers_rx, &decoded_tx, &signed_in))?;
        }
        let service = Service {
            deps,
            http,
            on_event,
            account: None,
            client: None,
            saved: None,
            restore: None,
            sign_in: None,
            cadence: Cadence::Slow,
            next_poll: None,
            paused_until: None,
            search: None,
            chosen: None,
            devices: Vec::new(),
            last: None,
            noticed: HashSet::new(),
            covers: covers_tx,
            signed_in,
        };
        let thread = thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || {
                let _done = DoneGuard::new(done_tx);
                service.run(&requests_rx, &decoded_rx);
            })?;
        Ok(SpotifyHandle {
            requests: requests_tx,
            done: done_rx,
            thread,
        })
    }
}

/// A sign-in waiting for the browser.
struct SignIn {
    pkce: Pkce,
    redirect: String,
    cancel: Arc<AtomicBool>,
    result: Receiver<Result<Secret, SpotifyError>>,
}

struct Service {
    deps: SpotifyDeps,
    http: Http,
    on_event: Box<dyn Fn(SpotifyEvent) + Send>,
    /// The account state last sent; `None` until the first is sent.
    account: Option<SpotifyAccount>,
    client: Option<Client>,
    /// The refresh token last written to the store.
    saved: Option<Secret>,
    /// A stored token whose session could not be restored yet (no network): tried at each poll.
    restore: Option<Secret>,
    sign_in: Option<SignIn>,
    cadence: Cadence,
    next_poll: Option<Instant>,
    /// Spotify asked to wait until then.
    paused_until: Option<Instant>,
    /// The newest query, and when it may be sent.
    search: Option<(String, Instant)>,
    chosen: Option<String>,
    devices: Vec<SpDevice>,
    last: Option<SpotifyPlayback>,
    /// Notice keys shown since the last request that succeeded.
    noticed: HashSet<&'static str>,
    covers: Sender<String>,
    signed_in: Arc<AtomicBool>,
}

impl Service {
    fn run(mut self, requests: &Receiver<SpotifyRequest>, decoded: &Receiver<Cover>) {
        self.start();
        let mut covers_alive = true;
        loop {
            let deadline = [
                self.poll_deadline(),
                self.search.as_ref().map(|(_, at)| *at),
            ]
            .into_iter()
            .flatten()
            .min();
            let timeout = deadline.map_or(IDLE, |d| d.saturating_duration_since(Instant::now()));
            let sign_in = self
                .sign_in
                .as_ref()
                .map_or_else(never, |s| s.result.clone());
            let covers = if covers_alive {
                decoded.clone()
            } else {
                never()
            };
            select! {
                recv(requests) -> request => match request {
                    Ok(SpotifyRequest::Shutdown) | Err(_) => break,
                    Ok(request) => self.handle(request),
                },
                recv(sign_in) -> result => {
                    self.finish_sign_in(result.unwrap_or(Err(SpotifyError::SignInCancelled)));
                }
                recv(covers) -> cover => match cover {
                    Ok(cover) => self.emit(SpotifyEvent::Cover(Arc::new(cover))),
                    Err(_) => covers_alive = false,
                },
                default(timeout) => {}
            }
            self.run_due();
        }
        if let Some(sign_in) = self.sign_in.take() {
            sign_in.cancel.store(true, Ordering::SeqCst);
        }
        self.signed_in.store(false, Ordering::SeqCst);
    }

    fn emit(&self, event: SpotifyEvent) {
        (self.on_event)(event);
    }

    fn set_account(&mut self, account: SpotifyAccount) {
        if self.account.as_ref() != Some(&account) {
            self.account = Some(account.clone());
            self.emit(SpotifyEvent::Account(account));
        }
    }

    fn signed_out(&self) -> SpotifyAccount {
        SpotifyAccount::SignedOut {
            keyring: self.deps.store.persistent(),
        }
    }

    // --- start, sign-in and sign-out ---

    fn start(&mut self) {
        match self.deps.store.load() {
            Ok(Some(token)) => {
                self.saved = Some(token.clone());
                self.restore_session(&token);
            }
            Ok(None) => {
                let account = self.signed_out();
                self.set_account(account);
            }
            Err(e) => {
                tracing::warn!("couldn't read the Spotify sign-in: {e}");
                let account = self.signed_out();
                self.set_account(account);
            }
        }
        if self.restore.is_some() {
            self.next_poll = self.interval().map(|i| Instant::now() + i);
        }
    }

    fn restore_session(&mut self, token: &Secret) {
        match refresh(&self.http, &self.deps.endpoints, token) {
            Ok(tokens) => {
                self.restore = None;
                self.signed_in_with(tokens);
            }
            Err(
                e @ (SpotifyError::Network(_)
                | SpotifyError::RateLimited { .. }
                | SpotifyError::QuotaExceeded { .. }),
            ) => {
                self.restore = Some(token.clone());
                let account = self.signed_out();
                self.set_account(account);
                self.fail(e);
            }
            Err(e) => {
                self.restore = None;
                let ended = e == SpotifyError::SessionEnded;
                if ended {
                    self.forget_token();
                }
                let account = self.signed_out();
                self.set_account(account);
                self.fail(e);
            }
        }
    }

    fn signed_in_with(&mut self, tokens: Tokens) {
        let client = Client::new(self.http.clone(), self.deps.endpoints.clone(), tokens);
        self.client = Some(client);
        let Some(me) = self.call(Client::me) else {
            // Signed out by the failure (allow-list, ended session), or no network: try later.
            if let Some(client) = self.client.take() {
                self.restore = Some(client.tokens().refresh.clone());
                self.next_poll = self.interval().map(|i| Instant::now() + i);
            }
            return;
        };
        self.signed_in.store(true, Ordering::SeqCst);
        self.set_account(SpotifyAccount::SignedIn { name: me.name });
        self.list_devices();
        self.load_library();
        if self.client.is_some() {
            self.next_poll = self.interval().map(|_| Instant::now());
        }
    }

    fn begin_sign_in(&mut self, done_page: String) {
        if self.sign_in.is_some() || self.client.is_some() {
            return;
        }
        self.restore = None;
        let pkce = match Pkce::new() {
            Ok(pkce) => pkce,
            Err(e) => return self.fail_sign_in(e),
        };
        let callback = match Callback::bind(self.deps.redirect_port) {
            Ok(callback) => callback,
            Err(e) => return self.fail_sign_in(e),
        };
        let redirect = redirect_uri(callback.port());
        let url = authorize_url(&self.deps.endpoints, &pkce, &redirect);
        if let Err(e) = (self.deps.open_browser)(&url) {
            tracing::warn!(
                "couldn't open the browser for the Spotify sign-in: {}",
                e.kind()
            );
            self.notice(NoticeLevel::Error, "spotify-browser-failed", Vec::new());
            let account = self.signed_out();
            return self.set_account(account);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = bounded(1);
        let (state, timeout, flag) = (
            pkce.state.clone(),
            self.deps.sign_in_timeout,
            Arc::clone(&cancel),
        );
        let spawned = thread::Builder::new()
            .name(SIGN_IN_THREAD.to_owned())
            .spawn(move || {
                // Fails only if the service has gone, when nobody is waiting.
                let _ = tx.send(callback.wait(&state, timeout, &flag, &done_page));
            });
        if spawned.is_err() {
            return self.fail_sign_in(SpotifyError::Network("no thread".to_owned()));
        }
        self.sign_in = Some(SignIn {
            pkce,
            redirect,
            cancel,
            result: rx,
        });
        self.set_account(SpotifyAccount::SigningIn);
    }

    fn finish_sign_in(&mut self, result: Result<Secret, SpotifyError>) {
        let Some(sign_in) = self.sign_in.take() else {
            return;
        };
        let tokens = result.and_then(|code| {
            exchange_code(
                &self.http,
                &self.deps.endpoints,
                &code,
                &sign_in.pkce,
                &sign_in.redirect,
            )
        });
        match tokens {
            Ok(tokens) => {
                self.saved = None;
                self.signed_in_with(tokens);
            }
            Err(e) => self.fail_sign_in(e),
        }
    }

    fn fail_sign_in(&mut self, error: SpotifyError) {
        let account = self.signed_out();
        self.set_account(account);
        match error {
            SpotifyError::SignInCancelled => {}
            SpotifyError::PortBusy => self.notice(NoticeLevel::Error, "spotify-port-busy", vec![]),
            SpotifyError::SignInDenied => {
                self.notice(NoticeLevel::Warning, "spotify-sign-in-denied", vec![]);
            }
            SpotifyError::SignInTimedOut => {
                self.notice(NoticeLevel::Warning, "spotify-sign-in-timed-out", vec![]);
            }
            other => self.fail(other),
        }
    }

    fn cancel_sign_in(&mut self) {
        if let Some(sign_in) = self.sign_in.take() {
            sign_in.cancel.store(true, Ordering::SeqCst);
            let account = self.signed_out();
            self.set_account(account);
        }
    }

    fn forget_token(&mut self) {
        self.saved = None;
        if let Err(e) = self.deps.store.delete() {
            tracing::warn!("couldn't delete the Spotify sign-in: {e}");
        }
    }

    /// Deletes the token and forgets the session. Makes no request.
    fn sign_out(&mut self) {
        self.cancel_sign_in();
        self.forget_token();
        self.client = None;
        self.restore = None;
        self.signed_in.store(false, Ordering::SeqCst);
        self.search = None;
        self.next_poll = None;
        self.paused_until = None;
        self.last = None;
        self.devices.clear();
        let account = self.signed_out();
        self.set_account(account);
        self.emit(SpotifyEvent::Library(Arc::new(SpotifyLibrary::default())));
        self.emit(SpotifyEvent::Search(Arc::new(SpotifySearch::default())));
        self.emit(SpotifyEvent::Playback(None));
        self.emit(SpotifyEvent::Devices(Vec::new()));
    }

    // --- requests ---

    fn handle(&mut self, request: SpotifyRequest) {
        match request {
            SpotifyRequest::SignIn { done_page } => self.begin_sign_in(done_page),
            SpotifyRequest::CancelSignIn => self.cancel_sign_in(),
            SpotifyRequest::SignOut => self.sign_out(),
            SpotifyRequest::Refresh => {
                self.list_devices();
                self.load_library();
            }
            SpotifyRequest::Search(query) => self.queue_search(query),
            SpotifyRequest::Play(request) => self.command(|c, d| c.play(d, &request)),
            SpotifyRequest::TogglePlay => {
                if self.last.as_ref().is_some_and(|p| p.state.playing) {
                    self.command(Client::pause);
                } else {
                    self.command(|c, d| c.play(d, &PlayRequest::Resume));
                }
            }
            SpotifyRequest::Pause => self.command(Client::pause),
            SpotifyRequest::Next => self.command(Client::next),
            SpotifyRequest::Previous => self.command(Client::previous),
            SpotifyRequest::Seek(ms) => self.command(|c, d| c.seek(d, ms)),
            SpotifyRequest::SetVolume(percent) => self.command(|c, d| c.volume(d, percent)),
            SpotifyRequest::SetShuffle(on) => self.command(|c, d| c.shuffle(d, on)),
            SpotifyRequest::SetRepeat(state) => self.command(|c, d| c.repeat(d, state)),
            SpotifyRequest::Device(id) => self.chosen = id,
            SpotifyRequest::ListDevices => self.list_devices(),
            SpotifyRequest::Cadence(cadence) => self.set_cadence(cadence),
            SpotifyRequest::Cover(url) => {
                if self.client.is_some() {
                    // A full queue drops the request; the UI asks again.
                    let _ = self.covers.try_send(url);
                }
            }
            SpotifyRequest::Shutdown => {}
        }
    }

    /// Runs `op` on the client; a success clears the shown notices, a failure is reported.
    fn call<T>(&mut self, op: impl FnOnce(&mut Client) -> Result<T, SpotifyError>) -> Option<T> {
        let client = self.client.as_mut()?;
        let result = op(client);
        self.keep_refresh_token();
        match result {
            Ok(value) => {
                self.noticed.clear();
                Some(value)
            }
            Err(e) => {
                self.fail(e);
                None
            }
        }
    }

    /// Saves the refresh token whenever Spotify has rotated it.
    fn keep_refresh_token(&mut self) {
        let Some(token) = self.client.as_ref().map(|c| c.tokens().refresh.clone()) else {
            return;
        };
        if self.saved.as_ref() == Some(&token) {
            return;
        }
        let saved = self.deps.store.save(&token);
        self.saved = Some(token);
        if let Err(e) = saved {
            self.fail(e);
        }
    }

    /// The chosen device, if Spotify still lists it; else `None` (the active device).
    fn target(&self) -> Option<String> {
        self.chosen
            .as_ref()
            .filter(|id| self.devices.iter().any(|d| d.id.as_ref() == Some(*id)))
            .cloned()
    }

    fn command(&mut self, op: impl Fn(&mut Client, Option<&str>) -> Result<(), SpotifyError>) {
        if self.client.is_none() {
            return;
        }
        let target = self.target();
        self.call(
            |client| match (op(client, target.as_deref()), target.as_deref()) {
                (Err(SpotifyError::NoActiveDevice), Some(id)) => {
                    client.transfer(id, true)?;
                    op(client, Some(id))
                }
                (result, _) => result,
            },
        );
        if self.client.is_some() {
            let at = Instant::now() + self.deps.after_command;
            self.next_poll = Some(self.next_poll.map_or(at, |t| t.min(at)));
        }
    }

    fn queue_search(&mut self, query: String) {
        if query.trim().is_empty() {
            self.search = None;
            let empty = SpotifySearch {
                query,
                results: SearchResults::default(),
            };
            self.emit(SpotifyEvent::Search(Arc::new(empty)));
        } else if self.client.is_some() {
            self.search = Some((query, Instant::now() + self.deps.search_quiet));
        }
    }

    fn list_devices(&mut self) {
        if let Some(devices) = self.call(Client::devices) {
            self.devices.clone_from(&devices);
            self.emit(SpotifyEvent::Devices(devices));
        }
    }

    /// Sends the library as it fills: playlists, albums and artists first, the saved tracks
    /// (the longest list) last.
    fn load_library(&mut self) {
        if self.client.is_none() {
            return;
        }
        let mut library = SpotifyLibrary {
            loading: true,
            ..SpotifyLibrary::default()
        };
        self.emit(SpotifyEvent::Library(Arc::new(library.clone())));
        if let Some(playlists) = self.call(Client::playlists) {
            library.playlists = playlists;
            self.emit(SpotifyEvent::Library(Arc::new(library.clone())));
        }
        if let Some(albums) = self.call(Client::saved_albums) {
            library.albums = albums;
            self.emit(SpotifyEvent::Library(Arc::new(library.clone())));
        }
        if let Some(artists) = self.call(Client::followed_artists) {
            library.artists = artists;
            self.emit(SpotifyEvent::Library(Arc::new(library.clone())));
        }
        if let Some(tracks) = self.call(Client::saved_tracks) {
            library.tracks = tracks;
        }
        if self.client.is_some() {
            library.loading = false;
            self.emit(SpotifyEvent::Library(Arc::new(library)));
        }
    }

    // --- timers ---

    fn interval(&self) -> Option<Duration> {
        match self.cadence {
            Cadence::Fast => Some(self.deps.fast),
            Cadence::Slow => Some(self.deps.slow),
            Cadence::Off => None,
        }
    }

    fn set_cadence(&mut self, cadence: Cadence) {
        if self.cadence == cadence {
            return;
        }
        self.cadence = cadence;
        let now = Instant::now();
        self.next_poll = self
            .interval()
            .map(|i| self.next_poll.map_or(now, |t| t.min(now + i)));
    }

    fn poll_deadline(&self) -> Option<Instant> {
        if self.client.is_none() && self.restore.is_none() {
            return None;
        }
        let next = self.next_poll?;
        Some(self.paused_until.map_or(next, |p| p.max(next)))
    }

    fn run_due(&mut self) {
        let now = Instant::now();
        if self.search.as_ref().is_some_and(|(_, at)| *at <= now)
            && let Some((query, _)) = self.search.take()
            && let Some(results) = self.call(|c| c.search(&query))
        {
            let search = SpotifySearch { query, results };
            self.emit(SpotifyEvent::Search(Arc::new(search)));
        }
        if self.poll_deadline().is_some_and(|d| d <= now) {
            self.poll();
        }
    }

    fn poll(&mut self) {
        self.next_poll = self.interval().map(|i| Instant::now() + i);
        self.paused_until = None;
        if self.client.is_none() {
            if let Some(token) = self.restore.clone() {
                self.restore_session(&token);
            }
            return;
        }
        if let Some(state) = self.call(Client::player) {
            self.playback(state);
        }
    }

    /// Sends the playback state if it changed, beyond the drift of `progress_ms`.
    fn playback(&mut self, state: Option<PlaybackState>) {
        let now = Instant::now();
        let changed = match (&self.last, &state) {
            (None, None) => false,
            (Some(last), Some(new)) => !same_playback(last, new, now),
            _ => true,
        };
        if changed {
            self.last = state.map(|state| SpotifyPlayback { state, at: now });
            self.emit(SpotifyEvent::Playback(self.last.clone().map(Arc::new)));
        }
    }

    // --- errors ---

    fn notice(&mut self, level: NoticeLevel, key: &'static str, args: Vec<(&'static str, String)>) {
        self.emit(SpotifyEvent::Notice(Notice { level, key, args }));
    }

    /// One notice per kind until a request succeeds again.
    fn notice_once(
        &mut self,
        level: NoticeLevel,
        key: &'static str,
        args: Vec<(&'static str, String)>,
    ) {
        if self.noticed.insert(key) {
            self.notice(level, key, args);
        }
    }

    fn fail(&mut self, error: SpotifyError) {
        tracing::warn!("Spotify: {error}");
        match error {
            SpotifyError::Network(_) => {
                self.notice_once(NoticeLevel::Warning, "spotify-offline", vec![]);
            }
            SpotifyError::RateLimited { retry_after }
            | SpotifyError::QuotaExceeded { retry_after } => {
                self.paused_until = Some(Instant::now() + retry_after);
                let seconds = retry_after.as_secs().max(1).to_string();
                self.notice_once(
                    NoticeLevel::Warning,
                    "spotify-rate-limited",
                    vec![("seconds", seconds)],
                );
            }
            SpotifyError::PremiumRequired => {
                self.notice_once(NoticeLevel::Warning, "spotify-premium-required", vec![]);
            }
            SpotifyError::NoActiveDevice => {
                self.notice_once(NoticeLevel::Warning, "spotify-no-device", vec![]);
            }
            SpotifyError::NotAllowListed => {
                self.sign_out();
                self.notice_once(NoticeLevel::Error, "spotify-not-allow-listed", vec![]);
            }
            SpotifyError::SessionEnded => {
                self.sign_out();
                self.notice_once(NoticeLevel::Error, "spotify-session-ended", vec![]);
            }
            SpotifyError::Store(_) => {
                self.notice_once(NoticeLevel::Warning, "spotify-keyring-failed", vec![]);
            }
            SpotifyError::Api { message, .. } | SpotifyError::BadAnswer(message) => {
                self.notice_once(
                    NoticeLevel::Warning,
                    "spotify-error",
                    vec![("message", message)],
                );
            }
            SpotifyError::Random => {
                self.notice_once(
                    NoticeLevel::Error,
                    "spotify-error",
                    vec![("message", "no random numbers".to_owned())],
                );
            }
            SpotifyError::PortBusy
            | SpotifyError::SignInDenied
            | SpotifyError::SignInTimedOut
            | SpotifyError::SignInCancelled => self.fail_sign_in(error),
        }
    }
}

fn same_playback(last: &SpotifyPlayback, new: &PlaybackState, now: Instant) -> bool {
    let old = &last.state;
    if old.device != new.device
        || old.item != new.item
        || old.playing != new.playing
        || old.shuffle != new.shuffle
        || old.repeat != new.repeat
    {
        return false;
    }
    let elapsed = if old.playing {
        u64::try_from(now.saturating_duration_since(last.at).as_millis()).unwrap_or(u64::MAX)
    } else {
        0
    };
    let expected = old.progress_ms.saturating_add(elapsed);
    new.progress_ms.abs_diff(expected) < DRIFT_MS
}

/// The covers thread: fetches and decodes, only while signed in.
fn covers(
    http: &Http,
    endpoints: &Endpoints,
    urls: &Receiver<String>,
    decoded: &Sender<Cover>,
    signed_in: &AtomicBool,
) {
    for url in urls {
        if !signed_in.load(Ordering::SeqCst) {
            continue;
        }
        match fetch_image(http, endpoints, &url) {
            Ok(bytes) => match decode_cover(url, &bytes) {
                Some(cover) => {
                    if decoded.send(cover).is_err() {
                        return;
                    }
                }
                None => tracing::debug!("a Spotify cover could not be decoded"),
            },
            Err(e) => tracing::debug!("a Spotify cover could not be fetched: {e}"),
        }
    }
}

fn decode_cover(url: String, bytes: &[u8]) -> Option<Cover> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(COVER_MAX_SIDE);
    limits.max_image_height = Some(COVER_MAX_SIDE);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let image = if image.width() > COVER_SIZE || image.height() > COVER_SIZE {
        image.thumbnail(COVER_SIZE, COVER_SIZE)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Some(Cover {
        url,
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_spotify::MemoryStore;
    use aria_spotify::fake::{FakeSpotify, Reply};
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::sync::Mutex;

    const WAIT: Duration = Duration::from_secs(5);

    /// A store the test can look into.
    struct Shared(Arc<MemoryStore>);

    impl TokenStore for Shared {
        fn load(&self) -> Result<Option<Secret>, SpotifyError> {
            self.0.load()
        }
        fn save(&self, token: &Secret) -> Result<(), SpotifyError> {
            self.0.save(token)
        }
        fn delete(&self) -> Result<(), SpotifyError> {
            self.0.delete()
        }
        fn persistent(&self) -> bool {
            false
        }
    }

    type Browser = OpenBrowser;

    fn no_browser() -> Browser {
        Box::new(|_| Ok(()))
    }

    /// Plays the browser: follows the authorize URL's redirect with a code.
    fn browser() -> Browser {
        Box::new(|url: &str| {
            let query = url.split_once('?').map_or("", |(_, q)| q);
            let (mut state, mut port) = (String::new(), 0u16);
            for pair in query.split('&') {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                if key == "state" {
                    state = value.to_owned();
                } else if key == "redirect_uri" {
                    let decoded = value.replace("%3A", ":").replace("%2F", "/");
                    port = decoded
                        .trim_start_matches("http://127.0.0.1:")
                        .trim_end_matches("/callback")
                        .parse()
                        .unwrap();
                }
            }
            thread::spawn(move || {
                let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
                write!(
                    stream,
                    "GET /callback?code=c0de&state={state} HTTP/1.1\r\nHost: x\r\n\r\n"
                )
                .unwrap();
            });
            Ok(())
        })
    }

    struct Harness {
        fake: FakeSpotify,
        store: Arc<MemoryStore>,
        handle: Option<SpotifyHandle>,
        events: Receiver<SpotifyEvent>,
        seen: Vec<SpotifyEvent>,
    }

    impl Harness {
        fn start(fake: FakeSpotify, store: MemoryStore, browser: Browser, port: u16) -> Harness {
            let store = Arc::new(store);
            let deps = SpotifyDeps {
                endpoints: fake.endpoints(),
                store: Box::new(Shared(Arc::clone(&store))),
                open_browser: browser,
                redirect_port: port,
                sign_in_timeout: WAIT,
                search_quiet: Duration::from_millis(50),
                fast: Duration::from_millis(100),
                slow: Duration::from_secs(1),
                after_command: Duration::from_millis(20),
            };
            let (tx, events) = unbounded();
            let tx = Mutex::new(tx);
            let handle = SpotifyService::start(
                deps,
                Box::new(move |event| {
                    let _ = tx.lock().unwrap().send(event);
                }),
            )
            .unwrap();
            Harness {
                fake,
                store,
                handle: Some(handle),
                events,
                seen: Vec::new(),
            }
        }

        /// A session restored from a stored token, with the whole library loaded.
        fn signed_in() -> Harness {
            let fake = FakeSpotify::start();
            script_library(&fake);
            let mut h = Harness::start(fake, MemoryStore::holding("stored"), no_browser(), 0);
            h.expect(|e| matches!(e, SpotifyEvent::Library(l) if !l.loading));
            h
        }

        fn send(&self, request: SpotifyRequest) {
            self.handle.as_ref().unwrap().send(request);
        }

        fn expect(&mut self, wanted: impl Fn(&SpotifyEvent) -> bool) -> SpotifyEvent {
            let deadline = Instant::now() + WAIT;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match self.events.recv_timeout(left) {
                    Ok(event) => {
                        self.seen.push(event.clone());
                        if wanted(&event) {
                            return event;
                        }
                    }
                    Err(_) => panic!("not seen; got {:#?}", self.seen),
                }
            }
        }

        /// Collects the events of the next `time`.
        fn during(&mut self, time: Duration) -> Vec<SpotifyEvent> {
            let deadline = Instant::now() + time;
            let mut got = Vec::new();
            while let Ok(event) = self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                got.push(event);
            }
            self.seen.extend(got.iter().cloned());
            got
        }

        fn count(&self, method: &str, path: &str) -> usize {
            self.fake
                .requests()
                .iter()
                .filter(|s| s.method == method && s.path == path)
                .count()
        }

        fn stored(&self) -> Option<String> {
            self.store.load().unwrap().map(|s| s.expose().to_owned())
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                handle.join(WAIT);
            }
        }
    }

    fn script_library(fake: &FakeSpotify) {
        fake.reply("POST", "/api/token", Reply::fixture(200, "token"));
        fake.reply("GET", "/v1/me", Reply::fixture(200, "me"));
        for page in ["saved-tracks-1", "saved-tracks-2", "saved-tracks-3"] {
            fake.reply("GET", "/v1/me/tracks", Reply::fixture(200, page));
        }
        fake.reply("GET", "/v1/me/albums", Reply::fixture(200, "saved-albums"));
        fake.reply(
            "GET",
            "/v1/me/following",
            Reply::fixture(200, "followed-artists"),
        );
        fake.reply(
            "GET",
            "/v1/me/following",
            Reply::fixture(200, "followed-artists-2"),
        );
        fake.reply("GET", "/v1/me/playlists", Reply::fixture(200, "playlists"));
        fake.reply(
            "GET",
            "/v1/me/player/devices",
            Reply::fixture(200, "devices"),
        );
        fake.reply("GET", "/v1/me/player", Reply::empty(204));
    }

    fn is_notice(event: &SpotifyEvent, key: &str) -> bool {
        matches!(event, SpotifyEvent::Notice(n) if n.key == key)
    }

    fn notices(events: &[SpotifyEvent], key: &str) -> usize {
        events.iter().filter(|e| is_notice(e, key)).count()
    }

    fn signed_out(event: &SpotifyEvent) -> bool {
        matches!(
            event,
            SpotifyEvent::Account(SpotifyAccount::SignedOut { .. })
        )
    }

    #[test]
    fn signed_out_at_start_makes_no_request() {
        let mut h = Harness::start(FakeSpotify::start(), MemoryStore::new(), no_browser(), 0);
        let event = h.expect(|e| matches!(e, SpotifyEvent::Account(_)));
        assert!(matches!(
            event,
            SpotifyEvent::Account(SpotifyAccount::SignedOut { keyring: false })
        ));
        h.send(SpotifyRequest::Search("anything".to_owned()));
        h.send(SpotifyRequest::Cover(
            "https://i.scdn.co/image/x".to_owned(),
        ));
        h.send(SpotifyRequest::Next);
        h.during(Duration::from_millis(300));
        assert!(h.fake.requests().is_empty(), "{:?}", h.fake.requests());
    }

    #[test]
    fn a_stored_token_signs_in_and_loads_the_library() {
        let h = Harness::signed_in();
        assert!(h.seen.iter().any(|e| matches!(
            e,
            SpotifyEvent::Account(SpotifyAccount::SignedIn { name }) if name == "Aria Tester"
        )));
        let Some(SpotifyEvent::Library(library)) = h
            .seen
            .iter()
            .rev()
            .find(|e| matches!(e, SpotifyEvent::Library(_)))
            .cloned()
        else {
            panic!("no library");
        };
        assert_eq!(library.tracks.len(), 107);
        assert_eq!(library.albums.len(), 2);
        assert_eq!(library.artists.len(), 3);
        assert_eq!(library.playlists.len(), 2);
        assert!(
            h.seen
                .iter()
                .any(|e| matches!(e, SpotifyEvent::Devices(d) if d.len() == 3))
        );
        assert_eq!(
            h.stored().as_deref(),
            Some("fixture-refresh-token-1"),
            "rotated and saved"
        );
    }

    #[test]
    fn an_ended_session_signs_out_with_a_notice() {
        let fake = FakeSpotify::start();
        fake.reply(
            "POST",
            "/api/token",
            Reply::json(
                400,
                r#"{"error":"invalid_grant","error_description":"revoked"}"#,
            ),
        );
        let mut h = Harness::start(fake, MemoryStore::holding("stored"), no_browser(), 0);
        h.expect(|e| is_notice(e, "spotify-session-ended"));
        assert!(h.seen.iter().any(signed_out));
        assert_eq!(h.stored(), None);
    }

    #[test]
    fn sign_in_saves_the_refresh_token() {
        let fake = FakeSpotify::start();
        script_library(&fake);
        let mut h = Harness::start(fake, MemoryStore::new(), browser(), 0);
        h.expect(signed_out);
        h.send(SpotifyRequest::SignIn {
            done_page: "<p>done</p>".to_owned(),
        });
        h.expect(|e| matches!(e, SpotifyEvent::Account(SpotifyAccount::SigningIn)));
        h.expect(|e| matches!(e, SpotifyEvent::Account(SpotifyAccount::SignedIn { .. })));
        assert_eq!(h.stored().as_deref(), Some("fixture-refresh-token-1"));
        let exchange = h
            .fake
            .requests()
            .into_iter()
            .find(|s| s.path == "/api/token")
            .unwrap();
        assert!(exchange.body.contains("code=c0de"), "{}", exchange.body);
        assert!(exchange.body.contains("code_verifier="));
    }

    #[test]
    fn sign_out_deletes_the_token_and_forgets_everything() {
        let mut h = Harness::signed_in();
        h.send(SpotifyRequest::Cadence(Cadence::Fast));
        h.send(SpotifyRequest::SignOut);
        h.expect(signed_out);
        h.expect(|e| matches!(e, SpotifyEvent::Library(l) if l.tracks.is_empty() && !l.loading));
        h.expect(|e| matches!(e, SpotifyEvent::Playback(None)));
        h.expect(|e| matches!(e, SpotifyEvent::Devices(d) if d.is_empty()));
        assert_eq!(h.stored(), None);
        let before = h.fake.requests().len();
        h.send(SpotifyRequest::Next);
        h.send(SpotifyRequest::Search("x".to_owned()));
        h.during(Duration::from_millis(300));
        assert_eq!(h.fake.requests().len(), before, "no request after sign-out");
    }

    #[test]
    fn cancelling_sign_in_returns_to_signed_out_quietly() {
        let mut h = Harness::start(FakeSpotify::start(), MemoryStore::new(), no_browser(), 0);
        h.expect(signed_out);
        h.send(SpotifyRequest::SignIn {
            done_page: String::new(),
        });
        h.expect(|e| matches!(e, SpotifyEvent::Account(SpotifyAccount::SigningIn)));
        h.send(SpotifyRequest::CancelSignIn);
        h.expect(signed_out);
        let later = h.during(Duration::from_millis(300));
        assert!(
            !later.iter().any(|e| matches!(e, SpotifyEvent::Notice(_))),
            "{later:?}"
        );
    }

    #[test]
    fn a_busy_port_gives_its_notice() {
        let taken = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let opened = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&opened);
        let browser: Browser = Box::new(move |_| {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });
        let mut h = Harness::start(FakeSpotify::start(), MemoryStore::new(), browser, port);
        h.expect(signed_out);
        h.send(SpotifyRequest::SignIn {
            done_page: String::new(),
        });
        h.expect(|e| is_notice(e, "spotify-port-busy"));
        assert!(
            !opened.load(Ordering::SeqCst),
            "no browser without a listener"
        );
        drop(taken);
    }

    #[test]
    fn search_waits_for_quiet_and_drops_stale_answers() {
        let mut h = Harness::signed_in();
        h.fake
            .reply("GET", "/v1/search", Reply::fixture(200, "search"));
        for query in ["d", "da", "daft"] {
            h.send(SpotifyRequest::Search(query.to_owned()));
            thread::sleep(Duration::from_millis(10));
        }
        let event = h.expect(|e| matches!(e, SpotifyEvent::Search(_)));
        let SpotifyEvent::Search(search) = event else {
            unreachable!()
        };
        assert_eq!(search.query, "daft");
        assert_eq!(search.results.tracks.len(), 2);
        let searches: Vec<_> = h
            .fake
            .requests()
            .into_iter()
            .filter(|s| s.path == "/v1/search")
            .collect();
        assert_eq!(searches.len(), 1);
        assert_eq!(searches[0].param("q"), Some("daft"));
        h.send(SpotifyRequest::Search("  ".to_owned()));
        h.expect(|e| matches!(e, SpotifyEvent::Search(s) if s.results.is_empty()));
        assert_eq!(
            h.count("GET", "/v1/search"),
            1,
            "an empty query asks nothing"
        );
    }

    #[test]
    fn cadence_off_stops_polling() {
        let mut h = Harness::signed_in();
        h.send(SpotifyRequest::Cadence(Cadence::Off));
        h.during(Duration::from_millis(100));
        let before = h.count("GET", "/v1/me/player");
        h.during(Duration::from_millis(400));
        assert_eq!(h.count("GET", "/v1/me/player"), before);
    }

    #[test]
    fn fast_polls_more_than_slow() {
        let mut h = Harness::signed_in();
        h.send(SpotifyRequest::Cadence(Cadence::Fast));
        h.during(Duration::from_millis(50));
        let start = h.count("GET", "/v1/me/player");
        h.during(Duration::from_millis(600));
        let fast = h.count("GET", "/v1/me/player") - start;
        h.send(SpotifyRequest::Cadence(Cadence::Slow));
        h.during(Duration::from_millis(150));
        let start = h.count("GET", "/v1/me/player");
        h.during(Duration::from_millis(600));
        let slow = h.count("GET", "/v1/me/player") - start;
        assert!(fast >= 4, "fast {fast}");
        assert!(slow <= 1, "slow {slow}");
    }

    #[test]
    fn a_command_is_followed_by_a_poll() {
        let mut h = Harness::signed_in();
        h.send(SpotifyRequest::Cadence(Cadence::Off));
        h.fake
            .reply("POST", "/v1/me/player/next", Reply::empty(204));
        h.during(Duration::from_millis(100));
        let before = h.count("GET", "/v1/me/player");
        h.send(SpotifyRequest::Next);
        h.during(Duration::from_millis(300));
        let seen = h.fake.requests();
        let next = seen
            .iter()
            .position(|s| s.path == "/v1/me/player/next")
            .unwrap();
        assert!(seen[next..].iter().any(|s| s.path == "/v1/me/player"));
        assert_eq!(h.count("GET", "/v1/me/player"), before + 1);
    }

    #[test]
    fn the_chosen_device_is_used_and_woken() {
        let mut h = Harness::signed_in();
        h.send(SpotifyRequest::Cadence(Cadence::Off));
        h.fake.reply(
            "PUT",
            "/v1/me/player/play",
            Reply::fixture(404, "error-no-device"),
        );
        h.fake.reply("PUT", "/v1/me/player/play", Reply::empty(204));
        h.fake.reply("PUT", "/v1/me/player", Reply::empty(204));
        h.send(SpotifyRequest::Device(Some("device-desk".to_owned())));
        h.send(SpotifyRequest::Play(PlayRequest::Context {
            uri: "spotify:album:albumA".to_owned(),
            offset_uri: None,
        }));
        let events = h.during(Duration::from_millis(400));
        let commands: Vec<_> = h
            .fake
            .requests()
            .into_iter()
            .filter(|s| s.method == "PUT")
            .collect();
        assert_eq!(commands.len(), 3, "{commands:?}");
        assert_eq!(commands[0].param("device_id"), Some("device-desk"));
        assert_eq!(commands[1].path, "/v1/me/player");
        assert!(commands[1].body.contains("device-desk"));
        assert_eq!(commands[2].param("device_id"), Some("device-desk"));
        assert_eq!(notices(&events, "spotify-no-device"), 0);
    }

    #[test]
    fn a_lost_network_gives_one_notice() {
        let mut h = Harness::signed_in();
        h.send(SpotifyRequest::Cadence(Cadence::Fast));
        h.during(Duration::from_millis(150));
        h.fake.set_offline(true);
        let offline = h.during(Duration::from_millis(700));
        assert_eq!(notices(&offline, "spotify-offline"), 1, "{offline:?}");
        h.fake.set_offline(false);
        let back = h.during(Duration::from_millis(400));
        assert_eq!(notices(&back, "spotify-offline"), 0);
        h.fake.set_offline(true);
        let again = h.during(Duration::from_millis(400));
        assert_eq!(
            notices(&again, "spotify-offline"),
            1,
            "a second loss, a second notice"
        );
        h.fake.set_offline(false);
    }

    #[test]
    fn a_long_rate_limit_pauses_polling() {
        let mut h = Harness::signed_in();
        h.fake.clear();
        h.fake.reply(
            "GET",
            "/v1/me/player",
            Reply::fixture(429, "error-rate").header("Retry-After", "120"),
        );
        h.send(SpotifyRequest::Cadence(Cadence::Fast));
        let event = h.expect(|e| is_notice(e, "spotify-rate-limited"));
        let SpotifyEvent::Notice(notice) = event else {
            unreachable!()
        };
        assert_eq!(notice.args, vec![("seconds", "120".to_owned())]);
        let after = h.count("GET", "/v1/me/player");
        h.during(Duration::from_millis(500));
        assert_eq!(
            h.count("GET", "/v1/me/player"),
            after,
            "no polling while asked to wait"
        );
    }

    #[test]
    fn not_allow_listed_signs_out() {
        let fake = FakeSpotify::start();
        fake.reply("POST", "/api/token", Reply::fixture(200, "token"));
        fake.reply("GET", "/v1/me", Reply::fixture(403, "error-not-registered"));
        let mut h = Harness::start(fake, MemoryStore::holding("stored"), no_browser(), 0);
        h.expect(|e| is_notice(e, "spotify-not-allow-listed"));
        assert!(h.seen.iter().any(signed_out));
        assert_eq!(h.stored(), None, "no token is kept");
    }

    #[test]
    fn covers_are_decoded_and_scaled() {
        let mut h = Harness::signed_in();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(300, 300, image::Rgba([200, 10, 10, 255]))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        h.fake
            .reply("GET", "/img/cover.png", Reply::bytes(200, "image/png", png));
        let url = format!("{}/img/cover.png", h.fake.base());
        h.send(SpotifyRequest::Cover(url.clone()));
        let SpotifyEvent::Cover(cover) = h.expect(|e| matches!(e, SpotifyEvent::Cover(_))) else {
            unreachable!()
        };
        assert_eq!(cover.url, url);
        assert_eq!((cover.width, cover.height), (128, 128));
        assert_eq!(cover.rgba.len(), 128 * 128 * 4);
        let cover_request = h
            .fake
            .requests()
            .into_iter()
            .find(|s| s.path == "/img/cover.png")
            .unwrap();
        assert_eq!(
            cover_request.authorization, None,
            "covers go without the token"
        );
    }
}
