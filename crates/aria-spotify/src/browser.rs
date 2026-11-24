//! Finding and starting the browser that runs Aria's Spotify player page.
//!
//! Aria starts Edge or Chrome (Chromium on Linux) as a small app window with a profile of its
//! own, so the user's browser, tabs and cookies are never touched. With none found, the page
//! opens in a tab of the default browser instead.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// The systems Aria looks for a browser on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

/// The system Aria runs on.
pub fn current_os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::MacOs
    } else {
        Os::Linux
    }
}

/// Windows: (environment variable, path under it), in the order tried.
const WINDOWS: [(&str, &str); 5] = [
    (
        "ProgramFiles(x86)",
        r"Microsoft\Edge\Application\msedge.exe",
    ),
    ("ProgramFiles", r"Microsoft\Edge\Application\msedge.exe"),
    ("ProgramFiles", r"Google\Chrome\Application\chrome.exe"),
    ("ProgramFiles(x86)", r"Google\Chrome\Application\chrome.exe"),
    ("LOCALAPPDATA", r"Google\Chrome\Application\chrome.exe"),
];

/// macOS: the app bundles' executables, tried in `/Applications` and then `~/Applications`.
const MACOS: [&str; 3] = [
    "Google Chrome.app/Contents/MacOS/Google Chrome",
    "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "Chromium.app/Contents/MacOS/Chromium",
];

/// Linux: names looked for on `PATH`, each across the whole `PATH` before the next.
const LINUX: [&str; 6] = [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "microsoft-edge-stable",
];

/// The first Chromium browser found on `os`, reading the environment through `var` and the
/// disk through `exists`.
pub fn find_browser(
    os: Os,
    var: &dyn Fn(&str) -> Option<OsString>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = match os {
        Os::Windows => WINDOWS
            .iter()
            .filter_map(|(name, rel)| {
                let base = var(name)?;
                Some(PathBuf::from(format!("{}\\{rel}", base.to_string_lossy())))
            })
            .collect(),
        Os::MacOs => {
            let mut dirs = vec!["/Applications".to_owned()];
            if let Some(home) = var("HOME") {
                dirs.push(format!("{}/Applications", home.to_string_lossy()));
            }
            dirs.iter()
                .flat_map(|dir| {
                    MACOS
                        .iter()
                        .map(move |app| PathBuf::from(format!("{dir}/{app}")))
                })
                .collect()
        }
        Os::Linux => {
            let path = var("PATH").unwrap_or_default();
            let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
            LINUX
                .iter()
                .flat_map(|name| dirs.iter().map(move |dir| dir.join(name)))
                .collect()
        }
    };
    candidates.into_iter().find(|path| exists(path))
}

/// The arguments for Aria's player window: its own profile in `profile`, an app window on
/// `url`, sound allowed without a click, the media keys left to Aria, no slowing down while
/// minimized, and no "restore pages" prompt after a window Aria had to end.
pub fn chromium_args(profile: &Path, url: &str) -> Vec<OsString> {
    let mut profile_arg = OsString::from("--user-data-dir=");
    profile_arg.push(profile);
    let mut args = vec![profile_arg, OsString::from(format!("--app={url}"))];
    args.extend(
        [
            "--window-size=420,260",
            "--no-first-run",
            "--no-default-browser-check",
            "--autoplay-policy=no-user-gesture-required",
            "--disable-features=HardwareMediaKeyHandling",
            "--disable-background-timer-throttling",
            "--disable-renderer-backgrounding",
            "--disable-backgrounding-occluded-windows",
            "--hide-crash-restore-bubble",
        ]
        .into_iter()
        .map(OsString::from),
    );
    args
}

/// A started player window.
pub trait Running: Send {
    /// Whether the process Aria started has ended.
    fn exited(&mut self) -> bool;
    /// Ends the process Aria started.
    fn kill(&mut self);
}

/// How the page was opened.
pub enum Launched {
    /// In Aria's own browser window.
    Window(Box<dyn Running>),
    /// In a tab of the default browser: no Chromium browser was found.
    Tab,
}

/// Opens the player page at `url`, with the browser profile kept in `profile`.
pub trait Launcher: Send {
    fn launch(&mut self, url: &str, profile: &Path) -> io::Result<Launched>;
}

/// The real browsers on this system.
pub struct SystemLauncher;

impl Launcher for SystemLauncher {
    fn launch(&mut self, url: &str, profile: &Path) -> io::Result<Launched> {
        let found = find_browser(current_os(), &|name| std::env::var_os(name), &|path| {
            path.is_file()
        });
        let Some(browser) = found else {
            open::that(format!("{url}?tab=1"))?;
            return Ok(Launched::Tab);
        };
        let child = Command::new(browser)
            .args(chromium_args(profile, url))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Launched::Window(Box::new(ChildRunning(child))))
    }
}

struct ChildRunning(Child);

impl Running for ChildRunning {
    fn exited(&mut self) -> bool {
        // An error asking means the process can't be waited on: it is as good as gone.
        !matches!(self.0.try_wait(), Ok(None))
    }

    fn kill(&mut self) {
        // It may have ended on its own already; either way nothing more can be done.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn find(os: Os, vars: &[(&str, &str)], files: &[&str]) -> Option<PathBuf> {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect();
        let files: HashSet<PathBuf> = files.iter().map(PathBuf::from).collect();
        find_browser(os, &|name| vars.get(name).cloned(), &|path| {
            files.contains(path)
        })
    }

    const WIN: [(&str, &str); 3] = [
        ("ProgramFiles(x86)", r"C:\PF86"),
        ("ProgramFiles", r"C:\PF"),
        ("LOCALAPPDATA", r"C:\L"),
    ];

    #[test]
    fn windows_prefers_edge() {
        let edge = r"C:\PF86\Microsoft\Edge\Application\msedge.exe";
        let chrome = r"C:\PF\Google\Chrome\Application\chrome.exe";
        assert_eq!(
            find(Os::Windows, &WIN, &[chrome, edge]),
            Some(PathBuf::from(edge))
        );
        let edge64 = r"C:\PF\Microsoft\Edge\Application\msedge.exe";
        assert_eq!(
            find(Os::Windows, &WIN, &[chrome, edge64]),
            Some(PathBuf::from(edge64))
        );
    }

    #[test]
    fn windows_falls_back_to_chrome_in_local_app_data() {
        let local = r"C:\L\Google\Chrome\Application\chrome.exe";
        assert_eq!(
            find(Os::Windows, &WIN, &[local]),
            Some(PathBuf::from(local))
        );
        let x86 = r"C:\PF86\Google\Chrome\Application\chrome.exe";
        assert_eq!(
            find(Os::Windows, &WIN, &[local, x86]),
            Some(PathBuf::from(x86))
        );
    }

    #[test]
    fn macos_looks_in_applications_then_home() {
        let home = [("HOME", "/Users/u")];
        let chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
        let edge = "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge";
        assert_eq!(
            find(Os::MacOs, &home, &[edge, chrome]),
            Some(PathBuf::from(chrome))
        );
        let own = "/Users/u/Applications/Chromium.app/Contents/MacOS/Chromium";
        assert_eq!(find(Os::MacOs, &home, &[own]), Some(PathBuf::from(own)));
    }

    #[test]
    fn linux_searches_path_in_order() {
        let path = std::env::join_paths([Path::new("/a"), Path::new("/b")]).unwrap();
        let path = path.to_string_lossy().into_owned();
        let vars = [("PATH", path.as_str())];
        let at = |dir: &str, name: &str| Path::new(dir).join(name).to_string_lossy().into_owned();
        let (edge, chromium) = (at("/a", "microsoft-edge"), at("/b", "chromium"));
        let found = find(Os::Linux, &vars, &[&edge, &chromium]);
        assert_eq!(found, Some(PathBuf::from(&chromium)));
        let (chrome, stable) = (at("/b", "google-chrome"), at("/a", "google-chrome-stable"));
        let found = find(Os::Linux, &vars, &[&chrome, &stable]);
        assert_eq!(found, Some(PathBuf::from(&chrome)));
    }

    #[test]
    fn nothing_found_is_none() {
        assert_eq!(find(Os::Windows, &WIN, &[]), None);
        assert_eq!(find(Os::MacOs, &[], &[]), None);
        assert_eq!(find(Os::Linux, &[], &["/usr/bin/chromium"]), None);
    }

    #[test]
    fn the_arguments_are_exact() {
        let args = chromium_args(Path::new("/p"), "http://127.0.0.1:5/p/k/");
        let profile = format!("--user-data-dir={}", Path::new("/p").display());
        let expected: Vec<OsString> = [
            profile.as_str(),
            "--app=http://127.0.0.1:5/p/k/",
            "--window-size=420,260",
            "--no-first-run",
            "--no-default-browser-check",
            "--autoplay-policy=no-user-gesture-required",
            "--disable-features=HardwareMediaKeyHandling",
            "--disable-background-timer-throttling",
            "--disable-renderer-backgrounding",
            "--disable-backgrounding-occluded-windows",
            "--hide-crash-restore-bubble",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        assert_eq!(args, expected);
    }
}
