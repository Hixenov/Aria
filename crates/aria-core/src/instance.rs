//! One Aria at a time for one data folder.
//!
//! Two copies of Aria on the same folder would write one library database at once. The first
//! copy holds an exclusive lock on a file in the data folder for as long as it runs, and the
//! system lets go of it when the process ends, however it ends. A second copy that finds the
//! lock taken asks the first, over a local named pipe, to show its window and to open the files
//! it was started with, and then quits before it opens anything else. The pipe carries a word and
//! those paths, and never leaves the computer.

use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{self, BufRead, BufReader, Read, Write},
    mem,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Name, Stream, ToNsName, prelude::*,
};

/// The file whose lock marks the running copy.
const LOCK_FILE: &str = "aria.lock";
/// What the second copy sends first: show your window.
const SHOW: &str = "show";
/// What starts each line after it that names a file or folder to open.
const OPEN: &str = "open ";
/// The most a request may hold, so a stray writer cannot fill memory.
const MAX_REQUEST: u64 = 4 * 1024 * 1024;
/// How long a second copy keeps trying to reach the first, which may still be starting.
pub const REACH_WAIT: Duration = Duration::from_secs(2);

/// What [`claim`] found.
pub enum Claim {
    /// No other copy runs on this folder; this one does now, for as long as the value lives.
    First(Instance),
    /// Another copy runs on this folder. `reached` says whether it was asked to show its
    /// window.
    Running { reached: bool },
}

/// The running copy's hold on its data folder, and the requests to show its window.
pub struct Instance {
    _lock: File,
    wakes: Arc<Wakes>,
}

#[derive(Default)]
struct Wakes {
    pending: AtomicBool,
    waker: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// Files and folders other copies were started with, not yet taken.
    opened: Mutex<Vec<PathBuf>>,
}

impl Wakes {
    fn wake(&self) {
        self.pending.store(true, Ordering::SeqCst);
        if let Some(waker) = self.waker.get() {
            waker();
        }
    }
}

impl Instance {
    /// Calls `waker` whenever another copy asks for the window, from another thread. Set once;
    /// a later call is ignored.
    pub fn set_waker(&self, waker: impl Fn() + Send + Sync + 'static) {
        if self.wakes.waker.set(Box::new(waker)).is_err() {
            tracing::warn!("the instance waker was already set");
        }
    }

    /// Whether another copy has asked for the window since the last call.
    pub fn take_wake(&self) -> bool {
        self.wakes.pending.swap(false, Ordering::SeqCst)
    }

    /// The files and folders other copies were started with since the last call, in order.
    pub fn take_opened(&self) -> Vec<PathBuf> {
        let mut opened = self
            .wakes
            .opened
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut *opened)
    }
}

/// Claims `data_dir` for this copy of Aria, or, if another copy holds it, asks that one to show
/// its window, trying for up to `reach_wait`.
pub fn claim(data_dir: &Path, reach_wait: Duration) -> io::Result<Claim> {
    claim_opening(data_dir, reach_wait, &[])
}

/// As [`claim`], and if another copy holds the folder, asks it to open `paths` too.
pub fn claim_opening(
    data_dir: &Path,
    reach_wait: Duration,
    paths: &[PathBuf],
) -> io::Result<Claim> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data_dir.join(LOCK_FILE))?;
    let name = pipe_name_text(data_dir);
    match lock.try_lock() {
        Ok(()) => {
            let wakes = Arc::new(Wakes::default());
            // Without the pipe this copy still holds the folder; a second copy then only finds
            // that it cannot reach it.
            if let Err(error) = listen(&name, Arc::clone(&wakes)) {
                tracing::warn!(%error, "another copy of Aria will not be able to show this one");
            }
            Ok(Claim::First(Instance { _lock: lock, wakes }))
        }
        Err(TryLockError::WouldBlock) => Ok(Claim::Running {
            reached: reach(&name, reach_wait, paths),
        }),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Answers other copies on the pipe named `name`, on a thread of its own that lasts as long
/// as the process.
fn listen(name: &str, wakes: Arc<Wakes>) -> io::Result<()> {
    // This copy holds the lock, so a name left behind by a copy that died is stale.
    let listener = ListenerOptions::new()
        .name(ns_name(name)?)
        .try_overwrite(true)
        .create_sync()?;
    thread::Builder::new()
        .name("aria-instance".into())
        .spawn(move || {
            for connection in listener.incoming() {
                match connection {
                    Ok(stream) => {
                        if let Some(paths) = read_request(stream) {
                            if !paths.is_empty() {
                                wakes
                                    .opened
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .extend(paths);
                            }
                            wakes.wake();
                        }
                    }
                    Err(error) => {
                        tracing::debug!(%error, "a call on the instance pipe failed");
                        // A pipe that keeps failing must not spin this thread.
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        })?;
    Ok(())
}

/// What came over `stream`: the paths to open, if it asks to show the window, which it does on
/// its first line. Each line after that names a path, after [`OPEN`]; other lines are ignored.
/// `None` for anything that is not such a request.
fn read_request(stream: Stream) -> Option<Vec<PathBuf>> {
    let mut lines = BufReader::new(stream.take(MAX_REQUEST)).lines();
    let first = lines.next()?.ok()?;
    if first.trim_end() != SHOW {
        return None;
    }
    let mut paths = Vec::new();
    for line in lines {
        // A request cut short keeps what came whole.
        let Ok(line) = line else { break };
        if let Some(path) = line.strip_prefix(OPEN).filter(|path| !path.is_empty()) {
            paths.push(PathBuf::from(path));
        }
    }
    Some(paths)
}

/// The request for the window and for `paths`, as [`read_request`] reads it. A path that cannot
/// be written on one line is left out.
fn request(paths: &[PathBuf]) -> String {
    let mut text = format!("{SHOW}\n");
    for path in paths {
        match path.to_str() {
            Some(path) if !path.contains(['\n', '\r']) => {
                text.push_str(OPEN);
                text.push_str(path);
                text.push('\n');
            }
            _ => tracing::debug!("a path that cannot be sent to the open copy is left out"),
        }
    }
    text
}

/// Asks the copy listening on `name` to show its window and open `paths`, trying until `wait`
/// has passed.
fn reach(name: &str, wait: Duration, paths: &[PathBuf]) -> bool {
    let request = request(paths);
    // The first line alone asks for the window. Aria 0.2.4 to 0.2.6 read only that, and may hang
    // up before the paths, which they cannot open anyway: that is not a failure to reach them.
    let (show, rest) = request.split_at(SHOW.len() + 1);
    let started = Instant::now();
    loop {
        let sent = ns_name(name)
            .and_then(Stream::connect)
            .and_then(|mut stream| {
                stream.write_all(show.as_bytes())?;
                if let Err(error) = stream.write_all(rest.as_bytes()) {
                    tracing::debug!(%error, "the open copy took the request for its window only");
                }
                Ok(())
            });
        match sent {
            Ok(()) => return true,
            Err(error) if started.elapsed() >= wait => {
                tracing::warn!(%error, "could not reach the running copy of Aria");
                return false;
            }
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn ns_name(name: &str) -> io::Result<Name<'_>> {
    name.to_ns_name::<GenericNamespaced>()
}

/// The pipe's name for `data_dir`.
///
/// Named pipes share one namespace on a computer, so the name comes from the folder: each
/// user's Aria, and each `ARIA` data folder, gets its own. The hash (64-bit FNV-1a) is fixed, so
/// that every version of Aria computes the same name.
fn pipe_name_text(data_dir: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data_dir.to_string_lossy().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("aria-{hash:016x}.sock")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    const SHORT: Duration = Duration::from_millis(300);

    fn wait_until(cond: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn first(dir: &Path) -> Instance {
        match claim(dir, SHORT).unwrap() {
            Claim::First(instance) => instance,
            Claim::Running { .. } => panic!("expected to be the first"),
        }
    }

    #[test]
    fn the_first_claim_on_a_folder_is_first() {
        let dir = tempfile::tempdir().unwrap();
        let instance = first(dir.path());
        assert!(!instance.take_wake());
    }

    #[test]
    fn a_second_claim_reaches_the_first_and_asks_for_its_window() {
        let dir = tempfile::tempdir().unwrap();
        let instance = first(dir.path());
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        instance.set_waker(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        let second = claim(dir.path(), REACH_WAIT).unwrap();
        assert!(matches!(second, Claim::Running { reached: true }));
        assert!(wait_until(|| woken.load(Ordering::SeqCst) == 1));
        assert!(instance.take_wake());
        assert!(!instance.take_wake(), "a wake is taken once");

        // And again, for a third launch.
        assert!(matches!(
            claim(dir.path(), REACH_WAIT).unwrap(),
            Claim::Running { reached: true }
        ));
        assert!(wait_until(|| woken.load(Ordering::SeqCst) == 2));
    }

    #[test]
    fn a_wake_before_the_waker_is_set_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let instance = first(dir.path());
        assert!(matches!(
            claim(dir.path(), REACH_WAIT).unwrap(),
            Claim::Running { reached: true }
        ));
        assert!(wait_until(|| instance.take_wake()));
    }

    #[test]
    fn a_second_claim_passes_its_files_on_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let instance = first(dir.path());
        let paths: Vec<PathBuf> = vec![
            r"C:\Music\one.mp3".into(),
            r"C:\Music\with spaces & ünïcode — 曲.flac".into(),
            "/home/someone/Music/Album".into(),
        ];
        let second = claim_opening(dir.path(), REACH_WAIT, &paths).unwrap();
        assert!(matches!(second, Claim::Running { reached: true }));
        assert!(wait_until(|| instance.take_wake()));
        assert_eq!(instance.take_opened(), paths);
        assert!(instance.take_opened().is_empty(), "taken once");
    }

    #[test]
    fn a_copy_that_only_asks_to_show_still_wakes_the_first() {
        // As Aria 0.2.4 to 0.2.6 ask.
        let dir = tempfile::tempdir().unwrap();
        let instance = first(dir.path());
        let name = pipe_name_text(dir.path());
        let mut stream = ns_name(&name).and_then(Stream::connect).unwrap();
        stream.write_all(b"show\n").unwrap();
        drop(stream);
        assert!(wait_until(|| instance.take_wake()));
        assert!(instance.take_opened().is_empty());
    }

    #[test]
    fn anything_else_on_the_pipe_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let instance = first(dir.path());
        let name = pipe_name_text(dir.path());
        let mut stream = ns_name(&name).and_then(Stream::connect).unwrap();
        stream.write_all(b"open C:\\x.mp3\nhello\n").unwrap();
        drop(stream);
        thread::sleep(SHORT);
        assert!(!instance.take_wake());
        assert!(instance.take_opened().is_empty());
    }

    #[test]
    fn the_folder_is_free_again_once_the_first_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        drop(first(dir.path()));
        let _again = first(dir.path());
    }

    #[test]
    fn other_folders_are_not_held() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let _a = first(a.path());
        let _b = first(b.path());
    }

    #[test]
    fn a_holder_that_does_not_answer_is_still_running() {
        let dir = tempfile::tempdir().unwrap();
        // A lock held by something that listens for nothing, like a copy that hangs.
        let lock = File::create(dir.path().join(LOCK_FILE)).unwrap();
        lock.try_lock().unwrap();
        let started = Instant::now();
        assert!(matches!(
            claim(dir.path(), SHORT).unwrap(),
            Claim::Running { reached: false }
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_pipe_name_is_the_same_for_the_same_folder_only() {
        let a = pipe_name_text(Path::new("C:\\Users\\a\\AppData\\Roaming\\Aria\\data"));
        assert_eq!(
            a,
            pipe_name_text(Path::new("C:\\Users\\a\\AppData\\Roaming\\Aria\\data"))
        );
        assert_ne!(
            a,
            pipe_name_text(Path::new("C:\\Users\\b\\AppData\\Roaming\\Aria\\data"))
        );
        assert!(a.starts_with("aria-"));
    }
}
