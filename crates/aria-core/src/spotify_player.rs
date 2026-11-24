//! Aria's own Spotify player: Spotify's Web Playback SDK in a browser window Aria opens.
//!
//! This holds the window's state for the Spotify thread: the page server, the browser process,
//! the device id the page reported, when the page was last heard from, and the play waiting for
//! it. The Spotify thread drives it from its loop and does the Web API calls itself. Aria still
//! decodes no Spotify audio: the browser plays it through Spotify's own player.

use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use aria_spotify::browser::{Launched, Launcher, Running, SystemLauncher};
use aria_spotify::player_page::{PageMsg, PageServer};
use aria_spotify::{PlayRequest, SpotifyError};
use crossbeam_channel::{Receiver, Sender, unbounded};

/// The device id kept in `spotify_device` for "Aria (this computer)". Spotify's id for the page
/// changes with every launch, so it is never saved.
pub const ARIA_DEVICE: &str = "aria";

/// How often a closing window is looked at.
const CLOSE_POLL: Duration = Duration::from_millis(20);

/// What the player needs from the outside, so tests can stand in for the browser.
pub struct PlayerDeps {
    pub launcher: Box<dyn Launcher>,
    /// Aria's own browser profile; `None` leaves the player out.
    pub profile_dir: Option<PathBuf>,
    /// How long to wait for the page's `ready`.
    pub ready: Duration,
    /// The same, the first time the profile is used (the browser fetches its Widevine part).
    pub first_ready: Duration,
    /// A ready page not heard from for this long is taken as gone.
    pub lost: Duration,
    /// How long a closing window has to end by itself before Aria ends it.
    pub close_grace: Duration,
}

impl PlayerDeps {
    /// The real browsers, with the profile kept in `profile_dir`.
    pub fn system(profile_dir: Option<PathBuf>) -> PlayerDeps {
        PlayerDeps {
            launcher: Box::new(SystemLauncher),
            profile_dir,
            ready: Duration::from_secs(20),
            first_ready: Duration::from_secs(90),
            lost: Duration::from_secs(15),
            close_grace: Duration::from_millis(700),
        }
    }

    /// No player at all: Spotify plays only on Spotify's own devices, as before 0.3.3.
    pub fn unavailable() -> PlayerDeps {
        PlayerDeps {
            launcher: Box::new(NoLauncher),
            profile_dir: None,
            ..PlayerDeps::system(None)
        }
    }
}

struct NoLauncher;

impl Launcher for NoLauncher {
    fn launch(&mut self, _url: &str, _profile: &Path) -> io::Result<Launched> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "no player"))
    }
}

/// What the UI is told about the player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AriaPlayerStatus {
    Off,
    /// The window is opening; the play waits for it.
    Starting,
    Ready {
        device_id: String,
    },
    /// The page has gone quiet; it may still come back.
    Lost,
    /// The session lacks the `streaming` permission: reconnect Spotify.
    NeedsReconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Off,
    Starting { until: Instant },
    Ready,
    Lost,
}

/// How a launch went.
pub(crate) enum Launch {
    Window,
    Tab,
    Failed(SpotifyError),
}

/// What a timer found.
pub(crate) enum Due {
    /// No `ready` came in time.
    TimedOut,
    /// The page went quiet; `exited` says whether the browser Aria started has ended.
    Lost { exited: bool },
}

pub(crate) struct Player {
    deps: PlayerDeps,
    on: bool,
    strings: String,
    state: State,
    server: Option<PageServer>,
    running: Option<Box<dyn Running>>,
    device: Option<String>,
    heard: Instant,
    pending: Option<PlayRequest>,
    to_service: Sender<PageMsg>,
    from_page: Receiver<PageMsg>,
}

impl Player {
    pub(crate) fn new(deps: PlayerDeps) -> Player {
        let (to_service, from_page) = unbounded();
        Player {
            deps,
            on: false,
            strings: "{}".to_owned(),
            state: State::Off,
            server: None,
            running: None,
            device: None,
            heard: Instant::now(),
            pending: None,
            to_service,
            from_page,
        }
    }

    /// What the page servers hand over, for the Spotify thread's `select!`.
    pub(crate) fn messages(&self) -> Receiver<PageMsg> {
        self.from_page.clone()
    }

    /// Turns the player on or off; the page's texts are `strings` from the next launch on.
    pub(crate) fn set(&mut self, on: bool, strings: String) {
        self.on = on;
        self.strings = strings;
    }

    /// Whether plays may go to the player.
    pub(crate) fn available(&self) -> bool {
        self.on && self.deps.profile_dir.is_some()
    }

    pub(crate) fn is_starting(&self) -> bool {
        matches!(self.state, State::Starting { .. })
    }

    /// The device id while the page is ready.
    pub(crate) fn ready_device(&self) -> Option<&str> {
        match self.state {
            State::Ready => self.device.as_deref(),
            _ => None,
        }
    }

    /// The device id of a page gone quiet.
    pub(crate) fn lost_device(&self) -> Option<&str> {
        match self.state {
            State::Lost => self.device.as_deref(),
            _ => None,
        }
    }

    /// The device to leave out of the list Spotify gives: the player's own.
    pub(crate) fn own_device(&self) -> Option<&str> {
        match self.state {
            State::Ready | State::Lost => self.device.as_deref(),
            State::Off | State::Starting { .. } => None,
        }
    }

    pub(crate) fn status(&self) -> AriaPlayerStatus {
        match (&self.state, &self.device) {
            (State::Off, _) => AriaPlayerStatus::Off,
            (State::Starting { .. }, _) => AriaPlayerStatus::Starting,
            (State::Ready, Some(id)) => AriaPlayerStatus::Ready {
                device_id: id.clone(),
            },
            (State::Ready, None) => AriaPlayerStatus::Starting,
            (State::Lost, _) => AriaPlayerStatus::Lost,
        }
    }

    /// Keeps `request` to play once the page is ready (a newer one replaces it).
    pub(crate) fn wait_with(&mut self, request: PlayRequest) {
        self.pending = Some(request);
    }

    /// Opens a new window, after ending any old one, and keeps `request` for its `ready`.
    pub(crate) fn launch(&mut self, request: PlayRequest) -> Launch {
        self.close();
        let Some(profile) = self.deps.profile_dir.clone() else {
            return Launch::Failed(SpotifyError::Network("no player".to_owned()));
        };
        let first = !profile.exists();
        let server = match PageServer::start(self.strings.clone(), self.to_service.clone()) {
            Ok(server) => server,
            Err(e) => return Launch::Failed(e),
        };
        let launched = self.deps.launcher.launch(&server.url(), &profile);
        let launch = match launched {
            Ok(Launched::Window(running)) => {
                self.running = Some(running);
                Launch::Window
            }
            Ok(Launched::Tab) => Launch::Tab,
            Err(e) => {
                tracing::warn!("couldn't open Aria's Spotify player: {}", e.kind());
                return Launch::Failed(SpotifyError::Network(e.kind().to_string()));
            }
        };
        let wait = if first {
            self.deps.first_ready
        } else {
            self.deps.ready
        };
        self.server = Some(server);
        self.state = State::Starting {
            until: Instant::now() + wait,
        };
        self.heard = Instant::now();
        self.pending = Some(request);
        launch
    }

    /// The page made a request: it is still there. A quiet page that speaks again is back.
    pub(crate) fn heard(&mut self) {
        self.heard = Instant::now();
        if self.state == State::Lost {
            self.state = State::Ready;
        }
    }

    /// The page is ready as `device_id`; gives the play that waited for it.
    pub(crate) fn ready(&mut self, device_id: String) -> Option<PlayRequest> {
        // A page of a window already closed has no say.
        self.server.as_ref()?;
        self.device = Some(device_id);
        self.state = State::Ready;
        self.pending.take()
    }

    /// A quiet page whose device Spotify still lists is taken as ready again.
    pub(crate) fn revive(&mut self) {
        if self.state == State::Lost {
            self.state = State::Ready;
            self.heard = Instant::now();
        }
    }

    /// When [`Player::due`] next has something to do.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        match self.state {
            State::Starting { until } => Some(until),
            State::Ready => Some(self.heard + self.deps.lost),
            State::Off | State::Lost => None,
        }
    }

    pub(crate) fn due(&mut self, now: Instant) -> Option<Due> {
        match self.state {
            State::Starting { until } if now >= until => Some(Due::TimedOut),
            State::Ready if now >= self.heard + self.deps.lost => {
                self.state = State::Lost;
                let exited = self.running.as_mut().is_some_and(|r| r.exited());
                Some(Due::Lost { exited })
            }
            _ => None,
        }
    }

    /// Closes the window: asks the page first, then ends the browser Aria started if it is
    /// still there after the grace, and stops the server.
    pub(crate) fn close(&mut self) {
        if let Some(server) = &self.server {
            server.close_page();
        }
        if let Some(mut running) = self.running.take() {
            let until = Instant::now() + self.deps.close_grace;
            while !running.exited() && Instant::now() < until {
                thread::sleep(CLOSE_POLL);
            }
            if !running.exited() {
                running.kill();
            }
        }
        if let Some(server) = self.server.take() {
            server.stop();
        }
        self.state = State::Off;
        self.device = None;
        self.pending = None;
    }
}
