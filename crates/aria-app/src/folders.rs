//! Opens Aria's folders in the file manager, and asks the user for a folder, without making
//! the UI wait.
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
/// Name of the thread that shows the folder dialog.
const PICK_THREAD_NAME: &str = "aria-pick-folder";

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

/// Shows the system's folder dialog and returns the folder chosen, or `None` if the dialog
/// was cancelled or could not be shown.
pub(crate) type Pick = fn() -> Option<PathBuf>;

/// The folder dialog, shown on a thread of its own. At most one is open at a time.
pub(crate) struct FolderPicker {
    pick: Pick,
    /// The thread sends what the dialog returned.
    results: Receiver<Option<PathBuf>>,
    /// Cloned into the thread.
    report: Sender<Option<PathBuf>>,
    /// A dialog is open, or its answer has not been collected yet.
    open: bool,
}

impl FolderPicker {
    /// Picks folders with `pick`.
    pub(crate) fn new(pick: Pick) -> Self {
        let (report, results) = mpsc::channel();
        Self {
            pick,
            results,
            report,
            open: false,
        }
    }

    /// Opens the dialog, and repaints `ctx` once it has closed, for [`Self::picked`] to
    /// collect the answer. Does nothing while a dialog is already open.
    ///
    /// # Errors
    ///
    /// Fails if no thread could be started, in which case no dialog opens.
    pub(crate) fn start(&mut self, ctx: &Context) -> io::Result<()> {
        if self.open {
            return Ok(());
        }
        let pick = self.pick;
        let report = PickReport {
            results: self.report.clone(),
            ctx: ctx.clone(),
            picked: None,
        };
        thread::Builder::new()
            .name(PICK_THREAD_NAME.into())
            .spawn(move || {
                // Bound whole, so the guard drops on this thread, after the dialog.
                let mut report = report;
                report.picked = Some(pick());
            })?;
        self.open = true;
        Ok(())
    }

    /// The folder the user chose since the last call. A cancelled dialog gives nothing.
    pub(crate) fn picked(&mut self) -> Option<PathBuf> {
        let result = self.results.try_recv().ok()?;
        self.open = false;
        let folder = result?;
        tracing::debug!(folder = %folder.display(), "folder picked");
        Some(folder)
    }

    /// Whether a dialog is open or its answer is waiting to be collected.
    #[cfg(test)]
    fn is_open(&self) -> bool {
        self.open
    }
}

impl Default for FolderPicker {
    /// Picks folders with the system's dialog.
    fn default() -> Self {
        Self::new(pick_with_system)
    }
}

/// Reports the dialog's answer when its thread ends, then repaints the UI. As a drop guard it
/// also reports a thread that panicked, as a cancelled dialog.
struct PickReport {
    results: Sender<Option<PathBuf>>,
    ctx: Context,
    /// Set once the dialog has closed: what it returned.
    picked: Option<Option<PathBuf>>,
}

impl Drop for PickReport {
    fn drop(&mut self) {
        let picked = self.picked.take().unwrap_or_else(|| {
            tracing::error!("the thread showing the folder dialog panicked");
            None
        });
        // Fails only once the picker is gone, when nobody is waiting for the answer.
        let _ = self.results.send(picked);
        self.ctx.request_repaint();
    }
}

/// Shows the system's folder dialog, and waits for it on this thread.
fn pick_with_system() -> Option<PathBuf> {
    let dialog = rfd::AsyncFileDialog::new().pick_folder();
    pollster::block_on(dialog).map(|handle| handle.path().to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Barrier,
            atomic::{AtomicBool, AtomicUsize, Ordering},
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

    /// The answer of a picker, waiting up to two seconds for it.
    fn answer(picker: &mut FolderPicker) -> Option<PathBuf> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while picker.is_open() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
            if let Some(folder) = picker.picked() {
                return Some(folder);
            }
        }
        None
    }

    #[test]
    fn a_picked_folder_is_reported_once() {
        let mut picker = FolderPicker::new(|| Some(PathBuf::from("music")));
        picker.start(&Context::default()).unwrap();
        assert_eq!(answer(&mut picker), Some(PathBuf::from("music")));
        assert!(!picker.is_open());
        assert_eq!(picker.picked(), None, "the answer was reported twice");
    }

    #[test]
    fn a_cancelled_dialog_picks_nothing_and_frees_the_picker() {
        let mut picker = FolderPicker::new(|| None);
        picker.start(&Context::default()).unwrap();
        assert_eq!(answer(&mut picker), None);
        assert!(!picker.is_open(), "a cancelled dialog left the picker busy");
    }

    #[test]
    fn a_panicking_dialog_frees_the_picker() {
        let mut picker = FolderPicker::new(|| panic!("dialog panics on purpose"));
        picker.start(&Context::default()).unwrap();
        assert_eq!(answer(&mut picker), None);
        assert!(!picker.is_open());
    }

    #[test]
    fn only_one_dialog_is_open_at_a_time() {
        static SHOWN: AtomicUsize = AtomicUsize::new(0);
        static RELEASE: AtomicBool = AtomicBool::new(false);
        let mut picker = FolderPicker::new(|| {
            SHOWN.fetch_add(1, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !RELEASE.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            Some(PathBuf::from("music"))
        });
        let ctx = Context::default();
        picker.start(&ctx).unwrap();
        picker.start(&ctx).unwrap();
        picker.start(&ctx).unwrap();
        RELEASE.store(true, Ordering::SeqCst);
        assert_eq!(answer(&mut picker), Some(PathBuf::from("music")));
        assert_eq!(
            SHOWN.load(Ordering::SeqCst),
            1,
            "a second dialog was opened"
        );
        assert_eq!(picker.picked(), None, "a second answer was waiting");

        // Once the first is answered, the next request opens a dialog again.
        picker.start(&ctx).unwrap();
        assert_eq!(answer(&mut picker), Some(PathBuf::from("music")));
        assert_eq!(SHOWN.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_answer_is_ready_when_the_repaint_is_requested() {
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
        let mut picker = FolderPicker::new(|| Some(PathBuf::from("music")));
        picker.start(&ctx).unwrap();

        woken.wait();
        let folder = picker.picked();
        collected.wait();
        assert_eq!(
            folder,
            Some(PathBuf::from("music")),
            "the woken frame saw no answer"
        );
    }
}
