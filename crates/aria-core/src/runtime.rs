//! The backend thread. It owns a Tokio runtime and does the work the UI
//! thread must never block on. In Phase 1 that work is debounced, durable
//! settings persistence; audio and network services join it in later phases.
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

use std::{io, mem, sync::Arc, thread, time::Duration};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    time::Instant,
};

use crate::{
    command::Command,
    error::SettingsError,
    event::Event,
    notice::{Notice, NoticeLevel},
    settings::{Settings, store::SettingsStore},
};

/// Name of the backend thread.
const THREAD_NAME: &str = "aria-backend";
/// How long the runtime may take to stop its threads once the loop has ended.
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);
/// Reported to the user when writing the settings fails.
const SAVE_FAILED: Notice = Notice {
    level: NoticeLevel::Error,
    key: "notice-settings-save-failed",
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
    /// # Errors
    ///
    /// Fails if the runtime cannot be built or the thread cannot be spawned.
    pub fn start(
        sink: Option<Box<dyn SettingsSink>>,
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
        let worker = Worker {
            sink,
            events: event_tx,
            waker,
            debounce: cfg.debounce,
            pending: None,
            deadline: None,
            failing: false,
        };
        let done_guard = DoneGuard(done_tx);
        let thread = thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                // Bound first, so dropped last: `shutdown` hears "done" only
                // after the runtime has stopped, even while a panic unwinds.
                let _done = done_guard;
                let runtime = runtime;
                runtime.block_on(worker.run(command_rx));
                runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
            })?;
        Ok(BackendHandle {
            commands,
            events,
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
#[derive(Debug)]
pub struct BackendHandle {
    commands: UnboundedSender<Command>,
    events: Receiver<Event>,
    done: Receiver<()>,
    thread: thread::JoinHandle<()>,
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

    /// Sends [`Command::Shutdown`], then waits up to `timeout` for the
    /// backend to write any pending settings and exit.
    ///
    /// Returns `true` if the backend exited cleanly in time. Returns `false`
    /// if it timed out, in which case its thread is detached and left to
    /// finish on its own, or if the thread panicked.
    pub fn shutdown(self, timeout: Duration) -> bool {
        self.send(Command::Shutdown);
        match self.done.recv_timeout(timeout) {
            // A disconnect also means the done guard was dropped.
            Ok(()) | Err(RecvTimeoutError::Disconnected) => match self.thread.join() {
                Ok(()) => true,
                Err(_) => {
                    tracing::error!("the backend thread panicked");
                    false
                }
            },
            Err(RecvTimeoutError::Timeout) => {
                tracing::warn!(?timeout, "the backend did not stop in time; detaching it");
                false
            }
        }
    }
}

/// Owned by the backend thread. Its drop signals [`BackendHandle::shutdown`],
/// so a panicking thread unblocks it too.
struct DoneGuard(Sender<()>);

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
    /// state enum once services that save concurrently arrive.
    sink: Option<Box<dyn SettingsSink>>,
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
    async fn run(mut self, mut commands: UnboundedReceiver<Command>) {
        loop {
            tokio::select! {
                cmd = commands.recv() => match cmd {
                    Some(Command::UpdateSettings(settings)) => self.update(settings),
                    Some(Command::ResetSettings) => self.reset().await,
                    // `None` means the handle was dropped: treat it as Shutdown.
                    Some(Command::Shutdown) | None => break,
                },
                () = deadline_reached(self.deadline) => self.flush().await,
            }
        }
        // Shutdown, or the handle was dropped. This writes the newest settings,
        // or retries the ones whose save failed, once. Nobody reads events any
        // more, so the outcome is only logged (by `save`).
        if let Some(settings) = self.pending.take() {
            self.save(settings).await;
        }
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
        io,
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::{
        command::Command,
        error::SettingsError,
        event::Event,
        notice::{Notice, NoticeLevel},
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
        let seen = RefCell::new(Vec::new());
        let met = wait_until(
            || {
                let mut seen = seen.borrow_mut();
                seen.extend(handle.drain());
                pred(&seen)
            },
            MAX,
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
        })
    }

    #[test]
    fn debounce_coalesces_bursts() {
        let (sink, saved) = recording_sink(0);
        let (waker, wakes) = counting_waker();
        let handle = Backend::start(Some(sink), waker, FAST).unwrap();
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
        let handle = Backend::start(Some(sink), waker, FAST).unwrap();
        let dark = with_theme(Theme::Dark);
        handle.send(Command::UpdateSettings(dark.clone()));
        assert!(handle.shutdown(MAX));
        assert_eq!(*saved.lock().unwrap(), [dark]);
    }

    #[test]
    fn reset_saves_defaults_and_replaces() {
        let (sink, saved) = recording_sink(0);
        let (waker, _) = counting_waker();
        let handle = Backend::start(Some(sink), waker, FAST).unwrap();
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
        let handle = Backend::start(Some(sink), waker, SLOW).unwrap();
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
        let handle = Backend::start(Some(sink), waker, FAST).unwrap();
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
        let handle = Backend::start(Some(sink), waker, FAST).unwrap();
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
        let handle = Backend::start(Some(sink), waker, SLOW).unwrap();
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
        let handle = Backend::start(Some(sink), waker, FAST).unwrap();
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
        let handle = Backend::start(None, waker, FAST).unwrap();
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
        let handle = Backend::start(None, waker, FAST).unwrap();
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
        let handle = Backend::start(Some(sink), waker, FAST).unwrap();
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
        let handle = Backend::start(Some(sink), waker, FAST).unwrap();
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
        let handle = Backend::start(Some(Box::new(sink)), waker, FAST).unwrap();
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
        let handle = Backend::start(Some(sink), waker, FAST).unwrap();
        // The reset wakes the UI, and the panicking waker takes the thread down.
        handle.send(Command::ResetSettings);
        let start = Instant::now();
        assert!(!handle.shutdown(Duration::from_secs(10)));
        assert!(start.elapsed() < Duration::from_secs(5));
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
