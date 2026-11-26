//! Media keys and the system's media controls: Windows' System Media Transport Controls (the
//! volume pop-up and the lock screen), macOS's Now Playing, and MPRIS on Linux.
//!
//! [`action_for`] turns what the system asks into what Aria does, and [`MediaSync`] decides what
//! to tell the system, and when. Both run on the UI thread. The system behind them is a
//! [`MediaBackend`]: `souvlaki` in the app, a fake in tests.

use std::fmt::Write as _;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use eframe::egui;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};

/// How often the position is sent again while playing, so the system's timeline stays close.
pub const PERIOD: Duration = Duration::from_secs(5);
/// A position this far from where playback should be is a seek, sent at once.
pub const JUMP_MS: u64 = 2_000;
/// How far a seek button with no amount of its own jumps.
pub const SEEK_STEP_MS: u64 = 10_000;

/// What plays now, as the system is shown it.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    /// The track's identity: the Spotify URI, or the file's path.
    pub identity: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    /// A `file:///` or `https://` URL of the cover.
    pub cover_url: Option<String>,
    pub position_ms: u64,
    pub playing: bool,
}

/// What the system asks of Aria.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaEvent {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    /// Go to this position, in milliseconds.
    SetPosition(u64),
    /// Seek forward or back, by this much or by [`SEEK_STEP_MS`].
    Seek {
        forward: bool,
        by_ms: Option<u64>,
    },
    /// The volume, 0.0 to 1.0 (MPRIS only).
    SetVolume(f64),
    /// Bring the window forward (MPRIS).
    Raise,
    /// Anything Aria does not act on (quit, open a URI).
    Ignored,
}

/// What Aria does for a [`MediaEvent`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaAction {
    TogglePlay,
    Next,
    Previous,
    SeekTo(u64),
    SetVolume(f32),
    Raise,
}

/// What `event` asks of Aria, given what plays now.
pub fn action_for(event: MediaEvent, now: Option<&NowPlaying>) -> Option<MediaAction> {
    // A seek target within the track, when its length is known.
    let within = |now: &NowPlaying, ms: u64| now.duration_ms.map_or(ms, |end| ms.min(end));
    let playing = now.is_some_and(|now| now.playing);
    match event {
        // With nothing loaded, play starts what would play next, as the bar's button does.
        MediaEvent::Play => (!playing).then_some(MediaAction::TogglePlay),
        MediaEvent::Pause | MediaEvent::Stop => playing.then_some(MediaAction::TogglePlay),
        MediaEvent::Toggle => Some(MediaAction::TogglePlay),
        MediaEvent::Next => Some(MediaAction::Next),
        MediaEvent::Previous => Some(MediaAction::Previous),
        MediaEvent::SetPosition(ms) => now.map(|now| MediaAction::SeekTo(within(now, ms))),
        MediaEvent::Seek { forward, by_ms } => now.map(|now| {
            let by = by_ms.unwrap_or(SEEK_STEP_MS);
            let to = if forward {
                now.position_ms.saturating_add(by)
            } else {
                now.position_ms.saturating_sub(by)
            };
            MediaAction::SeekTo(within(now, to))
        }),
        MediaEvent::SetVolume(volume) => {
            let volume = if volume.is_nan() {
                0.0
            } else {
                volume.clamp(0.0, 1.0)
            };
            Some(MediaAction::SetVolume(volume as f32))
        }
        MediaEvent::Raise => Some(MediaAction::Raise),
        MediaEvent::Ignored => None,
    }
}

/// The system's media controls.
pub trait MediaBackend {
    /// What plays; `None` clears it.
    fn metadata(&mut self, meta: Option<&NowPlaying>);
    /// Playing (`Some(true)`), paused (`Some(false)`) or stopped (`None`), at this position.
    fn playback(&mut self, playing: Option<bool>, position_ms: u64);
}

/// Tells a [`MediaBackend`] what plays, sending only what changed.
pub struct MediaSync {
    backend: Box<dyn MediaBackend>,
    /// What was last sent: `None` before the first sync, `Some(None)` for nothing loaded.
    last: Option<Option<NowPlaying>>,
    /// When the position was last sent, and what it was.
    sent_at: Option<Instant>,
    sent_position_ms: u64,
}

impl MediaSync {
    pub fn new(backend: Box<dyn MediaBackend>) -> MediaSync {
        MediaSync {
            backend,
            last: None,
            sent_at: None,
            sent_position_ms: 0,
        }
    }

    /// Brings the system up to date with `now`, as of `at`.
    pub fn sync(&mut self, now: Option<&NowPlaying>, at: Instant) {
        let Some(now) = now else {
            if self.last != Some(None) {
                self.backend.metadata(None);
                self.backend.playback(None, 0);
                self.last = Some(None);
                self.sent_at = None;
            }
            return;
        };
        let before = self.last.clone().flatten();
        let new_track = before
            .as_ref()
            .is_none_or(|before| before.identity != now.identity);
        if new_track
            || before
                .as_ref()
                .is_some_and(|before| !same_metadata(before, now))
        {
            self.backend.metadata(Some(now));
        }
        let send = match (&before, self.sent_at) {
            (Some(before), Some(sent_at)) if !new_track => {
                let since = at.saturating_duration_since(sent_at);
                let expected = if before.playing {
                    let since_ms = u64::try_from(since.as_millis()).unwrap_or(u64::MAX);
                    self.sent_position_ms.saturating_add(since_ms)
                } else {
                    self.sent_position_ms
                };
                before.playing != now.playing
                    || now.position_ms.abs_diff(expected) > JUMP_MS
                    || (now.playing && since >= PERIOD)
            }
            _ => true,
        };
        if send {
            self.backend.playback(Some(now.playing), now.position_ms);
            self.sent_at = Some(at);
            self.sent_position_ms = now.position_ms;
        }
        self.last = Some(Some(now.clone()));
    }
}

/// Whether `a` and `b` show the same to the system, apart from where and whether they play.
fn same_metadata(a: &NowPlaying, b: &NowPlaying) -> bool {
    (&a.title, &a.artist, &a.album, a.duration_ms, &a.cover_url)
        == (&b.title, &b.artist, &b.album, b.duration_ms, &b.cover_url)
}

/// The URL a cover file is handed to the system as. On Windows souvlaki strips `file://` and
/// opens the rest as a path, so the path follows as it is; elsewhere it is a percent-encoded
/// `file://` URL.
pub fn cover_file_url(path: &std::path::Path) -> String {
    if cfg!(windows) {
        return format!("file://{}", path.display());
    }
    let mut url = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            let _ = write!(url, "%{byte:02X}");
        }
    }
    url
}

/// What souvlaki is given for `meta`. Text that is missing is sent empty: Windows keeps what it
/// was told before for anything left out, so a track without an album would show the last one's.
fn system_metadata(meta: Option<&NowPlaying>) -> MediaMetadata<'_> {
    MediaMetadata {
        title: Some(meta.map_or("", |meta| meta.title.as_str())),
        album: Some(meta.and_then(|meta| meta.album.as_deref()).unwrap_or("")),
        artist: Some(meta.map_or("", |meta| meta.artist.as_str())),
        cover_url: meta.and_then(|meta| meta.cover_url.as_deref()),
        duration: meta
            .and_then(|meta| meta.duration_ms)
            .map(Duration::from_millis),
    }
}

/// Makes the system's media controls for the main window. The HWND is the window's on Windows.
/// Events go to the sender, and each wakes the UI through the context.
pub type MediaFactory = Box<
    dyn Fn(
        Option<isize>,
        Sender<MediaEvent>,
        egui::Context,
    ) -> Result<Box<dyn MediaBackend>, String>,
>;

/// The real system's media controls, through `souvlaki`.
pub fn system_factory() -> MediaFactory {
    Box::new(|hwnd, events, ctx| {
        SouvlakiBackend::new(hwnd, events, ctx)
            .map(|backend| Box::new(backend) as Box<dyn MediaBackend>)
    })
}

struct SouvlakiBackend {
    controls: MediaControls,
}

impl SouvlakiBackend {
    fn new(
        hwnd: Option<isize>,
        events: Sender<MediaEvent>,
        ctx: egui::Context,
    ) -> Result<Self, String> {
        let config = PlatformConfig {
            display_name: "Aria",
            dbus_name: "aria",
            // The window's handle as the pointer souvlaki asks for; nothing is dereferenced here.
            hwnd: hwnd.map(|hwnd| std::ptr::with_exposed_provenance_mut(hwnd.cast_unsigned())),
        };
        let mut controls = MediaControls::new(config).map_err(|error| format!("{error:?}"))?;
        controls
            .attach(move |event| {
                // Fails only once the app has dropped the controls' receiver.
                let _ = events.send(from_system(&event));
                ctx.request_repaint();
            })
            .map_err(|error| format!("{error:?}"))?;
        Ok(SouvlakiBackend { controls })
    }
}

impl MediaBackend for SouvlakiBackend {
    fn metadata(&mut self, meta: Option<&NowPlaying>) {
        let metadata = system_metadata(meta);
        let with_cover = metadata.cover_url.is_some();
        let Err(error) = self.controls.set_metadata(metadata) else {
            return;
        };
        // A cover the system cannot load would cost the title and the seek bar too: again
        // without it.
        let result = if with_cover {
            let without = MediaMetadata {
                cover_url: None,
                ..system_metadata(meta)
            };
            self.controls.set_metadata(without)
        } else {
            Err(error)
        };
        if let Err(error) = result {
            tracing::debug!(?error, "the system did not take the track's details");
        }
    }

    fn playback(&mut self, playing: Option<bool>, position_ms: u64) {
        let progress = Some(MediaPosition(Duration::from_millis(position_ms)));
        let playback = match playing {
            None => MediaPlayback::Stopped,
            Some(false) => MediaPlayback::Paused { progress },
            Some(true) => MediaPlayback::Playing { progress },
        };
        if let Err(error) = self.controls.set_playback(playback) {
            tracing::debug!(?error, "the system did not take the playback state");
        }
    }
}

/// What a system event asks, in Aria's terms.
fn from_system(event: &MediaControlEvent) -> MediaEvent {
    let millis = |duration: &Duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    match event {
        MediaControlEvent::Play => MediaEvent::Play,
        MediaControlEvent::Pause => MediaEvent::Pause,
        MediaControlEvent::Toggle => MediaEvent::Toggle,
        MediaControlEvent::Stop => MediaEvent::Stop,
        MediaControlEvent::Next => MediaEvent::Next,
        MediaControlEvent::Previous => MediaEvent::Previous,
        MediaControlEvent::Seek(direction) => MediaEvent::Seek {
            forward: matches!(direction, SeekDirection::Forward),
            by_ms: None,
        },
        MediaControlEvent::SeekBy(direction, by) => MediaEvent::Seek {
            forward: matches!(direction, SeekDirection::Forward),
            by_ms: Some(millis(by)),
        },
        MediaControlEvent::SetPosition(MediaPosition(at)) => MediaEvent::SetPosition(millis(at)),
        MediaControlEvent::SetVolume(volume) => MediaEvent::SetVolume(*volume),
        MediaControlEvent::Raise => MediaEvent::Raise,
        // Shuffle, opening a URI, quitting, and whatever a later souvlaki adds.
        _ => MediaEvent::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Metadata(Option<NowPlaying>),
        Playback(Option<bool>, u64),
    }

    struct Fake(Rc<RefCell<Vec<Call>>>);

    impl MediaBackend for Fake {
        fn metadata(&mut self, meta: Option<&NowPlaying>) {
            self.0.borrow_mut().push(Call::Metadata(meta.cloned()));
        }
        fn playback(&mut self, playing: Option<bool>, position_ms: u64) {
            self.0
                .borrow_mut()
                .push(Call::Playback(playing, position_ms));
        }
    }

    fn sync() -> (MediaSync, Rc<RefCell<Vec<Call>>>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        (MediaSync::new(Box::new(Fake(Rc::clone(&calls)))), calls)
    }

    fn track(identity: &str, position_ms: u64, playing: bool) -> NowPlaying {
        NowPlaying {
            identity: identity.to_owned(),
            title: format!("{identity} title"),
            artist: "Ann".to_owned(),
            album: Some("LP".to_owned()),
            duration_ms: Some(60_000),
            cover_url: None,
            position_ms,
            playing,
        }
    }

    fn take(calls: &Rc<RefCell<Vec<Call>>>) -> Vec<Call> {
        std::mem::take(&mut *calls.borrow_mut())
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_new_track_sends_metadata_not_a_seek() {
        let (mut s, calls) = sync();
        let t0 = Instant::now();
        let a = track("a", 0, true);
        let b = track("b", 0, true);
        s.sync(Some(&a), t0);
        s.sync(Some(&b), t0 + ms(1_000));
        assert_eq!(
            take(&calls),
            [
                Call::Metadata(Some(a)),
                Call::Playback(Some(true), 0),
                Call::Metadata(Some(b)),
                Call::Playback(Some(true), 0),
            ]
        );
    }

    #[test]
    fn only_changes_are_sent() {
        let (mut s, calls) = sync();
        let t0 = Instant::now();
        s.sync(Some(&track("a", 1_000, true)), t0);
        take(&calls);
        s.sync(Some(&track("a", 1_500, true)), t0 + ms(500));
        assert_eq!(take(&calls), []);
    }

    #[test]
    fn a_jump_is_sent_at_once() {
        let (mut s, calls) = sync();
        let t0 = Instant::now();
        s.sync(Some(&track("a", 1_000, true)), t0);
        take(&calls);
        s.sync(Some(&track("a", 30_000, true)), t0 + ms(500));
        assert_eq!(take(&calls), [Call::Playback(Some(true), 30_000)]);
    }

    #[test]
    fn a_jump_back_while_paused_is_sent() {
        let (mut s, calls) = sync();
        let t0 = Instant::now();
        s.sync(Some(&track("a", 40_000, false)), t0);
        take(&calls);
        s.sync(Some(&track("a", 5_000, false)), t0 + ms(500));
        assert_eq!(take(&calls), [Call::Playback(Some(false), 5_000)]);
    }

    #[test]
    fn the_position_is_resent_every_five_seconds_while_playing() {
        let (mut s, calls) = sync();
        let t0 = Instant::now();
        s.sync(Some(&track("a", 0, true)), t0);
        take(&calls);
        s.sync(Some(&track("a", 4_900, true)), t0 + ms(4_900));
        assert_eq!(take(&calls), []);
        s.sync(Some(&track("a", 5_100, true)), t0 + ms(5_100));
        assert_eq!(take(&calls), [Call::Playback(Some(true), 5_100)]);

        let (mut s, calls) = sync();
        s.sync(Some(&track("a", 0, false)), t0);
        take(&calls);
        s.sync(Some(&track("a", 0, false)), t0 + ms(10_000));
        assert_eq!(take(&calls), []);
    }

    #[test]
    fn pausing_is_sent() {
        let (mut s, calls) = sync();
        let t0 = Instant::now();
        s.sync(Some(&track("a", 1_000, true)), t0);
        take(&calls);
        s.sync(Some(&track("a", 1_200, false)), t0 + ms(200));
        assert_eq!(take(&calls), [Call::Playback(Some(false), 1_200)]);
    }

    #[test]
    fn a_late_cover_resends_metadata() {
        let (mut s, calls) = sync();
        let t0 = Instant::now();
        s.sync(Some(&track("a", 0, false)), t0);
        take(&calls);
        let covered = NowPlaying {
            cover_url: Some("file:///c/a.jpg".to_owned()),
            ..track("a", 0, false)
        };
        s.sync(Some(&covered), t0 + ms(100));
        assert_eq!(take(&calls), [Call::Metadata(Some(covered))]);
    }

    #[test]
    fn nothing_loaded_shows_stopped() {
        let (mut s, calls) = sync();
        let t0 = Instant::now();
        s.sync(Some(&track("a", 0, true)), t0);
        take(&calls);
        s.sync(None, t0 + ms(100));
        assert_eq!(
            take(&calls),
            [Call::Metadata(None), Call::Playback(None, 0)]
        );
        s.sync(None, t0 + ms(200));
        assert_eq!(take(&calls), []);
        assert_eq!(
            action_for(MediaEvent::Play, None),
            Some(MediaAction::TogglePlay)
        );
        assert_eq!(
            action_for(MediaEvent::Toggle, None),
            Some(MediaAction::TogglePlay)
        );
        assert_eq!(action_for(MediaEvent::Pause, None), None);
        assert_eq!(
            action_for(
                MediaEvent::Seek {
                    forward: true,
                    by_ms: None
                },
                None
            ),
            None
        );
    }

    #[test]
    fn the_first_sync_with_nothing_shows_stopped() {
        let (mut s, calls) = sync();
        s.sync(None, Instant::now());
        assert_eq!(
            take(&calls),
            [Call::Metadata(None), Call::Playback(None, 0)]
        );
    }

    #[test]
    fn play_and_pause_follow_the_state() {
        let playing = track("a", 1_000, true);
        let paused = track("a", 1_000, false);
        assert_eq!(action_for(MediaEvent::Play, Some(&playing)), None);
        assert_eq!(action_for(MediaEvent::Pause, Some(&paused)), None);
        assert_eq!(action_for(MediaEvent::Stop, Some(&paused)), None);
        assert_eq!(
            action_for(MediaEvent::Play, Some(&paused)),
            Some(MediaAction::TogglePlay)
        );
        assert_eq!(
            action_for(MediaEvent::Pause, Some(&playing)),
            Some(MediaAction::TogglePlay)
        );
        assert_eq!(
            action_for(MediaEvent::Stop, Some(&playing)),
            Some(MediaAction::TogglePlay)
        );
        assert_eq!(
            action_for(MediaEvent::Toggle, Some(&playing)),
            Some(MediaAction::TogglePlay)
        );
        assert_eq!(
            action_for(MediaEvent::Next, Some(&playing)),
            Some(MediaAction::Next)
        );
        assert_eq!(
            action_for(MediaEvent::Previous, Some(&playing)),
            Some(MediaAction::Previous)
        );
        assert_eq!(
            action_for(MediaEvent::Raise, None),
            Some(MediaAction::Raise)
        );
        assert_eq!(action_for(MediaEvent::Ignored, Some(&playing)), None);
    }

    #[test]
    fn seeks_are_clamped() {
        let near_end = track("a", 55_000, true);
        assert_eq!(
            action_for(
                MediaEvent::Seek {
                    forward: true,
                    by_ms: None
                },
                Some(&near_end)
            ),
            Some(MediaAction::SeekTo(60_000))
        );
        let near_start = track("a", 3_000, true);
        assert_eq!(
            action_for(
                MediaEvent::Seek {
                    forward: false,
                    by_ms: Some(5_000)
                },
                Some(&near_start)
            ),
            Some(MediaAction::SeekTo(0))
        );
        assert_eq!(
            action_for(
                MediaEvent::Seek {
                    forward: true,
                    by_ms: None
                },
                Some(&near_start)
            ),
            Some(MediaAction::SeekTo(13_000))
        );
        assert_eq!(
            action_for(MediaEvent::SetPosition(70_000), Some(&near_start)),
            Some(MediaAction::SeekTo(60_000))
        );
        let unknown_length = NowPlaying {
            duration_ms: None,
            ..track("a", 3_000, true)
        };
        assert_eq!(
            action_for(MediaEvent::SetPosition(70_000), Some(&unknown_length)),
            Some(MediaAction::SeekTo(70_000))
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_cover_file_is_a_windows_path_after_file_scheme() {
        // souvlaki strips `file://` and opens the rest as a path.
        assert_eq!(
            cover_file_url(std::path::Path::new(r"C:\Users\A B\Ария\x.jpg")),
            r"file://C:\Users\A B\Ария\x.jpg"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn a_cover_file_is_a_percent_encoded_url() {
        assert_eq!(
            cover_file_url(std::path::Path::new("/home/a b/é.jpg")),
            "file:///home/a%20b/%C3%A9.jpg"
        );
    }

    #[test]
    fn missing_details_clear_the_previous_ones() {
        let bare = NowPlaying {
            album: None,
            cover_url: None,
            duration_ms: None,
            ..track("b", 0, true)
        };
        let sent = system_metadata(Some(&bare));
        assert_eq!(sent.album, Some(""));
        assert_eq!(sent.title, Some("b title"));
        let cleared = system_metadata(None);
        assert_eq!(
            (cleared.title, cleared.artist, cleared.album),
            (Some(""), Some(""), Some(""))
        );
        assert_eq!(cleared.cover_url, None);
        assert_eq!(cleared.duration, None);
    }

    #[test]
    fn system_events_are_read() {
        use std::time::Duration;
        assert_eq!(from_system(&MediaControlEvent::Toggle), MediaEvent::Toggle);
        assert_eq!(
            from_system(&MediaControlEvent::SeekBy(
                SeekDirection::Backward,
                Duration::from_secs(5)
            )),
            MediaEvent::Seek {
                forward: false,
                by_ms: Some(5_000)
            }
        );
        assert_eq!(
            from_system(&MediaControlEvent::Seek(SeekDirection::Forward)),
            MediaEvent::Seek {
                forward: true,
                by_ms: None
            }
        );
        assert_eq!(
            from_system(&MediaControlEvent::SetPosition(MediaPosition(
                Duration::from_millis(1_234)
            ))),
            MediaEvent::SetPosition(1_234)
        );
        assert_eq!(from_system(&MediaControlEvent::Quit), MediaEvent::Ignored);
        assert_eq!(
            from_system(&MediaControlEvent::OpenUri("x".to_owned())),
            MediaEvent::Ignored
        );
    }

    #[test]
    fn volume_is_clamped() {
        let a = track("a", 0, true);
        assert_eq!(
            action_for(MediaEvent::SetVolume(1.7), Some(&a)),
            Some(MediaAction::SetVolume(1.0))
        );
        assert_eq!(
            action_for(MediaEvent::SetVolume(-1.0), None),
            Some(MediaAction::SetVolume(0.0))
        );
        assert_eq!(
            action_for(MediaEvent::SetVolume(0.25), None),
            Some(MediaAction::SetVolume(0.25))
        );
    }
}
