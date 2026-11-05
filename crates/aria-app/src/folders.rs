//! Opens Aria's folders in the file manager without making the UI wait.
//!
//! `open::that` waits for the system's launcher. On Windows that is PowerShell, which takes
//! about 0.3 s to start, and on some Linux desktops the launcher returns only once the file
//! manager is closed. So each request runs on a short-lived thread. The thread reports its
//! outcome first and only then repaints the UI, so the frame that repaint brings already
//! sees the outcome.

use std::{
    io,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use eframe::egui::Context;

/// Name of the threads that open folders.
const THREAD_NAME: &str = "aria-open-folder";

/// Opens a path with the system's default application.
pub(crate) type Open = fn(&Path) -> io::Result<()>;

/// Requests to open a folder, until their outcome is collected.
pub(crate) struct FolderOpener {
    open: Open,
    /// Each thread sends whether its folder opened.
    outcomes: Receiver<bool>,
    /// Cloned into each thread.
    report: Sender<bool>,
    /// Requests whose outcome has not been collected yet.
    outstanding: usize,
}

impl FolderOpener {
    /// Opens folders with `open`.
    pub(crate) fn new(open: Open) -> Self {
        let (report, outcomes) = mpsc::channel();
        Self {
            open,
            outcomes,
            report,
            outstanding: 0,
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
        let report = Report {
            outcomes: self.report.clone(),
            ctx: ctx.clone(),
            opened: None,
        };
        thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                // Bound whole: the closure would otherwise capture only `report.opened`,
                // leaving the guard to drop on the calling thread.
                let mut report = report;
                let opened = match open(&dir) {
                    Ok(()) => true,
                    Err(err) => {
                        tracing::warn!(%err, dir = %dir.display(), "could not open the folder");
                        false
                    }
                };
                report.opened = Some(opened);
                // Dropping `report` sends the outcome, then repaints.
            })?;
        self.outstanding += 1;
        Ok(())
    }

    /// How many of the requests that finished since the last call failed. Requests still
    /// running are counted by a later call.
    pub(crate) fn failures(&mut self) -> usize {
        let mut failures = 0;
        for opened in self.outcomes.try_iter() {
            self.outstanding = self.outstanding.saturating_sub(1);
            if !opened {
                failures += 1;
            }
        }
        failures
    }

    /// Requests whose outcome [`Self::failures`] has not collected yet.
    #[cfg(test)]
    fn outstanding(&self) -> usize {
        self.outstanding
    }
}

impl Default for FolderOpener {
    /// Opens folders with the system's default application.
    fn default() -> Self {
        Self::new(open_with_system)
    }
}

/// Reports a request's outcome when its thread ends, then repaints the UI. As a drop guard
/// it also reports a thread that panicked, as a failure.
struct Report {
    outcomes: Sender<bool>,
    ctx: Context,
    /// Set once the folder was opened, or failed to.
    opened: Option<bool>,
}

impl Drop for Report {
    fn drop(&mut self) {
        let opened = self.opened.unwrap_or_else(|| {
            tracing::error!("the thread opening a folder panicked");
            false
        });
        // Fails only once the opener is gone, when nobody is waiting for the outcome.
        let _ = self.outcomes.send(opened);
        self.ctx.request_repaint();
    }
}

/// Opens `path` with the system's default application.
fn open_with_system(path: &Path) -> io::Result<()> {
    open::that(path)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Barrier,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };

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
        assert_eq!(opener.outstanding(), 0);
    }

    #[test]
    fn success_is_not_a_failure() {
        let mut opener = FolderOpener::new(|path| {
            assert_eq!(path, Path::new("logs"));
            Ok(())
        });
        opener.open(&Context::default(), "logs".into()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut failures = 0;
        while opener.outstanding() > 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
            failures += opener.failures();
        }
        assert_eq!(failures, 0);
        assert_eq!(opener.outstanding(), 0);
    }

    #[test]
    fn a_panicking_opener_counts_as_a_failure() {
        let mut opener = FolderOpener::new(|_| panic!("opener panics on purpose"));
        opener.open(&Context::default(), "x".into()).unwrap();
        assert_eq!(failures_after(&mut opener, 1), 1);
    }

    #[test]
    fn the_outcome_is_ready_when_the_repaint_is_requested() {
        // The repaint callback holds the worker inside `request_repaint` until the "frame"
        // below has collected, so the collection runs while the repaint is in progress.
        let woken = Arc::new(Barrier::new(2));
        let collected = Arc::new(Barrier::new(2));
        let first = Arc::new(AtomicBool::new(true));
        let ctx = Context::default();
        {
            let (woken, collected) = (Arc::clone(&woken), Arc::clone(&collected));
            ctx.set_request_repaint_callback(move |_| {
                if first.swap(false, Ordering::SeqCst) {
                    woken.wait();
                    collected.wait();
                }
            });
        }
        let mut opener = FolderOpener::new(|_| Err(io::Error::other("no file manager")));
        opener.open(&ctx, "x".into()).unwrap();

        woken.wait();
        let failures = opener.failures();
        collected.wait();
        assert_eq!(failures, 1, "the woken frame did not see the failure");
    }
}
