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
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use aria_spotify::player_page::{PageError, PageEvent, PageMsg};
use aria_spotify::{
    Callback, Client, Endpoints, Http, Pkce, PlayRequest, PlaybackState, RepeatState,
    SearchResults, Secret, SpAlbum, SpArtist, SpDevice, SpPlaylist, SpTrack, SpotifyError,
    TokenStore, Tokens, authorize_url, exchange_code, fetch_image, redirect_uri, refresh,
    system_store,
};
use crossbeam_channel::{Receiver, Sender, bounded, never, select, unbounded};

use crate::notice::{Notice, NoticeLevel};
use crate::runtime::{DoneGuard, join_bounded};
use crate::spotify_player::{ARIA_DEVICE, AriaPlayerStatus, Due, Launch, Player, PlayerDeps};

const THREAD_NAME: &str = "aria-spotify";
const COVERS_THREAD: &str = "aria-spotify-covers";
const SIGN_IN_THREAD: &str = "aria-spotify-sign-in";
/// Cover requests waiting to be fetched; more are dropped (the UI asks again).
const COVERS_QUEUE: usize = 64;
/// Covers are scaled down to fit in this many pixels.
const COVER_SIZE: u32 = 128;
/// Large covers, for the full-screen record, fit in this many pixels.
const LARGE_COVER: u32 = 1024;
/// A cover may claim no more than this many pixels on a side before it is decoded.
const COVER_MAX_SIDE: u32 = 4096;
/// A change in `progress_ms` smaller than this is drift, not news.
const DRIFT_MS: u64 = 1500;
/// How long the loop sleeps when nothing is due.
const IDLE: Duration = Duration::from_secs(3600);
/// The page's token is refreshed first when it would expire within this.
const PAGE_TOKEN_MARGIN: Duration = Duration::from_secs(60);

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
    /// Aria's own player window.
    pub player: PlayerDeps,
}

impl SpotifyDeps {
    /// Spotify itself, the system keyring and the default browser. Aria's player keeps its
    /// browser profile in `cache_dir/spotify-player`; without a cache folder there is no player.
    pub fn system(cache_dir: Option<PathBuf>) -> SpotifyDeps {
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
            player: PlayerDeps::system(cache_dir.map(|dir| dir.join("spotify-player"))),
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
    /// Opens the browser to sign in. The browser then shows `done_page` (HTML), or
    /// `failed_page` when the sign-in did not finish.
    SignIn {
        done_page: String,
        failed_page: String,
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
    /// The device to play on; `None` is Spotify's active one, [`ARIA_DEVICE`] Aria's player.
    Device(Option<String>),
    /// Whether Spotify may play in Aria's own player window; `strings` are its page's texts
    /// (a JSON object), used from the next launch on.
    PlayInAria {
        on: bool,
        strings: String,
    },
    ListDevices,
    /// Saves the track to Liked Songs (`liked`) or removes it. Liked Songs change at once and
    /// change back if Spotify refuses.
    Like {
        track: SpTrack,
        liked: bool,
    },
    /// Loads the tracks of the playlist with this id (one the user owns or collaborates on).
    OpenPlaylist(String),
    /// Adds the track `uri` at the end of the playlist `playlist` (an id) called `name`.
    AddToPlaylist {
        playlist: String,
        name: String,
        uri: String,
    },
    /// Removes every copy of the track `uri` from the playlist `playlist` (an id).
    RemoveFromPlaylist {
        playlist: String,
        uri: String,
    },
    /// Creates a private playlist, starting with the track `first` if given.
    CreatePlaylist {
        name: String,
        first: Option<String>,
    },
    Cadence(Cadence),
    /// Asks for a cover image by URL.
    Cover(String),
    /// Asks for a large cover (for the full-screen record) by URL.
    LargeCover(String),
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
        /// Whether the session may like tracks and edit playlists (it has the permissions).
        can_edit: bool,
    },
}

/// The playlist the user opened: its tracks as last loaded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpotifyPlaylist {
    pub id: String,
    pub tracks: Vec<SpTrack>,
    /// The version last known, sent along with a removal.
    pub snapshot: Option<String>,
    pub loading: bool,
    /// The last load failed (Spotify refused, or no network).
    pub failed: bool,
}

/// The Spotify library, kept in memory only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpotifyLibrary {
    pub tracks: Vec<SpTrack>,
    pub albums: Vec<SpAlbum>,
    pub artists: Vec<SpArtist>,
    pub playlists: Vec<SpPlaylist>,
    pub loading: bool,
    /// The last load could not get every list; what is shown of those is from before.
    pub failed: bool,
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
    /// Decoded for the full-screen record ([`SpotifyRequest::LargeCover`]).
    pub large: bool,
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
    /// Spotify's devices, without Aria's own player (the UI lists that itself).
    Devices(Vec<SpDevice>),
    Player(AriaPlayerStatus),
    /// The open playlist; `None` once nothing is open (after signing out).
    Playlist(Option<Arc<SpotifyPlaylist>>),
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
        let mut deps = deps;
        let player = Player::new(std::mem::replace(
            &mut deps.player,
            PlayerDeps::unavailable(),
        ));
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
            library: Arc::default(),
            last: None,
            noticed: HashSet::new(),
            covers: covers_tx,
            signed_in,
            player,
            player_shown: None,
            needs_reconnect: false,
            me_id: None,
            playlist: None,
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
    /// The library last loaded in full.
    library: Arc<SpotifyLibrary>,
    last: Option<SpotifyPlayback>,
    /// Notice keys shown since the last request that succeeded.
    noticed: HashSet<&'static str>,
    covers: Sender<(String, u32)>,
    signed_in: Arc<AtomicBool>,
    player: Player,
    /// The player's status last sent.
    player_shown: Option<AriaPlayerStatus>,
    /// A play wanted the player but the session lacks `streaming`.
    needs_reconnect: bool,
    /// The signed-in user's id, for which playlists are theirs.
    me_id: Option<String>,
    /// The playlist the user opened.
    playlist: Option<SpotifyPlaylist>,
}

impl Service {
    fn run(mut self, requests: &Receiver<SpotifyRequest>, decoded: &Receiver<Cover>) {
        self.start();
        let mut covers_alive = true;
        let page = self.player.messages();
        loop {
            let deadline = [
                self.poll_deadline(),
                self.search.as_ref().map(|(_, at)| *at),
                self.player.deadline(),
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
                recv(page) -> msg => {
                    if let Ok(msg) = msg {
                        self.page_message(msg);
                    }
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
        self.player.close();
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
                let account = self.signed_out();
                self.set_account(account);
            }
            return;
        };
        self.signed_in.store(true, Ordering::SeqCst);
        self.needs_reconnect = false;
        self.sync_player();
        self.close_playlist();
        self.me_id = Some(me.id);
        let can_edit = self.client.as_ref().is_some_and(|c| c.tokens().can_edit());
        self.set_account(SpotifyAccount::SignedIn {
            name: me.name,
            can_edit,
        });
        self.list_devices();
        self.load_library();
        if self.client.is_some() {
            self.next_poll = self.interval().map(|_| Instant::now());
        }
    }

    fn begin_sign_in(&mut self, done_page: String, failed_page: String) {
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
                let _ = tx.send(callback.wait(&state, timeout, &flag, &done_page, &failed_page));
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
        self.player.close();
        self.needs_reconnect = false;
        self.sync_player();
        self.library = Arc::default();
        self.me_id = None;
        self.close_playlist();
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
            SpotifyRequest::SignIn {
                done_page,
                failed_page,
            } => self.begin_sign_in(done_page, failed_page),
            SpotifyRequest::CancelSignIn => self.cancel_sign_in(),
            SpotifyRequest::SignOut => self.sign_out(),
            SpotifyRequest::Refresh => {
                self.list_devices();
                self.load_library();
            }
            SpotifyRequest::Search(query) => self.queue_search(query),
            SpotifyRequest::Play(request) => self.play(request),
            SpotifyRequest::TogglePlay => {
                if self.last.as_ref().is_some_and(|p| p.state.playing) {
                    self.command(Client::pause);
                } else {
                    self.play(PlayRequest::Resume);
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
            SpotifyRequest::PlayInAria { on, strings } => {
                self.player.set(on, strings);
                if !on {
                    self.player.close();
                }
                self.sync_player();
            }
            SpotifyRequest::ListDevices => self.list_devices(),
            SpotifyRequest::Like { track, liked } => self.like(track, liked),
            SpotifyRequest::OpenPlaylist(id) => self.open_playlist(id),
            SpotifyRequest::AddToPlaylist {
                playlist,
                name,
                uri,
            } => self.add_to_playlist(&playlist, name, &uri),
            SpotifyRequest::RemoveFromPlaylist { playlist, uri } => {
                self.remove_from_playlist(&playlist, &uri);
            }
            SpotifyRequest::CreatePlaylist { name, first } => self.create_playlist(&name, first),
            SpotifyRequest::Cadence(cadence) => self.set_cadence(cadence),
            SpotifyRequest::Cover(url) => {
                if self.client.is_some() {
                    // A full queue drops the request; the UI asks again.
                    let _ = self.covers.try_send((url, COVER_SIZE));
                }
            }
            SpotifyRequest::LargeCover(url) => {
                if self.client.is_some() {
                    // A full queue drops the request; the UI asks again.
                    let _ = self.covers.try_send((url, LARGE_COVER));
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

    /// The chosen device, if Spotify still lists it; else `None` (the active device). Aria's
    /// player, when chosen, is its page's device while that is ready.
    fn target(&self) -> Option<String> {
        if self.chosen.as_deref() == Some(ARIA_DEVICE) {
            return self.player.ready_device().map(str::to_owned);
        }
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
        self.poll_soon();
    }

    /// Asks what is playing soon after a command.
    fn poll_soon(&mut self) {
        if self.client.is_some() {
            let at = Instant::now() + self.deps.after_command;
            self.next_poll = Some(self.next_poll.map_or(at, |t| t.min(at)));
        }
    }

    // --- Aria's own player ---

    /// Plays `request`. With nothing chosen and no device active, or with Aria's player chosen,
    /// it goes to Aria's player, which is opened when needed.
    fn play(&mut self, request: PlayRequest) {
        if self.client.is_none() {
            return;
        }
        let aria_chosen = self.chosen.as_deref() == Some(ARIA_DEVICE);
        let may_use_aria = self.player.available() && (aria_chosen || self.chosen.is_none());
        if !may_use_aria {
            return self.command(|c, d| c.play(d, &request));
        }
        let waiting = self.player.is_starting() || self.player.lost_device().is_some();
        if self.player.ready_device().is_none() && (aria_chosen || waiting) {
            return self.play_in_aria(request);
        }
        let target = self.target();
        let outcome = self
            .client
            .as_mut()
            .map(|client| client.play(target.as_deref(), &request));
        self.keep_refresh_token();
        match outcome {
            Some(Err(SpotifyError::NoActiveDevice)) => self.play_in_aria(request),
            Some(Ok(())) => self.noticed.clear(),
            Some(Err(e)) => self.fail(e),
            None => {}
        }
        self.poll_soon();
    }

    /// Plays `request` in Aria's player: on its page's device when ready, on a quiet page's
    /// device that Spotify still lists, or in a window opened now.
    fn play_in_aria(&mut self, request: PlayRequest) {
        if !self
            .client
            .as_ref()
            .is_some_and(|c| c.tokens().can_stream())
        {
            self.needs_reconnect = true;
            self.notice_once(NoticeLevel::Warning, "spotify-player-reconnect", vec![]);
            return self.sync_player();
        }
        if let Some(id) = self.player.ready_device().map(str::to_owned) {
            self.play_on(&id, &request);
            return self.sync_player();
        }
        if let Some(id) = self.player.lost_device().map(str::to_owned) {
            self.list_devices();
            if self
                .devices
                .iter()
                .any(|d| d.id.as_deref() == Some(id.as_str()))
            {
                self.player.revive();
                self.play_on(&id, &request);
                return self.sync_player();
            }
        }
        if self.player.is_starting() {
            return self.player.wait_with(request);
        }
        match self.player.launch(request) {
            Launch::Window => {
                self.notice(NoticeLevel::Info, "spotify-player-starting", vec![]);
            }
            Launch::Tab => self.notice(NoticeLevel::Info, "spotify-player-tab", vec![]),
            Launch::Failed(e) => {
                tracing::warn!("couldn't start Aria's Spotify player: {e}");
                self.notice(NoticeLevel::Warning, "spotify-player-launch-failed", vec![]);
            }
        }
        self.sync_player();
    }

    /// Plays on the player's device `id`, moving playback there first if Spotify asks.
    fn play_on(&mut self, id: &str, request: &PlayRequest) {
        self.call(|c| match c.play(Some(id), request) {
            Err(SpotifyError::NoActiveDevice) => {
                c.transfer(id, true)?;
                c.play(Some(id), request)
            }
            result => result,
        });
        self.poll_soon();
    }

    fn page_message(&mut self, msg: PageMsg) {
        self.player.heard();
        match msg {
            PageMsg::Token(reply) => {
                let token = self
                    .client
                    .as_mut()
                    .map(|c| c.fresh_access(PAGE_TOKEN_MARGIN));
                self.keep_refresh_token();
                match token {
                    Some(Ok(token)) => {
                        // The page may have gone; then nobody needs it.
                        let _ = reply.send(Some(token));
                    }
                    Some(Err(e)) => {
                        let _ = reply.send(None);
                        self.fail(e);
                    }
                    None => {
                        let _ = reply.send(None);
                    }
                }
            }
            PageMsg::Event(PageEvent::Ready { device_id }) => {
                let pending = self.player.ready(device_id.clone());
                self.sync_player();
                if let Some(request) = pending {
                    self.call(|c| {
                        c.transfer(&device_id, true)?;
                        if matches!(request, PlayRequest::Resume) {
                            Ok(())
                        } else {
                            c.play(Some(&device_id), &request)
                        }
                    });
                    self.poll_soon();
                }
                self.list_devices();
            }
            PageMsg::Event(PageEvent::NotReady { device_id }) => {
                tracing::debug!("Aria's Spotify player went offline");
                self.player.not_ready(&device_id);
            }
            PageMsg::Event(PageEvent::Error { kind, message }) => {
                tracing::debug!("Aria's Spotify player: {kind:?}: {message}");
                let key = match kind {
                    PageError::Authentication => "spotify-player-reconnect",
                    PageError::Account => "spotify-player-premium",
                    PageError::Initialization => "spotify-player-no-drm",
                    PageError::Playback => "spotify-player-failed",
                };
                self.notice_once(NoticeLevel::Warning, key, vec![]);
            }
            PageMsg::Event(PageEvent::AutoplayFailed) => {
                self.notice_once(NoticeLevel::Warning, "spotify-player-autoplay", vec![]);
            }
            PageMsg::Event(PageEvent::Alive) => {}
        }
        self.sync_player();
    }

    fn player_due(&mut self, now: Instant) {
        match self.player.due(now) {
            Some(Due::TimedOut) => {
                self.notice(NoticeLevel::Warning, "spotify-player-timeout", vec![]);
                self.player.close();
            }
            Some(Due::Lost { exited: true }) => {
                self.notice(NoticeLevel::Info, "spotify-player-closed", vec![]);
            }
            Some(Due::Lost { exited: false }) | None => {}
        }
        self.sync_player();
    }

    /// Tells the UI the player's status when it changed.
    fn sync_player(&mut self) {
        let status = match self.player.status() {
            AriaPlayerStatus::Off if self.needs_reconnect => AriaPlayerStatus::NeedsReconnect,
            status => status,
        };
        if self.player_shown.as_ref() != Some(&status) {
            self.player_shown = Some(status.clone());
            self.emit(SpotifyEvent::Player(status));
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
            let own = self.player.own_device().map(str::to_owned);
            let shown = devices
                .into_iter()
                .filter(|d| d.id.is_none() || d.id != own)
                .collect();
            self.emit(SpotifyEvent::Devices(shown));
        }
    }

    /// Sends the library as it fills: playlists, albums and artists first, the saved tracks
    /// (the longest list) last.
    fn load_library(&mut self) {
        if self.client.is_none() {
            return;
        }
        // A list that cannot be loaded keeps what it had.
        let mut library = SpotifyLibrary {
            loading: true,
            failed: false,
            ..(*self.library).clone()
        };
        self.emit(SpotifyEvent::Library(Arc::new(library.clone())));
        match self.call(Client::playlists) {
            Some(mut playlists) => {
                for playlist in &mut playlists {
                    playlist.editable = self.is_editable(playlist);
                }
                library.playlists = playlists;
                self.emit(SpotifyEvent::Library(Arc::new(library.clone())));
            }
            None => library.failed = true,
        }
        match self.call(Client::saved_albums) {
            Some(albums) => {
                library.albums = albums;
                self.emit(SpotifyEvent::Library(Arc::new(library.clone())));
            }
            None => library.failed = true,
        }
        match self.call(Client::followed_artists) {
            Some(artists) => {
                library.artists = artists;
                self.emit(SpotifyEvent::Library(Arc::new(library.clone())));
            }
            None => library.failed = true,
        }
        match self.call(Client::saved_tracks) {
            Some(tracks) => library.tracks = tracks,
            None => library.failed = true,
        }
        if self.client.is_some() {
            library.loading = false;
            let library = Arc::new(library);
            self.library = Arc::clone(&library);
            self.emit(SpotifyEvent::Library(library));
        }
    }

    // --- likes and playlists ---

    /// Whether the user may open and change `playlist`: theirs, or collaborative.
    fn is_editable(&self, playlist: &SpPlaylist) -> bool {
        playlist.collaborative || self.me_id.as_deref() == Some(playlist.owner_id.as_str())
    }

    /// Whether the session may edit; if signed in without the permissions, says to reconnect.
    fn may_edit(&mut self) -> bool {
        let Some(client) = self.client.as_ref() else {
            return false;
        };
        if client.tokens().can_edit() {
            return true;
        }
        self.notice(NoticeLevel::Warning, "spotify-reconnect-to-edit", vec![]);
        false
    }

    fn set_library(&mut self, library: SpotifyLibrary) {
        let library = Arc::new(library);
        self.library = Arc::clone(&library);
        self.emit(SpotifyEvent::Library(library));
    }

    fn like(&mut self, track: SpTrack, liked: bool) {
        let Some(uri) = track.uri.clone() else {
            return;
        };
        if !self.may_edit() {
            return;
        }
        let before = Arc::clone(&self.library);
        let mut library = (*self.library).clone();
        let is = |t: &SpTrack| t.uri.as_deref() == Some(uri.as_str());
        if liked {
            if !library.tracks.iter().any(is) {
                library.tracks.insert(0, track);
            }
        } else {
            library.tracks.retain(|t| !is(t));
        }
        self.set_library(library);
        let done = self.call(|c| {
            if liked {
                c.save_track(&uri)
            } else {
                c.unsave_track(&uri)
            }
        });
        // Signed out by the failure: there is nothing to put back.
        if done.is_none() && self.client.is_some() {
            self.library = Arc::clone(&before);
            self.emit(SpotifyEvent::Library(before));
        }
    }

    fn emit_playlist(&self) {
        let open = self.playlist.clone().map(Arc::new);
        self.emit(SpotifyEvent::Playlist(open));
    }

    /// Forgets the open playlist, telling the UI if one was open.
    fn close_playlist(&mut self) {
        if self.playlist.take().is_some() {
            self.emit(SpotifyEvent::Playlist(None));
        }
    }

    fn open_playlist(&mut self, id: String) {
        if self.client.is_none() {
            return;
        }
        let snapshot = self
            .library
            .playlists
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.snapshot.clone());
        self.playlist = Some(SpotifyPlaylist {
            id,
            tracks: Vec::new(),
            snapshot,
            loading: true,
            failed: false,
        });
        self.emit_playlist();
        self.load_playlist();
    }

    /// Loads the open playlist's tracks again; what it had stays if that fails.
    fn load_playlist(&mut self) {
        let Some(id) = self.playlist.as_ref().map(|p| p.id.clone()) else {
            return;
        };
        let tracks = self.call(|c| c.playlist_tracks(&id));
        let Some(open) = self.playlist.as_mut().filter(|p| p.id == id) else {
            return;
        };
        open.loading = false;
        match tracks {
            Some(tracks) => {
                open.tracks = tracks;
                open.failed = false;
            }
            None => open.failed = true,
        }
        self.emit_playlist();
    }

    /// After a change to playlist `id`: keeps its new snapshot and reloads it if it is open.
    fn changed(&mut self, id: &str, snapshot: Option<String>) {
        if let Some(snapshot) = snapshot {
            if let Some(playlist) = Arc::make_mut(&mut self.library)
                .playlists
                .iter_mut()
                .find(|p| p.id == id)
            {
                playlist.snapshot = Some(snapshot.clone());
            }
            if let Some(open) = self.playlist.as_mut().filter(|p| p.id == id) {
                open.snapshot = Some(snapshot);
            }
        }
        if self.playlist.as_ref().is_some_and(|p| p.id == id) {
            self.load_playlist();
        }
    }

    fn add_to_playlist(&mut self, id: &str, name: String, uri: &str) {
        if !self.may_edit() {
            return;
        }
        let Some(snapshot) = self.call(|c| c.add_to_playlist(id, uri)) else {
            return;
        };
        self.notice(NoticeLevel::Info, "spotify-added", vec![("playlist", name)]);
        self.changed(id, snapshot);
    }

    fn remove_from_playlist(&mut self, id: &str, uri: &str) {
        if !self.may_edit() {
            return;
        }
        // No snapshot: Spotify checks one against the items, and the one Aria knows comes from
        // the playlist list, which may be older than the tracks shown.
        let Some(snapshot) = self.call(|c| c.remove_from_playlist(id, uri, None)) else {
            return;
        };
        self.changed(id, snapshot);
    }

    fn create_playlist(&mut self, name: &str, first: Option<String>) {
        if !self.may_edit() {
            return;
        }
        let Some(mut playlist) = self.call(|c| c.create_playlist(name)) else {
            return;
        };
        playlist.editable = true;
        let id = playlist.id.clone();
        let shown = playlist.name.clone();
        let mut library = (*self.library).clone();
        library.playlists.insert(0, playlist);
        self.set_library(library);
        self.notice(
            NoticeLevel::Info,
            "spotify-created",
            vec![("playlist", shown)],
        );
        if let Some(uri) = first
            && let Some(snapshot) = self.call(|c| c.add_to_playlist(&id, &uri))
        {
            self.changed(&id, snapshot);
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
        if self.player.deadline().is_some_and(|d| d <= now) {
            self.player_due(now);
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
    urls: &Receiver<(String, u32)>,
    decoded: &Sender<Cover>,
    signed_in: &AtomicBool,
) {
    for (url, max) in urls {
        if !signed_in.load(Ordering::SeqCst) {
            continue;
        }
        match fetch_image(http, endpoints, &url) {
            Ok(bytes) => match decode_cover(url, &bytes, max) {
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

fn decode_cover(url: String, bytes: &[u8], max: u32) -> Option<Cover> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(COVER_MAX_SIDE);
    limits.max_image_height = Some(COVER_MAX_SIDE);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let image = if image.width() > max || image.height() > max {
        image.thumbnail(max, max)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Some(Cover {
        url,
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
        large: max >= LARGE_COVER,
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
            Harness::start_with(fake, store, browser, port, PlayerDeps::unavailable())
        }

        fn start_with(
            fake: FakeSpotify,
            store: MemoryStore,
            browser: Browser,
            port: u16,
            player: PlayerDeps,
        ) -> Harness {
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
                player,
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

    /// A token whose session may like tracks and edit playlists.
    const EDIT_TOKEN: &str = r#"{"access_token":"edit-access","token_type":"Bearer","scope":"user-read-private user-library-read user-library-modify playlist-modify-public playlist-modify-private","expires_in":3600,"refresh_token":"edit-refresh"}"#;

    impl Harness {
        /// Signed in with the edit permissions; `setup` scripts answers before the library's.
        fn editor_with(setup: impl FnOnce(&FakeSpotify)) -> Harness {
            let fake = FakeSpotify::start();
            fake.reply("POST", "/api/token", Reply::json(200, EDIT_TOKEN));
            setup(&fake);
            script_library(&fake);
            let mut h = Harness::start(fake, MemoryStore::holding("stored"), no_browser(), 0);
            h.expect(|e| matches!(e, SpotifyEvent::Library(l) if !l.loading));
            h
        }

        /// Opens `plA` and waits for its tracks.
        fn open_pl_a(&mut self) {
            self.send(SpotifyRequest::OpenPlaylist("plA".to_owned()));
            self.expect(
                |e| matches!(e, SpotifyEvent::Playlist(Some(p)) if p.id == "plA" && !p.loading),
            );
        }
    }

    fn script_pl_a(fake: &FakeSpotify, times: usize) {
        for _ in 0..times {
            for page in ["playlist-items", "playlist-items-2"] {
                fake.reply("GET", "/v1/playlists/plA/items", Reply::fixture(200, page));
            }
        }
    }

    fn sp_track(uri: &str, name: &str) -> SpTrack {
        SpTrack {
            uri: Some(uri.to_owned()),
            name: name.to_owned(),
            artists: vec!["Someone".to_owned()],
            album: String::new(),
            album_uri: None,
            duration_ms: 1000,
            image: None,
            image_large: None,
        }
    }

    fn first_is(event: &SpotifyEvent, uri: &str) -> bool {
        matches!(event, SpotifyEvent::Library(l)
            if l.tracks.first().and_then(|t| t.uri.as_deref()) == Some(uri))
    }

    fn has(library: &SpotifyLibrary, uri: &str) -> bool {
        library.tracks.iter().any(|t| t.uri.as_deref() == Some(uri))
    }

    const BOOM: &str = r#"{"error":{"status":500,"message":"boom"}}"#;

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
            SpotifyEvent::Account(SpotifyAccount::SignedIn { name, .. }) if name == "Aria Tester"
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
            failed_page: String::new(),
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
            failed_page: String::new(),
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
            failed_page: String::new(),
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
        // Typed faster than the quiet time; no sleep, which a slow runner can overshoot.
        for query in ["d", "da", "daft"] {
            h.send(SpotifyRequest::Search(query.to_owned()));
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
        // Long enough for a slow CI runner: ten polls are due, four are asked for.
        h.during(Duration::from_millis(1000));
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
        let offline = h.during(Duration::from_millis(1000));
        assert_eq!(notices(&offline, "spotify-offline"), 1, "{offline:?}");
        h.fake.set_offline(false);
        let back = h.during(Duration::from_millis(800));
        assert_eq!(notices(&back, "spotify-offline"), 0);
        h.fake.set_offline(true);
        let again = h.during(Duration::from_millis(1000));
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
    fn a_failed_reload_keeps_the_old_list() {
        let mut h = Harness::signed_in();
        h.fake.clear();
        script_library(&h.fake);
        h.fake.clear();
        h.fake.reply(
            "GET",
            "/v1/me/tracks",
            Reply::json(500, r#"{"error":{"status":500,"message":"down"}}"#),
        );
        for (path, name) in [
            ("/v1/me/albums", "saved-albums"),
            ("/v1/me/playlists", "playlists"),
            ("/v1/me/player/devices", "devices"),
        ] {
            h.fake.reply("GET", path, Reply::fixture(200, name));
        }
        h.fake.reply(
            "GET",
            "/v1/me/following",
            Reply::fixture(200, "followed-artists-2"),
        );
        h.fake.reply("GET", "/v1/me/player", Reply::empty(204));
        h.send(SpotifyRequest::Refresh);
        let SpotifyEvent::Library(library) =
            h.expect(|e| matches!(e, SpotifyEvent::Library(l) if !l.loading && l.failed))
        else {
            unreachable!()
        };
        assert_eq!(library.tracks.len(), 107, "the old list is kept");
        assert_eq!(library.artists.len(), 1, "what loaded is new");
    }

    #[test]
    fn a_failed_lookup_after_sign_in_returns_to_signed_out() {
        let fake = FakeSpotify::start();
        fake.reply("POST", "/api/token", Reply::fixture(200, "token"));
        fake.reply(
            "GET",
            "/v1/me",
            Reply::json(500, r#"{"error":{"status":500,"message":"down"}}"#),
        );
        let mut h = Harness::start(fake, MemoryStore::new(), browser(), 0);
        h.expect(signed_out);
        h.send(SpotifyRequest::SignIn {
            done_page: String::new(),
            failed_page: String::new(),
        });
        h.expect(|e| matches!(e, SpotifyEvent::Account(SpotifyAccount::SigningIn)));
        h.expect(signed_out);
    }

    #[test]
    fn a_large_cover_is_decoded_to_1024() {
        let mut h = Harness::signed_in();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2000, 1000, image::Rgba([20, 40, 200, 255]))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        h.fake
            .reply("GET", "/img/large.png", Reply::bytes(200, "image/png", png));
        let url = format!("{}/img/large.png", h.fake.base());
        h.send(SpotifyRequest::LargeCover(url.clone()));
        let SpotifyEvent::Cover(cover) = h.expect(|e| matches!(e, SpotifyEvent::Cover(_))) else {
            unreachable!()
        };
        assert_eq!(cover.url, url);
        assert_eq!((cover.width, cover.height), (1024, 512));
        assert!(cover.large);
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

    mod player {
        use super::*;
        use crate::spotify_player::{ARIA_DEVICE, AriaPlayerStatus, PlayerDeps};
        use aria_spotify::browser::{Launched, Launcher, Running};
        use aria_spotify::fake::Seen;
        use aria_spotify::player_page::PageClient;
        use std::path::Path;
        use std::sync::atomic::AtomicUsize;

        /// What the fake launcher saw: the pages it opened, and its window's fate.
        #[derive(Clone, Default)]
        struct Launches {
            urls: Arc<Mutex<Vec<String>>>,
            exited: Arc<AtomicBool>,
            killed: Arc<AtomicUsize>,
        }

        struct FakeLauncher(Launches);
        struct FakeRunning(Launches);

        impl Running for FakeRunning {
            fn exited(&mut self) -> bool {
                self.0.exited.load(Ordering::SeqCst)
            }
            fn kill(&mut self) {
                self.0.killed.fetch_add(1, Ordering::SeqCst);
                self.0.exited.store(true, Ordering::SeqCst);
            }
        }

        impl Launcher for FakeLauncher {
            fn launch(&mut self, url: &str, _profile: &Path) -> io::Result<Launched> {
                self.0.urls.lock().unwrap().push(url.to_owned());
                Ok(Launched::Window(Box::new(FakeRunning(self.0.clone()))))
            }
        }

        const STREAMING: &str = r#"{"access_token":"a1","refresh_token":"r1","expires_in":3600,"scope":"streaming user-read-private"}"#;
        const NOT_STREAMING: &str = r#"{"access_token":"a1","refresh_token":"r1","expires_in":3600,"scope":"user-read-private"}"#;

        fn harness(l: &Launches, token: &str, ready: Duration, lost: Duration) -> Harness {
            let fake = FakeSpotify::start();
            fake.reply("POST", "/api/token", Reply::json(200, token));
            script_library(&fake);
            let deps = PlayerDeps {
                launcher: Box::new(FakeLauncher(l.clone())),
                profile_dir: Some(std::env::temp_dir().join("aria-test-player-profile-none")),
                ready,
                first_ready: ready,
                lost,
                close_grace: Duration::from_millis(200),
            };
            let mut h =
                Harness::start_with(fake, MemoryStore::holding("stored"), no_browser(), 0, deps);
            h.expect(|e| matches!(e, SpotifyEvent::Library(l) if !l.loading));
            h.send(SpotifyRequest::Cadence(Cadence::Off));
            h.send(SpotifyRequest::PlayInAria {
                on: true,
                strings: "{}".to_owned(),
            });
            h
        }

        fn launched(l: &Launches) -> Vec<String> {
            l.urls.lock().unwrap().clone()
        }

        /// The page of the last launch, once there is one, with its session taken.
        fn page_of(l: &Launches) -> PageClient {
            let deadline = Instant::now() + WAIT;
            loop {
                if let Some(url) = launched(l).last() {
                    let mut page = PageClient::new(url);
                    page.session().unwrap();
                    return page;
                }
                assert!(Instant::now() < deadline, "never launched");
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn album(id: &str) -> PlayRequest {
            PlayRequest::Context {
                uri: format!("spotify:album:{id}"),
                offset_uri: None,
            }
        }

        fn no_device_then_ok(fake: &FakeSpotify) {
            fake.reply(
                "PUT",
                "/v1/me/player/play",
                Reply::fixture(404, "error-no-device"),
            );
            fake.reply("PUT", "/v1/me/player/play", Reply::empty(204));
            fake.reply("PUT", "/v1/me/player", Reply::empty(204));
        }

        fn is_status(event: &SpotifyEvent, wanted: &AriaPlayerStatus) -> bool {
            matches!(event, SpotifyEvent::Player(status) if status == wanted)
        }

        fn ready(id: &str) -> AriaPlayerStatus {
            AriaPlayerStatus::Ready {
                device_id: id.to_owned(),
            }
        }

        fn puts(h: &Harness) -> Vec<Seen> {
            h.fake
                .requests()
                .into_iter()
                .filter(|s| s.method == "PUT")
                .collect()
        }

        /// Plays an album with no device active, and has the page say it is ready as `web1`.
        fn started(l: &Launches, lost: Duration) -> (Harness, PageClient) {
            let mut h = harness(l, STREAMING, WAIT, lost);
            no_device_then_ok(&h.fake);
            h.send(SpotifyRequest::Play(album("albumA")));
            h.expect(|e| is_status(e, &AriaPlayerStatus::Starting));
            let page = page_of(l);
            assert_eq!(page.event(r#"{"type":"ready","device_id":"web1"}"#).0, 200);
            h.expect(|e| is_status(e, &ready("web1")));
            (h, page)
        }

        #[test]
        fn play_with_no_device_starts_the_player() {
            let l = Launches::default();
            let (mut h, _page) = started(&l, WAIT);
            h.during(Duration::from_millis(200));
            assert_eq!(launched(&l).len(), 1);
            let puts = puts(&h);
            assert_eq!(puts.len(), 3, "{puts:?}");
            assert_eq!(puts[0].param("device_id"), None);
            assert_eq!(puts[1].path, "/v1/me/player");
            assert!(puts[1].body.contains("web1"), "{}", puts[1].body);
            assert_eq!(puts[2].path, "/v1/me/player/play");
            assert_eq!(puts[2].param("device_id"), Some("web1"));
            assert!(puts[2].body.contains("albumA"));
        }

        #[test]
        fn choosing_aria_starts_the_player_on_play() {
            let l = Launches::default();
            let mut h = harness(&l, STREAMING, WAIT, WAIT);
            h.send(SpotifyRequest::Device(Some(ARIA_DEVICE.to_owned())));
            h.send(SpotifyRequest::Play(album("albumA")));
            h.expect(|e| is_status(e, &AriaPlayerStatus::Starting));
            assert_eq!(launched(&l).len(), 1);
            assert!(
                puts(&h).is_empty(),
                "nothing is played before the player is ready"
            );
        }

        #[test]
        fn a_chosen_phone_does_not_start_the_player() {
            let l = Launches::default();
            let mut h = harness(&l, STREAMING, WAIT, WAIT);
            no_device_then_ok(&h.fake);
            h.send(SpotifyRequest::Device(Some("device-phone".to_owned())));
            h.send(SpotifyRequest::Play(album("albumA")));
            h.during(Duration::from_millis(400));
            assert!(launched(&l).is_empty());
            let puts = puts(&h);
            assert_eq!(puts.len(), 3, "{puts:?}");
            assert!(puts[1].body.contains("device-phone"));
        }

        #[test]
        fn turning_it_off_closes_the_window() {
            let l = Launches::default();
            let (mut h, page) = started(&l, WAIT);
            let waiting = {
                let page = page.clone();
                thread::spawn(move || page.wait())
            };
            thread::sleep(Duration::from_millis(100));
            h.send(SpotifyRequest::PlayInAria {
                on: false,
                strings: "{}".to_owned(),
            });
            assert_eq!(waiting.join().unwrap().1, r#"{"close":true}"#);
            h.expect(|e| is_status(e, &AriaPlayerStatus::Off));
            assert_eq!(l.killed.load(Ordering::SeqCst), 1, "killed after the grace");
        }

        #[test]
        fn play_in_aria_off_is_0_3_2() {
            let l = Launches::default();
            let mut h = harness(&l, STREAMING, WAIT, WAIT);
            h.send(SpotifyRequest::PlayInAria {
                on: false,
                strings: "{}".to_owned(),
            });
            h.fake.reply(
                "PUT",
                "/v1/me/player/play",
                Reply::fixture(404, "error-no-device"),
            );
            h.send(SpotifyRequest::Play(album("albumA")));
            h.expect(|e| is_notice(e, "spotify-no-device"));
            assert!(launched(&l).is_empty());
        }

        #[test]
        fn without_streaming_it_asks_to_reconnect() {
            let l = Launches::default();
            let mut h = harness(&l, NOT_STREAMING, WAIT, WAIT);
            no_device_then_ok(&h.fake);
            h.send(SpotifyRequest::Play(album("albumA")));
            h.expect(|e| is_status(e, &AriaPlayerStatus::NeedsReconnect));
            assert_eq!(notices(&h.seen, "spotify-player-reconnect"), 1);
            assert!(launched(&l).is_empty());
        }

        #[test]
        fn plays_while_starting_launch_once() {
            let l = Launches::default();
            let mut h = harness(&l, STREAMING, WAIT, WAIT);
            no_device_then_ok(&h.fake);
            h.send(SpotifyRequest::Play(album("albumA")));
            h.expect(|e| is_status(e, &AriaPlayerStatus::Starting));
            h.send(SpotifyRequest::Play(album("albumB")));
            h.during(Duration::from_millis(100));
            let page = page_of(&l);
            page.event(r#"{"type":"ready","device_id":"web1"}"#);
            h.expect(|e| is_status(e, &ready("web1")));
            h.during(Duration::from_millis(200));
            assert_eq!(launched(&l).len(), 1);
            let plays: Vec<Seen> = puts(&h)
                .into_iter()
                .filter(|s| s.path == "/v1/me/player/play")
                .collect();
            assert_eq!(plays.len(), 2, "{plays:?}");
            assert!(plays[1].body.contains("albumB"), "{}", plays[1].body);
        }

        #[test]
        fn no_ready_in_time_is_a_timeout() {
            let l = Launches::default();
            let mut h = harness(&l, STREAMING, Duration::from_millis(150), WAIT);
            no_device_then_ok(&h.fake);
            h.send(SpotifyRequest::Play(album("albumA")));
            h.expect(|e| is_status(e, &AriaPlayerStatus::Starting));
            h.expect(|e| is_notice(e, "spotify-player-timeout"));
            h.expect(|e| is_status(e, &AriaPlayerStatus::Off));
        }

        #[test]
        fn a_quiet_page_is_lost() {
            let l = Launches::default();
            let (mut h, _page) = started(&l, Duration::from_millis(300));
            h.expect(|e| is_status(e, &AriaPlayerStatus::Lost));
        }

        #[test]
        fn a_lost_player_still_listed_is_reused() {
            let l = Launches::default();
            let (mut h, _page) = started(&l, Duration::from_millis(300));
            h.expect(|e| is_status(e, &AriaPlayerStatus::Lost));
            h.fake.clear();
            h.fake.reply(
                "GET",
                "/v1/me/player/devices",
                Reply::json(
                    200,
                    r#"{"devices":[{"id":"web1","is_active":true,"is_restricted":false,"name":"Aria","type":"Computer","volume_percent":50,"supports_volume":true}]}"#,
                ),
            );
            h.fake.reply("PUT", "/v1/me/player/play", Reply::empty(204));
            let before = puts(&h).len();
            h.send(SpotifyRequest::Play(album("albumC")));
            h.expect(|e| is_status(e, &ready("web1")));
            h.during(Duration::from_millis(200));
            assert_eq!(launched(&l).len(), 1, "no second window");
            let after = puts(&h);
            assert!(after.len() > before);
            let last = after.last().unwrap();
            assert_eq!(last.param("device_id"), Some("web1"));
            assert!(last.body.contains("albumC"));
        }

        #[test]
        fn the_ready_device_is_not_listed_twice() {
            let l = Launches::default();
            let mut h = harness(&l, STREAMING, WAIT, WAIT);
            h.fake.clear();
            no_device_then_ok(&h.fake);
            h.fake.reply("GET", "/v1/me/player", Reply::empty(204));
            h.fake.reply(
                "GET",
                "/v1/me/player/devices",
                Reply::json(
                    200,
                    r#"{"devices":[{"id":"web1","is_active":true,"is_restricted":false,"name":"Aria","type":"Computer","volume_percent":50,"supports_volume":true},{"id":"device-phone","is_active":false,"is_restricted":false,"name":"My Phone","type":"Smartphone","volume_percent":60,"supports_volume":true}]}"#,
                ),
            );
            h.send(SpotifyRequest::Play(album("albumA")));
            let page = page_of(&l);
            page.event(r#"{"type":"ready","device_id":"web1"}"#);
            h.expect(|e| is_status(e, &ready("web1")));
            let listed = h.expect(|e| matches!(e, SpotifyEvent::Devices(d) if !d.is_empty()));
            let SpotifyEvent::Devices(devices) = listed else {
                unreachable!()
            };
            let ids: Vec<_> = devices.iter().filter_map(|d| d.id.clone()).collect();
            assert_eq!(ids, ["device-phone"]);
        }

        #[test]
        fn a_token_that_cannot_be_had_answers_503() {
            let l = Launches::default();
            // Expires in 70 s: fresh enough for the API, not for the page (60 s).
            let token =
                r#"{"access_token":"a1","refresh_token":"r1","expires_in":70,"scope":"streaming"}"#;
            let mut h = harness(&l, token, WAIT, WAIT);
            no_device_then_ok(&h.fake);
            h.send(SpotifyRequest::Play(album("albumA")));
            h.expect(|e| is_status(e, &AriaPlayerStatus::Starting));
            let page = page_of(&l);
            h.fake.set_offline(true);
            let asked = Instant::now();
            let (status, body) = page.token();
            assert_eq!(status, 503);
            assert!(!body.contains("a1"));
            assert!(asked.elapsed() < Duration::from_secs(11));
        }

        #[test]
        fn a_fresh_token_reaches_the_page() {
            let l = Launches::default();
            let (_h, page) = started(&l, WAIT);
            assert_eq!(page.token(), (200, r#"{"access_token":"a1"}"#.to_owned()));
        }

        #[test]
        fn a_page_gone_offline_waits_for_its_next_ready() {
            let l = Launches::default();
            let (mut h, page) = started(&l, WAIT);
            page.event(r#"{"type":"not_ready","device_id":"web1"}"#);
            h.expect(|e| is_status(e, &AriaPlayerStatus::Starting));
            let before = puts(&h).len();
            h.send(SpotifyRequest::Play(album("albumD")));
            h.during(Duration::from_millis(200));
            assert_eq!(puts(&h).len(), before, "nothing goes to an offline device");
            assert_eq!(launched(&l).len(), 1, "no second window");
            page.event(r#"{"type":"ready","device_id":"web2"}"#);
            h.expect(|e| is_status(e, &ready("web2")));
            h.during(Duration::from_millis(200));
            let after = puts(&h);
            let last = after.last().unwrap();
            assert_eq!(last.param("device_id"), Some("web2"));
            assert!(last.body.contains("albumD"), "{}", last.body);
        }

        #[test]
        fn sign_out_closes_the_window() {
            let l = Launches::default();
            let (mut h, page) = started(&l, WAIT);
            let waiting = {
                let page = page.clone();
                thread::spawn(move || page.wait())
            };
            thread::sleep(Duration::from_millis(100));
            h.send(SpotifyRequest::SignOut);
            assert_eq!(waiting.join().unwrap().1, r#"{"close":true}"#);
            h.expect(|e| is_status(e, &AriaPlayerStatus::Off));
            assert_eq!(l.killed.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn shutdown_closes_the_window() {
            let l = Launches::default();
            let (mut h, _page) = started(&l, WAIT);
            let handle = h.handle.take().unwrap();
            assert!(
                handle.join(Duration::from_secs(1)),
                "ended within the runtime's join"
            );
            assert_eq!(l.killed.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn signed_in_says_whether_the_session_can_edit() {
        let h = Harness::signed_in();
        assert!(h.seen.iter().any(|e| matches!(
            e,
            SpotifyEvent::Account(SpotifyAccount::SignedIn {
                can_edit: false,
                ..
            })
        )));
        let h = Harness::editor_with(|_| {});
        assert!(h.seen.iter().any(|e| matches!(
            e,
            SpotifyEvent::Account(SpotifyAccount::SignedIn { can_edit: true, .. })
        )));
    }

    #[test]
    fn own_and_collaborative_playlists_are_editable() {
        let h = Harness::editor_with(|fake| {
            fake.reply(
                "GET",
                "/v1/me/playlists",
                Reply::json(
                    200,
                    r#"{"next":null,"items":[
                        {"id":"own","uri":"spotify:playlist:own","name":"Mine","owner":{"id":"aria-tester"},"collaborative":false},
                        {"id":"col","uri":"spotify:playlist:col","name":"Ours","owner":{"id":"friend"},"collaborative":true},
                        {"id":"fol","uri":"spotify:playlist:fol","name":"Theirs","owner":{"id":"friend"},"collaborative":false}
                    ]}"#,
                ),
            );
        });
        let Some(SpotifyEvent::Library(library)) = h
            .seen
            .iter()
            .rev()
            .find(|e| matches!(e, SpotifyEvent::Library(l) if !l.loading))
        else {
            panic!("no library");
        };
        let editable: Vec<bool> = library.playlists.iter().map(|p| p.editable).collect();
        assert_eq!(editable, [true, true, false]);
    }

    #[test]
    fn a_like_shows_at_once_then_is_sent() {
        let mut h =
            Harness::editor_with(|fake| fake.reply("PUT", "/v1/me/library", Reply::empty(200)));
        h.send(SpotifyRequest::Like {
            track: sp_track("spotify:track:new1", "New"),
            liked: true,
        });
        let SpotifyEvent::Library(library) = h.expect(|e| first_is(e, "spotify:track:new1")) else {
            unreachable!()
        };
        assert_eq!(library.tracks.len(), 108);
        let later = h.during(Duration::from_millis(300));
        assert_eq!(h.count("PUT", "/v1/me/library"), 1);
        let put = h
            .fake
            .requests()
            .into_iter()
            .find(|s| s.method == "PUT")
            .unwrap();
        assert_eq!(put.param("uris"), Some("spotify:track:new1"));
        assert!(
            !later.iter().any(|e| matches!(e, SpotifyEvent::Library(_))),
            "kept as it is"
        );
    }

    #[test]
    fn a_refused_like_is_undone_and_reported() {
        let mut h = Harness::editor_with(|fake| {
            fake.reply("PUT", "/v1/me/library", Reply::json(500, BOOM))
        });
        h.send(SpotifyRequest::Like {
            track: sp_track("spotify:track:new1", "New"),
            liked: true,
        });
        h.expect(|e| first_is(e, "spotify:track:new1"));
        let SpotifyEvent::Library(library) = h.expect(|e| matches!(e, SpotifyEvent::Library(_)))
        else {
            unreachable!()
        };
        assert!(!has(&library, "spotify:track:new1"));
        assert_eq!(library.tracks.len(), 107);
        assert_eq!(notices(&h.seen, "spotify-error"), 1);
    }

    #[test]
    fn a_second_refused_like_keeps_the_first() {
        let mut h = Harness::editor_with(|fake| {
            fake.reply("PUT", "/v1/me/library", Reply::empty(200));
            fake.reply("PUT", "/v1/me/library", Reply::json(500, BOOM));
        });
        for uri in ["spotify:track:a", "spotify:track:b"] {
            h.send(SpotifyRequest::Like {
                track: sp_track(uri, "x"),
                liked: true,
            });
        }
        h.expect(|e| first_is(e, "spotify:track:b"));
        let SpotifyEvent::Library(library) = h.expect(|e| matches!(e, SpotifyEvent::Library(_)))
        else {
            unreachable!()
        };
        assert!(has(&library, "spotify:track:a"));
        assert!(!has(&library, "spotify:track:b"));
        assert_eq!(library.tracks.len(), 108);
    }

    #[test]
    fn an_unlike_takes_the_track_out_at_once() {
        let mut h = Harness::editor_with(|fake| {
            fake.reply("DELETE", "/v1/me/library", Reply::empty(200));
        });
        h.send(SpotifyRequest::Like {
            track: sp_track("spotify:track:track0000", "Song 0"),
            liked: false,
        });
        let SpotifyEvent::Library(library) = h.expect(|e| matches!(e, SpotifyEvent::Library(_)))
        else {
            unreachable!()
        };
        assert!(!has(&library, "spotify:track:track0000"));
        assert_eq!(library.tracks.len(), 106);
        h.during(Duration::from_millis(300));
        assert_eq!(h.count("DELETE", "/v1/me/library"), 1);
    }

    #[test]
    fn opening_a_playlist_loads_its_tracks() {
        let fake = FakeSpotify::start();
        script_pl_a(&fake, 1);
        script_library(&fake);
        let mut h = Harness::start(fake, MemoryStore::holding("stored"), no_browser(), 0);
        h.expect(|e| matches!(e, SpotifyEvent::Library(l) if !l.loading));
        h.send(SpotifyRequest::OpenPlaylist("plA".to_owned()));
        let SpotifyEvent::Playlist(Some(first)) =
            h.expect(|e| matches!(e, SpotifyEvent::Playlist(_)))
        else {
            panic!("closed");
        };
        assert!(first.loading);
        assert!(first.tracks.is_empty());
        assert_eq!(first.snapshot.as_deref(), Some("snapA"));
        let SpotifyEvent::Playlist(Some(loaded)) =
            h.expect(|e| matches!(e, SpotifyEvent::Playlist(_)))
        else {
            panic!("closed");
        };
        assert!(!loaded.loading);
        assert!(!loaded.failed);
        assert_eq!(loaded.id, "plA");
        let names: Vec<&str> = loaded.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["One", "Two", "Home Song", "Three"]);
    }

    #[test]
    fn a_playlist_spotify_refuses_shows_failed() {
        let mut h = Harness::editor_with(|fake| {
            fake.reply(
                "GET",
                "/v1/playlists/plB/items",
                Reply::json(403, r#"{"error":{"status":403,"message":"Forbidden"}}"#),
            );
        });
        h.send(SpotifyRequest::OpenPlaylist("plB".to_owned()));
        let SpotifyEvent::Playlist(Some(p)) =
            h.expect(|e| matches!(e, SpotifyEvent::Playlist(Some(p)) if !p.loading))
        else {
            unreachable!()
        };
        assert!(p.failed);
        assert!(p.tracks.is_empty());
    }

    #[test]
    fn adding_says_added_and_reloads_the_open_playlist() {
        let mut h = Harness::editor_with(|fake| {
            script_pl_a(fake, 2);
            fake.reply(
                "POST",
                "/v1/playlists/plA/items",
                Reply::fixture(201, "snapshot"),
            );
        });
        h.open_pl_a();
        h.send(SpotifyRequest::AddToPlaylist {
            playlist: "plA".to_owned(),
            name: "Road Trip".to_owned(),
            uri: "spotify:track:t1".to_owned(),
        });
        let SpotifyEvent::Notice(notice) = h.expect(|e| is_notice(e, "spotify-added")) else {
            unreachable!()
        };
        assert_eq!(notice.level, NoticeLevel::Info);
        assert_eq!(notice.args, vec![("playlist", "Road Trip".to_owned())]);
        let SpotifyEvent::Playlist(Some(p)) = h.expect(|e| matches!(e, SpotifyEvent::Playlist(_)))
        else {
            panic!("closed");
        };
        assert_eq!(p.snapshot.as_deref(), Some("snap2"));
        assert_eq!(p.tracks.len(), 4);
        assert_eq!(h.count("GET", "/v1/playlists/plA/items"), 4);
    }

    #[test]
    fn removing_sends_no_snapshot_and_reloads() {
        let mut h = Harness::editor_with(|fake| {
            script_pl_a(fake, 2);
            fake.reply(
                "DELETE",
                "/v1/playlists/plA/items",
                Reply::fixture(200, "snapshot"),
            );
        });
        h.open_pl_a();
        h.send(SpotifyRequest::RemoveFromPlaylist {
            playlist: "plA".to_owned(),
            uri: "spotify:track:t1".to_owned(),
        });
        let SpotifyEvent::Playlist(Some(p)) = h.expect(|e| matches!(e, SpotifyEvent::Playlist(_)))
        else {
            panic!("closed");
        };
        assert_eq!(p.snapshot.as_deref(), Some("snap2"));
        let delete = h
            .fake
            .requests()
            .into_iter()
            .find(|s| s.method == "DELETE")
            .unwrap();
        let body: String = delete.body.split_whitespace().collect();
        // Spotify checks a snapshot against the items; the one Aria has may predate them.
        assert!(!body.contains("snapshot_id"), "{body}");
        assert!(body.contains(r#"{"uri":"spotify:track:t1"}"#), "{body}");
        assert_eq!(h.count("GET", "/v1/playlists/plA/items"), 4);
    }

    #[test]
    fn creating_a_playlist_adds_the_first_track_and_lists_it_first() {
        let mut h = Harness::editor_with(|fake| {
            fake.reply(
                "POST",
                "/v1/me/playlists",
                Reply::fixture(201, "created-playlist"),
            );
            fake.reply(
                "POST",
                "/v1/playlists/plNew/items",
                Reply::fixture(201, "snapshot"),
            );
        });
        h.send(SpotifyRequest::CreatePlaylist {
            name: "Mix".to_owned(),
            first: Some("spotify:track:t1".to_owned()),
        });
        let SpotifyEvent::Library(library) = h.expect(|e| matches!(e, SpotifyEvent::Library(_)))
        else {
            unreachable!()
        };
        assert_eq!(library.playlists[0].id, "plNew");
        assert!(library.playlists[0].editable);
        assert_eq!(library.playlists.len(), 3);
        let SpotifyEvent::Notice(notice) = h.expect(|e| is_notice(e, "spotify-created")) else {
            unreachable!()
        };
        assert_eq!(notice.args, vec![("playlist", "Mix".to_owned())]);
        h.during(Duration::from_millis(300));
        assert_eq!(h.count("POST", "/v1/playlists/plNew/items"), 1);
    }

    #[test]
    fn edits_without_the_permissions_send_nothing_and_say_reconnect() {
        let mut h = Harness::signed_in();
        h.send(SpotifyRequest::Like {
            track: sp_track("spotify:track:t1", "x"),
            liked: true,
        });
        h.send(SpotifyRequest::AddToPlaylist {
            playlist: "plA".to_owned(),
            name: "Road Trip".to_owned(),
            uri: "spotify:track:t1".to_owned(),
        });
        h.send(SpotifyRequest::RemoveFromPlaylist {
            playlist: "plA".to_owned(),
            uri: "spotify:track:t1".to_owned(),
        });
        h.send(SpotifyRequest::CreatePlaylist {
            name: "Mix".to_owned(),
            first: None,
        });
        let events = h.during(Duration::from_millis(400));
        assert_eq!(notices(&events, "spotify-reconnect-to-edit"), 4);
        assert!(!events.iter().any(|e| matches!(e, SpotifyEvent::Library(_))));
        let edits = h
            .fake
            .requests()
            .into_iter()
            .filter(|s| {
                s.path.contains("/library")
                    || s.path.contains("/items")
                    || s.method == "POST" && s.path == "/v1/me/playlists"
            })
            .count();
        assert_eq!(edits, 0);
    }

    #[test]
    fn signing_out_closes_the_open_playlist() {
        let mut h = Harness::editor_with(|fake| script_pl_a(fake, 1));
        h.open_pl_a();
        h.send(SpotifyRequest::SignOut);
        h.expect(|e| matches!(e, SpotifyEvent::Playlist(None)));
    }
}
