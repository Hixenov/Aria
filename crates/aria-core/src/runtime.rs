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
    sync::Arc,
    thread,
    time::Duration,
};

use aria_audio::{DeviceKind, Engine, EngineEvent, OutputFactory, PlaybackClock, list_devices};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    time::{Instant, MissedTickBehavior},
};

use crate::{
    command::Command,
    error::SettingsError,
    event::Event,
    library_service::{self, LibraryEvent, LibraryHandle, LibraryRequest, LibraryService},
    notice::{Notice, NoticeLevel},
    playback_service::PlaybackService,
    settings::{Settings, store::SettingsStore},
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
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(500),
        }
    }
}

/// What the backend runs besides settings persistence.
pub struct BackendServices {
    /// Where the library lives; `None` leaves the library disabled.
    pub library: Option<LibraryPaths>,
    /// Opens audio outputs.
    pub output: Box<dyn OutputFactory>,
    /// The settings in effect at startup, which playback starts from.
    pub settings: Settings,
}

/// A running library service, as the backend holds it.
struct LibraryLink {
    handle: LibraryHandle,
    events: UnboundedReceiver<LibraryEvent>,
}

impl LibraryLink {
    fn start(paths: LibraryPaths) -> io::Result<Self> {
        let (events_tx, events) = unbounded_channel();
        let on_event = Box::new(move |event| {
            // Fails only once the backend has exited, when nobody is listening.
            let _ = events_tx.send(event);
        });
        let handle = LibraryService::start(paths, on_event)?;
        Ok(Self { handle, events })
    }
}

/// The audio engine and the service driving it, as the backend holds them.
struct PlaybackLink {
    service: PlaybackService,
    events: UnboundedReceiver<EngineEvent>,
    clock: Arc<PlaybackClock>,
}

impl PlaybackLink {
    fn start(output: Box<dyn OutputFactory>, settings: &Settings) -> io::Result<Self> {
        let (events_tx, events) = unbounded_channel();
        let on_event = Box::new(move |event| {
            // Fails only once the backend has exited, when nobody is listening.
            let _ = events_tx.send(event);
        });
        let engine = Engine::start(output, on_event)?;
        let clock = engine.clock();
        let mut service = PlaybackService::new(engine, RandomState::new().hash_one(0_u8));
        service.on_settings(settings);
        Ok(Self {
            service,
            events,
            clock,
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
            settings,
        } = services;
        let playback = PlaybackLink::start(output, &settings)?;
        let library = library.map(LibraryLink::start).transpose()?;
        Self::spawn(sink, library, playback, waker, cfg)
    }

    fn spawn(
        sink: Option<Box<dyn SettingsSink>>,
        library: Option<LibraryLink>,
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
        let (library, library_events) = match library {
            Some(link) => (Some(link.handle), Some(link.events)),
            None => (None, None),
        };
        let PlaybackLink {
            service: playback,
            events: engine_events,
            clock,
        } = playback;
        let worker = Worker {
            sink,
            library,
            playback,
            events: event_tx,
            waker,
            debounce: cfg.debounce,
            pending: None,
            deadline: None,
            failing: false,
        };
        let done_guard = DoneGuard::new(done_tx);
        let thread = thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                // Bound first, so dropped last: `shutdown` hears "done" only
                // after the runtime has stopped, even while a panic unwinds.
                let _done = done_guard;
                let runtime = runtime;
                runtime.block_on(worker.run(command_rx, library_events, engine_events));
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
}

impl Worker {
    async fn run(
        mut self,
        mut commands: UnboundedReceiver<Command>,
        mut library_events: Option<UnboundedReceiver<LibraryEvent>>,
        engine_events: UnboundedReceiver<EngineEvent>,
    ) {
        let mut engine_events = Some(engine_events);
        let mut tick = tokio::time::interval(PLAYBACK_TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        self.flush_playback();
        loop {
            tokio::select! {
                cmd = commands.recv() => match cmd {
                    Some(Command::UpdateSettings(settings)) => {
                        self.playback.on_settings(&settings);
                        self.flush_playback();
                        self.update(settings);
                    }
                    Some(Command::ResetSettings) => {
                        self.playback.on_settings(&Settings::default());
                        self.flush_playback();
                        self.reset().await;
                    }
                    Some(Command::AddFolder(path)) => {
                        self.to_library(LibraryRequest::AddFolder(path));
                    }
                    Some(Command::RemoveFolder(id)) => {
                        self.to_library(LibraryRequest::RemoveFolder(id));
                    }
                    Some(Command::Rescan) => self.to_library(LibraryRequest::Rescan),
                    Some(Command::ClearArtwork) => self.to_library(LibraryRequest::ClearArtwork),
                    Some(Command::ListDevices) => self.list_devices(),
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
                        self.playback.on_engine(EngineEvent::NoDevice);
                        self.flush_playback();
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
        if let Some(library) = library {
            let stop = move || library.shutdown(LIBRARY_SHUTDOWN_TIMEOUT);
            let _ = tokio::task::spawn_blocking(stop).await;
        }
        let _ = engine.await;
    }

    /// Passes on what the playback service has collected. An empty batch wakes nobody.
    fn flush_playback(&mut self) {
        for request in self.playback.take_requests() {
            self.to_library(request);
        }
        let events = self.playback.take_events();
        if !events.is_empty() {
            self.emit(events);
        }
    }

    /// Lists the output devices on the blocking pool, since that can be slow, and sends the answer
    /// from there. If they cannot be listed the answer is empty, and the UI still offers the
    /// system default.
    fn list_devices(&self) {
        let (events, waker) = (self.events.clone(), Arc::clone(&self.waker));
        tokio::task::spawn_blocking(move || {
            let devices = list_devices(DeviceKind::Output).unwrap_or_else(|error| {
                tracing::warn!(%error, "the output devices could not be listed");
                Vec::new()
            });
            // Fails only once the handle is gone, when nobody is listening.
            let _ = events.send(Event::OutputDevices(devices));
            waker();
        });
    }

    /// Forwards `request` without waiting. Without a library it goes nowhere.
    fn to_library(&self, request: LibraryRequest) {
        if let Some(library) = &self.library {
            library.send(request);
        }
    }

    fn library_event(&mut self, event: LibraryEvent) {
        match event {
            LibraryEvent::Snapshot(snapshot) => {
                // The UI hears of the snapshot before any view that refers to it.
                self.emit([Event::LibraryUpdated(Arc::clone(&snapshot))]);
                self.playback.on_snapshot(snapshot);
            }
            LibraryEvent::Progress(progress) => self.emit([Event::ScanProgress(progress)]),
            LibraryEvent::Notice(notice) => self.emit([Event::Notice(notice)]),
            LibraryEvent::QueueLoaded(record) => self.playback.on_queue_loaded(record),
        }
        self.flush_playback();
    }

    /// The library thread died. It can tell nobody, so this ends any scan it left showing and
    /// disables the library, as a library that cannot be opened is disabled.
    fn library_lost(&mut self) {
        tracing::error!("the library thread stopped unexpectedly; the library is disabled");
        self.library = None;
        self.emit([
            Event::ScanProgress(None),
            Event::LibraryUpdated(Arc::default()),
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

    async fn reset(&mut self) {
        self.pending = None;
        self.deadline = None;
        let defaults = Settings::default();
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

    use aria_audio::{DeviceId, OutputFormat, fake::FakeFactory};
    use aria_library::{FolderId, TrackId};

    use super::*;
    use crate::{
        command::Command,
        error::SettingsError,
        event::Event,
        notice::{Notice, NoticeLevel},
        playback_service::PlayerView,
        settings::{Settings, Theme, store::SettingsStore},
    };

    const FAST: BackendConfig = BackendConfig {
        debounce: Duration::from_millis(50),
    };
    /// A debounce no test outlives, so only shutdown can write a pending
    /// update.
    const SLOW: BackendConfig = BackendConfig {
        debounce: Duration::from_secs(3600),
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
            library,
            output: Box::new(FakeFactory::new(format)),
            settings: Settings::default(),
        }
    }

    /// Playback on an output that nothing listens to.
    fn silent_playback() -> PlaybackLink {
        let factory = FakeFactory::new(OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        });
        PlaybackLink::start(Box::new(factory), &Settings::default()).unwrap()
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
        LibraryLink { handle, events }
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
            library: Some(library_paths(dir.path())),
            output: Box::new(factory.clone()),
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
        assert!(wait_until(|| factory.outputs().len() == 2, LIBRARY_MAX));
        assert_eq!(factory.outputs()[1].device(), Some(headphones));

        // Only the engine's events can take the view to the end of the list.
        let last = ids[1];
        let at_end = |e: &[Event]| {
            last_view(e).is_some_and(|view| view.current == Some(last) && !view.playing)
        };
        play_until(&handle, &factory, at_end);
        assert!(handle.shutdown(LIBRARY_MAX));

        let factory = FakeFactory::new(format);
        let services = BackendServices {
            library: Some(library_paths(dir.path())),
            output: Box::new(factory.clone()),
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
        let (waker, wakes) = counting_waker();
        let handle = Backend::start(None, no_library(), waker, FAST).unwrap();

        handle.send(Command::ListDevices);

        drain_until_within(&handle, LIBRARY_MAX, |e| {
            e.iter().any(|e| matches!(e, Event::OutputDevices(_)))
        });
        assert!(wakes.load(Ordering::SeqCst) >= 1);
        assert!(handle.shutdown(MAX));
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
}
