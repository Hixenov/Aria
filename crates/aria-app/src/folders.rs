//! Opens Aria's folders in the file manager without making the UI wait.
//!
//! `open::that` waits for the system's launcher. On Windows that is PowerShell, which takes
//! about 0.3 s to start, and on some Linux desktops the launcher returns only once the file
//! manager is closed. So each request runs on a short-lived thread that repaints the UI when
//! it is done, and the UI collects the outcome on that frame.

use std::{
    io,
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
};

use eframe::egui::Context;

/// Name of the threads that open folders.
const THREAD_NAME: &str = "aria-open-folder";

/// Opens a path with the system's default application.
pub(crate) type Open = fn(&Path) -> io::Result<()>;

/// Requests to open a folder, until their outcome is collected.
pub(crate) struct FolderOpener {
    open: Open,
    /// Each thread returns whether its folder opened.
    running: Vec<JoinHandle<bool>>,
}

impl FolderOpener {
    /// Opens folders with `open`.
    pub(crate) fn new(open: Open) -> Self {
        Self {
            open,
            running: Vec::new(),
        }
    }

    /// Starts opening `dir`, and repaints `ctx` once that has finished, for
    /// [`Self::failures`] to collect.
    ///
    /// # Errors
    ///
    /// Fails if no thread could be started, in which case nothing is opened.
    pub(crate) fn open(&mut self, ctx: &Context, dir: PathBuf) -> io::Result<()> {
        let open = self.open;
        let ctx = ctx.clone();
        let thread = thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                let opened = match open(&dir) {
                    Ok(()) => true,
                    Err(err) => {
                        tracing::warn!(%err, dir = %dir.display(), "could not open the folder");
                        false
                    }
                };
                ctx.request_repaint();
                opened
            })?;
        self.running.push(thread);
        Ok(())
    }

    /// How many of the requests that finished since the last call failed. Requests still
    /// running are kept for a later call.
    pub(crate) fn failures(&mut self) -> usize {
        let mut failures = 0;
        let mut running = Vec::new();
        for thread in self.running.drain(..) {
            if !thread.is_finished() {
                running.push(thread);
                continue;
            }
            match thread.join() {
                Ok(true) => {}
                Ok(false) => failures += 1,
                Err(_) => {
                    tracing::error!("the thread opening a folder panicked");
                    failures += 1;
                }
            }
        }
        self.running = running;
        failures
    }
}

impl Default for FolderOpener {
    /// Opens folders with the system's default application.
    fn default() -> Self {
        Self::new(open_with_system)
    }
}

/// Opens `path` with the system's default application.
fn open_with_system(path: &Path) -> io::Result<()> {
    open::that(path)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// Collects failures until `want` of them have arrived, or two seconds have passed.
    fn failures_after(opener: &mut FolderOpener, want: usize) -> usize {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut failures = 0;
        while failures < want && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
            failures += opener.failures();
        }
        failures
    }

    #[test]
    fn reports_each_failure_once() {
        let mut opener = FolderOpener::new(|_| Err(io::Error::other("no file manager")));
        let ctx = Context::default();
        opener.open(&ctx, "a".into()).unwrap();
        opener.open(&ctx, "b".into()).unwrap();
        assert_eq!(failures_after(&mut opener, 2), 2);
        assert_eq!(opener.failures(), 0, "a failure was reported twice");
        assert!(opener.running.is_empty());
    }

    #[test]
    fn success_is_not_a_failure() {
        let mut opener = FolderOpener::new(|path| {
            assert_eq!(path, Path::new("logs"));
            Ok(())
        });
        opener.open(&Context::default(), "logs".into()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !opener.running.iter().all(JoinHandle::is_finished) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(opener.failures(), 0);
        assert!(opener.running.is_empty());
    }

    #[test]
    fn a_panicking_opener_counts_as_a_failure() {
        let mut opener = FolderOpener::new(|_| panic!("opener panics on purpose"));
        opener.open(&Context::default(), "x".into()).unwrap();
        assert_eq!(failures_after(&mut opener, 1), 1);
    }
}
