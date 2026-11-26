//! The update service: a thread that asks GitHub for a newer Aria, but only once the user has
//! said yes (or clicked "Check for updates"). On an installed Aria on Windows it downloads the
//! installer, checks it against the release's `SHA256SUMS.txt`, and keeps it for "Restart to
//! update". Anywhere else it only says a newer version is out.
//!
//! What is kept, in `<cache>/updates`: `Aria-<version>-setup.exe` and beside it
//! `Aria-<version>-setup.exe.sha256` (the checksum it matched, as hex). A download in progress is
//! `<name>.part` and is never run.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aria_update::{
    Asset, Client, Endpoints, InstallKind, UpdateError, Version, installer_name, is_newer,
    sha256_file, sha256_for,
};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};

use crate::notice::{Notice, NoticeLevel};
use crate::runtime::{DoneGuard, join_bounded};
use crate::settings::UpdateChecks;

const THREAD_NAME: &str = "aria-update";
/// The folder inside the cache directory.
const UPDATES_DIR: &str = "updates";
/// The largest installer Aria downloads.
pub const INSTALLER_CAP: u64 = 200 * 1024 * 1024;
/// The largest `SHA256SUMS.txt` Aria reads.
pub const SUMS_CAP: u64 = 64 * 1024;

/// What the update service needs.
#[derive(Clone)]
pub struct UpdateDeps {
    pub endpoints: Endpoints,
    /// The version that is running.
    pub running: Version,
    /// Where downloads are kept; `None` means nothing can be downloaded.
    pub cache_dir: Option<PathBuf>,
    pub kind: InstallKind,
    /// How long after start the first automatic check waits.
    pub start_delay: Duration,
    /// The time between automatic checks.
    pub interval: Duration,
    /// How soon to try again after a failure, or while a release is still being published.
    pub retry: Duration,
    /// Seconds since 1970, UTC.
    pub now: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl UpdateDeps {
    /// GitHub, this build's version, a first check 10 s after start, then once a day.
    pub fn system(cache_dir: Option<PathBuf>, kind: InstallKind) -> UpdateDeps {
        UpdateDeps {
            endpoints: Endpoints::github(),
            running: Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version::new(0, 0, 0)),
            cache_dir,
            kind,
            start_delay: Duration::from_secs(10),
            interval: Duration::from_secs(24 * 60 * 60),
            retry: Duration::from_secs(60 * 60),
            now: Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())
            }),
        }
    }
}

impl std::fmt::Debug for UpdateDeps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateDeps")
            .field("running", &self.running)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateRequest {
    /// The user's choice and the last check, at start and whenever either changes.
    Settings {
        checks: UpdateChecks,
        checked_at: Option<u64>,
    },
    /// The user clicked "Check for updates": check now, whatever the setting.
    CheckNow,
    /// The user clicked "Restart to update": check the installer once more and hand it over.
    Install,
    Shutdown,
}

/// Where the update checker stands, for the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Not checked since Aria started.
    Idle,
    Checking,
    UpToDate,
    /// A newer release is out but its files are not all there yet.
    NotReady(Version),
    Downloading {
        version: Version,
        percent: u8,
    },
    /// Downloaded and checked; "Restart to update".
    Ready(Version),
    /// A newer release this Aria cannot install itself; its page is `page`.
    Available {
        version: Version,
        page: String,
    },
    Failed(UpdateError),
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateEvent {
    Status(UpdateStatus),
    /// A check finished at this time (seconds since 1970); the app stores it in the settings.
    Checked(u64),
    /// The installer is checked and ready to start, once Aria has shut down.
    Install(PathBuf),
    Notice(Notice),
}

pub struct UpdateHandle {
    requests: Sender<UpdateRequest>,
    done: Receiver<()>,
    thread: thread::JoinHandle<()>,
}

impl UpdateHandle {
    /// Queues `request` without blocking. If the thread has exited, it is dropped.
    pub fn send(&self, request: UpdateRequest) {
        // Fails only once the thread has exited, when nothing is listening.
        let _ = self.requests.send(request);
    }

    /// Sends [`UpdateRequest::Shutdown`] and waits up to `timeout` for the thread to end. A
    /// download in progress is not waited for.
    pub fn join(self, timeout: Duration) -> bool {
        self.send(UpdateRequest::Shutdown);
        join_bounded(&self.done, self.thread, timeout, THREAD_NAME)
    }
}

pub struct UpdateService;

impl UpdateService {
    pub fn start(
        deps: UpdateDeps,
        on_event: Box<dyn Fn(UpdateEvent) + Send>,
    ) -> io::Result<UpdateHandle> {
        let (requests_tx, requests_rx) = unbounded();
        let (done_tx, done_rx) = bounded(1);
        let service = Service::new(deps, on_event);
        let thread = thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || {
                let _done = DoneGuard::new(done_tx);
                service.run(&requests_rx);
            })?;
        Ok(UpdateHandle {
            requests: requests_tx,
            done: done_rx,
            thread,
        })
    }
}

struct Service {
    deps: UpdateDeps,
    client: Client,
    on_event: Box<dyn Fn(UpdateEvent) + Send>,
    started: Instant,
    /// Whether automatic checks are on.
    auto: bool,
    /// When the next check runs by itself.
    due: Option<Instant>,
    /// A retry after a failure or a release still being published, kept across settings.
    retry_at: Option<Instant>,
    /// The checked installer waiting for "Restart to update".
    ready: Option<(Version, PathBuf)>,
}

/// What a check found.
enum Found {
    UpToDate,
    Available { version: Version, page: String },
    NotReady(Version),
    Ready(Version, PathBuf),
}

impl Service {
    fn new(deps: UpdateDeps, on_event: Box<dyn Fn(UpdateEvent) + Send>) -> Service {
        Service {
            client: Client::new(deps.endpoints.clone()),
            deps,
            on_event,
            started: Instant::now(),
            auto: false,
            due: None,
            retry_at: None,
            ready: None,
        }
    }

    fn run(mut self, requests: &Receiver<UpdateRequest>) {
        self.tidy();
        loop {
            let request = match self.due {
                Some(due) => requests.recv_deadline(due),
                None => requests.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match request {
                Ok(UpdateRequest::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(UpdateRequest::Settings { checks, checked_at }) => {
                    self.settings(checks, checked_at);
                }
                Ok(UpdateRequest::CheckNow) => self.check(),
                Ok(UpdateRequest::Install) => self.install(),
                Err(RecvTimeoutError::Timeout) => {
                    self.due = None;
                    self.retry_at = None;
                    self.check();
                }
            }
        }
    }

    fn emit(&self, event: UpdateEvent) {
        (self.on_event)(event);
    }

    fn dir(&self) -> Option<PathBuf> {
        self.deps
            .cache_dir
            .as_ref()
            .map(|cache| cache.join(UPDATES_DIR))
    }

    /// Settings arrived: schedule the next automatic check, or cancel it.
    fn settings(&mut self, checks: UpdateChecks, checked_at: Option<u64>) {
        self.auto = checks == UpdateChecks::On;
        if !self.auto {
            self.due = None;
            self.retry_at = None;
            return;
        }
        if self.ready.is_some() {
            return;
        }
        let now = Instant::now();
        let mut due = self.started + self.deps.start_delay;
        if let Some(at) = checked_at {
            let since = Duration::from_secs((self.deps.now)().saturating_sub(at));
            due = due.max(now + self.deps.interval.saturating_sub(since));
        }
        if let Some(retry) = self.retry_at {
            due = due.min(retry);
        }
        self.due = Some(due);
    }

    fn check(&mut self) {
        if let Some((version, _)) = &self.ready {
            self.emit(UpdateEvent::Status(UpdateStatus::Ready(version.clone())));
            return;
        }
        self.emit(UpdateEvent::Status(UpdateStatus::Checking));
        match self.find() {
            Ok(Found::UpToDate) => {
                self.emit(UpdateEvent::Status(UpdateStatus::UpToDate));
                self.checked();
            }
            Ok(Found::Available { version, page }) => {
                self.emit(UpdateEvent::Status(UpdateStatus::Available {
                    version,
                    page,
                }));
                self.checked();
            }
            Ok(Found::NotReady(version)) => {
                self.emit(UpdateEvent::Status(UpdateStatus::NotReady(version)));
                // A click is one check: only automatic checks come back for it.
                self.retry();
            }
            Ok(Found::Ready(version, path)) => {
                self.emit(UpdateEvent::Status(UpdateStatus::Ready(version.clone())));
                self.ready = Some((version.clone(), path));
                self.checked();
                // Nothing more to look for until this one is installed.
                self.due = None;
                self.emit(UpdateEvent::Notice(
                    Notice::new(NoticeLevel::Info, "notice-update-ready")
                        .with_arg("version", version.to_string()),
                ));
            }
            // A tag that is not a plain version is nothing to offer: the same as no news.
            Err(UpdateError::NotARelease) => {
                self.emit(UpdateEvent::Status(UpdateStatus::UpToDate));
                self.checked();
            }
            // GitHub out of reach, busy, or an answer that may be better soon: in an hour.
            Err(error @ (UpdateError::Offline | UpdateError::Busy | UpdateError::Answer)) => {
                tracing::info!(%error, "update check failed");
                self.emit(UpdateEvent::Status(UpdateStatus::Failed(error)));
                self.retry();
            }
            // The same release would fail the same way: not before the next day's check.
            Err(error) => {
                tracing::info!(%error, "update check failed");
                self.emit(UpdateEvent::Status(UpdateStatus::Failed(error)));
                self.checked();
            }
        }
    }

    /// A check finished: tell the app, and plan the next one.
    fn checked(&mut self) {
        let at = (self.deps.now)();
        self.retry_at = None;
        self.due = self.auto.then(|| Instant::now() + self.deps.interval);
        self.emit(UpdateEvent::Checked(at));
    }

    /// Automatic checks only: the next one in an hour, not a day.
    fn retry(&mut self) {
        if self.auto {
            let at = Instant::now() + self.deps.retry;
            self.retry_at = Some(at);
            self.due = Some(at);
        }
    }

    fn find(&self) -> Result<Found, UpdateError> {
        let release = self.client.latest()?;
        let version = release.version;
        if !is_newer(&version, &self.deps.running) {
            return Ok(Found::UpToDate);
        }
        if self.deps.kind == InstallKind::TellOnly {
            return Ok(Found::Available {
                version,
                page: release.page,
            });
        }
        let dir = self
            .dir()
            .ok_or(UpdateError::Disk(io::ErrorKind::NotFound))?;
        fs::create_dir_all(&dir).map_err(disk)?;
        // The checksums are attached last, a few minutes after the release appears: until then
        // the release is still being published.
        let sums = match self.client.text(&release.sums, SUMS_CAP) {
            Err(UpdateError::Missing) => return Ok(Found::NotReady(version)),
            sums => sums?,
        };
        let Some(want) = sha256_for(&sums, &release.installer.name) else {
            return Ok(Found::NotReady(version));
        };
        let path = self.download(&dir, &version, &release.installer, want)?;
        Ok(Found::Ready(version, path))
    }

    /// Downloads `installer` into `dir`, keeps it only if it matches `want`, and writes its
    /// checksum beside it.
    fn download(
        &self,
        dir: &Path,
        version: &Version,
        installer: &Asset,
        want: [u8; 32],
    ) -> Result<PathBuf, UpdateError> {
        let part = dir.join(format!("{}.part", installer.name));
        let mut shown = None;
        let on_event = &self.on_event;
        self.client
            .download(installer, &part, INSTALLER_CAP, &mut |done, total| {
                // An unknown size (0) shows 0% until the end.
                let percent = done
                    .saturating_mul(100)
                    .checked_div(total)
                    .map_or(0, |p| u8::try_from(p.min(100)).unwrap_or(100));
                if shown != Some(percent) {
                    shown = Some(percent);
                    on_event(UpdateEvent::Status(UpdateStatus::Downloading {
                        version: version.clone(),
                        percent,
                    }));
                }
            })?;
        let kept = keep(&part, &dir.join(&installer.name), want);
        if kept.is_err() {
            remove(&part);
        }
        kept
    }

    /// At start: deletes leftovers, and offers an installer downloaded before that still checks.
    fn tidy(&mut self) {
        let Some(dir) = self.dir() else { return };
        let Ok(entries) = fs::read_dir(&dir) else {
            return;
        };
        let mut best: Option<(Version, PathBuf)> = None;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(version) = installer_version(&name) {
                let wanted = is_newer(&version, &self.deps.running)
                    && best.as_ref().is_none_or(|(kept, _)| version > *kept);
                if wanted {
                    if let Some((_, older)) = best.replace((version, path)) {
                        remove_pair(&older);
                    }
                } else {
                    remove_pair(&path);
                }
            } else if !name.ends_with(".sha256") {
                // `.part` files, and anything else Aria did not leave there to keep.
                remove(&path);
            }
        }
        // Checksums whose installer is gone.
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.filter_map(Result::ok) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some(exe) = name.strip_suffix(".sha256")
                    && !dir.join(exe).is_file()
                {
                    remove(&entry.path());
                }
            }
        }
        if let Some((version, path)) = best {
            if self.deps.kind == InstallKind::Installed && verified(&path) {
                self.emit(UpdateEvent::Status(UpdateStatus::Ready(version.clone())));
                self.ready = Some((version, path));
            } else {
                remove_pair(&path);
            }
        }
    }

    fn install(&mut self) {
        let Some((_, path)) = &self.ready else { return };
        if verified(path) {
            self.emit(UpdateEvent::Install(path.clone()));
            return;
        }
        tracing::warn!("the downloaded update no longer matches its checksum; deleting it");
        remove_pair(path);
        self.ready = None;
        self.emit(UpdateEvent::Notice(Notice::new(
            NoticeLevel::Warning,
            "notice-update-broken",
        )));
        self.emit(UpdateEvent::Status(UpdateStatus::Idle));
        if self.auto {
            self.due = Some(Instant::now());
        }
    }
}

/// Keeps the download `part` as `path` if it matches `want`, with its checksum beside it.
fn keep(part: &Path, path: &Path, want: [u8; 32]) -> Result<PathBuf, UpdateError> {
    if sha256_file(part).map_err(disk)? != want {
        return Err(UpdateError::Checksum);
    }
    fs::write(sidecar(path), hex::encode(want)).map_err(disk)?;
    fs::rename(part, path).map_err(disk)?;
    Ok(path.to_path_buf())
}

fn disk(error: io::Error) -> UpdateError {
    UpdateError::Disk(error.kind())
}

/// The checksum file kept beside `installer`.
fn sidecar(installer: &Path) -> PathBuf {
    let mut name = installer.as_os_str().to_owned();
    name.push(".sha256");
    PathBuf::from(name)
}

/// Whether `installer` still matches the checksum kept beside it.
fn verified(installer: &Path) -> bool {
    let Ok(text) = fs::read_to_string(sidecar(installer)) else {
        return false;
    };
    let mut want = [0u8; 32];
    hex::decode_to_slice(text.trim(), &mut want).is_ok()
        && sha256_file(installer).is_ok_and(|got| got == want)
}

/// The version in `Aria-<version>-setup.exe`, if `name` is exactly that.
fn installer_version(name: &str) -> Option<Version> {
    let version = name.strip_prefix("Aria-")?.strip_suffix("-setup.exe")?;
    let version = Version::parse(version).ok()?;
    (installer_name(&version) == name).then_some(version)
}

fn remove(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(%error, "could not delete an old update file");
    }
}

fn remove_pair(installer: &Path) {
    remove(installer);
    remove(&sidecar(installer));
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_spotify::fake::{FakeSpotify, Reply};

    const LATEST: &str = "/Hixenov/Aria/releases/latest";
    const TAG: &str = "/Hixenov/Aria/releases/tag/";
    const DOWNLOADS: &str = "/Hixenov/Aria/releases/download/v0.3.6/";
    const INSTALLER: &[u8] = b"hello world";
    const INSTALLER_SHA: &str = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
    const NOW: u64 = 1_800_000_000;
    const WAIT: Duration = Duration::from_secs(5);
    const QUIET: Duration = Duration::from_millis(300);

    struct Harness {
        fake: FakeSpotify,
        handle: Option<UpdateHandle>,
        events: Receiver<UpdateEvent>,
        dir: tempfile::TempDir,
    }

    impl Harness {
        fn start(kind: InstallKind, running: Version, prepare: impl FnOnce(&Path)) -> Harness {
            let fake = FakeSpotify::start();
            let dir = tempfile::tempdir().unwrap();
            prepare(dir.path());
            let deps = UpdateDeps {
                endpoints: Endpoints {
                    latest: format!("{}{LATEST}", fake.base()),
                    tag_prefix: format!("{}{TAG}", fake.base()),
                    download_prefix: format!("{}/Hixenov/Aria/releases/download/", fake.base()),
                },
                running,
                cache_dir: Some(dir.path().to_path_buf()),
                kind,
                start_delay: Duration::ZERO,
                interval: Duration::from_secs(24 * 60 * 60),
                retry: Duration::from_millis(100),
                now: Arc::new(|| NOW),
            };
            let (tx, events) = unbounded();
            let handle = UpdateService::start(
                deps,
                Box::new(move |event| {
                    let _ = tx.send(event);
                }),
            )
            .unwrap();
            Harness {
                fake,
                handle: Some(handle),
                events,
                dir,
            }
        }

        fn installed() -> Harness {
            Harness::start(InstallKind::Installed, Version::new(0, 3, 5), |_| {})
        }

        fn send(&self, request: UpdateRequest) {
            if let Some(handle) = &self.handle {
                handle.send(request);
            }
        }

        fn settings(&self, checks: UpdateChecks, checked_at: Option<u64>) {
            self.send(UpdateRequest::Settings { checks, checked_at });
        }

        /// Events until one matches `stop` (included), or panics after [`WAIT`].
        fn until(&self, stop: impl Fn(&UpdateEvent) -> bool) -> Vec<UpdateEvent> {
            let deadline = Instant::now() + WAIT;
            let mut seen = Vec::new();
            loop {
                match self.events.recv_deadline(deadline) {
                    Ok(event) => {
                        let done = stop(&event);
                        seen.push(event);
                        if done {
                            return seen;
                        }
                    }
                    Err(_) => panic!("timed out; seen {seen:?}"),
                }
            }
        }

        /// Events that arrive within [`QUIET`].
        fn quiet(&self) -> Vec<UpdateEvent> {
            let deadline = Instant::now() + QUIET;
            let mut seen = Vec::new();
            while let Ok(event) = self.events.recv_deadline(deadline) {
                seen.push(event);
            }
            seen
        }

        fn updates(&self) -> PathBuf {
            self.dir.path().join(UPDATES_DIR)
        }

        fn files(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.updates())
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }

        fn latest_count(&self) -> usize {
            self.fake
                .requests()
                .iter()
                .filter(|seen| seen.path == LATEST)
                .count()
        }

        /// GitHub's "latest release" page, redirecting to the release tagged `tag`.
        fn redirect(&self, tag: &str) -> Reply {
            Reply::empty(302).header("Location", &format!("{}{TAG}{tag}", self.fake.base()))
        }

        /// The release v0.3.6 with its installer (`body`) and checksums (`sums`).
        fn serve_release(&self, body: &[u8], sums: Option<&str>) {
            self.fake.reply("GET", LATEST, self.redirect("v0.3.6"));
            self.fake.reply(
                "GET",
                &format!("{DOWNLOADS}Aria-0.3.6-setup.exe"),
                Reply::bytes(200, "application/octet-stream", body.to_vec()),
            );
            if let Some(sums) = sums {
                self.fake.reply(
                    "GET",
                    &format!("{DOWNLOADS}SHA256SUMS.txt"),
                    Reply::bytes(200, "text/plain", sums.as_bytes().to_vec()),
                );
            }
        }

        fn serve_good_release(&self) {
            self.serve_release(INSTALLER, Some(&good_sums()));
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                handle.join(WAIT);
            }
        }
    }

    fn good_sums() -> String {
        format!(
            "{INSTALLER_SHA}  Aria-0.3.6-setup.exe\n{}  aria.exe\n",
            "00".repeat(32)
        )
    }

    fn status(event: &UpdateEvent) -> Option<&UpdateStatus> {
        match event {
            UpdateEvent::Status(status) => Some(status),
            _ => None,
        }
    }

    fn v036() -> Version {
        Version::new(0, 3, 6)
    }

    fn is_ready(event: &UpdateEvent) -> bool {
        matches!(event, UpdateEvent::Status(UpdateStatus::Ready(_)))
    }

    fn is_final(event: &UpdateEvent) -> bool {
        matches!(
            event,
            UpdateEvent::Status(
                UpdateStatus::UpToDate
                    | UpdateStatus::Ready(_)
                    | UpdateStatus::Available { .. }
                    | UpdateStatus::NotReady(_)
                    | UpdateStatus::Failed(_)
            )
        )
    }

    /// A downloaded, checked v0.3.6 already in `<cache>/updates`.
    fn ready_pair(cache: &Path) {
        let dir = cache.join(UPDATES_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Aria-0.3.6-setup.exe"), INSTALLER).unwrap();
        fs::write(dir.join("Aria-0.3.6-setup.exe.sha256"), INSTALLER_SHA).unwrap();
    }

    #[test]
    fn no_request_until_the_user_says_yes() {
        let h = Harness::installed();
        h.serve_good_release();
        h.settings(UpdateChecks::NotAsked, None);
        assert_eq!(h.quiet(), []);
        h.settings(UpdateChecks::Off, None);
        assert_eq!(h.quiet(), []);
        assert!(h.fake.requests().is_empty());
    }

    #[test]
    fn on_checks_after_the_delay_and_reports_up_to_date() {
        let h = Harness::start(InstallKind::Installed, Version::new(0, 3, 6), |_| {});
        h.serve_good_release();
        h.settings(UpdateChecks::On, None);
        let seen = h.until(|e| matches!(e, UpdateEvent::Checked(_)));
        assert_eq!(
            seen,
            [
                UpdateEvent::Status(UpdateStatus::Checking),
                UpdateEvent::Status(UpdateStatus::UpToDate),
                UpdateEvent::Checked(NOW),
            ]
        );
        assert_eq!(h.latest_count(), 1);
        // Nothing else was downloaded.
        assert_eq!(h.fake.requests().len(), 1);
    }

    #[test]
    fn a_recent_check_is_not_repeated() {
        let h = Harness::installed();
        h.serve_good_release();
        h.settings(UpdateChecks::On, Some(NOW - 3600));
        assert_eq!(h.quiet(), []);
        assert!(h.fake.requests().is_empty());
    }

    #[test]
    fn a_day_old_check_is_repeated() {
        let h = Harness::start(InstallKind::Installed, Version::new(0, 3, 6), |_| {});
        h.serve_good_release();
        h.settings(UpdateChecks::On, Some(NOW - 24 * 60 * 60));
        h.until(|e| matches!(e, UpdateEvent::Checked(_)));
        assert_eq!(h.latest_count(), 1);
    }

    #[test]
    fn check_now_works_with_checks_off() {
        let h = Harness::start(InstallKind::Installed, Version::new(0, 3, 6), |_| {});
        h.serve_good_release();
        h.settings(UpdateChecks::Off, None);
        h.send(UpdateRequest::CheckNow);
        let seen = h.until(is_final);
        assert_eq!(seen.last().and_then(status), Some(&UpdateStatus::UpToDate));
        assert_eq!(h.latest_count(), 1);
    }

    #[test]
    fn turning_checks_off_stops_them() {
        let h = Harness::installed();
        h.serve_good_release();
        h.settings(UpdateChecks::On, Some(NOW - 3600));
        h.settings(UpdateChecks::Off, Some(NOW - 3600));
        assert_eq!(h.quiet(), []);
        assert!(h.fake.requests().is_empty());
    }

    #[test]
    fn a_newer_release_downloads_verifies_and_is_ready() {
        let h = Harness::installed();
        h.serve_good_release();
        h.settings(UpdateChecks::On, None);
        let seen = h.until(|e| matches!(e, UpdateEvent::Notice(_)));
        assert_eq!(
            h.files(),
            ["Aria-0.3.6-setup.exe", "Aria-0.3.6-setup.exe.sha256"]
        );
        assert_eq!(
            fs::read_to_string(h.updates().join("Aria-0.3.6-setup.exe.sha256")).unwrap(),
            INSTALLER_SHA
        );
        assert!(
            seen.contains(&UpdateEvent::Status(UpdateStatus::Downloading {
                version: v036(),
                percent: 100
            }))
        );
        let n = seen.len();
        assert_eq!(
            seen[n - 3],
            UpdateEvent::Status(UpdateStatus::Ready(v036()))
        );
        assert_eq!(seen[n - 2], UpdateEvent::Checked(NOW));
        assert_eq!(
            seen[n - 1],
            UpdateEvent::Notice(
                Notice::new(NoticeLevel::Info, "notice-update-ready").with_arg("version", "0.3.6")
            )
        );
    }

    #[test]
    fn a_short_download_is_not_kept() {
        // GitHub gives the size with the file itself, so a shorter file arrives whole and its
        // checksum is what refuses it. (A download cut short is refused by the client.)
        let h = Harness::installed();
        h.serve_release(b"hello", Some(&good_sums()));
        h.send(UpdateRequest::CheckNow);
        let seen = h.until(is_final);
        assert_eq!(
            seen.last().and_then(status),
            Some(&UpdateStatus::Failed(UpdateError::Checksum))
        );
        assert_eq!(h.files(), Vec::<String>::new());
    }

    #[test]
    fn a_wrong_checksum_is_not_kept() {
        let h = Harness::installed();
        h.serve_release(b"hello earth", Some(&good_sums()));
        h.send(UpdateRequest::CheckNow);
        let seen = h.until(is_final);
        assert_eq!(
            seen.last().and_then(status),
            Some(&UpdateStatus::Failed(UpdateError::Checksum))
        );
        assert_eq!(h.files(), Vec::<String>::new());
    }

    #[test]
    fn missing_sums_means_not_ready_and_a_retry() {
        let h = Harness::installed();
        // Its checksums are not there yet: GitHub answers 404.
        h.fake.reply("GET", LATEST, h.redirect("v0.3.6"));
        h.settings(UpdateChecks::On, None);
        let first = h.until(is_final);
        assert_eq!(
            first.last().and_then(status),
            Some(&UpdateStatus::NotReady(v036()))
        );
        let second = h.until(is_final);
        assert_eq!(
            second.last().and_then(status),
            Some(&UpdateStatus::NotReady(v036()))
        );
        assert_eq!(h.latest_count(), 2);
        assert!(
            !first
                .iter()
                .chain(&second)
                .any(|e| matches!(e, UpdateEvent::Checked(_))),
            "a release still being published is not a finished check"
        );
    }

    #[test]
    fn a_click_with_checks_off_is_one_check_only() {
        let h = Harness::installed();
        // Its checksums are not there yet: GitHub answers 404.
        h.fake.reply("GET", LATEST, h.redirect("v0.3.6"));
        h.settings(UpdateChecks::Off, None);
        h.send(UpdateRequest::CheckNow);
        let seen = h.until(is_final);
        assert_eq!(
            seen.last().and_then(status),
            Some(&UpdateStatus::NotReady(v036()))
        );
        assert_eq!(h.quiet(), []);
        assert_eq!(h.latest_count(), 1);
    }

    #[test]
    fn a_wrong_checksum_waits_a_day() {
        let h = Harness::installed();
        h.serve_release(b"hello earth", Some(&good_sums()));
        h.serve_release(b"hello earth", Some(&good_sums()));
        h.settings(UpdateChecks::On, None);
        let seen = h.until(|e| matches!(e, UpdateEvent::Checked(_)));
        assert!(seen.contains(&UpdateEvent::Status(UpdateStatus::Failed(
            UpdateError::Checksum
        ))));
        assert_eq!(h.quiet(), []);
        assert_eq!(h.latest_count(), 1);
    }

    #[test]
    fn an_odd_tag_counts_as_up_to_date() {
        let h = Harness::installed();
        h.fake.reply("GET", LATEST, h.redirect("latest"));
        h.fake.reply("GET", LATEST, h.redirect("v0.3.6"));
        h.settings(UpdateChecks::On, None);
        let seen = h.until(|e| matches!(e, UpdateEvent::Checked(_)));
        assert_eq!(
            seen,
            [
                UpdateEvent::Status(UpdateStatus::Checking),
                UpdateEvent::Status(UpdateStatus::UpToDate),
                UpdateEvent::Checked(NOW),
            ]
        );
        assert_eq!(h.quiet(), []);
        assert_eq!(h.latest_count(), 1);
    }

    #[test]
    fn a_failed_check_is_retried_while_on() {
        let h = Harness::start(InstallKind::Installed, Version::new(0, 3, 6), |_| {});
        h.fake.reply("GET", LATEST, Reply::json(500, "{}"));
        h.fake.reply("GET", LATEST, h.redirect("v0.3.6"));
        h.settings(UpdateChecks::On, None);
        let first = h.until(is_final);
        assert_eq!(
            first.last().and_then(status),
            Some(&UpdateStatus::Failed(UpdateError::Answer))
        );
        let second = h.until(is_final);
        assert_eq!(
            second.last().and_then(status),
            Some(&UpdateStatus::UpToDate)
        );
    }

    #[test]
    fn sums_without_the_installer_are_not_ready() {
        let h = Harness::installed();
        h.serve_release(INSTALLER, Some(&format!("{}  aria.exe\n", "00".repeat(32))));
        h.settings(UpdateChecks::Off, None);
        h.send(UpdateRequest::CheckNow);
        let seen = h.until(is_final);
        assert_eq!(
            seen.last().and_then(status),
            Some(&UpdateStatus::NotReady(v036()))
        );
        assert!(
            !h.fake
                .requests()
                .iter()
                .any(|seen| seen.path.ends_with("setup.exe")),
            "nothing is downloaded without its checksum"
        );
    }

    #[test]
    fn a_busy_github_is_retried_while_on() {
        let h = Harness::start(InstallKind::Installed, Version::new(0, 3, 6), |_| {});
        h.fake.reply("GET", LATEST, Reply::empty(429));
        h.fake.reply("GET", LATEST, h.redirect("v0.3.6"));
        h.settings(UpdateChecks::On, None);
        let first = h.until(is_final);
        assert_eq!(
            first.last().and_then(status),
            Some(&UpdateStatus::Failed(UpdateError::Busy))
        );
        assert!(
            !first.iter().any(|e| matches!(e, UpdateEvent::Checked(_))),
            "a refusal is not a finished check"
        );
        let second = h.until(is_final);
        assert_eq!(
            second.last().and_then(status),
            Some(&UpdateStatus::UpToDate)
        );
        assert_eq!(h.latest_count(), 2);
    }

    #[test]
    fn a_busy_click_is_one_check() {
        let h = Harness::installed();
        h.fake.reply("GET", LATEST, Reply::empty(403));
        h.settings(UpdateChecks::Off, None);
        h.send(UpdateRequest::CheckNow);
        let seen = h.until(is_final);
        assert_eq!(
            seen.last().and_then(status),
            Some(&UpdateStatus::Failed(UpdateError::Busy))
        );
        assert_eq!(h.quiet(), []);
        assert_eq!(h.latest_count(), 1);
    }

    #[test]
    fn tell_only_never_downloads() {
        let h = Harness::start(InstallKind::TellOnly, Version::new(0, 3, 5), |_| {});
        h.serve_good_release();
        h.send(UpdateRequest::CheckNow);
        let seen = h.until(is_final);
        assert_eq!(
            seen.last().and_then(status),
            Some(&UpdateStatus::Available {
                version: v036(),
                page: format!("{}{TAG}v0.3.6", h.fake.base())
            })
        );
        assert_eq!(h.fake.requests().len(), 1);
        assert_eq!(h.files(), Vec::<String>::new());
    }

    #[test]
    fn a_ready_installer_from_before_is_offered_without_asking() {
        let h = Harness::start(InstallKind::Installed, Version::new(0, 3, 5), ready_pair);
        let seen = h.until(is_ready);
        assert_eq!(seen, [UpdateEvent::Status(UpdateStatus::Ready(v036()))]);
        assert!(h.fake.requests().is_empty());
    }

    #[test]
    fn a_ready_installer_is_not_offered_to_a_tell_only_aria() {
        let h = Harness::start(InstallKind::TellOnly, Version::new(0, 3, 5), ready_pair);
        assert_eq!(h.quiet(), []);
        assert_eq!(h.files(), Vec::<String>::new());
    }

    #[test]
    fn old_installers_and_parts_are_deleted_at_start() {
        let h = Harness::start(InstallKind::Installed, Version::new(0, 3, 6), |cache| {
            ready_pair(cache);
            let dir = cache.join(UPDATES_DIR);
            fs::write(dir.join("Aria-0.3.5-setup.exe"), b"old").unwrap();
            fs::write(dir.join("Aria-0.3.5-setup.exe.sha256"), "00").unwrap();
            fs::write(dir.join("Aria-0.3.7-setup.exe.part"), b"half").unwrap();
        });
        assert_eq!(h.quiet(), []);
        assert_eq!(h.files(), Vec::<String>::new());
    }

    #[test]
    fn install_sends_the_verified_path() {
        let h = Harness::start(InstallKind::Installed, Version::new(0, 3, 5), ready_pair);
        h.until(is_ready);
        h.send(UpdateRequest::Install);
        let seen = h.until(|e| matches!(e, UpdateEvent::Install(_)));
        assert_eq!(
            seen,
            [UpdateEvent::Install(
                h.updates().join("Aria-0.3.6-setup.exe")
            )]
        );
    }

    #[test]
    fn a_changed_installer_is_not_started() {
        let h = Harness::start(InstallKind::Installed, Version::new(0, 3, 5), ready_pair);
        h.until(is_ready);
        fs::write(h.updates().join("Aria-0.3.6-setup.exe"), b"hello worle").unwrap();
        h.send(UpdateRequest::Install);
        let seen = h.until(|e| matches!(e, UpdateEvent::Notice(_)));
        assert!(!seen.iter().any(|e| matches!(e, UpdateEvent::Install(_))));
        assert_eq!(
            seen.last(),
            Some(&UpdateEvent::Notice(Notice::new(
                NoticeLevel::Warning,
                "notice-update-broken"
            )))
        );
        assert_eq!(h.files(), Vec::<String>::new());
    }

    #[test]
    fn check_now_while_ready_asks_nothing() {
        let h = Harness::start(InstallKind::Installed, Version::new(0, 3, 5), ready_pair);
        h.until(is_ready);
        h.send(UpdateRequest::CheckNow);
        let seen = h.until(is_final);
        assert_eq!(seen, [UpdateEvent::Status(UpdateStatus::Ready(v036()))]);
        assert!(h.fake.requests().is_empty());
    }
}
