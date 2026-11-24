//! The backend thread. It owns a Tokio runtime and does the work the UI
//! thread must never block on: debounced, durable settings persistence, and
//! coordinating the services that run on threads of their own. The library
//! service and the audio engine are those so far: the events of each arrive on
//! one more channel, and commands for them go out without waiting. The
//! [`PlaybackService`] ties the two together here. Network services join them
//! in later phases.
//!
//! The UI sends [`Command`]s through a [`BackendHandle`] and drains
//! [`Event`]s once per frame. After each batch of events the backend calls
//! the [`Waker`], so the UI repaints only when something has changed.
//!
//! # Deviation from spec §3.2
//!
//! Spec §3.2 carries both directions over `crossbeam_channel`. Commands
//! instead travel over `tokio::sync::mpsc::unbounded_channel`. Its `send` is
//! synchronous and never blocks the UI, and its async receiver lets the
//! backend loop `select!` on the next command and the debounce timer
//! together. Events travel over `crossbeam_channel::unbounded`, so the UI can
//! `try_recv` without an async context.

use std::{
    fmt,
    hash::{BuildHasher, RandomState},
    io, mem,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, SystemTime},
};

use aria_audio::{
    CaptureFactory, DeviceInfo, DeviceLister, Engine, EngineEvent, OutputFactory, PlaybackClock,
};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    time::{Instant, MissedTickBehavior},
};

use crate::{
    backup, chat,
    command::Command,
    error::SettingsError,
    event::{ActivePlayer, Event},
    library_service::{self, LibraryEvent, LibraryHandle, LibraryRequest, LibraryService},
    notice::{Notice, NoticeLevel},
    opened,
    playback_service::PlaybackService,
    settings::{Settings, store::SettingsStore},
    spotify_service::{
        Cadence, SpotifyAccount, SpotifyDeps, SpotifyEvent, SpotifyHandle, SpotifyRequest,
        SpotifyService,
    },
    watch::{self, FolderWatcher},
};

pub use crate::library_service::LibraryPaths;

/// Name of the backend thread.
const THREAD_NAME: &str = "aria-backend";
/// How long the runtime may take to stop its threads once the loop has ended.
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);
/// How long the backend waits for the library thread to stop. It leaves room within the time the
/// app gives the whole backend, so a stuck library cannot cost the settings their clean exit.
const LIBRARY_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
/// The same for the audio engine, whose wait runs beside the library's.
const ENGINE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
/// How often the playback service gets its [`tick`](PlaybackService::tick).
const PLAYBACK_TICK: Duration = Duration::from_secs(1);
/// Reported to the user when writing the settings fails.
const SAVE_FAILED: Notice = Notice {
    level: NoticeLevel::Error,
    key: "notice-settings-save-failed",
    args: Vec::new(),
};

/// Where the backend writes settings. [`SettingsStore`] is the real one;
/// tests use their own.
pub trait SettingsSink: Send + 'static {
    /// Durably writes `s`. The backend calls it on a blocking thread.
    fn save(&mut self, s: &Settings) -> Result<(), SettingsError>;
}

impl SettingsSink for SettingsStore {
    fn save(&mut self, s: &Settings) -> Result<(), SettingsError> {
        // The inherent method. `self.save(s)` would resolve to this trait
        // method and recurse.
        SettingsStore::save(self, s)
    }
}

/// Wakes the UI thread. In the app it wraps `egui::Context::request_repaint`.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Backend tuning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendConfig {
    /// How long settings updates must be quiet before the latest is written.
    pub debounce: Duration,
    /// How long changes in the library's folders must have stopped before they are scanned.
    pub watch_quiet: Duration,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(500),
            watch_quiet: watch::QUIET,
        }
    }
}

/// What the backend runs besides settings persistence.
pub struct BackendServices {
    /// Where the library lives; `None` leaves the library disabled.
    pub library: Option<LibraryPaths>,
    /// Opens audio outputs.
    pub output: Box<dyn OutputFactory>,
    /// Opens the microphone, only while the user has it on and a second device is set.
    pub capture: Box<dyn CaptureFactory>,
    /// The settings in effect at startup, which playback starts from.
    pub settings: Settings,
    /// Spotify; `None` leaves it out (tests).
    pub spotify: Option<SpotifyDeps>,
}

/// A running Spotify service, as the backend holds it.
struct SpotifyLink {
    handle: SpotifyHandle,
    events: UnboundedReceiver<SpotifyEvent>,
}

impl SpotifyLink {
    fn start(deps: SpotifyDeps) -> io::Result<Self> {
        let (events_tx, events) = unbounded_channel();
        let on_event = Box::new(move |event| {
            // Fails only once the backend has exited, when nobody is listening.
            let _ = events_tx.send(event);
        });
        let handle = SpotifyService::start(deps, on_event)?;
        Ok(Self { handle, events })
    }
}

/// A running library service, as the backend holds it.
struct LibraryLink {
    handle: LibraryHandle,
    events: UnboundedReceiver<LibraryEvent>,
    /// The folder of its database, where a restore is got ready.
    data_dir: Option<PathBuf>,
}

impl LibraryLink {
    fn start(paths: LibraryPaths) -> io::Result<Self> {
        let (events_tx, events) = unbounded_channel();
        let on_event = Box::new(move |event| {
            // Fails only once the backend has exited, when nobody is listening.
            let _ = events_tx.send(event);
        });
        let data_dir = paths.db.parent().map(Path::to_path_buf);
        let handle = LibraryService::start(paths, on_event)?;
        Ok(Self {
            handle,
            events,
            data_dir,
        })
    }
}

/// The audio engine and the service driving it, as the backend holds them.
struct PlaybackLink {
    service: PlaybackService,
    events: UnboundedReceiver<EngineEvent>,
    clock: Arc<PlaybackClock>,
    devices: DeviceLister,
    input_devices: DeviceLister,
    /// The settings it started with.
    settings: Settings,
}

impl PlaybackLink {
    fn start(
        output: Box<dyn OutputFactory>,
        capture: Box<dyn CaptureFactory>,
        settings: &Settings,
    ) -> io::Result<Self> {
        let (events_tx, events) = unbounded_channel();
        let on_event = Box::new(move |event| {
            // Fails only once the backend has exited, when nobody is listening.
            let _ = events_tx.send(event);
        });
        let devices = output.device_lister();
        let input_devices = capture.device_lister();
        let engine = Engine::start(output, capture, on_event)?;
        let clock = engine.clock();
        let mut service = PlaybackService::new(engine, RandomState::new().hash_one(0_u8));
        service.on_settings(settings);
        Ok(Self {
            service,
            events,
            clock,
            devices,
            input_devices,
            settings: settings.clone(),
        })
    }
}

/// Starts the backend thread. See [`Backend::start`].
#[derive(Debug)]
pub struct Backend;

impl Backend {
    /// Builds the Tokio runtime on the calling thread, then moves it into a
    /// new thread named `aria-backend` that runs the command loop.
    ///
    /// With `sink: None` the backend is read-only: updates are accepted and
    /// nothing is written, and a reset still announces the defaults.
    ///
    /// A failed save keeps its settings pending, without a deadline: the next
    /// update replaces them and schedules a new write, and shutdown retries
    /// them once. Only the first failure in a row is reported to the UI.
    ///
    /// The audio engine starts too, with no output open until something plays. With
    /// `services.library` set, the library thread starts as well. If it dies, the backend keeps
    /// running with the library disabled.
    ///
    /// # Errors
    ///
    /// Fails if the runtime cannot be built or a thread cannot be spawned.
    pub fn start(
        sink: Option<Box<dyn SettingsSink>>,
        services: BackendServices,
        waker: Waker,
        cfg: BackendConfig,
    ) -> io::Result<BackendHandle> {
        let BackendServices {
            library,
            output,
            capture,
            settings,
            spotify,
        } = services;
        let playback = PlaybackLink::start(output, capture, &settings)?;
        let library = library.map(LibraryLink::start).transpose()?;
        let spotify = spotify.map(SpotifyLink::start).transpose()?;
        Self::spawn_with(sink, library, spotify, playback, waker, cfg)
    }

    #[cfg(test)]
    fn spawn(
        sink: Option<Box<dyn SettingsSink>>,
        library: Option<LibraryLink>,
        playback: PlaybackLink,
        waker: Waker,
        cfg: BackendConfig,
    ) -> io::Result<BackendHandle> {
        Self::spawn_with(sink, library, None, playback, waker, cfg)
    }

    fn spawn_with(
        sink: Option<Box<dyn SettingsSink>>,
        library: Option<LibraryLink>,
        spotify: Option<SpotifyLink>,
        playback: PlaybackLink,
        waker: Waker,
        cfg: BackendConfig,
    ) -> io::Result<BackendHandle> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()?;
        let (commands, command_rx) = unbounded_channel();
        let (event_tx, events) = crossbeam_channel::unbounded();
        let (done_tx, done) = crossbeam_channel::bounded(1);
        let (library, library_events, data_dir) = match library {
            Some(link) => (Some(link.handle), Some(link.events), link.data_dir),
            None => (None, None, None),
        };
        let (spotify, spotify_events) = match spotify {
            Some(link) => (Some(link.handle), Some(link.events)),
            None => (None, None),
        };
        // Without it the library is scanned at start and by hand only.
        let watcher = library.as_ref().and_then(|library| {
            FolderWatcher::start(cfg.watch_quiet, library.rescanner())
                .map_err(|error| tracing::warn!(%error, "the library's folders cannot be watched"))
                .ok()
        });
        let PlaybackLink {
            service: playback,
            events: engine_events,
            clock,
            devices,
            input_devices,
            settings,
        } = playback;
        let worker = Worker {
            sink,
            library,
            playback,
            devices,
            input_devices,
            events: event_tx,
            waker,
            debounce: cfg.debounce,
            pending: None,
            deadline: None,
            failing: false,
            settings,
            watcher,
            library_folders: Vec::new(),
            data_dir,
            spotify,
            active: ActivePlayer::Local,
            window_shown: true,
            cadence: None,
            local_playing: false,
            spotify_playing: false,
        };
        let done_guard = DoneGuard::new(done_tx);
        let thread = thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                // Bound first, so dropped last: `shutdown` hears "done" only
                // after the runtime has stopped, even while a panic unwinds.
                let _done = done_guard;
                let runtime = runtime;
                runtime.block_on(worker.run(
                    command_rx,
                    library_events,
                    engine_events,
                    spotify_events,
                ));
                runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
            })?;
        Ok(BackendHandle {
            commands,
            events,
            clock,
            done,
            thread,
        })
    }
}

/// The UI thread's handle to the backend.
///
/// Dropping it without calling [`shutdown`](Self::shutdown) closes the
/// command channel. The backend treats that like [`Command::Shutdown`]: it
/// writes any pending settings, then exits.
pub struct BackendHandle {
    commands: UnboundedSender<Command>,
    events: Receiver<Event>,
    clock: Arc<PlaybackClock>,
    done: Receiver<()>,
    thread: thread::JoinHandle<()>,
}

impl fmt::Debug for BackendHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackendHandle").finish_non_exhaustive()
    }
}

impl BackendHandle {
    /// Queues `cmd` without blocking. If the backend has already exited, the
    /// command is silently dropped.
    pub fn send(&self, cmd: Command) {
        // Fails only once the backend has exited, when nothing is listening.
        let _ = self.commands.send(cmd);
    }

    /// Takes every event that has arrived so far, without blocking.
    pub fn drain(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Where playback is, for the UI to read without waiting.
    pub fn clock(&self) -> Arc<PlaybackClock> {
        Arc::clone(&self.clock)
    }

    /// Sends [`Command::Shutdown`], then waits up to `timeout` for the
    /// backend to write any pending settings and exit.
    ///
    /// Returns `true` if the backend exited cleanly in time. Returns `false`
    /// if it timed out, in which case its thread is detached and left to
    /// finish on its own, or if the thread panicked.
    pub fn shutdown(self, timeout: Duration) -> bool {
        self.send(Command::Shutdown);
        join_bounded(&self.done, self.thread, timeout, THREAD_NAME)
    }
}

/// Waits up to `timeout` for a thread with a [`DoneGuard`] to finish, then joins it. `name` is
/// the thread's, for the log.
///
/// Returns `true` if it exited cleanly in time. Returns `false` if it timed out, in which case it
/// is detached and left to finish on its own, or if it panicked.
pub(crate) fn join_bounded(
    done: &Receiver<()>,
    thread: thread::JoinHandle<()>,
    timeout: Duration,
    name: &str,
) -> bool {
    match done.recv_timeout(timeout) {
        // A disconnect also means the done guard was dropped.
        Ok(()) | Err(RecvTimeoutError::Disconnected) => match thread.join() {
            Ok(()) => true,
            Err(_) => {
                tracing::error!(thread = name, "a thread panicked");
                false
            }
        },
        Err(RecvTimeoutError::Timeout) => {
            tracing::warn!(
                thread = name,
                ?timeout,
                "a thread did not stop in time; detaching it"
            );
            false
        }
    }
}

/// Owned by a service's thread. Its drop signals the handle's `shutdown`, so a
/// panicking thread unblocks it too.
pub(crate) struct DoneGuard(Sender<()>);

impl DoneGuard {
    /// `done` must be the sender of a capacity-1 channel.
    pub(crate) fn new(done: Sender<()>) -> Self {
        Self(done)
    }
}

impl Drop for DoneGuard {
    fn drop(&mut self) {
        // This is the only send on a capacity-1 channel, so it never blocks.
        // It fails only if the handle is gone and nobody is waiting.
        let _ = self.0.try_send(());
    }
}

/// State of the backend loop.
struct Worker {
    /// `None` in read-only mode, or after the sink panicked, and also while a save
    /// has taken the sink to its blocking thread. Telling these apart is safe for
    /// now: every save is awaited inside the `select!` arm that started it, so no
    /// command is handled while the sink is away. Replace this with an explicit
    /// state enum once services that save concurrently arrive. The library
    /// service runs on its own thread and never touches the sink.
    sink: Option<Box<dyn SettingsSink>>,
    /// `None` when the backend runs without a library, or after its thread died.
    library: Option<LibraryHandle>,
    playback: PlaybackService,
    /// Lists the devices the engine's factory opens.
    devices: DeviceLister,
    /// Lists the microphones the engine can open.
    input_devices: DeviceLister,
    events: Sender<Event>,
    waker: Waker,
    debounce: Duration,
    /// The newest settings not yet written.
    pending: Option<Settings>,
    /// When `pending` is written, unless another update arrives first. `None`
    /// while `pending` holds settings whose save failed.
    deadline: Option<Instant>,
    /// The last save failed. Further failures are not reported again until a
    /// save succeeds.
    failing: bool,
    /// The settings in effect. A reset keeps their volume and mute, and the microphone hotkey
    /// flips their microphone switch.
    settings: Settings,
    /// Watches the library's folders while the settings ask for it.
    watcher: Option<FolderWatcher>,
    /// The folder of the library's database, where a restore is got ready.
    data_dir: Option<PathBuf>,
    /// The library's folders, as the last snapshot listed them.
    library_folders: Vec<PathBuf>,
    /// `None` without Spotify, or after its thread died.
    spotify: Option<SpotifyHandle>,
    /// The player the bar shows: the one last started.
    active: ActivePlayer,
    /// The window is neither minimized nor hidden.
    window_shown: bool,
    /// The polling cadence last sent to Spotify.
    cadence: Option<Cadence>,
    /// Aria's own player is playing, as its last view said.
    local_playing: bool,
    /// Spotify is playing, as its last playback state said.
    spotify_playing: bool,
}

impl Worker {
    async fn run(
        mut self,
        mut commands: UnboundedReceiver<Command>,
        mut library_events: Option<UnboundedReceiver<LibraryEvent>>,
        engine_events: UnboundedReceiver<EngineEvent>,
        mut spotify_events: Option<UnboundedReceiver<SpotifyEvent>>,
    ) {
        let mut engine_events = Some(engine_events);
        let mut tick = tokio::time::interval(PLAYBACK_TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        self.flush_playback();
        if let Some(spotify) = &self.spotify {
            spotify.send(SpotifyRequest::Device(self.settings.spotify_device.clone()));
            self.emit([Event::ActivePlayer(self.active)]);
            self.sync_cadence();
        }
        loop {
            tokio::select! {
                cmd = commands.recv() => match cmd {
                    Some(Command::UpdateSettings(settings)) => {
                        let settings = self.vetted(settings);
                        self.apply(settings);
                    }
                    Some(Command::ResetSettings) => {
                        let defaults = self.defaults();
                        self.settings = defaults.clone();
                        self.playback.on_settings(&defaults);
                        self.flush_playback();
                        self.reset().await;
                    }
                    Some(Command::ToggleMicrophone) => self.toggle_microphone(),
                    Some(Command::AddFolder(path)) => {
                        self.to_library(LibraryRequest::AddFolder(path));
                    }
                    Some(Command::RemoveFolder(id)) => {
                        self.to_library(LibraryRequest::RemoveFolder(id));
                    }
                    Some(Command::Rescan) => self.to_library(LibraryRequest::Rescan),
                    Some(Command::BackUp(parent)) => self.back_up(&parent),
                    Some(Command::ImportPlaylist(file)) => {
                        self.to_collections(LibraryRequest::ImportPlaylist(file));
                    }
                    Some(Command::ExportPlaylist { id, to }) => {
                        self.to_collections(LibraryRequest::ExportPlaylist { id, to });
                    }
                    Some(Command::Restore(folder)) => self.restore(folder),
                    Some(Command::ClearArtwork) => self.to_library(LibraryRequest::ClearArtwork),
                    // A file played from outside the library has no heart and no playlist.
                    Some(
                        Command::SetFavorite { track, .. }
                        | Command::AddToPlaylist { track, .. }
                        | Command::CreatePlaylist {
                            first: Some(track), ..
                        },
                    ) if opened::is_opened(track) => {
                        tracing::debug!("ignoring a collection change for an opened track");
                    }
                    Some(Command::SetFavorite { track, favorite }) => {
                        self.to_collections(LibraryRequest::SetFavorite { track, favorite });
                    }
                    Some(Command::CreatePlaylist { name, first }) => {
                        self.to_collections(LibraryRequest::CreatePlaylist { name, first });
                    }
                    Some(Command::RenamePlaylist { id, name }) => {
                        self.to_collections(LibraryRequest::RenamePlaylist { id, name });
                    }
                    Some(Command::DeletePlaylist(id)) => {
                        self.to_collections(LibraryRequest::DeletePlaylist(id));
                    }
                    Some(Command::AddToPlaylist { playlist, track }) => {
                        self.to_collections(LibraryRequest::AddToPlaylist { playlist, track });
                    }
                    Some(Command::RemoveFromPlaylist { playlist, position }) => {
                        self.to_collections(LibraryRequest::RemoveFromPlaylist {
                            playlist,
                            position,
                        });
                    }
                    Some(Command::MoveInPlaylist { playlist, from, to }) => {
                        self.to_collections(LibraryRequest::MoveInPlaylist { playlist, from, to });
                    }
                    Some(Command::ClearHistory) => self.to_collections(LibraryRequest::ClearHistory),
                    Some(Command::AddSounds(paths)) => {
                        self.to_collections(LibraryRequest::AddSounds(paths));
                    }
                    Some(Command::RenameSound { id, name }) => {
                        self.to_collections(LibraryRequest::RenameSound { id, name });
                    }
                    Some(Command::SetSoundVolume { id, volume }) => {
                        self.to_collections(LibraryRequest::SetSoundVolume { id, volume });
                    }
                    Some(Command::SetSoundHotkey { id, hotkey }) => {
                        let reserved = self
                            .playback
                            .stop_hotkey()
                            .into_iter()
                            .chain(self.playback.mic_hotkey())
                            .collect();
                        self.to_collections(LibraryRequest::SetSoundHotkey {
                            id,
                            hotkey,
                            reserved,
                        });
                    }
                    Some(Command::RemoveSound(id)) => {
                        self.to_collections(LibraryRequest::RemoveSound(id));
                    }
                    // Fired from the list the worker holds, so it never waits for the library.
                    Some(Command::PlaySound(id)) => {
                        if self.library.is_some() {
                            self.playback.handle(Command::PlaySound(id));
                            self.flush_playback();
                        } else {
                            self.emit([Event::Notice(library_service::UNAVAILABLE)]);
                        }
                    }
                    Some(Command::ListDevices) => {
                        self.list(Arc::clone(&self.devices), Event::OutputDevices);
                    }
                    Some(Command::ListInputDevices) => {
                        self.list(Arc::clone(&self.input_devices), Event::InputDevices);
                    }
                    Some(Command::Spotify(request)) => self.spotify_request(request),
                    Some(Command::WindowShown(shown)) => {
                        self.window_shown = shown;
                        self.sync_cadence();
                    }
                    // Starting a local track takes the bar back from Spotify, and pauses it.
                    Some(command @ (Command::PlayContext { .. } | Command::OpenPaths(_))) => {
                        if self.spotify_playing {
                            self.to_spotify(SpotifyRequest::Pause);
                        }
                        self.show(ActivePlayer::Local);
                        self.playback.handle(command);
                        self.flush_playback();
                    }
                    // The transport acts on the player the bar shows.
                    Some(Command::TogglePlay) if self.routes_to_spotify() => {
                        self.to_spotify(SpotifyRequest::TogglePlay);
                    }
                    // Resuming the local track while Spotify plays elsewhere pauses Spotify.
                    Some(Command::TogglePlay) if !self.local_playing && self.spotify_playing => {
                        self.to_spotify(SpotifyRequest::Pause);
                        self.spotify_playing = false;
                        self.playback.handle(Command::TogglePlay);
                        self.flush_playback();
                    }
                    Some(Command::Next) if self.routes_to_spotify() => {
                        self.to_spotify(SpotifyRequest::Next);
                    }
                    Some(Command::Previous) if self.routes_to_spotify() => {
                        self.to_spotify(SpotifyRequest::Previous);
                    }
                    Some(Command::Seek(ms)) if self.routes_to_spotify() => {
                        self.to_spotify(SpotifyRequest::Seek(ms));
                    }
                    // `None` means the handle was dropped: treat it as Shutdown.
                    Some(Command::Shutdown) | None => break,
                    Some(command) => {
                        self.playback.handle(command);
                        self.flush_playback();
                    }
                },
                event = next_event(&mut library_events) => match event {
                    Some(event) => self.library_event(event),
                    // Only the thread's end closes this channel, and until shutdown nothing
                    // asks it to end.
                    None => {
                        library_events = None;
                        self.library_lost();
                    }
                },
                event = next_event(&mut engine_events) => match event {
                    Some(event) => {
                        self.playback.on_engine(event);
                        self.flush_playback();
                    }
                    // Only the thread's end closes this channel. No sound can come any more,
                    // which to the user is the same as having no device.
                    None => {
                        tracing::error!("the audio engine stopped unexpectedly");
                        engine_events = None;
                        self.playback.engine_lost();
                        self.flush_playback();
                    }
                },
                event = next_event(&mut spotify_events) => match event {
                    Some(event) => self.spotify_event(event),
                    // Only the thread's end closes this channel.
                    None => {
                        tracing::error!("the Spotify thread stopped unexpectedly");
                        spotify_events = None;
                        self.spotify = None;
                        self.spotify_playing = false;
                        self.show(ActivePlayer::Local);
                    }
                },
                _ = tick.tick() => {
                    self.playback.tick();
                    self.flush_playback();
                }
                () = deadline_reached(self.deadline) => self.flush().await,
            }
        }
        // Shutdown, or the handle was dropped. The queue goes to the library first, ahead of the
        // request that stops it. The sound stops and a running scan starts winding down while the
        // settings are written.
        self.playback.finish();
        self.flush_playback();
        let library = self.library.take();
        if let Some(library) = &library {
            library.send(LibraryRequest::Shutdown);
        }
        // This writes the newest settings, or retries the ones whose save
        // failed, once. Nobody reads events any more, so the outcome is only
        // logged (by `save`).
        if let Some(settings) = self.pending.take() {
            self.save(settings).await;
        }
        // The handles log a thread that timed out or panicked. The results of the
        // tasks only say the same again.
        let engine = self.playback.into_engine();
        let engine = tokio::task::spawn_blocking(move || engine.shutdown(ENGINE_SHUTDOWN_TIMEOUT));
        let spotify = self.spotify.take().map(|spotify| {
            tokio::task::spawn_blocking(move || spotify.join(LIBRARY_SHUTDOWN_TIMEOUT))
        });
        if let Some(library) = library {
            let stop = move || library.shutdown(LIBRARY_SHUTDOWN_TIMEOUT);
            let _ = tokio::task::spawn_blocking(stop).await;
        }
        let _ = engine.await;
        if let Some(spotify) = spotify {
            let _ = spotify.await;
        }
    }

    fn routes_to_spotify(&self) -> bool {
        self.active == ActivePlayer::Spotify && self.spotify.is_some()
    }

    /// Forwards `request` to Spotify without waiting. Without Spotify it goes nowhere.
    fn to_spotify(&self, request: SpotifyRequest) {
        if let Some(spotify) = &self.spotify {
            spotify.send(request);
        }
    }

    /// Shows `active` in the bar, and tells the UI if that changed.
    fn show(&mut self, active: ActivePlayer) {
        if self.active != active {
            self.active = active;
            self.emit([Event::ActivePlayer(active)]);
            self.sync_cadence();
        }
    }

    /// Polling stops while the window is hidden, is quick while the bar shows Spotify and slow
    /// otherwise.
    fn sync_cadence(&mut self) {
        let cadence = if !self.window_shown {
            Cadence::Off
        } else if self.active == ActivePlayer::Spotify {
            Cadence::Fast
        } else {
            Cadence::Slow
        };
        if self.spotify.is_some() && self.cadence != Some(cadence) {
            self.cadence = Some(cadence);
            self.to_spotify(SpotifyRequest::Cadence(cadence));
        }
    }

    fn spotify_request(&mut self, request: SpotifyRequest) {
        if self.spotify.is_none() {
            return;
        }
        match request {
            // The backend's own.
            SpotifyRequest::Cadence(_) | SpotifyRequest::Shutdown => {}
            SpotifyRequest::Play(play) => {
                if self.local_playing {
                    self.playback.handle(Command::TogglePlay);
                    self.flush_playback();
                }
                self.show(ActivePlayer::Spotify);
                self.to_spotify(SpotifyRequest::Play(play));
            }
            SpotifyRequest::Device(id) => {
                self.to_spotify(SpotifyRequest::Device(id.clone()));
                self.set_spotify_device(id);
            }
            SpotifyRequest::PlayInAria { on, strings } => {
                self.to_spotify(SpotifyRequest::PlayInAria { on, strings });
                self.set_play_in_aria(on);
            }
            SpotifyRequest::SignOut => {
                self.to_spotify(SpotifyRequest::SignOut);
                self.set_spotify_device(None);
                self.spotify_playing = false;
                self.show(ActivePlayer::Local);
            }
            other => self.to_spotify(other),
        }
    }

    /// Keeps "Play in Aria" in the settings, and tells the UI.
    fn set_play_in_aria(&mut self, on: bool) {
        if self.settings.spotify_play_in_aria != on {
            let settings = Settings {
                spotify_play_in_aria: on,
                ..self.settings.clone()
            };
            self.emit([Event::SettingsReplaced(settings.clone())]);
            self.apply(settings);
        }
    }

    /// Keeps the chosen Spotify device in the settings, and tells the UI.
    fn set_spotify_device(&mut self, id: Option<String>) {
        let settings = Settings {
            spotify_device: id,
            ..self.settings.clone()
        }
        .sanitized();
        if settings.spotify_device != self.settings.spotify_device {
            self.emit([Event::SettingsReplaced(settings.clone())]);
            self.apply(settings);
        }
    }

    fn spotify_event(&mut self, event: SpotifyEvent) {
        match &event {
            SpotifyEvent::Playback(playback) => {
                self.spotify_playing = playback.as_ref().is_some_and(|p| p.state.playing);
                // Started on a phone or a speaker: the bar follows it.
                if self.spotify_playing && !self.local_playing {
                    self.show(ActivePlayer::Spotify);
                }
            }
            SpotifyEvent::Account(SpotifyAccount::SignedOut { .. }) => {
                self.spotify_playing = false;
                self.show(ActivePlayer::Local);
            }
            _ => {}
        }
        self.emit([Event::Spotify(event)]);
    }

    /// Passes on what the playback service has collected. An empty batch wakes nobody.
    fn flush_playback(&mut self) {
        for request in self.playback.take_requests() {
            self.to_library(request);
        }
        let events = self.playback.take_events();
        if let Some(view) = events.iter().rev().find_map(|event| match event {
            Event::Player(view) => Some(view),
            _ => None,
        }) {
            self.local_playing = view.playing;
        }
        if !events.is_empty() {
            self.emit(events);
        }
    }

    /// Lists devices with `list` on the blocking pool, since that can be slow, and sends the
    /// answer from there, as `event`. If they cannot be listed the answer is empty, and the UI
    /// still offers the system default.
    fn list(&self, list: DeviceLister, event: fn(Vec<DeviceInfo>) -> Event) {
        let (events, waker) = (self.events.clone(), Arc::clone(&self.waker));
        tokio::task::spawn_blocking(move || {
            let devices = list().unwrap_or_else(|error| {
                tracing::warn!(%error, "the devices could not be listed");
                Vec::new()
            });
            // Fails only once the handle is gone, when nobody is listening.
            let _ = events.send(event(devices));
            waker();
        });
    }

    /// Backs the library and the settings up into a new folder inside `parent`. The settings
    /// are written here; the library thread copies the database, and says when it is done.
    fn back_up(&mut self, parent: &Path) {
        if self.library.is_none() {
            return self.emit([Event::Notice(library_service::UNAVAILABLE)]);
        }
        let failed = || Event::Notice(Notice::new(NoticeLevel::Error, backup::BACKUP_FAILED));
        let folder = match backup::new_backup_folder(parent, SystemTime::now()) {
            Ok(folder) => folder,
            Err(error) => {
                tracing::warn!(%error, "no folder could be made for the backup");
                return self.emit([failed()]);
            }
        };
        let written = toml::to_string(&self.settings)
            .map_err(io::Error::other)
            .and_then(|text| std::fs::write(folder.join(backup::SETTINGS_FILE), text));
        if let Err(error) = written {
            tracing::warn!(%error, "the settings could not be backed up");
            return self.emit([failed()]);
        }
        let name = folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.to_library(LibraryRequest::BackUp {
            to: folder.join(backup::LIBRARY_FILE),
            name,
        });
    }

    /// Checks the backup in `folder` and gets it ready for the next start, on the blocking
    /// pool, since it copies the whole database.
    fn restore(&self, folder: PathBuf) {
        let Some(data_dir) = self.data_dir.clone() else {
            return self.emit([Event::Notice(library_service::UNAVAILABLE)]);
        };
        let (events, waker) = (self.events.clone(), Arc::clone(&self.waker));
        tokio::task::spawn_blocking(move || {
            let notice = match backup::stage_restore(&folder, &data_dir) {
                Ok(()) => Notice::new(NoticeLevel::Info, backup::RESTORE_STAGED),
                Err(backup::RestoreError::NotABackup) => {
                    Notice::new(NoticeLevel::Warning, backup::RESTORE_NOT_A_BACKUP)
                }
                Err(backup::RestoreError::Newer) => {
                    Notice::new(NoticeLevel::Warning, backup::RESTORE_NEWER)
                }
                Err(backup::RestoreError::Io(error)) => {
                    tracing::warn!(%error, "the backup could not be copied for the restore");
                    Notice::new(NoticeLevel::Error, backup::RESTORE_FAILED)
                }
            };
            // Fails only once the handle is gone, when nobody is listening.
            let _ = events.send(Event::Notice(notice));
            waker();
        });
    }

    /// Watches the library's folders if the settings ask for it, and none if not.
    fn sync_watch(&mut self) {
        let folders = if self.settings.watch_folders {
            self.library_folders.as_slice()
        } else {
            &[]
        };
        if let Some(watcher) = &mut self.watcher {
            watcher.watch(folders);
        }
    }

    /// Puts `settings` in effect and schedules their save.
    fn apply(&mut self, settings: Settings) {
        let watching = self.settings.watch_folders;
        self.settings = settings.clone();
        if settings.watch_folders != watching {
            self.sync_watch();
        }
        self.playback.on_settings(&settings);
        self.flush_playback();
        self.update(settings);
    }

    /// Flips "Send my microphone", as the microphone hotkey does. It needs no library. Without a
    /// second device it is not turned on, as in Settings, and the notice says why; it can always
    /// be turned off.
    fn toggle_microphone(&mut self) {
        if !self.settings.mic_enabled && self.settings.soundboard_device.is_none() {
            self.emit([Event::Notice(chat::MICROPHONE_NEEDS_DEVICE)]);
            return;
        }
        let settings = Settings {
            mic_enabled: !self.settings.mic_enabled,
            ..self.settings.clone()
        };
        let notice = if settings.mic_enabled {
            chat::MICROPHONE_ON
        } else {
            chat::MICROPHONE_OFF
        };
        self.emit([
            Event::SettingsReplaced(settings.clone()),
            Event::Notice(notice),
        ]);
        self.apply(settings);
    }

    /// Forwards `request` without waiting. Without a library it goes nowhere.
    fn to_library(&self, request: LibraryRequest) {
        if let Some(library) = &self.library {
            library.send(request);
        }
    }

    /// Like [`to_library`](Self::to_library), but a request that finds no library is answered
    /// with the "library unavailable" notice, so the click that caused it is not lost silently.
    /// Collections and the soundboard go this way.
    fn to_collections(&self, request: LibraryRequest) {
        if self.library.is_some() {
            self.to_library(request);
        } else {
            self.emit([Event::Notice(library_service::UNAVAILABLE)]);
        }
    }

    fn library_event(&mut self, event: LibraryEvent) {
        match event {
            LibraryEvent::Snapshot(snapshot) => {
                let folders: Vec<PathBuf> = snapshot
                    .folders
                    .iter()
                    .map(|folder| folder.path.clone())
                    .collect();
                if folders != self.library_folders {
                    self.library_folders = folders;
                    self.sync_watch();
                }
                // The UI hears of the snapshot before any view that refers to it.
                self.emit([Event::LibraryUpdated(Arc::clone(&snapshot))]);
                self.playback.on_snapshot(snapshot);
            }
            LibraryEvent::Collections(collections) => self.emit([Event::Collections(collections)]),
            LibraryEvent::Progress(progress) => self.emit([Event::ScanProgress(progress)]),
            LibraryEvent::Notice(notice) => self.emit([Event::Notice(notice)]),
            LibraryEvent::QueueLoaded(record) => self.playback.on_queue_loaded(record),
            LibraryEvent::Sounds(sounds) => self.playback.on_sounds(sounds),
        }
        self.flush_playback();
    }

    /// `settings` with a "Stop all" or microphone hotkey that may not be used put back as it
    /// was. The notice says why, and the UI is given the settings as they now are.
    fn vetted(&self, settings: Settings) -> Settings {
        let stop = self.playback.stop_hotkey_refusal(&settings);
        let mic = self.playback.mic_hotkey_refusal(&settings);
        let Some(notice) = stop.clone().or_else(|| mic.clone()) else {
            return settings;
        };
        let kept = Settings {
            stop_sounds_hotkey: if stop.is_some() {
                self.playback.stop_hotkey()
            } else {
                settings.stop_sounds_hotkey
            },
            mic_toggle_hotkey: if mic.is_some() {
                self.playback.mic_hotkey()
            } else {
                settings.mic_toggle_hotkey
            },
            ..settings
        };
        self.emit([Event::Notice(notice), Event::SettingsReplaced(kept.clone())]);
        kept
    }

    /// The library thread died. It can tell nobody, so this ends any scan it left showing and
    /// disables the library, as a library that cannot be opened is disabled: the pages go to
    /// their empty states, collections included.
    fn library_lost(&mut self) {
        tracing::error!("the library thread stopped unexpectedly; the library is disabled");
        self.library = None;
        self.emit([
            Event::ScanProgress(None),
            Event::LibraryUpdated(Arc::default()),
            Event::Collections(Arc::default()),
            Event::Notice(library_service::UNAVAILABLE),
        ]);
        self.playback.library_lost();
        self.flush_playback();
    }

    fn update(&mut self, settings: Settings) {
        // `None` here means read-only: no save is in flight, since saves are
        // awaited where they start.
        if self.sink.is_none() {
            // Read-only: nothing will be written, so nothing is scheduled.
            return;
        }
        self.pending = Some(settings);
        // An overflowing deadline (an absurd debounce) leaves the write to
        // shutdown.
        self.deadline = Instant::now().checked_add(self.debounce);
    }

    /// The default settings, except for the volume and mute: someone who resets to fix a theme
    /// should not get full volume in their headphones.
    fn defaults(&self) -> Settings {
        Settings {
            volume: self.settings.volume,
            muted: self.settings.muted,
            ..Settings::default()
        }
    }

    async fn reset(&mut self) {
        self.pending = None;
        self.deadline = None;
        let defaults = self.defaults();
        let outcome = self.save(defaults.clone()).await;
        let mut batch = vec![Event::SettingsReplaced(defaults)];
        batch.extend(outcome);
        self.emit(batch);
    }

    /// Writes the pending settings, if any, once the debounce deadline is
    /// reached, and reports the outcome.
    async fn flush(&mut self) {
        self.deadline = None;
        let Some(settings) = self.pending.take() else {
            return;
        };
        if let Some(outcome) = self.save(settings).await {
            self.emit([outcome]);
        }
    }

    /// Writes `settings` on a blocking thread and logs the outcome. Returns
    /// the event that reports it to the UI, or `None` in read-only mode and
    /// for a failure after one already reported.
    ///
    /// Settings that fail to save stay pending, with no deadline, unless newer
    /// ones are already pending.
    async fn save(&mut self, settings: Settings) -> Option<Event> {
        // `None` here means read-only: no other save is in flight, since saves
        // are awaited where they start.
        let mut sink = self.sink.take()?;
        let task = tokio::task::spawn_blocking(move || {
            let result = sink.save(&settings);
            (sink, settings, result)
        });
        let saved = match task.await {
            Ok((sink, settings, result)) => {
                self.sink = Some(sink);
                match result {
                    Ok(()) => {
                        tracing::debug!("settings saved");
                        true
                    }
                    Err(error) => {
                        tracing::error!(%error, "cannot save settings; keeping them for a retry");
                        if self.pending.is_none() {
                            self.pending = Some(settings);
                        }
                        false
                    }
                }
            }
            // The sink panicked and was lost with its task, so the backend is
            // read-only from now on.
            Err(error) => {
                tracing::error!(%error, "the settings sink panicked; settings will not be saved");
                false
            }
        };
        if saved {
            self.failing = false;
            Some(Event::SettingsSaved)
        } else if mem::replace(&mut self.failing, true) {
            // Already reported; the toast for it is still on screen or was dismissed.
            None
        } else {
            Some(Event::Notice(SAVE_FAILED))
        }
    }

    /// Sends a batch of events, then wakes the UI once.
    fn emit(&self, batch: impl IntoIterator<Item = Event>) {
        for event in batch {
            // Fails only once the handle is gone, when nobody is listening.
            let _ = self.events.send(event);
        }
        (self.waker)();
    }
}

/// The next event of a service's thread, or `None` once its channel has closed. Without a channel
/// it never completes.
async fn next_event<T>(events: &mut Option<UnboundedReceiver<T>>) -> Option<T> {
    match events {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    }
}

/// Completes at `deadline`, or never if there is none.
async fn deadline_reached(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        fs, io,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    use aria_audio::{
        CaptureFormat, DeviceId, DeviceInfo, OutputFormat,
        fake::{FakeCapture, FakeFactory},
    };
    use aria_library::{FolderId, SoundId, TrackId};

    use super::*;
    use crate::{
        chat::MicState,
        command::Command,
        error::SettingsError,
        event::Event,
        hotkey::Hotkey,
        notice::{Notice, NoticeLevel},
        playback_service::PlayerView,
        settings::{Settings, Theme, store::SettingsStore},
        soundboard::SoundboardState,
    };

    const FAST: BackendConfig = BackendConfig {
        debounce: Duration::from_millis(50),
        watch_quiet: watch::QUIET,
    };
    /// A debounce no test outlives, so only shutdown can write a pending
    /// update.
    const SLOW: BackendConfig = BackendConfig {
        debounce: Duration::from_secs(3600),
        watch_quiet: watch::QUIET,
    };
    /// Upper bound for every wait; generous so slow CI runners do not flake.
    const MAX: Duration = Duration::from_secs(2);
    /// The same for waits on the library thread, which opens a database and scans.
    const LIBRARY_MAX: Duration = Duration::from_secs(30);

    /// Polls `cond` every 10 ms until it holds or `max` has elapsed.
    fn wait_until(cond: impl Fn() -> bool, max: Duration) -> bool {
        let start = Instant::now();
        loop {
            if cond() {
                return true;
            }
            if start.elapsed() >= max {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Drains events until `pred` holds for all events drained so far.
    fn drain_until(handle: &BackendHandle, pred: impl Fn(&[Event]) -> bool) -> Vec<Event> {
        drain_until_within(handle, MAX, pred)
    }

    /// [`drain_until`] with a limit of the caller's.
    fn drain_until_within(
        handle: &BackendHandle,
        max: Duration,
        pred: impl Fn(&[Event]) -> bool,
    ) -> Vec<Event> {
        let seen = RefCell::new(Vec::new());
        let met = wait_until(
            || {
                let mut seen = seen.borrow_mut();
                seen.extend(handle.drain());
                pred(&seen)
            },
            max,
        );
        let seen = seen.into_inner();
        assert!(met, "condition not met; events drained: {seen:?}");
        seen
    }

    /// Records every settings value it is asked to save; the first
    /// `fail_first` calls fail with an I/O error.
    struct RecordingSink {
        calls: Arc<Mutex<Vec<Settings>>>,
        fail_first: usize,
    }

    impl SettingsSink for RecordingSink {
        fn save(&mut self, s: &Settings) -> Result<(), SettingsError> {
            self.calls.lock().unwrap().push(s.clone());
            if self.fail_first > 0 {
                self.fail_first -= 1;
                return Err(SettingsError::Io {
                    path: PathBuf::from("settings.toml"),
                    source: io::Error::other("disk full"),
                });
            }
            Ok(())
        }
    }

    /// The sink plus its call log. The backend holds the only other reference
    /// to the log, so its strong count drops to 1 once the backend has
    /// dropped the sink, which happens after its loop ends.
    fn recording_sink(fail_first: usize) -> (Box<dyn SettingsSink>, Arc<Mutex<Vec<Settings>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let sink = RecordingSink {
            calls: Arc::clone(&calls),
            fail_first,
        };
        (Box::new(sink), calls)
    }

    /// Blocks every save until the test drops the gate's sender.
    struct GatedSink {
        gate: mpsc::Receiver<()>,
        _alive: Arc<()>,
    }

    impl SettingsSink for GatedSink {
        fn save(&mut self, _: &Settings) -> Result<(), SettingsError> {
            // Returns once the sender is dropped; no message is ever sent.
            let _ = self.gate.recv();
            Ok(())
        }
    }

    fn counting_waker() -> (Waker, Arc<AtomicUsize>) {
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let waker: Waker = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        (waker, wakes)
    }

    /// Services that make no sound on this machine.
    fn services(library: Option<LibraryPaths>) -> BackendServices {
        let format = OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        };
        BackendServices {
            spotify: None,
            library,
            output: Box::new(FakeFactory::new(format)),
            capture: Box::new(no_microphone()),
            settings: Settings::default(),
        }
    }

    /// Playback on an output that nothing listens to.
    fn silent_playback() -> PlaybackLink {
        let factory = FakeFactory::new(OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        });
        PlaybackLink::start(
            Box::new(factory),
            Box::new(no_microphone()),
            &Settings::default(),
        )
        .unwrap()
    }

    /// A microphone nobody speaks into, so no test opens a real one.
    fn no_microphone() -> FakeCapture {
        FakeCapture::new(CaptureFormat {
            sample_rate: 48_000,
            channels: 1,
        })
        .0
    }

    fn no_library() -> BackendServices {
        services(None)
    }

    fn library_paths(dir: &Path) -> LibraryPaths {
        LibraryPaths {
            db: dir.join("library.db"),
            artwork_dir: dir.join("artwork"),
        }
    }

    /// A library service reporting to a channel the way the real link does, whose `on_event` then
    /// runs `after` on the library thread.
    fn link_with(dir: &Path, after: impl Fn(&LibraryEvent) + Send + 'static) -> LibraryLink {
        let (events_tx, events) = unbounded_channel();
        let on_event = Box::new(move |event: LibraryEvent| {
            let _ = events_tx.send(event.clone());
            after(&event);
        });
        let handle = LibraryService::start(library_paths(dir), on_event).unwrap();
        LibraryLink {
            handle,
            events,
            data_dir: Some(dir.to_path_buf()),
        }
    }

    fn with_theme(theme: Theme) -> Settings {
        Settings {
            theme,
            ..Settings::default()
        }
    }

    fn save_failed() -> Event {
        Event::Notice(Notice {
            level: NoticeLevel::Error,
            key: "notice-settings-save-failed",
            args: Vec::new(),
        })
    }

    fn library_unavailable() -> Event {
        Event::Notice(Notice::new(NoticeLevel::Error, "library-unavailable"))
    }

    #[test]
    fn backend_forwards_library_events() {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("music");
        fs::create_dir(&music).unwrap();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../aria-audio/tests/fixtures/tone.ogg");
        fs::copy(fixture, music.join("tone.ogg")).unwrap();
        let (waker, wakes) = counting_waker();
        let services = services(Some(library_paths(dir.path())));
        let handle = Backend::start(None, services, waker, FAST).unwrap();

        handle.send(Command::AddFolder(music));
        let has_track = |e: &Event| matches!(e, Event::LibraryUpdated(s) if s.tracks.len() == 1);
        let events = drain_until_within(&handle, LIBRARY_MAX, |e| e.iter().any(has_track));
        assert!(events.contains(&Event::ScanProgress(None)));
        assert!(wakes.load(Ordering::SeqCst) >= 1);
        assert!(handle.shutdown(LIBRARY_MAX));
    }

    /// Plays by hand on `factory`'s latest output until `done` holds of the events so far.
    fn play_until(
        handle: &BackendHandle,
        factory: &FakeFactory,
        done: impl Fn(&[Event]) -> bool,
    ) -> Vec<Event> {
        let seen = RefCell::new(Vec::new());
        let met = wait_until(
            || {
                if let Some(output) = factory.outputs().last() {
                    output.pull(2_048);
                }
                let mut seen = seen.borrow_mut();
                seen.extend(handle.drain());
                done(&seen)
            },
            LIBRARY_MAX,
        );
        let seen = seen.into_inner();
        assert!(met, "condition not met; events drained: {seen:?}");
        seen
    }

    fn last_view(events: &[Event]) -> Option<&PlayerView> {
        events.iter().rev().find_map(|event| match event {
            Event::Player(view) => Some(view),
            _ => None,
        })
    }

    #[test]
    fn a_queue_plays_through_the_backend_and_comes_back_paused() {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("music");
        fs::create_dir(&music).unwrap();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../aria-audio/tests/fixtures/tone.ogg");
        fs::copy(&fixture, music.join("1.ogg")).unwrap();
        fs::copy(&fixture, music.join("2.ogg")).unwrap();
        let format = OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        };
        let speakers = DeviceId("speakers".into());
        let headphones = DeviceId("headphones".into());
        let factory = FakeFactory::new(format);
        let services = BackendServices {
            spotify: None,
            library: Some(library_paths(dir.path())),
            output: Box::new(factory.clone()),
            capture: Box::new(no_microphone()),
            settings: Settings {
                output_device: Some(speakers.clone()),
                ..Settings::default()
            },
        };
        let (waker, _) = counting_waker();
        let handle = Backend::start(None, services, waker, FAST).unwrap();

        handle.send(Command::AddFolder(music));
        let has_tracks = |e: &Event| matches!(e, Event::LibraryUpdated(s) if s.tracks.len() == 2);
        let events = drain_until_within(&handle, LIBRARY_MAX, |e| e.iter().any(has_tracks));
        let snapshot = events.iter().find_map(|e| match e {
            Event::LibraryUpdated(s) if s.tracks.len() == 2 => Some(Arc::clone(s)),
            _ => None,
        });
        let ids: Vec<TrackId> = snapshot.unwrap().tracks.iter().map(|t| t.id).collect();

        handle.send(Command::PlayContext {
            ids: ids.clone(),
            start: 0,
        });
        // The startup settings apply, and the clock moves.
        let clock = handle.clock();
        play_until(&handle, &factory, |_| clock.read().position_ms > 0);
        assert_eq!(factory.outputs()[0].device(), Some(speakers));

        // Settings reach playback even with nothing to save them to.
        handle.send(Command::UpdateSettings(Settings {
            output_device: Some(headphones.clone()),
            ..Settings::default()
        }));
        // On a slow machine the pulls above can play both one-second tracks out before the clock
        // is read; then the new device opens only with the next track. Starting the list again
        // gives the engine a track either way.
        handle.send(Command::PlayContext {
            ids: ids.clone(),
            start: 0,
        });
        let on_headphones = || {
            factory
                .outputs()
                .last()
                .is_some_and(|output| output.device() == Some(headphones.clone()))
        };
        assert!(wait_until(on_headphones, LIBRARY_MAX));

        // Only the engine's events can take the view to the end of the list.
        let last = ids[1];
        let at_end = |e: &[Event]| {
            last_view(e).is_some_and(|view| view.current == Some(last) && !view.playing)
        };
        play_until(&handle, &factory, at_end);
        assert!(handle.shutdown(LIBRARY_MAX));

        let factory = FakeFactory::new(format);
        let services = BackendServices {
            spotify: None,
            library: Some(library_paths(dir.path())),
            output: Box::new(factory.clone()),
            capture: Box::new(no_microphone()),
            settings: Settings::default(),
        };
        let (waker, _) = counting_waker();
        let handle = Backend::start(None, services, waker, FAST).unwrap();
        drain_until_within(&handle, LIBRARY_MAX, at_end);
        assert!(factory.outputs().is_empty(), "an output opened");
        assert!(handle.shutdown(LIBRARY_MAX));
    }

    #[test]
    fn output_devices_are_listed_off_the_worker() {
        let factory = FakeFactory::new(OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        });
        let speakers = DeviceInfo {
            id: DeviceId("speakers".into()),
            name: "Speakers".into(),
            is_default: true,
        };
        factory.set_devices(vec![speakers.clone()]);
        let services = BackendServices {
            spotify: None,
            library: None,
            output: Box::new(factory),
            capture: Box::new(no_microphone()),
            settings: Settings::default(),
        };
        let (waker, wakes) = counting_waker();
        let handle = Backend::start(None, services, waker, FAST).unwrap();

        handle.send(Command::ListDevices);

        let listed = Event::OutputDevices(vec![speakers]);
        drain_until(&handle, |e| e.contains(&listed));
        assert!(wakes.load(Ordering::SeqCst) >= 1);
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn a_collection_command_without_a_library_gives_the_unavailable_notice() {
        let (waker, _wakes) = counting_waker();
        let handle = Backend::start(None, no_library(), waker, FAST).unwrap();
        handle.send(Command::SetFavorite {
            track: TrackId(1),
            favorite: true,
        });
        drain_until(&handle, |e| e.contains(&library_unavailable()));
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn collection_commands_reach_the_library_and_come_back_as_events() {
        let dir = tempfile::tempdir().unwrap();
        let (waker, _wakes) = counting_waker();
        let services = services(Some(library_paths(dir.path())));
        let handle = Backend::start(None, services, waker, FAST).unwrap();

        handle.send(Command::CreatePlaylist {
            name: "Road".into(),
            first: None,
        });
        let named = |e: &Event| {
            matches!(e, Event::Collections(c)
                if c.playlists.first().is_some_and(|p| p.name == "Road"))
        };
        drain_until_within(&handle, LIBRARY_MAX, |e| e.iter().any(named));
        assert!(handle.shutdown(LIBRARY_MAX));
    }

    // Soundboard.

    fn soundboard(events: &[Event]) -> Option<&SoundboardState> {
        events.iter().rev().find_map(|event| match event {
            Event::Soundboard(state) => Some(state.as_ref()),
            _ => None,
        })
    }

    fn hotkey(text: &str) -> Hotkey {
        text.parse().unwrap()
    }

    fn sound_fixture(dir: &Path, name: &str) -> PathBuf {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../aria-audio/tests/fixtures/tone.ogg");
        let path = dir.join(name);
        fs::copy(fixture, &path).unwrap();
        path
    }

    /// A backend with a library holding one sound, the factory it plays on, and the sound's id.
    fn with_a_sound(dir: &Path, sink: Option<Box<dyn SettingsSink>>) -> (BackendHandle, SoundId) {
        let (waker, _) = counting_waker();
        let handle = Backend::start(sink, services(Some(library_paths(dir))), waker, FAST).unwrap();
        handle.send(Command::AddSounds(vec![sound_fixture(dir, "horn.ogg")]));
        let events = drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds.len() == 1)
        });
        let id = soundboard(&events).unwrap().sounds[0].id;
        (handle, id)
    }

    /// The loudest sample a sound at `volume` reaches on a fake output, fired through the whole
    /// backend: the volume set by command, stored by the library, and sent to the engine.
    fn loudest_at(volume: f32) -> f32 {
        let dir = tempfile::tempdir().unwrap();
        let factory = FakeFactory::new(OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        });
        let services = BackendServices {
            spotify: None,
            output: Box::new(factory.clone()),
            ..services(Some(library_paths(dir.path())))
        };
        let (waker, _) = counting_waker();
        let handle = Backend::start(None, services, waker, FAST).unwrap();
        handle.send(Command::AddSounds(vec![sound_fixture(
            dir.path(),
            "horn.ogg",
        )]));
        let events = drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds.len() == 1)
        });
        let id = soundboard(&events).unwrap().sounds[0].id;
        handle.send(Command::SetSoundVolume { id, volume });
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds[0].volume == volume)
        });
        handle.send(Command::PlaySound(id));
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds[0].playing)
        });
        let output = factory.outputs().pop().unwrap();
        let mut loudest = 0.0f32;
        // Half a second, past the fade at its start, reading the events as the app does.
        for _ in 0..50 {
            thread::sleep(Duration::from_millis(2));
            handle.drain();
            for sample in output.pull(480) {
                loudest = loudest.max(sample.abs());
            }
        }
        assert!(handle.shutdown(LIBRARY_MAX));
        loudest
    }

    #[test]
    fn a_sound_plays_at_the_volume_set_for_it() {
        let full = loudest_at(1.0);
        let quarter = loudest_at(0.25);
        assert!(full > 0.05, "the sound was not heard: {full}");
        assert!(
            (quarter / full - 0.25).abs() < 0.03,
            "at 25 % the sound peaked at {quarter}, at 100 % at {full}"
        );
    }

    #[test]
    fn sound_commands_come_back_as_soundboard_events() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, id) = with_a_sound(dir.path(), None);

        handle.send(Command::SetSoundVolume { id, volume: 0.5 });
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds[0].volume == 0.5)
        });
        handle.send(Command::PlaySound(id));
        let playing = |e: &[Event]| soundboard(e).is_some_and(|board| board.sounds[0].playing);
        let events = drain_until_within(&handle, LIBRARY_MAX, playing);
        assert!(
            !events.iter().any(|e| matches!(e, Event::Player(_))),
            "a sound does not move the player: {events:?}"
        );
        handle.send(Command::StopSounds);
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| !board.sounds[0].playing)
        });
        handle.send(Command::RemoveSound(id));
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds.is_empty())
        });
        assert!(handle.shutdown(LIBRARY_MAX));
    }

    #[test]
    fn a_sound_fires_while_the_library_is_busy() {
        let dir = tempfile::tempdir().unwrap();
        let path = sound_fixture(dir.path(), "horn.ogg");
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        // The library thread stops inside the event that brings the sound, as one stuck in a scan
        // or a slow read would. The event itself has gone out by then.
        let link = link_with(dir.path(), move |event| {
            if matches!(event, LibraryEvent::Sounds(sounds) if !sounds.is_empty()) {
                let _ = gate.lock().unwrap().recv();
            }
        });
        link.handle.send(LibraryRequest::AddSounds(vec![path]));
        let (waker, _) = counting_waker();
        let handle = Backend::spawn(None, Some(link), silent_playback(), waker, FAST).unwrap();
        let events = drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds.len() == 1)
        });
        let id = soundboard(&events).unwrap().sounds[0].id;

        handle.send(Command::PlaySound(id));
        // A slow machine takes a while to open the fake output and the sound's decoder.
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds[0].playing)
        });
        drop(release);
        assert!(handle.shutdown(LIBRARY_MAX));
    }

    #[test]
    fn a_missing_or_broken_sound_file_gives_its_own_notice() {
        let dir = tempfile::tempdir().unwrap();
        let (waker, _) = counting_waker();
        let handle =
            Backend::start(None, services(Some(library_paths(dir.path()))), waker, FAST).unwrap();
        let gone = dir.path().join("gone.ogg");
        let broken = dir.path().join("broken.ogg");
        fs::write(&broken, b"not audio").unwrap();
        handle.send(Command::AddSounds(vec![gone, broken]));
        let events = drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds.len() == 2)
        });
        let board = soundboard(&events).unwrap();
        let (gone, broken) = (board.sounds[0].id, board.sounds[1].id);
        let notice = |key| Event::Notice(Notice::new(NoticeLevel::Warning, key));

        handle.send(Command::PlaySound(gone));
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            e.contains(&notice("sound-missing"))
        });
        handle.send(Command::PlaySound(broken));
        let events = drain_until_within(&handle, LIBRARY_MAX, |e| {
            e.contains(&notice("sound-failed"))
        });
        assert!(
            !events.contains(&notice("sound-missing")),
            "the file is there: {events:?}"
        );
        assert!(handle.shutdown(LIBRARY_MAX));
    }

    #[test]
    fn a_stop_hotkey_used_by_a_sound_is_refused_when_the_settings_change() {
        let dir = tempfile::tempdir().unwrap();
        let (sink, saved) = recording_sink(0);
        let (handle, id) = with_a_sound(dir.path(), Some(sink));
        handle.send(Command::SetSoundHotkey {
            id,
            hotkey: Some(hotkey("Ctrl+F1")),
        });
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds[0].hotkey.is_some())
        });

        let wanted = Settings {
            theme: Theme::Dark,
            stop_sounds_hotkey: Some(hotkey("Ctrl+F1")),
            ..Settings::default()
        };
        handle.send(Command::UpdateSettings(wanted));
        let kept = with_theme(Theme::Dark);
        let taken = Event::Notice(Notice::new(NoticeLevel::Warning, "hotkey-taken"));
        drain_until(&handle, |e| {
            e.contains(&taken) && e.contains(&Event::SettingsReplaced(kept.clone()))
        });
        // A free one is taken as it is.
        let free = Settings {
            stop_sounds_hotkey: Some(hotkey("Ctrl+F2")),
            ..kept.clone()
        };
        handle.send(Command::UpdateSettings(free.clone()));
        assert!(handle.shutdown(LIBRARY_MAX));
        assert_eq!(saved.lock().unwrap().last(), Some(&free));
    }

    #[test]
    fn a_sound_hotkey_equal_to_stop_all_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, id) = with_a_sound(dir.path(), None);
        handle.send(Command::UpdateSettings(Settings {
            stop_sounds_hotkey: Some(hotkey("Ctrl+F2")),
            ..Settings::default()
        }));
        handle.send(Command::SetSoundHotkey {
            id,
            hotkey: Some(hotkey("Ctrl+F2")),
        });
        let taken = Event::Notice(Notice::new(NoticeLevel::Warning, "hotkey-taken"));
        drain_until_within(&handle, LIBRARY_MAX, |e| e.contains(&taken));
        assert!(handle.shutdown(LIBRARY_MAX));
    }

    fn microphone_notice(key: &'static str) -> Event {
        Event::Notice(Notice::new(NoticeLevel::Info, key))
    }

    /// Settings with a second device, which the microphone needs.
    fn with_cable() -> Settings {
        Settings {
            soundboard_device: Some(DeviceId("cable".into())),
            ..Settings::default()
        }
    }

    /// No library, and a second device.
    fn no_library_with_cable() -> BackendServices {
        BackendServices {
            spotify: None,
            settings: with_cable(),
            ..no_library()
        }
    }

    /// A backend whose library has the empty folder `music` in it, watched as `watch` says.
    fn watching(dir: &Path, watch: bool) -> (BackendHandle, PathBuf) {
        let music = dir.join("music");
        fs::create_dir(&music).unwrap();
        let services = BackendServices {
            spotify: None,
            settings: Settings {
                watch_folders: watch,
                ..Settings::default()
            },
            ..services(Some(library_paths(dir)))
        };
        let (waker, _) = counting_waker();
        let cfg = BackendConfig {
            watch_quiet: Duration::from_millis(200),
            ..FAST
        };
        let handle = Backend::start(None, services, waker, cfg).unwrap();
        handle.send(Command::AddFolder(music.clone()));
        let has_folder = |e: &Event| matches!(e, Event::LibraryUpdated(s) if s.folders.len() == 1);
        drain_until_within(&handle, LIBRARY_MAX, |e| e.iter().any(has_folder));
        // The folder's first scan, and the watch, settle.
        thread::sleep(Duration::from_millis(500));
        handle.drain();
        (handle, music)
    }

    fn copy_a_track_into(music: &Path) {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../aria-audio/tests/fixtures/tone.ogg");
        fs::copy(fixture, music.join("new.ogg")).unwrap();
    }

    fn notice_with(events: &[Event], key: &str) -> Option<Notice> {
        events.iter().find_map(|event| match event {
            Event::Notice(notice) if notice.key == key => Some(notice.clone()),
            _ => None,
        })
    }

    #[test]
    fn a_backup_holds_the_library_and_the_settings_and_can_be_restored() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, music) = watching(dir.path(), false);
        let parent = dir.path().join("backups");
        fs::create_dir(&parent).unwrap();

        handle.send(Command::BackUp(parent.clone()));
        let events = drain_until_within(&handle, LIBRARY_MAX, |e| {
            notice_with(e, backup::BACKUP_DONE).is_some()
        });
        let done = notice_with(&events, backup::BACKUP_DONE).unwrap();
        let folder = fs::read_dir(&parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .next()
            .expect("the backup folder");
        assert!(
            folder
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("Aria backup ")
        );
        assert_eq!(
            done.args[0].1,
            folder.file_name().unwrap().to_string_lossy()
        );
        aria_library::Library::check_backup(&folder.join(backup::LIBRARY_FILE)).unwrap();
        let settings = fs::read_to_string(folder.join(backup::SETTINGS_FILE)).unwrap();
        assert!(settings.contains("watch_folders = false"), "{settings}");

        handle.send(Command::Restore(folder));
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            notice_with(e, backup::RESTORE_STAGED).is_some()
        });
        assert!(backup::restore_pending(dir.path()));
        drop(music);
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn a_playlist_is_imported_from_m3u_and_exported_again() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, music) = watching(dir.path(), false);
        let list = dir.path().join("Road trip.m3u");
        fs::write(
            &list,
            "#EXTM3U
music/one.mp3
# a note
music/two.flac
",
        )
        .unwrap();

        handle.send(Command::ImportPlaylist(list));
        let events = drain_until_within(&handle, LIBRARY_MAX, |e| {
            notice_with(e, library_service::PLAYLIST_IMPORTED).is_some()
        });
        let imported = notice_with(&events, library_service::PLAYLIST_IMPORTED).unwrap();
        assert_eq!(
            imported.args,
            [("name", "Road trip".into()), ("count", "2".into())]
        );
        let collections = events
            .iter()
            .rev()
            .find_map(|event| match event {
                Event::Collections(collections) => Some(Arc::clone(collections)),
                _ => None,
            })
            .expect("the collections with the new playlist");
        let playlist = &collections.playlists[0];
        assert_eq!(playlist.name, "Road trip");
        // The files are not in the library: kept, and counted as unavailable.
        assert_eq!(playlist.unavailable, 2);

        let out = dir.path().join("out.m3u8");
        handle.send(Command::ExportPlaylist {
            id: playlist.id,
            to: out.clone(),
        });
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            notice_with(e, library_service::PLAYLIST_EXPORTED).is_some()
        });
        let text = fs::read_to_string(&out).unwrap();
        assert_eq!(
            aria_library::parse_m3u(&text, Path::new("/")),
            [music.join("one.mp3"), music.join("two.flac")]
        );
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn a_folder_that_is_not_a_backup_is_not_restored() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _music) = watching(dir.path(), false);
        handle.send(Command::Restore(dir.path().join("music")));
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            notice_with(e, backup::RESTORE_NOT_A_BACKUP).is_some()
        });
        assert!(!backup::restore_pending(dir.path()));
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn music_copied_into_a_library_folder_appears_without_a_rescan() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, music) = watching(dir.path(), true);
        copy_a_track_into(&music);
        let has_track = |e: &Event| matches!(e, Event::LibraryUpdated(s) if s.tracks.len() == 1);
        drain_until_within(&handle, LIBRARY_MAX, |e| e.iter().any(has_track));
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn with_watching_off_a_folder_waits_for_a_rescan() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, music) = watching(dir.path(), false);
        copy_a_track_into(&music);
        thread::sleep(Duration::from_millis(1_500));
        let has_track = |e: &Event| matches!(e, Event::LibraryUpdated(s) if !s.tracks.is_empty());
        assert!(
            !handle.drain().iter().any(has_track),
            "a folder was scanned though watching is off"
        );
        handle.send(Command::Rescan);
        drain_until_within(&handle, LIBRARY_MAX, |e| e.iter().any(has_track));
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn the_music_volume_in_the_chat_changes_what_the_cable_gets() {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("music");
        fs::create_dir(&music).unwrap();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../aria-audio/tests/fixtures/tone.ogg");
        fs::copy(&fixture, music.join("1.ogg")).unwrap();
        let factory = FakeFactory::new(OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        });
        let cable = DeviceId("cable".into());
        let sharing = |volume: f32| Settings {
            share_music: true,
            share_music_volume: volume,
            ..with_cable()
        };
        let services = BackendServices {
            spotify: None,
            output: Box::new(factory.clone()),
            settings: sharing(1.0),
            ..services(Some(library_paths(dir.path())))
        };
        let (waker, _) = counting_waker();
        let handle = Backend::start(None, services, waker, FAST).unwrap();
        handle.send(Command::AddFolder(music));
        let has_track = |e: &Event| matches!(e, Event::LibraryUpdated(s) if s.tracks.len() == 1);
        let events = drain_until_within(&handle, LIBRARY_MAX, |e| e.iter().any(has_track));
        let id = events
            .iter()
            .find_map(|e| match e {
                Event::LibraryUpdated(s) if s.tracks.len() == 1 => Some(s.tracks[0].id),
                _ => None,
            })
            .unwrap();
        handle.send(Command::PlayContext {
            ids: vec![id],
            start: 0,
        });
        let deadline = Instant::now() + LIBRARY_MAX;
        while factory.outputs_on(Some(&cable)).is_empty() {
            handle.drain();
            assert!(Instant::now() < deadline, "the cable never opened");
            thread::sleep(Duration::from_millis(5));
        }
        let mine = factory.outputs_on(None).pop().unwrap();
        let copy = factory.outputs_on(Some(&cable)).pop().unwrap();
        // The cable opens beside the music; the copy goes on it at the engine's next pass.
        while copy.inputs_added() == 0 {
            handle.drain();
            assert!(
                Instant::now() < deadline,
                "the copy never reached the cable"
            );
            thread::sleep(Duration::from_millis(5));
        }
        // The loudest the copy gets over a quarter of a second, pulled beside the music.
        // The backend's events are read meanwhile, as the app does: a queue left full holds the
        // engine up.
        let loudest = || {
            let mut loudest = 0.0f32;
            for _ in 0..25 {
                thread::sleep(Duration::from_millis(2));
                handle.drain();
                mine.pull(480);
                for sample in copy.pull(480) {
                    loudest = loudest.max(sample.abs());
                }
            }
            loudest
        };
        loudest();
        let full = loudest();
        // Turned down while the music plays, as the slider in Settings does.
        handle.send(Command::UpdateSettings(sharing(0.25)));
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(5));
            handle.drain();
        }
        loudest();
        let quarter = loudest();
        assert!(handle.shutdown(MAX));
        assert!(full > 0.05, "the cable got no music: {full}");
        assert!(
            (quarter / full - 0.25).abs() < 0.05,
            "at 25 % the cable peaked at {quarter}, at 100 % at {full}"
        );
    }

    #[test]
    fn toggle_microphone_flips_saves_and_notices() {
        let (sink, saved) = recording_sink(0);
        let (waker, _wakes) = counting_waker();
        let handle = Backend::start(Some(sink), no_library_with_cable(), waker, FAST).unwrap();

        handle.send(Command::ToggleMicrophone);
        let on = Settings {
            mic_enabled: true,
            ..with_cable()
        };
        drain_until(&handle, |e| {
            e.contains(&Event::SettingsReplaced(on.clone()))
                && e.contains(&microphone_notice("microphone-on"))
        });

        handle.send(Command::ToggleMicrophone);
        drain_until(&handle, |e| {
            e.contains(&Event::SettingsReplaced(with_cable()))
                && e.contains(&microphone_notice("microphone-off"))
        });
        assert!(handle.shutdown(MAX));
        assert_eq!(saved.lock().unwrap().last(), Some(&with_cable()));
    }

    #[test]
    fn toggle_microphone_without_a_second_device_says_so_and_stays_off() {
        let (sink, saved) = recording_sink(0);
        let (waker, _wakes) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, FAST).unwrap();

        handle.send(Command::ToggleMicrophone);
        let needs = Event::Notice(Notice::new(NoticeLevel::Warning, "microphone-needs-device"));
        let events = drain_until(&handle, |e| e.contains(&needs));
        assert!(
            !events.contains(&microphone_notice("microphone-on")),
            "{events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::SettingsReplaced(s) if s.mic_enabled)),
            "{events:?}"
        );
        assert!(handle.shutdown(MAX));
        assert!(saved.lock().unwrap().iter().all(|s| !s.mic_enabled));
    }

    #[test]
    fn toggle_microphone_turns_it_off_without_a_second_device_too() {
        // Left on from before the second device was removed.
        let services = BackendServices {
            spotify: None,
            settings: Settings {
                mic_enabled: true,
                ..Settings::default()
            },
            ..no_library()
        };
        let (waker, _wakes) = counting_waker();
        let handle = Backend::start(None, services, waker, FAST).unwrap();

        handle.send(Command::ToggleMicrophone);
        drain_until(&handle, |e| {
            e.contains(&Event::SettingsReplaced(Settings::default()))
                && e.contains(&microphone_notice("microphone-off"))
        });
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn toggle_microphone_keeps_the_other_settings() {
        let (waker, _wakes) = counting_waker();
        let handle = Backend::start(None, no_library(), waker, FAST).unwrap();
        let dark = Settings {
            soundboard_device: Some(DeviceId("cable".into())),
            ..with_theme(Theme::Dark)
        };
        handle.send(Command::UpdateSettings(dark.clone()));

        handle.send(Command::ToggleMicrophone);

        let on = Settings {
            mic_enabled: true,
            ..dark
        };
        drain_until(&handle, |e| {
            e.contains(&Event::SettingsReplaced(on.clone()))
        });
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn with_the_library_off_toggle_microphone_still_works() {
        let (waker, _wakes) = counting_waker();
        let handle = Backend::start(None, no_library_with_cable(), waker, FAST).unwrap();

        handle.send(Command::ToggleMicrophone);

        let events = drain_until(&handle, |e| e.contains(&microphone_notice("microphone-on")));
        assert!(!events.contains(&library_unavailable()), "{events:?}");
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn input_devices_are_listed_on_request() {
        let mut services = no_library();
        let (capture, control) = FakeCapture::new(CaptureFormat {
            sample_rate: 48_000,
            channels: 1,
        });
        let mic = DeviceInfo {
            id: DeviceId("mic".into()),
            name: "Microphone".into(),
            is_default: true,
        };
        control.set_devices(vec![mic.clone()]);
        services.capture = Box::new(capture);
        let (waker, _wakes) = counting_waker();
        let handle = Backend::start(None, services, waker, FAST).unwrap();

        handle.send(Command::ListInputDevices);

        drain_until(&handle, |e| {
            e.contains(&Event::InputDevices(vec![mic.clone()]))
        });
        // Listing opens nothing.
        assert!(!control.open_now());
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn the_microphone_opens_with_a_second_device_and_shows_in_the_view() {
        let mut services = no_library();
        let (capture, control) = FakeCapture::new(CaptureFormat {
            sample_rate: 48_000,
            channels: 1,
        });
        services.capture = Box::new(capture);
        let (waker, _wakes) = counting_waker();
        let handle = Backend::start(None, services, waker, FAST).unwrap();

        // No second device: the toggle says so, and nothing opens.
        handle.send(Command::ToggleMicrophone);
        let needs = Event::Notice(Notice::new(NoticeLevel::Warning, "microphone-needs-device"));
        drain_until(&handle, |e| e.contains(&needs));
        assert!(!control.open_now());

        handle.send(Command::UpdateSettings(Settings {
            mic_enabled: true,
            soundboard_device: Some(DeviceId("cable".into())),
            ..Settings::default()
        }));
        drain_until(&handle, |e| {
            last_view(e).is_some_and(|view| view.microphone == MicState::On)
        });
        assert!(control.open_now());

        handle.send(Command::ToggleMicrophone);
        drain_until(&handle, |e| {
            last_view(e).is_some_and(|view| view.microphone == MicState::Off)
        });
        assert!(wait_until(|| !control.open_now(), MAX));
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn the_microphone_hotkey_is_unique() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, id) = with_a_sound(dir.path(), None);
        let taken = Event::Notice(Notice::new(NoticeLevel::Warning, "hotkey-taken"));
        handle.send(Command::SetSoundHotkey {
            id,
            hotkey: Some(hotkey("Ctrl+F1")),
        });
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            soundboard(e).is_some_and(|board| board.sounds[0].hotkey.is_some())
        });

        // Against a sound's hotkey.
        handle.send(Command::UpdateSettings(Settings {
            mic_toggle_hotkey: Some(hotkey("Ctrl+F1")),
            ..Settings::default()
        }));
        drain_until(&handle, |e| {
            e.contains(&taken) && e.contains(&Event::SettingsReplaced(Settings::default()))
        });

        // Against "Stop all", in the same settings, either way round.
        let both = Settings {
            stop_sounds_hotkey: Some(hotkey("Ctrl+F3")),
            mic_toggle_hotkey: Some(hotkey("Ctrl+F3")),
            ..Settings::default()
        };
        handle.send(Command::UpdateSettings(both));
        drain_until(&handle, |e| e.contains(&taken));

        // Against Aria's own shortcuts, which take the Command key on macOS.
        let own = if cfg!(target_os = "macos") {
            "Win+1"
        } else {
            "Ctrl+1"
        };
        handle.send(Command::UpdateSettings(Settings {
            mic_toggle_hotkey: Some(hotkey(own)),
            ..Settings::default()
        }));
        drain_until(&handle, |e| e.contains(&taken));

        // A free one is kept, and then a sound may not take it, nor "Stop all".
        let free = Settings {
            mic_toggle_hotkey: Some(hotkey("Ctrl+F4")),
            ..Settings::default()
        };
        handle.send(Command::UpdateSettings(free.clone()));
        handle.send(Command::SetSoundHotkey {
            id,
            hotkey: Some(hotkey("Ctrl+F4")),
        });
        drain_until_within(&handle, LIBRARY_MAX, |e| e.contains(&taken));
        handle.send(Command::UpdateSettings(Settings {
            stop_sounds_hotkey: Some(hotkey("Ctrl+F4")),
            ..free.clone()
        }));
        drain_until(&handle, |e| {
            e.contains(&taken) && e.contains(&Event::SettingsReplaced(free.clone()))
        });
        assert!(handle.shutdown(LIBRARY_MAX));
    }

    #[test]
    fn sound_commands_without_a_library_give_the_unavailable_notice() {
        let (waker, _wakes) = counting_waker();
        let handle = Backend::start(None, no_library(), waker, FAST).unwrap();
        let id = SoundId(1);
        handle.send(Command::AddSounds(vec!["horn.ogg".into()]));
        handle.send(Command::RenameSound {
            id,
            name: "Horn".into(),
        });
        handle.send(Command::SetSoundVolume { id, volume: 0.5 });
        handle.send(Command::SetSoundHotkey { id, hotkey: None });
        handle.send(Command::RemoveSound(id));
        handle.send(Command::PlaySound(id));
        // Stop all needs no library, and has nothing to say.
        handle.send(Command::StopSounds);
        let unavailable = |e: &[Event]| e.iter().filter(|e| **e == library_unavailable()).count();
        let events = drain_until(&handle, |e| unavailable(e) == 6);
        assert!(handle.shutdown(MAX));
        assert_eq!(unavailable(&events), 6, "{events:?}");
    }

    #[test]
    fn library_commands_without_a_library_go_nowhere() {
        let (waker, wakes) = counting_waker();
        let handle = Backend::start(None, no_library(), waker, FAST).unwrap();
        handle.send(Command::AddFolder("music".into()));
        handle.send(Command::RemoveFolder(FolderId(1)));
        handle.send(Command::Rescan);
        handle.send(Command::ClearArtwork);
        assert!(handle.shutdown(MAX));
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_library_panic_leaves_the_backend_running() {
        let dir = tempfile::tempdir().unwrap();
        let link = link_with(dir.path(), |event| {
            if matches!(event, LibraryEvent::Snapshot(_)) {
                panic!("test library thread panics");
            }
        });
        let (sink, saved) = recording_sink(0);
        let (waker, _) = counting_waker();
        let handle =
            Backend::spawn(Some(sink), Some(link), silent_playback(), waker, FAST).unwrap();

        let events =
            drain_until_within(&handle, LIBRARY_MAX, |e| e.contains(&library_unavailable()));
        assert!(events.contains(&Event::ScanProgress(None)), "{events:?}");
        // The snapshot the thread died on, then the empty one that disables the library.
        let snapshots = events
            .iter()
            .filter(|e| **e == Event::LibraryUpdated(Arc::default()))
            .count();
        assert_eq!(snapshots, 2, "{events:?}");
        // The thread died before it sent any collections, so these empty ones come from the loss:
        // the pages show their empty states, as with a library that failed to open.
        assert!(
            events.contains(&Event::Collections(Arc::default())),
            "{events:?}"
        );

        // Commands for it go nowhere, and the rest of the backend still works.
        handle.send(Command::Rescan);
        let dark = with_theme(Theme::Dark);
        handle.send(Command::UpdateSettings(dark.clone()));
        drain_until_within(&handle, LIBRARY_MAX, |e| e.contains(&Event::SettingsSaved));
        let start = Instant::now();
        assert!(handle.shutdown(LIBRARY_MAX));
        assert!(start.elapsed() < LIBRARY_MAX);
        assert_eq!(*saved.lock().unwrap(), [dark]);
    }

    #[test]
    fn a_stuck_library_cannot_hold_up_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let (release, gate) = mpsc::channel::<()>();
        let alive = Arc::new(());
        let held = Arc::clone(&alive);
        // Stuck inside its first event, as a thread caught in a slow read would be.
        let link = link_with(dir.path(), move |_| {
            let _ = gate.recv();
            let _ = &held;
        });
        let (sink, saved) = recording_sink(0);
        let (waker, _) = counting_waker();
        let handle =
            Backend::spawn(Some(sink), Some(link), silent_playback(), waker, FAST).unwrap();
        drain_until_within(&handle, LIBRARY_MAX, |e| {
            e.iter().any(|e| matches!(e, Event::LibraryUpdated(_)))
        });
        let dark = with_theme(Theme::Dark);
        handle.send(Command::UpdateSettings(dark.clone()));

        // A backend that waited for the thread without a limit would time out here.
        assert!(handle.shutdown(Duration::from_secs(10)));
        assert_eq!(*saved.lock().unwrap(), [dark]);
        assert_eq!(Arc::strong_count(&alive), 2, "the thread was left behind");

        // Unblock the detached thread so it finishes on its own.
        drop(release);
        assert!(wait_until(|| Arc::strong_count(&alive) == 1, LIBRARY_MAX));
    }

    #[test]
    fn debounce_coalesces_bursts() {
        let (sink, saved) = recording_sink(0);
        let (waker, wakes) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, FAST).unwrap();
        for width in 200u8..=209 {
            handle.send(Command::UpdateSettings(Settings {
                sidebar_width: f32::from(width),
                ..Settings::default()
            }));
        }
        assert!(wait_until(|| !saved.lock().unwrap().is_empty(), MAX));
        // Leave room for a wrong second save before asserting there is none.
        thread::sleep(FAST.debounce * 4);
        {
            let saved = saved.lock().unwrap();
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0].sidebar_width, 209.0);
        }
        // The waker runs after a batch is sent, so the event is queued by now.
        assert!(wait_until(|| wakes.load(Ordering::SeqCst) >= 1, MAX));
        assert!(handle.drain().contains(&Event::SettingsSaved));
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn shutdown_flushes_pending() {
        let (sink, saved) = recording_sink(0);
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, FAST).unwrap();
        let dark = with_theme(Theme::Dark);
        handle.send(Command::UpdateSettings(dark.clone()));
        assert!(handle.shutdown(MAX));
        assert_eq!(*saved.lock().unwrap(), [dark]);
    }

    #[test]
    fn reset_saves_defaults_and_replaces() {
        let (sink, saved) = recording_sink(0);
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, FAST).unwrap();
        handle.send(Command::ResetSettings);
        let replaced = Event::SettingsReplaced(Settings::default());
        drain_until(&handle, |e| {
            e.contains(&replaced) && e.contains(&Event::SettingsSaved)
        });
        assert_eq!(saved.lock().unwrap().last(), Some(&Settings::default()));
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn reset_keeps_the_volume_and_mute() {
        let (sink, saved) = recording_sink(0);
        let (waker, _) = counting_waker();
        let services = BackendServices {
            spotify: None,
            library: None,
            output: Box::new(FakeFactory::new(OutputFormat {
                sample_rate: 48_000,
                channels: 2,
            })),
            capture: Box::new(no_microphone()),
            settings: Settings {
                volume: 0.2,
                ..Settings::default()
            },
        };
        let handle = Backend::start(Some(sink), services, waker, FAST).unwrap();
        handle.send(Command::UpdateSettings(Settings {
            volume: 0.4,
            muted: true,
            theme: Theme::Dark,
            ..Settings::default()
        }));
        handle.send(Command::ResetSettings);
        let kept = Settings {
            volume: 0.4,
            muted: true,
            ..Settings::default()
        };
        let replaced = Event::SettingsReplaced(kept.clone());
        drain_until(&handle, |e| e.contains(&replaced));
        assert!(handle.shutdown(MAX));
        assert_eq!(saved.lock().unwrap().last(), Some(&kept));
    }

    #[test]
    fn reset_drops_pending_update() {
        let (sink, saved) = recording_sink(0);
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, SLOW).unwrap();
        handle.send(Command::UpdateSettings(with_theme(Theme::Dark)));
        handle.send(Command::ResetSettings);
        // Nothing is pending after the reset, so shutdown writes nothing more.
        assert!(handle.shutdown(MAX));
        assert_eq!(*saved.lock().unwrap(), [Settings::default()]);
    }

    #[test]
    fn save_failure_emits_error_notice() {
        let (sink, calls) = recording_sink(1);
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, FAST).unwrap();
        let dark = with_theme(Theme::Dark);
        handle.send(Command::UpdateSettings(dark.clone()));
        let events = drain_until(&handle, |e| e.contains(&save_failed()));
        assert!(!events.contains(&Event::SettingsSaved));
        // The failed value stays pending until the next update replaces it.
        let light = with_theme(Theme::Light);
        handle.send(Command::UpdateSettings(light.clone()));
        drain_until(&handle, |e| e.contains(&Event::SettingsSaved));
        assert!(handle.shutdown(MAX));
        assert_eq!(*calls.lock().unwrap(), [dark, light]);
    }

    #[test]
    fn shutdown_retries_a_failed_save() {
        let (sink, calls) = recording_sink(1);
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, FAST).unwrap();
        let dark = with_theme(Theme::Dark);
        handle.send(Command::UpdateSettings(dark.clone()));
        drain_until(&handle, |e| e.contains(&save_failed()));
        // No deadline is left: nothing is retried before shutdown.
        thread::sleep(FAST.debounce * 4);
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(handle.shutdown(MAX));
        assert_eq!(*calls.lock().unwrap(), [dark.clone(), dark]);
    }

    #[test]
    fn failed_reset_is_retried_at_shutdown() {
        let (sink, calls) = recording_sink(1);
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, SLOW).unwrap();
        handle.send(Command::ResetSettings);
        drain_until(&handle, |e| e.contains(&save_failed()));
        assert!(handle.shutdown(MAX));
        assert_eq!(
            *calls.lock().unwrap(),
            [Settings::default(), Settings::default()]
        );
    }

    #[test]
    fn repeated_failures_are_reported_once() {
        let (sink, calls) = recording_sink(3);
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, FAST).unwrap();
        let mut events = Vec::new();
        for (n, theme) in [Theme::Dark, Theme::Light, Theme::System]
            .into_iter()
            .enumerate()
        {
            handle.send(Command::UpdateSettings(with_theme(theme)));
            assert!(wait_until(|| calls.lock().unwrap().len() > n, MAX));
        }
        // Leave room for the last outcome to arrive.
        thread::sleep(FAST.debounce * 2);
        events.extend(handle.drain());
        let failures = events.iter().filter(|e| **e == save_failed()).count();
        assert_eq!(failures, 1, "events: {events:?}");

        // A success ends the streak.
        handle.send(Command::UpdateSettings(with_theme(Theme::Dark)));
        drain_until(&handle, |e| e.contains(&Event::SettingsSaved));
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn read_only_mode_accepts_updates() {
        let (waker, wakes) = counting_waker();
        let handle = Backend::start(None, no_library(), waker, FAST).unwrap();
        handle.send(Command::UpdateSettings(with_theme(Theme::Dark)));
        // Leave room for a wrong debounced save to report itself.
        thread::sleep(FAST.debounce * 4);
        assert!(handle.drain().is_empty());
        assert!(handle.shutdown(MAX));
        // Every batch of events is followed by a wake: no wake, no event.
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn read_only_reset_still_replaces() {
        let (waker, wakes) = counting_waker();
        let handle = Backend::start(None, no_library(), waker, FAST).unwrap();
        handle.send(Command::ResetSettings);
        assert!(wait_until(|| wakes.load(Ordering::SeqCst) >= 1, MAX));
        // The whole batch: no save event, since nothing was written.
        assert_eq!(
            handle.drain(),
            [Event::SettingsReplaced(Settings::default())]
        );
        assert!(handle.shutdown(MAX));
    }

    #[test]
    fn dropping_handle_flushes_and_stops() {
        let (sink, saved) = recording_sink(0);
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, FAST).unwrap();
        let light = with_theme(Theme::Light);
        handle.send(Command::UpdateSettings(light.clone()));
        drop(handle);
        assert!(
            wait_until(|| Arc::strong_count(&saved) == 1, MAX),
            "the backend did not stop"
        );
        assert_eq!(*saved.lock().unwrap(), [light]);
    }

    #[test]
    fn send_after_exit_is_ignored() {
        let (sink, saved) = recording_sink(0);
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(sink), no_library(), waker, FAST).unwrap();
        handle.send(Command::Shutdown);
        assert!(wait_until(|| Arc::strong_count(&saved) == 1, MAX));
        handle.send(Command::UpdateSettings(with_theme(Theme::Dark)));
        assert!(handle.shutdown(MAX));
        assert!(saved.lock().unwrap().is_empty());
    }

    #[test]
    fn shutdown_times_out_while_a_save_blocks() {
        let (release, gate) = mpsc::channel::<()>();
        let alive = Arc::new(());
        let sink = GatedSink {
            gate,
            _alive: Arc::clone(&alive),
        };
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(Box::new(sink)), no_library(), waker, FAST).unwrap();
        handle.send(Command::UpdateSettings(with_theme(Theme::Dark)));
        assert!(!handle.shutdown(Duration::from_millis(100)));
        // Unblock the detached backend so it finishes on its own.
        drop(release);
        assert!(wait_until(|| Arc::strong_count(&alive) == 1, MAX));
    }

    #[test]
    fn shutdown_returns_promptly_after_a_backend_panic() {
        let (sink, _) = recording_sink(0);
        let waker: Waker = Arc::new(|| panic!("test waker panics"));
        let handle = Backend::start(Some(sink), no_library(), waker, FAST).unwrap();
        // The reset wakes the UI, and the panicking waker takes the thread down.
        handle.send(Command::ResetSettings);
        let start = Instant::now();
        assert!(!handle.shutdown(Duration::from_secs(10)));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    /// A thread running `body` with a done guard, as the services run theirs.
    fn guarded_thread(
        body: impl FnOnce() + Send + 'static,
    ) -> (Receiver<()>, thread::JoinHandle<()>) {
        let (done_tx, done) = crossbeam_channel::bounded(1);
        let guard = DoneGuard::new(done_tx);
        let thread = thread::spawn(move || {
            let _done = guard;
            body();
        });
        (done, thread)
    }

    #[test]
    fn bounded_join_reports_a_clean_exit() {
        let (done, thread) = guarded_thread(|| {});
        assert!(join_bounded(&done, thread, LIBRARY_MAX, "test"));
    }

    #[test]
    fn bounded_join_reports_a_panic() {
        let (done, thread) = guarded_thread(|| panic!("test thread panics"));
        assert!(!join_bounded(&done, thread, LIBRARY_MAX, "test"));
    }

    #[test]
    fn bounded_join_gives_up_on_a_stuck_thread() {
        let (release, gate) = mpsc::channel::<()>();
        let alive = Arc::new(());
        let held = Arc::clone(&alive);
        let (done, thread) = guarded_thread(move || {
            let _ = gate.recv();
            drop(held);
        });
        assert!(!join_bounded(
            &done,
            thread,
            Duration::from_millis(50),
            "test"
        ));
        assert_eq!(Arc::strong_count(&alive), 2, "the thread was left running");
        // Unblock the detached thread so it finishes on its own.
        drop(release);
        assert!(wait_until(|| Arc::strong_count(&alive) == 1, LIBRARY_MAX));
    }

    #[test]
    fn settings_store_is_a_sink() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::new(dir.path().join("settings.toml"));
        let light = with_theme(Theme::Light);
        SettingsSink::save(&mut store, &light).unwrap();
        assert_eq!(store.load().settings, light);
    }

    // Spotify: which player the bar shows, and where its commands go.

    mod spotify {
        use super::*;
        use crate::spotify_service::{SpotifyAccount, SpotifyEvent};
        use aria_spotify::MemoryStore;
        use aria_spotify::PlayRequest;
        use aria_spotify::fake::{FakeSpotify, Reply};

        /// A fake Spotify with a signed-in account whose player answers `player`.
        fn fake(player: Reply) -> FakeSpotify {
            let fake = FakeSpotify::start();
            fake.reply("POST", "/api/token", Reply::fixture(200, "token"));
            fake.reply("GET", "/v1/me", Reply::fixture(200, "me"));
            fake.reply(
                "GET",
                "/v1/me/tracks",
                Reply::fixture(200, "saved-tracks-3"),
            );
            fake.reply("GET", "/v1/me/albums", Reply::fixture(200, "saved-albums"));
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
            fake.reply("GET", "/v1/me/player", player);
            for path in ["play", "pause", "seek"] {
                fake.reply("PUT", &format!("/v1/me/player/{path}"), Reply::empty(204));
            }
            fake.reply("POST", "/v1/me/player/next", Reply::empty(204));
            fake
        }

        fn backend(fake: &FakeSpotify, settings: Settings) -> BackendHandle {
            backend_until(fake, settings, |_| true)
        }

        /// A backend signed in to `fake`, once the events since its start also satisfy `also`.
        /// Waiting for both in one drain keeps an event that came with the sign-in, such as the
        /// first poll's, which a later drain would never see.
        fn backend_until(
            fake: &FakeSpotify,
            settings: Settings,
            also: impl Fn(&[Event]) -> bool,
        ) -> BackendHandle {
            let mut services = services(None);
            services.settings = settings;
            services.spotify = Some(SpotifyDeps {
                endpoints: fake.endpoints(),
                store: Box::new(MemoryStore::holding("stored")),
                open_browser: Box::new(|_| Ok(())),
                redirect_port: 0,
                sign_in_timeout: Duration::from_secs(5),
                search_quiet: Duration::from_millis(50),
                fast: Duration::from_millis(100),
                slow: Duration::from_secs(1),
                after_command: Duration::from_millis(20),
                player: crate::spotify_player::PlayerDeps::unavailable(),
            });
            let (waker, _) = counting_waker();
            let handle = Backend::start(None, services, waker, SLOW).unwrap();
            drain_until_within(&handle, LIBRARY_MAX, |e| {
                also(e)
                    && e.iter().any(|e| {
                        matches!(
                            e,
                            Event::Spotify(SpotifyEvent::Account(SpotifyAccount::SignedIn { .. }))
                        )
                    })
            });
            handle
        }

        fn count(fake: &FakeSpotify, method: &str, path: &str) -> usize {
            fake.requests()
                .iter()
                .filter(|s| s.method == method && s.path == path)
                .count()
        }

        fn album() -> Command {
            Command::Spotify(SpotifyRequest::Play(PlayRequest::Context {
                uri: "spotify:album:albumA".to_owned(),
                offset_uri: None,
            }))
        }

        fn tone() -> PathBuf {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../aria-audio/tests/fixtures/tone.ogg")
        }

        fn shows(player: ActivePlayer) -> impl Fn(&[Event]) -> bool {
            move |e| e.contains(&Event::ActivePlayer(player))
        }

        #[test]
        fn spotify_play_pauses_local_and_shows_spotify() {
            let fake = fake(Reply::empty(204));
            let handle = backend(&fake, Settings::default());
            handle.send(Command::OpenPaths(vec![tone()]));
            drain_until_within(&handle, LIBRARY_MAX, |e| {
                last_view(e).is_some_and(|view| view.playing)
            });
            handle.send(album());
            let events = drain_until(&handle, |e| {
                shows(ActivePlayer::Spotify)(e) && last_view(e).is_some_and(|view| !view.playing)
            });
            assert!(!events.contains(&Event::ActivePlayer(ActivePlayer::Local)));
            assert!(wait_until(
                || count(&fake, "PUT", "/v1/me/player/play") == 1,
                MAX
            ));
            assert!(handle.shutdown(LIBRARY_MAX));
        }

        #[test]
        fn playback_started_elsewhere_takes_the_bar() {
            let fake = fake(Reply::fixture(200, "player"));
            let handle = backend_until(&fake, Settings::default(), shows(ActivePlayer::Spotify));
            assert!(handle.shutdown(LIBRARY_MAX));
        }

        #[test]
        fn local_play_pauses_spotify_and_shows_local() {
            let fake = fake(Reply::fixture(200, "player"));
            let handle = backend_until(&fake, Settings::default(), shows(ActivePlayer::Spotify));
            handle.send(Command::OpenPaths(vec![tone()]));
            drain_until_within(&handle, LIBRARY_MAX, shows(ActivePlayer::Local));
            assert!(wait_until(
                || count(&fake, "PUT", "/v1/me/player/pause") == 1,
                MAX
            ));
            assert_eq!(
                count(&fake, "PUT", "/v1/me/player/play"),
                0,
                "never a toggle"
            );
            assert!(handle.shutdown(LIBRARY_MAX));
        }

        #[test]
        fn transport_goes_to_the_player_shown() {
            let fake = fake(Reply::empty(204));
            let handle = backend(&fake, Settings::default());
            handle.send(Command::Next);
            thread::sleep(Duration::from_millis(200));
            assert_eq!(
                count(&fake, "POST", "/v1/me/player/next"),
                0,
                "local by default"
            );

            handle.send(album());
            drain_until(&handle, shows(ActivePlayer::Spotify));
            handle.send(Command::Next);
            handle.send(Command::Seek(5000));
            handle.send(Command::TogglePlay);
            assert!(wait_until(
                || count(&fake, "POST", "/v1/me/player/next") == 1,
                MAX
            ));
            assert!(wait_until(
                || count(&fake, "PUT", "/v1/me/player/seek") == 1,
                MAX
            ));
            let seek = fake
                .requests()
                .into_iter()
                .find(|s| s.path == "/v1/me/player/seek")
                .unwrap();
            assert_eq!(seek.param("position_ms"), Some("5000"));
            // Nothing is playing on Spotify, so the toggle resumes.
            assert!(wait_until(
                || count(&fake, "PUT", "/v1/me/player/play") == 2,
                MAX
            ));
            assert!(handle.shutdown(LIBRARY_MAX));
        }

        #[test]
        fn resuming_local_pauses_spotify() {
            let fake = fake(Reply::fixture(200, "player"));
            let handle = backend_until(&fake, Settings::default(), shows(ActivePlayer::Spotify));
            handle.send(Command::OpenPaths(vec![tone()]));
            drain_until_within(&handle, LIBRARY_MAX, |e| {
                shows(ActivePlayer::Local)(e) && last_view(e).is_some_and(|view| view.playing)
            });
            assert!(wait_until(
                || count(&fake, "PUT", "/v1/me/player/pause") == 1,
                MAX
            ));
            // Pause and resume the local track at once, before a poll can move the bar.
            handle.send(Command::TogglePlay);
            handle.send(Command::TogglePlay);
            assert!(wait_until(
                || count(&fake, "PUT", "/v1/me/player/pause") == 2,
                MAX
            ));
            assert!(handle.shutdown(LIBRARY_MAX));
        }

        #[test]
        fn a_hidden_window_stops_polling() {
            let fake = fake(Reply::fixture(200, "player"));
            let handle = backend_until(&fake, Settings::default(), shows(ActivePlayer::Spotify));
            handle.send(Command::WindowShown(false));
            thread::sleep(Duration::from_millis(150));
            let before = count(&fake, "GET", "/v1/me/player");
            thread::sleep(Duration::from_millis(500));
            assert_eq!(count(&fake, "GET", "/v1/me/player"), before);
            handle.send(Command::WindowShown(true));
            assert!(wait_until(
                || count(&fake, "GET", "/v1/me/player") > before,
                MAX
            ));
            assert!(handle.shutdown(LIBRARY_MAX));
        }

        #[test]
        fn choosing_a_device_is_saved_in_the_settings() {
            let fake = fake(Reply::empty(204));
            let handle = backend(&fake, Settings::default());
            handle.send(Command::Spotify(SpotifyRequest::Device(Some(
                "device-desk".to_owned(),
            ))));
            drain_until(&handle, |e| {
                e.iter().any(|e| {
                    matches!(e, Event::SettingsReplaced(s)
                        if s.spotify_device.as_deref() == Some("device-desk"))
                })
            });
            assert!(handle.shutdown(LIBRARY_MAX));
        }

        #[test]
        fn signing_out_clears_the_device() {
            let fake = fake(Reply::empty(204));
            let settings = Settings {
                spotify_device: Some("device-desk".to_owned()),
                ..Settings::default()
            };
            let handle = backend(&fake, settings);
            handle.send(Command::Spotify(SpotifyRequest::SignOut));
            let events = drain_until(&handle, |e| {
                e.iter()
                    .any(|e| matches!(e, Event::SettingsReplaced(s) if s.spotify_device.is_none()))
                    && e.iter().any(|e| {
                        matches!(
                            e,
                            Event::Spotify(SpotifyEvent::Account(SpotifyAccount::SignedOut { .. }))
                        )
                    })
            });
            assert!(!events.contains(&Event::ActivePlayer(ActivePlayer::Spotify)));
            assert!(handle.shutdown(LIBRARY_MAX));
        }

        #[test]
        fn without_spotify_deps_nothing_changes() {
            let (waker, _) = counting_waker();
            let handle = Backend::start(None, no_library(), waker, SLOW).unwrap();
            handle.send(album());
            handle.send(Command::Spotify(SpotifyRequest::SignOut));
            handle.send(Command::WindowShown(false));
            handle.send(Command::TogglePlay);
            handle.send(Command::ListDevices);
            let events = drain_until(&handle, |e| {
                e.iter().any(|e| matches!(e, Event::OutputDevices(_)))
            });
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, Event::Spotify(_) | Event::ActivePlayer(_))),
                "{events:?}"
            );
            assert!(handle.shutdown(LIBRARY_MAX));
        }
    }
}
