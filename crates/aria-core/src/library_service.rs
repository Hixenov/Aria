//! The library thread: the only owner of the [`Library`], and so its single writer.
//!
//! The thread opens the library, sends a first snapshot so the UI shows it at once, then scans.
//! After that it serves [`LibraryRequest`]s and reports through `on_event`.
//!
//! A scan keeps the thread busy, so requests are read in the scanner's `on_batch`, which runs at
//! least every 250 ms. What cannot wait is served there: the queue and its position are saved, the
//! queue is loaded, durations
//! are stored and a second rescan is noted. Folder changes and clearing the artwork wait for the
//! scan to end, because the scanner holds a folder's rows across `on_batch` calls.
//!
//! [`LibraryRequest::Shutdown`] reaches a running scan through a flag the handle shares with the
//! thread. The requests sent before it are still served; the changes among them are applied, but
//! no scan starts and no snapshot is built, since nobody is left to read them.
//!
//! [`LibraryError`]s are shown by kind only: a notice has a fixed text, and logs use the error's
//! `Display`, which carries no path. Its source chain does, so it is never formatted.

use std::{
    collections::VecDeque,
    io, mem,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use aria_library::{
    Folder, FolderId, Library, LibraryError, LibrarySnapshot, Opened, QueueRecord, ScanProgress,
    ScanSummary, TrackId,
};
use crossbeam_channel::{Receiver, Sender, TryRecvError};

use crate::{
    notice::{Notice, NoticeLevel},
    runtime::{DoneGuard, join_bounded},
};

/// Name of the library thread.
const THREAD_NAME: &str = "aria-library";
/// The shortest time between two progress reports: at most 4 a second (spec section 4.2).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// The database was damaged, moved aside and replaced by a new one.
const RECOVERED: Notice = Notice {
    level: NoticeLevel::Warning,
    key: "library-recovered",
    args: Vec::new(),
};
/// The database comes from a newer Aria, so the library is disabled.
const NEWER: Notice = Notice {
    level: NoticeLevel::Error,
    key: "library-newer",
    args: Vec::new(),
};
/// The library cannot be used this session.
pub(crate) const UNAVAILABLE: Notice = Notice {
    level: NoticeLevel::Error,
    key: "library-unavailable",
    args: Vec::new(),
};
/// A folder inside a folder of the library was refused.
const FOLDER_NESTED: Notice = Notice {
    level: NoticeLevel::Warning,
    key: "library-folder-nested",
    args: Vec::new(),
};
/// A folder already in the library was refused.
const FOLDER_DUPLICATE: Notice = Notice {
    level: NoticeLevel::Warning,
    key: "library-folder-duplicate",
    args: Vec::new(),
};
/// A scan or a change the user asked for failed.
const ACTION_FAILED: Notice = Notice {
    level: NoticeLevel::Error,
    key: "library-action-failed",
    args: Vec::new(),
};

/// Where the library keeps its files.
#[derive(Clone)]
pub struct LibraryPaths {
    /// The database file.
    pub db: PathBuf,
    /// The directory of the artwork cache.
    pub artwork_dir: PathBuf,
}

/// A request to the library thread.
#[derive(Debug, Clone, PartialEq)]
pub enum LibraryRequest {
    /// Add a folder, then scan. A folder that cannot be added gives a notice.
    AddFolder(PathBuf),
    /// Remove a folder and its tracks from the library. Files on disk are not touched.
    RemoveFolder(FolderId),
    /// Scan all folders. During a scan, this queues one more.
    Rescan,
    /// Delete the thumbnails. The next scan rebuilds them.
    ClearArtwork,
    /// Replace the saved queue.
    SaveQueue(QueueRecord),
    /// Replace only the current track and position of the saved queue.
    SavePosition {
        current: Option<TrackId>,
        position_ms: u64,
    },
    /// Read the saved queue. Always answered with [`LibraryEvent::QueueLoaded`], even while the
    /// library is disabled, so nobody waits for it in vain.
    LoadQueue,
    /// Store a duration learned during playback, in milliseconds. It shows in the next snapshot.
    SetDuration(TrackId, u64),
    /// Cancel any scan and stop the thread.
    Shutdown,
}

/// What the library thread reports.
#[derive(Debug, Clone, PartialEq)]
pub enum LibraryEvent {
    /// The library as it now is.
    Snapshot(Arc<LibrarySnapshot>),
    /// A scan is running, or `None` when it has ended, however it ended. Scans report at most 4
    /// times a second.
    Progress(Option<ScanProgress>),
    /// A notice to show the user.
    Notice(Notice),
    /// The answer to [`LibraryRequest::LoadQueue`]; `None` when no queue could be read.
    QueueLoaded(Option<QueueRecord>),
}

/// Receives the thread's [`LibraryEvent`]s. It runs on the library thread and must not block.
pub type OnEvent = Box<dyn Fn(LibraryEvent) + Send>;

/// Starts the library thread. See [`LibraryService::start`].
#[derive(Debug)]
pub struct LibraryService;

impl LibraryService {
    /// Starts the thread named `aria-library`, which opens the library at `paths`.
    ///
    /// A library that cannot be opened is reported with a notice and an empty snapshot. The
    /// thread then stays up with the library disabled, ignoring requests.
    ///
    /// # Errors
    ///
    /// Fails if the thread cannot be spawned.
    pub fn start(paths: LibraryPaths, on_event: OnEvent) -> io::Result<LibraryHandle> {
        let (requests, receiver) = crossbeam_channel::unbounded();
        let (done_tx, done) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&cancel);
        let done_guard = DoneGuard::new(done_tx);
        let thread = thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                // Bound first, so dropped last: `shutdown` hears "done" only after the database
                // has been closed, even while a panic unwinds.
                let _done = done_guard;
                run(&paths, receiver, shared, on_event);
            })?;
        Ok(LibraryHandle {
            requests,
            cancel,
            done,
            thread,
        })
    }
}

/// The way to reach the library thread.
///
/// Dropping it without calling [`shutdown`](Self::shutdown) closes the request channel. The thread
/// treats that like [`LibraryRequest::Shutdown`].
#[derive(Debug)]
pub struct LibraryHandle {
    requests: Sender<LibraryRequest>,
    cancel: Arc<AtomicBool>,
    done: Receiver<()>,
    thread: thread::JoinHandle<()>,
}

impl LibraryHandle {
    /// Queues `request` without blocking. If the thread has exited, it is silently dropped.
    ///
    /// [`LibraryRequest::Shutdown`] also cancels a running scan.
    pub fn send(&self, request: LibraryRequest) {
        let shutdown = matches!(request, LibraryRequest::Shutdown);
        // Fails only once the thread has exited, when nothing is listening.
        let _ = self.requests.send(request);
        // After the send, so a scan stopped by the flag finds the request behind it.
        if shutdown {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Sends [`LibraryRequest::Shutdown`], then waits up to `timeout` for the thread to exit.
    ///
    /// Returns `true` if it exited cleanly in time. Returns `false` if it timed out, in which case
    /// the thread is detached and left to finish on its own, or if it panicked.
    pub fn shutdown(self, timeout: Duration) -> bool {
        self.send(LibraryRequest::Shutdown);
        join_bounded(&self.done, self.thread, timeout, THREAD_NAME)
    }
}

/// The body of the thread.
fn run(
    paths: &LibraryPaths,
    requests: Receiver<LibraryRequest>,
    cancel: Arc<AtomicBool>,
    on_event: OnEvent,
) {
    match Library::open(&paths.db, &paths.artwork_dir) {
        Ok(Opened {
            mut library,
            recovered,
        }) => {
            if recovered {
                on_event(LibraryEvent::Notice(RECOVERED));
            }
            Service::new(requests, cancel, on_event).serve(&mut library);
        }
        Err(error) => {
            tracing::error!(%error, "the library cannot be opened and is disabled");
            let notice = match error {
                LibraryError::NewerSchema { .. } => NEWER,
                _ => UNAVAILABLE,
            };
            on_event(LibraryEvent::Notice(notice));
            on_event(LibraryEvent::Snapshot(Arc::default()));
            serve_disabled(&requests, &on_event);
        }
    }
}

/// Ignores requests until shutdown, answering only the one somebody may be waiting for.
fn serve_disabled(requests: &Receiver<LibraryRequest>, on_event: &OnEvent) {
    while let Ok(request) = requests.recv() {
        match request {
            LibraryRequest::LoadQueue => on_event(LibraryEvent::QueueLoaded(None)),
            LibraryRequest::Shutdown => break,
            _ => {}
        }
    }
}

/// The last path component of a folder, for a notice. A root has none and is shown whole, which
/// gives away nothing personal.
fn folder_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// Lets something through at most once per [`PROGRESS_INTERVAL`].
#[derive(Default)]
struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    fn ready(&mut self, now: Instant) -> bool {
        let ready = self
            .last
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if ready {
            self.last = Some(now);
        }
        ready
    }
}

/// A request that must wait for a running scan to end.
enum Change {
    AddFolder(PathBuf),
    RemoveFolder(FolderId),
    ClearArtwork,
}

/// State of the thread while the library is open. The [`Library`] itself stays outside, because
/// the scanner hands it back to `on_batch`.
struct Service {
    requests: Receiver<LibraryRequest>,
    cancel: Arc<AtomicBool>,
    on_event: OnEvent,
    /// Changes received during a scan, oldest first.
    deferred: VecDeque<Change>,
    /// A scan should start once the changes above are applied.
    want_scan: bool,
    /// Shutdown was requested, or the handle is gone.
    stopping: bool,
    progress: Throttle,
}

impl Service {
    fn new(requests: Receiver<LibraryRequest>, cancel: Arc<AtomicBool>, on_event: OnEvent) -> Self {
        Self {
            requests,
            cancel,
            on_event,
            deferred: VecDeque::new(),
            // The startup scan.
            want_scan: true,
            stopping: false,
            progress: Throttle::default(),
        }
    }

    fn serve(mut self, library: &mut Library) {
        self.send_snapshot(library);
        loop {
            while let Some(change) = self.deferred.pop_front() {
                self.apply(library, change);
            }
            if self.stopping {
                break;
            }
            if mem::take(&mut self.want_scan) {
                self.scan(library);
                continue;
            }
            match self.requests.recv() {
                Ok(request) => {
                    if let Some(change) = self.serve_now(library, request) {
                        self.apply(library, change);
                    }
                }
                Err(_) => self.stop(),
            }
        }
    }

    fn emit(&self, event: LibraryEvent) {
        (self.on_event)(event);
    }

    fn stop(&mut self) {
        self.stopping = true;
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Serves what never waits for a scan, and returns what must.
    fn serve_now(&mut self, library: &mut Library, request: LibraryRequest) -> Option<Change> {
        match request {
            LibraryRequest::SaveQueue(record) => {
                if let Err(error) = library.save_queue(&record) {
                    tracing::warn!(%error, "the queue could not be saved");
                }
            }
            LibraryRequest::SavePosition {
                current,
                position_ms,
            } => {
                if let Err(error) = library.save_position(current, position_ms) {
                    tracing::warn!(%error, "the queue position could not be saved");
                }
            }
            LibraryRequest::LoadQueue => {
                let record = library.load_queue().unwrap_or_else(|error| {
                    tracing::warn!(%error, "the saved queue could not be read");
                    None
                });
                self.emit(LibraryEvent::QueueLoaded(record));
            }
            LibraryRequest::SetDuration(id, ms) => {
                if let Err(error) = library.set_duration(id, ms) {
                    tracing::warn!(%error, "a track's duration could not be stored");
                }
            }
            LibraryRequest::Rescan => self.want_scan = true,
            LibraryRequest::Shutdown => self.stop(),
            LibraryRequest::AddFolder(path) => return Some(Change::AddFolder(path)),
            LibraryRequest::RemoveFolder(id) => return Some(Change::RemoveFolder(id)),
            LibraryRequest::ClearArtwork => return Some(Change::ClearArtwork),
        }
        None
    }

    fn apply(&mut self, library: &mut Library, change: Change) {
        match change {
            Change::AddFolder(path) => match library.add_folder(&path) {
                Ok(_) => {
                    tracing::debug!(path = %path.display(), "folder added");
                    self.want_scan = true;
                    self.send_snapshot(library);
                }
                Err(LibraryError::FolderInsideExisting) => {
                    self.emit(LibraryEvent::Notice(FOLDER_NESTED));
                }
                Err(LibraryError::FolderAlreadyAdded) => {
                    self.emit(LibraryEvent::Notice(FOLDER_DUPLICATE));
                }
                Err(error) => self.report_failure(&error),
            },
            Change::RemoveFolder(id) => match library.remove_folder(id) {
                Ok(()) => self.send_snapshot(library),
                Err(error) => self.report_failure(&error),
            },
            Change::ClearArtwork => {
                // A failure can leave some of the work done, so the snapshot goes out either way.
                if let Err(error) = library.clear_artwork() {
                    self.report_failure(&error);
                }
                self.send_snapshot(library);
            }
        }
    }

    /// Reads the requests that have arrived, without waiting for more.
    fn drain(&mut self, library: &mut Library) {
        while !self.stopping {
            match self.requests.try_recv() {
                Ok(request) => {
                    if let Some(change) = self.serve_now(library, request) {
                        self.deferred.push_back(change);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => self.stop(),
            }
        }
    }

    /// What a running scan does between files.
    fn on_batch(&mut self, library: &mut Library, progress: ScanProgress) {
        self.drain(library);
        self.report_progress(progress, Instant::now());
    }

    /// Sends `progress`, unless the last report was too recent at `now`.
    fn report_progress(&mut self, progress: ScanProgress, now: Instant) {
        if self.progress.ready(now) {
            self.emit(LibraryEvent::Progress(Some(progress)));
        }
    }

    fn scan(&mut self, library: &mut Library) {
        let before = library.folders().unwrap_or_else(|error| {
            tracing::warn!(%error, "the folders could not be read before a scan");
            Vec::new()
        });
        let cancel = Arc::clone(&self.cancel);
        let result = library.scan(&cancel, &mut |library, progress| {
            self.on_batch(library, progress);
        });
        self.emit(LibraryEvent::Progress(None));
        // A shutdown that ended the scan is read here, so no snapshot is built for it.
        self.drain(library);
        match result {
            Ok(summary) => {
                self.report_scan(&summary, &before);
                if changed_anything(library, &summary, &before) {
                    self.send_snapshot(library);
                }
            }
            Err(error) => {
                self.report_failure(&error);
                // Batches written before the failure stay written.
                self.send_snapshot(library);
            }
        }
    }

    /// Turns a scan's results into notices. `before` is what was known when it began.
    fn report_scan(&self, summary: &ScanSummary, before: &[Folder]) {
        if summary.skipped > 0 {
            let notice = Notice::new(NoticeLevel::Warning, "library-scan-skipped")
                .with_arg("count", summary.skipped.to_string());
            self.emit(LibraryEvent::Notice(notice));
        }
        let newly_unavailable = before
            .iter()
            .filter(|folder| folder.available && summary.unavailable.contains(&folder.id));
        for folder in newly_unavailable {
            let notice = Notice::new(NoticeLevel::Warning, "library-folder-unavailable")
                .with_arg("folder", folder_name(&folder.path));
            self.emit(LibraryEvent::Notice(notice));
        }
    }

    /// Builds a snapshot and sends it, unless the thread is stopping.
    fn send_snapshot(&self, library: &Library) {
        if self.stopping {
            return;
        }
        match library.snapshot() {
            Ok(snapshot) => self.emit(LibraryEvent::Snapshot(Arc::new(snapshot))),
            Err(error) => self.report_failure(&error),
        }
    }

    fn report_failure(&self, error: &LibraryError) {
        tracing::warn!(%error, "a library action failed");
        self.emit(LibraryEvent::Notice(ACTION_FAILED));
    }
}

/// Whether a finished scan left the library different: its tracks, or a folder's availability.
fn changed_anything(library: &Library, summary: &ScanSummary, before: &[Folder]) -> bool {
    summary.added + summary.updated + summary.removed > 0
        || library.folders().map_or(true, |after| after != before)
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fs,
        path::{Path, PathBuf},
        sync::{atomic::Ordering, mpsc},
        thread,
        time::{Duration, Instant},
    };

    use aria_library::{Library, SCHEMA_VERSION, TrackId};
    use tempfile::TempDir;

    use super::*;

    /// Upper bound for every wait. A whole scan fits many times over, even on a loaded machine.
    const WAIT: Duration = Duration::from_secs(60);
    /// Files in a tree that is still being scanned when `on_batch` first runs. The scanner calls
    /// it after 500 files at the latest, so files are left over, and writing those calls it once
    /// more, however slow or fast the machine is.
    const TREE: usize = 600;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../aria-audio/tests/fixtures")
            .join(name)
    }

    /// A temporary home for the database, the artwork cache and the music folders.
    struct Env {
        dir: TempDir,
    }

    impl Env {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn paths(&self) -> LibraryPaths {
            LibraryPaths {
                db: self.dir.path().join("library.db"),
                artwork_dir: self.dir.path().join("artwork"),
            }
        }

        /// A folder called `name` holding `count` copies of a small track.
        fn folder(&self, name: &str, count: usize) -> PathBuf {
            let folder = self.dir.path().join(name);
            fs::create_dir_all(&folder).unwrap();
            for n in 0..count {
                fs::copy(fixture("tone.ogg"), folder.join(format!("{n}.ogg"))).unwrap();
            }
            folder
        }

        /// A connection of the test's own, beside the service's.
        fn open(&self) -> Library {
            let paths = self.paths();
            Library::open(&paths.db, &paths.artwork_dir)
                .unwrap_or_else(|e| panic!("cannot open the library: {e}"))
                .library
        }

        /// A library with `folder` in it, closed again, for the service to scan at startup.
        fn seed(&self, folder: &Path) {
            self.open().add_folder(folder).unwrap();
        }
    }

    /// The events received so far, and the way to wait for more.
    struct Log {
        events: mpsc::Receiver<LibraryEvent>,
        seen: Vec<LibraryEvent>,
    }

    impl Log {
        fn new(events: mpsc::Receiver<LibraryEvent>) -> Self {
            Self {
                events,
                seen: Vec::new(),
            }
        }

        /// Takes events until one matches, and returns it.
        fn wait_for(&mut self, matches: impl Fn(&LibraryEvent) -> bool) -> LibraryEvent {
            let start = Instant::now();
            loop {
                let left = WAIT.saturating_sub(start.elapsed());
                let Ok(event) = self.events.recv_timeout(left) else {
                    panic!("no such event; seen: {:?}", self.seen);
                };
                self.seen.push(event.clone());
                if matches(&event) {
                    return event;
                }
            }
        }

        /// Takes what is left, once the service has stopped.
        fn finish(&mut self) {
            self.seen.extend(self.events.try_iter());
        }

        fn scan_ends(&self) -> usize {
            self.seen.iter().filter(|e| is_scan_end(e)).count()
        }

        fn snapshots(&self) -> Vec<&LibrarySnapshot> {
            self.seen
                .iter()
                .filter_map(|e| match e {
                    LibraryEvent::Snapshot(s) => Some(s.as_ref()),
                    _ => None,
                })
                .collect()
        }

        fn notices(&self, key: &str) -> Vec<&Notice> {
            self.seen
                .iter()
                .filter_map(|e| match e {
                    LibraryEvent::Notice(n) if n.key == key => Some(n),
                    _ => None,
                })
                .collect()
        }
    }

    fn is_scan_end(event: &LibraryEvent) -> bool {
        *event == LibraryEvent::Progress(None)
    }

    fn is_progress(event: &LibraryEvent) -> bool {
        matches!(event, LibraryEvent::Progress(Some(_)))
    }

    fn is_queue(event: &LibraryEvent) -> bool {
        matches!(event, LibraryEvent::QueueLoaded(_))
    }

    fn is_snapshot(event: &LibraryEvent) -> bool {
        matches!(event, LibraryEvent::Snapshot(_))
    }

    fn snapshot_with(event: &LibraryEvent, tracks: usize) -> bool {
        matches!(event, LibraryEvent::Snapshot(s) if s.tracks.len() == tracks)
    }

    fn notice_is(event: &LibraryEvent, key: &str) -> bool {
        matches!(event, LibraryEvent::Notice(n) if n.key == key)
    }

    fn recording() -> (OnEvent, Log) {
        let (tx, rx) = mpsc::channel();
        let on_event: OnEvent = Box::new(move |event| {
            let _ = tx.send(event);
        });
        (on_event, Log::new(rx))
    }

    /// Like [`recording`], but the library thread stops inside `on_event` after the first event of
    /// each kind `pause_on` lists, until the test sends on the returned channel. While it is
    /// stopped there, a scan that sent the event is provably still running.
    fn pausing(pause_on: &'static [fn(&LibraryEvent) -> bool]) -> (OnEvent, Log, mpsc::Sender<()>) {
        let (tx, rx) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let paused: Vec<Cell<bool>> = pause_on.iter().map(|_| Cell::new(false)).collect();
        let on_event: OnEvent = Box::new(move |event| {
            let pause = pause_on
                .iter()
                .zip(&paused)
                .any(|(matches, done)| matches(&event) && !done.replace(true));
            let _ = tx.send(event);
            if pause {
                let _ = gate.recv();
            }
        });
        (on_event, Log::new(rx), release)
    }

    fn record() -> QueueRecord {
        QueueRecord {
            context: vec![TrackId(7), TrackId(8)],
            original: Vec::new(),
            user: vec![TrackId(9)],
            current: Some(TrackId(8)),
            position_ms: 4_321,
            shuffle_seed: 99,
            context_index: Some(1),
        }
    }

    #[test]
    fn service_sends_snapshot_then_scans() {
        let env = Env::new();
        let music = env.folder("music", 3);
        let (on_event, mut log) = recording();
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        let first = log.wait_for(|_| true);
        assert!(snapshot_with(&first, 0), "first event: {first:?}");
        handle.send(LibraryRequest::AddFolder(music));
        log.wait_for(|e| snapshot_with(e, 3));
        assert!(handle.shutdown(WAIT));
    }

    #[test]
    fn service_runs_on_its_named_thread() {
        let env = Env::new();
        let (tx, rx) = mpsc::channel();
        let on_event: OnEvent = Box::new(move |_| {
            let _ = tx.send(thread::current().name().map(str::to_owned));
        });
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        assert_eq!(
            rx.recv_timeout(WAIT).unwrap().as_deref(),
            Some("aria-library")
        );
        assert!(handle.shutdown(WAIT));
    }

    #[test]
    fn first_snapshot_comes_before_the_startup_scan() {
        let env = Env::new();
        let music = env.folder("music", 2);
        env.seed(&music);
        let (on_event, mut log) = recording();
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        let first = log.wait_for(|_| true);
        let LibraryEvent::Snapshot(first) = first else {
            panic!("first event: {first:?}");
        };
        assert_eq!(first.folders.len(), 1, "the library shows before any scan");
        assert!(first.tracks.is_empty());
        log.wait_for(|e| snapshot_with(e, 2));
        assert_eq!(log.scan_ends(), 1, "the scan ended before its snapshot");
        assert!(handle.shutdown(WAIT));
    }

    #[test]
    fn save_queue_during_scan_is_not_delayed() {
        let env = Env::new();
        let music = env.folder("music", TREE);
        env.seed(&music);
        let (on_event, mut log, release) = pausing(&[is_progress, is_queue]);
        let handle = LibraryService::start(env.paths(), on_event).unwrap();

        // Stopped inside the scan's first `on_batch`, which wrote the first tracks.
        log.wait_for(is_progress);
        handle.send(LibraryRequest::SaveQueue(record()));
        handle.send(LibraryRequest::SavePosition {
            current: Some(TrackId(8)),
            position_ms: 9_876,
        });
        handle.send(LibraryRequest::SetDuration(TrackId(1), 123_456));
        handle.send(LibraryRequest::LoadQueue);
        release.send(()).unwrap();

        // Stopped again, by the answer. No scan end has been seen, so the scan is running here,
        // and what another connection reads was written during it.
        let moved = QueueRecord {
            position_ms: 9_876,
            ..record()
        };
        let answer = log.wait_for(is_queue);
        assert_eq!(answer, LibraryEvent::QueueLoaded(Some(moved.clone())));
        assert_eq!(log.scan_ends(), 0, "the scan had already ended");
        let other = env.open();
        assert_eq!(other.load_queue().unwrap(), Some(moved.clone()));
        let snapshot = other.snapshot().unwrap();
        assert_eq!(snapshot.track(TrackId(1)).unwrap().duration_ms, 123_456);
        drop(other);
        release.send(()).unwrap();

        log.wait_for(|e| snapshot_with(e, TREE));
        assert_eq!(log.scan_ends(), 1);
        assert!(handle.shutdown(WAIT));
        assert_eq!(env.open().load_queue().unwrap(), Some(moved));
    }

    #[test]
    fn folder_changes_wait_for_the_scan() {
        let env = Env::new();
        let music = env.folder("music", TREE);
        let more = env.folder("more", 2);
        env.seed(&music);
        let (on_event, mut log, release) = pausing(&[is_progress]);
        let handle = LibraryService::start(env.paths(), on_event).unwrap();

        log.wait_for(is_progress);
        handle.send(LibraryRequest::AddFolder(more));
        release.send(()).unwrap();

        // The folder shows as soon as the scan is over, before the scan it starts has finished.
        log.wait_for(|e| matches!(e, LibraryEvent::Snapshot(s) if s.folders.len() == 2));
        assert_eq!(log.scan_ends(), 1, "it was added during the scan");
        log.wait_for(|e| snapshot_with(e, TREE + 2));
        assert_eq!(log.scan_ends(), 2, "an added folder starts a scan");
        assert!(handle.shutdown(WAIT));
    }

    #[test]
    fn rescans_during_a_scan_queue_one_more() {
        let env = Env::new();
        let music = env.folder("music", TREE);
        env.seed(&music);
        let (on_event, mut log, release) = pausing(&[is_progress]);
        let handle = LibraryService::start(env.paths(), on_event).unwrap();

        log.wait_for(is_progress);
        for _ in 0..3 {
            handle.send(LibraryRequest::Rescan);
        }
        release.send(()).unwrap();

        log.wait_for(is_scan_end);
        log.wait_for(is_scan_end);
        // A third scan would start before the service reads the shutdown, and end with its event.
        assert!(handle.shutdown(WAIT));
        log.finish();
        assert_eq!(log.scan_ends(), 2);
    }

    #[test]
    fn progress_goes_out_at_most_four_times_a_second() {
        let (tx, rx) = mpsc::channel();
        let (_requests, receiver) = crossbeam_channel::unbounded();
        let on_event: OnEvent = Box::new(move |event| {
            let _ = tx.send(event);
        });
        let mut service = Service::new(receiver, Arc::default(), on_event);
        let progress = ScanProgress {
            seen: 1,
            changed: 0,
        };
        // The instants are given, so a stalled machine changes nothing here.
        let start = Instant::now();
        for ms in [0, 100, 249, 250, 499, 500] {
            service.report_progress(progress, start + Duration::from_millis(ms));
        }
        let sent: Vec<_> = rx.try_iter().collect();
        assert_eq!(sent, vec![LibraryEvent::Progress(Some(progress)); 3]);
    }

    #[test]
    fn throttle_opens_every_interval() {
        let start = Instant::now();
        let mut throttle = Throttle::default();
        assert!(throttle.ready(start), "the first one goes out");
        assert!(!throttle.ready(start + PROGRESS_INTERVAL / 2));
        assert!(throttle.ready(start + PROGRESS_INTERVAL));
        assert!(!throttle.ready(start + PROGRESS_INTERVAL));
    }

    #[test]
    fn newer_database_disables_the_library() {
        let env = Env::new();
        let music = env.folder("music", 1);
        env.seed(&music);
        // `user_version` is the big-endian number at offset 60 of the file.
        let paths = env.paths();
        let mut bytes = fs::read(&paths.db).unwrap();
        bytes[60..64].copy_from_slice(&(SCHEMA_VERSION + 1).to_be_bytes());
        fs::write(&paths.db, &bytes).unwrap();

        let (on_event, mut log) = recording();
        let handle = LibraryService::start(paths.clone(), on_event).unwrap();
        log.wait_for(|e| notice_is(e, "library-newer"));
        log.wait_for(|e| snapshot_with(e, 0));

        handle.send(LibraryRequest::Rescan);
        handle.send(LibraryRequest::AddFolder(music));
        // Requests are served in order, so this answer comes after the two above.
        handle.send(LibraryRequest::LoadQueue);
        let answer = log.wait_for(is_queue);
        assert_eq!(answer, LibraryEvent::QueueLoaded(None));
        assert!(handle.shutdown(WAIT));
        log.finish();
        assert_eq!(log.seen.len(), 3, "events: {:?}", log.seen);
        assert_eq!(
            fs::read(&paths.db).unwrap(),
            bytes,
            "the file is left alone"
        );
    }

    #[test]
    fn a_failed_open_is_not_corruption() {
        let env = Env::new();
        let paths = env.paths();
        // A directory where the database should be cannot be opened.
        fs::create_dir(&paths.db).unwrap();
        let (on_event, mut log) = recording();
        let handle = LibraryService::start(paths.clone(), on_event).unwrap();
        log.wait_for(|e| notice_is(e, "library-unavailable"));
        log.wait_for(|e| snapshot_with(e, 0));
        assert!(handle.shutdown(WAIT));
        assert!(paths.db.is_dir());
        assert!(!env.dir.path().join("library.db.bak").exists());
    }

    #[test]
    fn recovered_database_is_reported() {
        let env = Env::new();
        let paths = env.paths();
        fs::write(&paths.db, vec![0x5A; 8192]).unwrap();
        let (on_event, mut log) = recording();
        let handle = LibraryService::start(paths, on_event).unwrap();
        log.wait_for(|e| notice_is(e, "library-recovered"));
        log.wait_for(is_snapshot);
        assert!(handle.shutdown(WAIT));
        assert!(env.dir.path().join("library.db.bak").exists());
    }

    #[test]
    fn shutdown_cancels_a_scan() {
        let env = Env::new();
        let music = env.folder("music", TREE);
        env.seed(&music);
        let (on_event, mut log, release) = pausing(&[is_progress]);
        let handle = LibraryService::start(env.paths(), on_event).unwrap();

        log.wait_for(is_progress);
        handle.send(LibraryRequest::Shutdown);
        // The thread is inside the scan and reads no requests, so the flag is what reaches it.
        assert!(handle.cancel.load(Ordering::SeqCst));
        release.send(()).unwrap();

        assert!(handle.shutdown(Duration::from_secs(2)));
        log.finish();
        assert_eq!(log.scan_ends(), 1, "a cancelled scan still ends");
        assert_eq!(log.snapshots().len(), 1, "no snapshot during shutdown");
        assert!(env.open().track_count().unwrap() < TREE as u64);
    }

    #[test]
    fn requests_before_shutdown_are_still_served() {
        let env = Env::new();
        let music = env.folder("music", TREE);
        let more = env.folder("more", 1);
        env.seed(&music);
        let (on_event, mut log, release) = pausing(&[is_progress]);
        let handle = LibraryService::start(env.paths(), on_event).unwrap();

        log.wait_for(is_progress);
        handle.send(LibraryRequest::AddFolder(more));
        handle.send(LibraryRequest::SaveQueue(record()));
        handle.send(LibraryRequest::Shutdown);
        release.send(()).unwrap();

        assert!(handle.shutdown(WAIT));
        let other = env.open();
        assert_eq!(other.load_queue().unwrap(), Some(record()));
        assert_eq!(other.folders().unwrap().len(), 2);
    }

    #[test]
    fn a_dropped_handle_stops_the_thread_during_a_scan() {
        let env = Env::new();
        let music = env.folder("music", TREE);
        env.seed(&music);
        let (on_event, mut log, release) = pausing(&[is_progress]);
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        log.wait_for(is_progress);
        drop(handle);
        release.send(()).unwrap();
        // The thread drops `on_event` as it ends, which closes the channel. A thread still
        // waiting for requests would keep it open until this times out.
        loop {
            match log.events.recv_timeout(WAIT) {
                Ok(event) => log.seen.push(event),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => panic!("the thread did not stop"),
            }
        }
        assert_eq!(log.scan_ends(), 1);
    }

    #[test]
    fn a_cancelled_scan_sends_what_it_changed() {
        let env = Env::new();
        let music = env.folder("music", TREE);
        env.seed(&music);
        let (on_event, mut log, release) = pausing(&[is_progress]);
        let handle = LibraryService::start(env.paths(), on_event).unwrap();

        log.wait_for(is_progress);
        // Cancelled without a shutdown.
        handle.cancel.store(true, Ordering::SeqCst);
        release.send(()).unwrap();

        let last = log.wait_for(|e| matches!(e, LibraryEvent::Snapshot(s) if !s.tracks.is_empty()));
        let LibraryEvent::Snapshot(last) = last else {
            panic!("not a snapshot: {last:?}");
        };
        assert!(last.tracks.len() < TREE);
        assert!(handle.shutdown(WAIT));
    }

    #[test]
    fn a_scan_that_changed_nothing_sends_no_snapshot() {
        let env = Env::new();
        let music = env.folder("music", 2);
        env.seed(&music);
        let (on_event, mut log) = recording();
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        log.wait_for(|e| snapshot_with(e, 2));

        handle.send(LibraryRequest::Rescan);
        log.wait_for(is_scan_end);
        // A later scan only starts once the one before has sent everything it had to send.
        handle.send(LibraryRequest::Rescan);
        log.wait_for(is_scan_end);
        assert_eq!(log.snapshots().len(), 2, "events: {:?}", log.seen);
        assert!(handle.shutdown(WAIT));
    }

    #[test]
    fn unreadable_files_are_counted_in_a_notice() {
        let env = Env::new();
        let music = env.folder("music", 1);
        fs::write(music.join("broken.mp3"), b"not audio").unwrap();
        let (on_event, mut log) = recording();
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        handle.send(LibraryRequest::AddFolder(music));
        let notice = log.wait_for(|e| notice_is(e, "library-scan-skipped"));
        let LibraryEvent::Notice(notice) = notice else {
            panic!("not a notice: {notice:?}");
        };
        assert_eq!(notice.args, [("count", "1".to_owned())]);
        assert!(handle.shutdown(WAIT));
    }

    #[test]
    fn unavailable_notice_fires_when_a_folder_becomes_unavailable() {
        let env = Env::new();
        let music = env.folder("Very Personal Music", 2);
        env.seed(&music);
        let (on_event, mut log) = recording();
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        log.wait_for(|e| snapshot_with(e, 2));
        assert!(log.notices("library-folder-unavailable").is_empty());

        fs::remove_dir_all(&music).unwrap();
        handle.send(LibraryRequest::Rescan);
        let notice = log.wait_for(|e| notice_is(e, "library-folder-unavailable"));
        let LibraryEvent::Notice(notice) = notice else {
            panic!("not a notice: {notice:?}");
        };
        assert_eq!(
            notice.args,
            [("folder", "Very Personal Music".to_owned())],
            "the last path component only"
        );
        // The folder's flag changed, and the tracks are kept.
        let last = log.wait_for(is_snapshot);
        assert!(matches!(
            last,
            LibraryEvent::Snapshot(s) if !s.folders[0].available && s.tracks.len() == 2
        ));

        // Already known: later scans stay quiet. Each scan finishes before the next one starts.
        for _ in 0..2 {
            handle.send(LibraryRequest::Rescan);
            log.wait_for(is_scan_end);
        }
        assert_eq!(log.notices("library-folder-unavailable").len(), 1);
        assert!(handle.shutdown(WAIT));
    }

    #[test]
    fn a_folder_known_to_be_unavailable_is_quiet_after_a_restart() {
        let env = Env::new();
        let music = env.folder("music", 1);
        env.seed(&music);
        let mut library = env.open();
        let never = AtomicBool::new(false);
        library.scan(&never, &mut |_, _| {}).unwrap();
        fs::remove_dir_all(&music).unwrap();
        library.scan(&never, &mut |_, _| {}).unwrap();
        drop(library);

        let (on_event, mut log) = recording();
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        log.wait_for(is_scan_end);
        assert!(handle.shutdown(WAIT));
        log.finish();
        assert!(log.notices("library-folder-unavailable").is_empty());
    }

    #[test]
    fn add_folder_refusals_are_notices() {
        let env = Env::new();
        let music = env.folder("music", 1);
        let (on_event, mut log) = recording();
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        handle.send(LibraryRequest::AddFolder(music.clone()));
        log.wait_for(|e| snapshot_with(e, 1));

        handle.send(LibraryRequest::AddFolder(music.clone()));
        log.wait_for(|e| notice_is(e, "library-folder-duplicate"));
        handle.send(LibraryRequest::AddFolder(music.join("inner")));
        log.wait_for(|e| notice_is(e, "library-folder-nested"));
        assert!(handle.shutdown(WAIT));
    }

    #[test]
    fn remove_folder_sends_a_snapshot_and_keeps_the_files() {
        let env = Env::new();
        let music = env.folder("music", 2);
        env.seed(&music);
        let (on_event, mut log) = recording();
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        let last = log.wait_for(|e| snapshot_with(e, 2));
        let LibraryEvent::Snapshot(last) = last else {
            panic!("not a snapshot: {last:?}");
        };
        handle.send(LibraryRequest::RemoveFolder(last.folders[0].id));
        log.wait_for(|e| snapshot_with(e, 0));
        assert!(handle.shutdown(WAIT));
        assert_eq!(fs::read_dir(&music).unwrap().count(), 2);
    }

    #[test]
    fn clear_artwork_sends_a_snapshot_without_artwork() {
        let env = Env::new();
        let music = env.folder("music", 0);
        fs::copy(fixture("cover.flac"), music.join("cover.flac")).unwrap();
        env.seed(&music);
        let (on_event, mut log) = recording();
        let handle = LibraryService::start(env.paths(), on_event).unwrap();
        log.wait_for(|e| {
            matches!(e, LibraryEvent::Snapshot(s) if s.tracks.len() == 1
                && s.tracks[0].artwork.is_some())
        });

        handle.send(LibraryRequest::ClearArtwork);
        log.wait_for(|e| {
            matches!(e, LibraryEvent::Snapshot(s) if s.tracks.len() == 1
                && s.tracks[0].artwork.is_none())
        });
        assert!(handle.shutdown(WAIT));
    }

    #[test]
    fn folder_names_are_the_last_component() {
        assert_eq!(folder_name(Path::new("some/where/Music")), "Music");
        assert_eq!(folder_name(Path::new("some/where/Music/")), "Music");
        // A drive root has no last name; what there is names no person.
        assert_eq!(folder_name(Path::new("/")), "/");
    }
}
