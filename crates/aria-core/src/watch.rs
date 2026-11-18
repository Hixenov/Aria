//! Watching the library's folders, so that files added, removed or changed there are picked up
//! without a manual rescan.
//!
//! The system tells of changes as they happen (`notify`: ReadDirectoryChangesW on Windows,
//! inotify on Linux, FSEvents on macOS). Changes come in bursts, as when an album is copied in,
//! so [`FolderWatcher`] waits until they have stopped for a while and then calls back once.
//! Only changes to music files and to folders count: a cover image or a text file does not.
//! Folders on a network share may not report changes at all; the manual rescan stays for them.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::opened::is_audio_file;

/// How long changes must have stopped before the library is scanned again.
pub const QUIET: Duration = Duration::from_secs(2);

/// Watches folders, and calls back once changes in them have stopped for a while.
pub struct FolderWatcher {
    watcher: RecommendedWatcher,
    watched: Vec<PathBuf>,
}

impl std::fmt::Debug for FolderWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FolderWatcher")
            .field("watched", &self.watched.len())
            .finish_non_exhaustive()
    }
}

impl FolderWatcher {
    /// A watcher of no folders yet, that calls `on_change`, on a thread of its own, once changes
    /// that count have stopped for `quiet`.
    ///
    /// # Errors
    ///
    /// Fails if the system's watcher or the thread cannot be started.
    pub fn start(quiet: Duration, on_change: impl Fn() + Send + 'static) -> notify::Result<Self> {
        let (changed, changes) = mpsc::channel::<()>();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            match event {
                Ok(event) if counts(&event) => {
                    // Fails only once the watcher is being dropped.
                    let _ = changed.send(());
                }
                Ok(_) => {}
                Err(error) => tracing::debug!(%error, "a folder watch reported an error"),
            }
        })?;
        thread::Builder::new()
            .name("aria-watch".into())
            .spawn(move || {
                // Ends when the watcher, and the sender in its handler, are dropped.
                while changes.recv().is_ok() {
                    loop {
                        match changes.recv_timeout(quiet) {
                            Ok(()) => {}
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    on_change();
                }
            })
            .map_err(notify::Error::io)?;
        Ok(Self {
            watcher,
            watched: Vec::new(),
        })
    }

    /// Watches exactly `folders`, with everything inside them: those no longer named are let go
    /// of, and new ones are added. A folder that cannot be watched, such as one that is not
    /// there now, is logged and left out.
    pub fn watch(&mut self, folders: &[PathBuf]) {
        let gone: Vec<PathBuf> = self
            .watched
            .iter()
            .filter(|folder| !folders.contains(folder))
            .cloned()
            .collect();
        for folder in &gone {
            if let Err(error) = self.watcher.unwatch(folder) {
                tracing::debug!(%error, "a folder could not be let go of");
            }
        }
        self.watched.retain(|folder| !gone.contains(folder));
        for folder in folders {
            if self.watched.contains(folder) {
                continue;
            }
            match self.watcher.watch(folder, RecursiveMode::Recursive) {
                Ok(()) => self.watched.push(folder.clone()),
                Err(error) => tracing::debug!(%error, "a library folder cannot be watched"),
            }
        }
    }
}

/// Whether `event` may change the library: a file or folder made, changed or removed, where a
/// path is music or has no extension, as a folder has.
fn counts(event: &notify::Event) -> bool {
    let kind = matches!(
        event.kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(
                notify::event::ModifyKind::Data(_)
                    | notify::event::ModifyKind::Name(_)
                    | notify::event::ModifyKind::Any
            )
            | EventKind::Any
    );
    kind && event.paths.iter().any(|path| matters(path))
}

/// Whether a change at `path` may change the library.
fn matters(path: &Path) -> bool {
    is_audio_file(path) || path.extension().is_none()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use super::*;

    const SHORT_QUIET: Duration = Duration::from_millis(300);
    /// Time for the system to start delivering a new watch's changes.
    const SETTLE: Duration = Duration::from_millis(300);

    fn counting() -> (Arc<AtomicUsize>, FolderWatcher) {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let watcher = FolderWatcher::start(SHORT_QUIET, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .expect("the watcher starts");
        (count, watcher)
    }

    fn wait_for(count: &AtomicUsize, at_least: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if count.load(Ordering::SeqCst) >= at_least {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn music_added_to_a_watched_folder_is_noticed_once_when_it_settles() {
        let dir = tempfile::tempdir().unwrap();
        let (count, mut watcher) = counting();
        watcher.watch(&[dir.path().to_path_buf()]);
        thread::sleep(SETTLE);

        fs::write(dir.path().join("a.mp3"), b"x").unwrap();
        fs::create_dir(dir.path().join("Album")).unwrap();
        fs::write(dir.path().join("Album").join("b.flac"), b"x").unwrap();
        assert!(wait_for(&count, 1), "the change was not noticed");
        thread::sleep(SHORT_QUIET * 3);
        assert_eq!(count.load(Ordering::SeqCst), 1, "one burst, one call");

        // And a removal later, once more.
        fs::remove_file(dir.path().join("a.mp3")).unwrap();
        assert!(wait_for(&count, 2), "the removal was not noticed");
    }

    // FSEvents reports changes by folder as well, so on macOS any change in a watched folder
    // counts.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_files_do_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let (count, mut watcher) = counting();
        watcher.watch(&[dir.path().to_path_buf()]);
        thread::sleep(SETTLE);

        fs::write(dir.path().join("cover.jpg"), b"x").unwrap();
        fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        thread::sleep(SHORT_QUIET * 4);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_folder_no_longer_watched_is_left_alone() {
        let (watched, gone) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (count, mut watcher) = counting();
        watcher.watch(&[watched.path().to_path_buf(), gone.path().to_path_buf()]);
        watcher.watch(&[watched.path().to_path_buf()]);
        thread::sleep(SETTLE);

        fs::write(gone.path().join("a.mp3"), b"x").unwrap();
        thread::sleep(SHORT_QUIET * 4);
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "a folder let go of was noticed"
        );
        // The one still watched is.
        fs::write(watched.path().join("a.mp3"), b"x").unwrap();
        assert!(wait_for(&count, 1));
    }

    #[test]
    fn a_folder_that_is_not_there_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let (_count, mut watcher) = counting();
        watcher.watch(&[dir.path().join("missing")]);
        assert!(watcher.watched.is_empty());
    }
}
