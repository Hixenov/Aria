//! The player bar along the bottom of the window: what is playing, the transport and seek
//! bar, and the volume, queue, device and lyrics controls.
//!
//! The bar draws what it is given and returns what the user asked for as [`PlayerAction`]s;
//! the app turns them into commands and settings. While the bar shows Spotify, the transport,
//! the seek bar, shuffle, repeat and the volume act on Spotify's device, and what Spotify cannot
//! do from here (the heart, mute, the sleep timer) is disabled. The devices button lists
//! Spotify's devices while signed in. The lyrics button stays disabled and says which phase
//! brings it.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use aria_core::{
    ClockReading, NowItem, OutputPath, RepeatState, SpDevice,
    chat::MicState,
    command::SleepTimer,
    i18n::{FluentArgs, Localizer},
    playback_service::{PlayerView, SleepState, TrackFormat},
    player::queue::RepeatMode,
    settings::Settings,
    spotify_service::{SpotifyPlayback, SpotifyRequest},
};
use aria_library::{ArtworkId, Collections, LibrarySnapshot, Quality, TrackId};
use eframe::egui::{
    Align, Align2, Label, Layout, Margin, Panel, Rect, RichText, Sense, TextStyle, Ui, UiBuilder,
    Vec2, pos2, vec2,
};

use super::surface_frame;
use crate::{
    artwork::{ArtworkTextures, paint_cover},
    spotify::RemoteCovers,
    ui::{
        pages::PageAction,
        theme::{
            Palette,
            fonts::{icon_font, semibold},
            icons,
            visuals::SPACE,
        },
        widgets::{
            IconButton, Scrub,
            track_menu::{self, HEART_BUTTON},
        },
    },
};

/// Panel id, unique among the root panels.
const ID: &str = "aria_player_bar";
/// Height of the bar.
pub const HEIGHT: f32 = 72.0;
/// Edge length of the artwork, and its placeholder icon size.
const ARTWORK: [f32; 2] = [48.0, 22.0];
/// Corner radius of the artwork.
const ARTWORK_RADIUS: u8 = 6;
/// Edge length of the play button, and its icon size.
const PLAY_BUTTON: [f32; 2] = [36.0, 18.0];
/// Edge length of the other buttons, and their icon size.
const BUTTON: [f32; 2] = [32.0, 18.0];
/// Edge length of the transport's side buttons, and their icon size.
const SMALL_BUTTON: [f32; 2] = [28.0, 16.0];
/// Share of the bar's width taken by each side; the transport is centered between them.
const SIDE_SHARE: f32 = 0.3;
/// The widest the transport and seek bar grow.
const CENTER_MAX_WIDTH: f32 = 560.0;
/// Height of the seek bar's row.
const SEEK_ROW: f32 = 16.0;
/// Width of the time texts beside the seek bar.
const TIME_WIDTH: f32 = 44.0;
/// Width of the volume bar.
const VOLUME_WIDTH: f32 = 88.0;
/// How far an arrow key moves the seek bar.
const SEEK_STEP_MS: u64 = 5_000;
/// How far an arrow key moves the volume bar.
const VOLUME_STEP: f32 = 0.05;
/// How close the clock must be to a seek's target for the seek to count as done.
const SEEK_SETTLE_MS: u64 = 750;
/// How long, in seconds, a seek is shown before the clock's position is trusted again.
const SEEK_SETTLE_TIMEOUT: f64 = 1.0;
/// How often the bar redraws while a seek settles.
const SETTLE_REPAINT: Duration = Duration::from_millis(50);
/// How often the bar redraws while playing, to move the position.
const REPAINT_EVERY: Duration = Duration::from_millis(250);

/// How long a Spotify volume just set is shown before the device's own is trusted again.
const VOLUME_SETTLE: Duration = Duration::from_secs(3);
/// Phase that brings lyrics.
const LYRICS_PHASE: u8 = 5;

/// What the user did on the bar. The app carries it out.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerAction {
    TogglePlay,
    Next,
    Previous,
    /// Jump to this position, in milliseconds, in the current track.
    Seek(u64),
    /// Set the volume, `0.0..=1.0`.
    SetVolume(f32),
    ToggleMute,
    ToggleShuffle,
    /// Go to the next repeat mode: off, all, one.
    CycleRepeat,
    ToggleQueue,
    /// Mark or unmark the current track as a favorite.
    SetFavorite {
        track: TrackId,
        favorite: bool,
    },
    /// Turn the microphone on or off.
    ToggleMicrophone,
    /// Set the sleep timer, or turn it off.
    SetSleepTimer(SleepTimer),
    /// A request for Spotify: its transport, seek, volume, shuffle and repeat, a cover, or the
    /// device list.
    Spotify(SpotifyRequest),
    /// Play Spotify on this device; `None` is Spotify's active one.
    SpotifyDevice(Option<String>),
    /// Open or close the full-screen record.
    ToggleFullScreen,
    /// Open or close the mini-player.
    ToggleMiniPlayer,
}

/// The sleep timer's lengths, in minutes, as its menu offers them.
const SLEEP_MINUTES: [u64; 5] = [15, 30, 45, 60, 90];

/// The track on the bar.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub artwork: Option<ArtworkId>,
}

impl NowPlaying {
    /// The track `view` names, as `snapshot` knows it. A track whose row is gone, or no
    /// snapshot at all, still gives the localized unknown title and artist, never a path.
    /// `None` when nothing is current.
    pub fn of(view: &PlayerView, snapshot: &LibrarySnapshot, tr: &Localizer) -> Option<Self> {
        Some(Self::track(view.current?, snapshot, tr))
    }

    /// Track `id` as `snapshot` knows it, with the localized unknown title and artist for what
    /// is missing; never a path. The queue panel's rows use it too.
    pub fn track(id: TrackId, snapshot: &LibrarySnapshot, tr: &Localizer) -> Self {
        let track = snapshot.track(id);
        let title = track
            .map(|track| track.title.trim())
            .filter(|title| !title.is_empty())
            .map_or_else(|| tr.tr("player-unknown-title").into_owned(), str::to_owned);
        let artist = track
            .and_then(|track| track.artist.as_deref())
            .map(str::trim)
            .filter(|artist| !artist.is_empty())
            .map_or_else(
                || tr.tr("library-unknown-artist").into_owned(),
                str::to_owned,
            );
        Self {
            title,
            artist,
            artwork: track.and_then(|track| track.artwork.clone()),
        }
    }
}

/// Where the current track is: the clock's position if the clock is on the track `view` names,
/// else 0. The clock lags a track change, and holds the old track's position until it catches up.
pub fn position_ms(reading: ClockReading, view: &PlayerView) -> u64 {
    if view.token == 0 || reading.token != view.token {
        return 0;
    }
    match view.duration_ms {
        0 => reading.position_ms,
        duration => reading.position_ms.min(duration),
    }
}

/// Where Spotify's track is now: its reported position plus the time since, while playing,
/// never past the track's end.
pub fn spotify_position_ms(playback: &SpotifyPlayback, now: Instant) -> u64 {
    let state = &playback.state;
    let elapsed = if state.playing {
        u64::try_from(now.saturating_duration_since(playback.at).as_millis()).unwrap_or(u64::MAX)
    } else {
        0
    };
    let position = state.progress_ms.saturating_add(elapsed);
    match spotify_duration(playback) {
        0 => position,
        duration => position.min(duration),
    }
}

/// The length of what Spotify plays, or 0 when it is not a track.
fn spotify_duration(playback: &SpotifyPlayback) -> u64 {
    match &playback.state.item {
        Some(NowItem::Track(track)) => track.duration_ms,
        _ => 0,
    }
}

/// Everything the bar draws.
pub struct PlayerBar<'a> {
    pub tr: &'a Localizer,
    pub palette: &'a Palette,
    pub view: &'a PlayerView,
    /// `None` while nothing is current.
    pub now: Option<&'a NowPlaying>,
    /// Where the current track is, from [`position_ms`].
    pub position_ms: u64,
    pub settings: &'a Settings,
    /// Marks the queue button as on.
    pub queue_open: bool,
    /// Marks the mini-player button as on.
    pub mini_open: bool,
    /// Where covers come from. Without it the placeholder shows.
    pub artwork: Option<&'a mut ArtworkTextures>,
    /// Which tracks are favorites, for the heart.
    pub collections: &'a Collections,
    /// What Spotify plays, while the bar shows Spotify.
    pub spotify: Option<&'a SpotifyPlayback>,
    /// Spotify's devices, while signed in to Spotify.
    pub spotify_devices: Option<&'a [SpDevice]>,
    /// Spotify's covers.
    pub covers: Option<&'a mut RemoteCovers>,
}

/// Shows the bar and returns what the user did on it this frame.
pub fn show(ui: &mut Ui, bar: &mut PlayerBar<'_>) -> Vec<PlayerAction> {
    let margin = Margin::symmetric(SPACE[3] as i8, SPACE[1] as i8);
    let mut actions = Vec::new();
    let playing = match bar.spotify {
        Some(playback) => playback.state.playing,
        None => bar.view.playing,
    };
    if playing {
        ui.ctx().request_repaint_after(REPAINT_EVERY);
    }
    Panel::bottom(ID)
        .exact_size(HEIGHT)
        .frame(surface_frame(bar.palette, margin))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let side = rect.width() * SIDE_SHARE;
            let left = Rect::from_min_max(rect.min, pos2(rect.left() + side, rect.bottom()));
            let right = Rect::from_min_max(pos2(rect.right() - side, rect.top()), rect.max);
            zone(ui, left, Layout::left_to_right(Align::Center), |ui| {
                now_playing(ui, bar, &mut actions);
            });
            center(ui, rect, bar, &mut actions);
            zone(ui, right, Layout::right_to_left(Align::Center), |ui| {
                extras(ui, bar, &mut actions);
            });
        });
    actions
}

/// Runs `add` in a child [`Ui`] confined to `rect`.
fn zone(ui: &mut Ui, rect: Rect, layout: Layout, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(layout), add);
}

/// The artwork, the title and artist, or "Nothing playing", and the heart.
fn now_playing(ui: &mut Ui, bar: &mut PlayerBar<'_>, out: &mut Vec<PlayerAction>) {
    if let Some(playback) = bar.spotify {
        return spotify_now_playing(ui, bar, playback, out);
    }
    let p = bar.palette;
    let [size, icon] = ARTWORK;
    let (art, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(art) {
        let painter = ui.painter();
        painter.rect_filled(art, ARTWORK_RADIUS, p.surface_raised);
        let cover = bar
            .now
            .and_then(|now| now.artwork.as_ref())
            .and_then(|id| bar.artwork.as_deref_mut()?.get(id));
        match cover {
            Some(texture) => paint_cover(painter, art, ARTWORK_RADIUS, texture),
            None => {
                let font = icon_font(icon);
                painter.text(
                    art.center(),
                    Align2::CENTER_CENTER,
                    icons::MUSIC_NOTES,
                    font,
                    p.text_muted,
                );
            }
        }
    }
    ui.add_space(SPACE[1]);
    let (Some(now), Some(track)) = (bar.now, bar.view.current) else {
        let text = RichText::new(bar.tr.tr("empty-nothing-playing")).color(p.text_muted);
        ui.add(Label::new(text).selectable(false).truncate());
        let label = bar.tr.tr(track_menu::favorite_key(false));
        let [size, icon] = HEART_BUTTON;
        let heart = IconButton::new(icons::HEART, &label, p).size(size, icon);
        ui.add_enabled(false, heart);
        return;
    };
    // The text gives way to the heart, which stays beside it.
    let text_width = ui.available_width() - HEART_BUTTON[0] - ui.spacing().item_spacing.x;
    ui.scope(|ui| {
        ui.set_max_width(text_width.max(0.0));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let body = TextStyle::Body.resolve(ui.style()).size;
            let title = RichText::new(&now.title).font(semibold(body)).color(p.text);
            ui.add(Label::new(title).selectable(false).truncate());
            let artist = RichText::new(&now.artist).small().color(p.text_muted);
            ui.add(Label::new(artist).selectable(false).truncate());
            output_line(ui, bar);
        });
    });
    if aria_core::opened::is_opened(track) {
        // Played from outside the library: no row to keep a heart on. The label says why.
        let label = bar.tr.tr("player-opened-no-heart");
        let [size, icon] = HEART_BUTTON;
        let heart = IconButton::new(icons::HEART, &label, p).size(size, icon);
        ui.add_enabled(false, heart).on_disabled_hover_text(&*label);
    } else if let Some(PageAction::SetFavorite { track, favorite }) =
        track_menu::heart(ui, bar.tr, p, bar.collections, track)
    {
        out.push(PlayerAction::SetFavorite { track, favorite });
    }
}

/// What Spotify plays: its cover, its name and artists ("Spotify" for an episode), the device
/// it plays on, and a heart that does nothing here.
fn spotify_now_playing(
    ui: &mut Ui,
    bar: &mut PlayerBar<'_>,
    playback: &SpotifyPlayback,
    out: &mut Vec<PlayerAction>,
) {
    let p = bar.palette;
    let [size, icon] = ARTWORK;
    let (title, artist, image) = match &playback.state.item {
        Some(NowItem::Track(track)) => {
            let artist = if track.artists.is_empty() {
                bar.tr.tr("library-unknown-artist").into_owned()
            } else {
                track.artists.join(", ")
            };
            (Some(track.name.clone()), artist, track.image.clone())
        }
        Some(NowItem::Other { name }) => (Some(name.clone()), "Spotify".to_owned(), None),
        None => (None, String::new(), None),
    };
    let (art, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(art) {
        let mut asked = Vec::new();
        let cover = match (&image, bar.covers.as_deref_mut()) {
            (Some(url), Some(covers)) => covers.get(url, &mut asked).cloned(),
            _ => None,
        };
        out.extend(asked.into_iter().filter_map(|action| match action {
            PageAction::Spotify(request) => Some(PlayerAction::Spotify(request)),
            _ => None,
        }));
        let painter = ui.painter();
        painter.rect_filled(art, ARTWORK_RADIUS, p.surface_raised);
        match cover {
            Some(texture) => paint_cover(painter, art, ARTWORK_RADIUS, &texture),
            None => {
                painter.text(
                    art.center(),
                    Align2::CENTER_CENTER,
                    icons::MUSIC_NOTES,
                    icon_font(icon),
                    p.text_muted,
                );
            }
        }
    }
    ui.add_space(SPACE[1]);
    let not_here = bar.tr.tr("player-spotify-not-here");
    let heart_label = bar.tr.tr(track_menu::favorite_key(false));
    let [heart_size, heart_icon] = HEART_BUTTON;
    let Some(title) = title else {
        let text = RichText::new(bar.tr.tr("empty-nothing-playing")).color(p.text_muted);
        ui.add(Label::new(text).selectable(false).truncate());
        let heart = IconButton::new(icons::HEART, &heart_label, p).size(heart_size, heart_icon);
        ui.add_enabled(false, heart)
            .on_disabled_hover_text(&*not_here);
        return;
    };
    let text_width = ui.available_width() - heart_size - ui.spacing().item_spacing.x;
    ui.scope(|ui| {
        ui.set_max_width(text_width.max(0.0));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let body = TextStyle::Body.resolve(ui.style()).size;
            let title = RichText::new(&title).font(semibold(body)).color(p.text);
            ui.add(Label::new(title).selectable(false).truncate());
            let artist = RichText::new(&artist).small().color(p.text_muted);
            ui.add(Label::new(artist).selectable(false).truncate());
            if let Some(device) = &playback.state.device {
                let mut args = FluentArgs::new();
                args.set("device", device.name.as_str());
                let line = bar.tr.tr_args("player-spotify-on", &args);
                let text = RichText::new(&*line).small().color(p.text_muted);
                ui.add(Label::new(text).selectable(false).truncate());
            }
        });
    });
    let heart = IconButton::new(icons::HEART, &heart_label, p).size(heart_size, heart_icon);
    ui.add_enabled(false, heart)
        .on_disabled_hover_text(&*not_here);
}

/// Between the format and the path on the bar's line.
pub const LINE_SEPARATOR: &str = " · ";

/// The file's format and how its sound reaches the device, as small text under the artist. The
/// path has a tooltip saying what it means. Nothing is shown of a path the engine has not
/// reported: while exclusive mode is on and no state has arrived, the format stands alone, and
/// the system mixer is named only when exclusive mode is off.
fn output_line(ui: &mut Ui, bar: &PlayerBar<'_>) {
    let p = bar.palette;
    let format = bar
        .view
        .format
        .as_ref()
        .map(format_label)
        .filter(|text| !text.is_empty());
    let path = path_key(bar.view);
    if format.is_none() && path.is_none() {
        return;
    }
    // One line that gives way to the heart with an ellipsis; the tooltip has all of it.
    let name = path.map(|key| bar.tr.tr(key));
    let parts: Vec<&str> = format
        .as_deref()
        .into_iter()
        .chain(name.as_deref())
        .collect();
    let line = parts.join(LINE_SEPARATOR);
    let text = RichText::new(&line).small().color(p.text_muted);
    let label = Label::new(text)
        .selectable(false)
        .truncate()
        .sense(Sense::hover());
    ui.add(label).on_hover_ui(|ui| {
        ui.label(&line);
        if let Some(key) = path {
            ui.label(RichText::new(bar.tr.tr(&format!("{key}-hint"))).color(p.text_muted));
        }
    });
}

/// The locale key naming how the current track reaches the device. `None` with nothing
/// current, and while an exclusive output is being opened (the volume is locked) but the engine
/// has not said how it turned out.
fn path_key(view: &PlayerView) -> Option<&'static str> {
    view.current?;
    match view.output {
        Some(state) => Some(match state.path {
            OutputPath::BitPerfect => "output-bit-perfect",
            OutputPath::Resampled => "output-resampled",
            OutputPath::Converted => "output-converted",
            OutputPath::MixedWithSound => "output-mixed",
            OutputPath::SystemMixer => "output-system-mixer",
        }),
        None if view.volume_locked => None,
        None => Some("output-system-mixer"),
    }
}

/// A file's format as the bar shows it: the codec, the depth for lossless files only (a lossy
/// file's tags can report one that means nothing), and the rate in kHz. "FLAC 24/96", "MP3 44.1".
pub fn format_label(format: &TrackFormat) -> String {
    let codec = match &*format.codec {
        "unknown" => String::new(),
        codec => codec.to_uppercase(),
    };
    if format.sample_rate == 0 {
        // A file whose rate could not be read: no "0/0", only what is known.
        return codec;
    }
    let (whole, part) = (format.sample_rate / 1_000, format.sample_rate % 1_000);
    let rate = if part == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{part:03}")
            .trim_end_matches('0')
            .to_owned()
    };
    let rate = match format.quality {
        Quality::Lossless | Quality::HiRes if format.bits_per_sample > 0 => {
            format!("{}/{rate}", format.bits_per_sample)
        }
        _ => rate,
    };
    if codec.is_empty() {
        rate
    } else {
        format!("{codec} {rate}")
    }
}

/// Shuffle, previous, play, next and repeat, with the seek bar under them, centered in `bar`.
pub(crate) fn center(
    ui: &mut Ui,
    area: Rect,
    bar: &mut PlayerBar<'_>,
    out: &mut Vec<PlayerAction>,
) {
    let width = (area.width() * (1.0 - 2.0 * SIDE_SHARE)).min(CENTER_MAX_WIDTH);
    let height = PLAY_BUTTON[0] + SEEK_ROW;
    let column = Rect::from_center_size(area.center(), vec2(width, height));
    controls(ui, column, bar, out, true);
}

/// The transport at the top of `column` and the seek bar at its bottom. Without `sides`,
/// shuffle and repeat are left out.
pub(crate) fn controls(
    ui: &mut Ui,
    column: Rect,
    bar: &mut PlayerBar<'_>,
    out: &mut Vec<PlayerAction>,
    sides: bool,
) {
    let width = column.width();
    let buttons = Rect::from_min_size(column.min, vec2(width, PLAY_BUTTON[0]));
    let seek = Rect::from_min_size(
        pos2(column.left(), column.bottom() - SEEK_ROW),
        vec2(width, SEEK_ROW),
    );
    zone(ui, buttons, Layout::left_to_right(Align::Center), |ui| {
        transport(ui, bar, out, sides);
    });
    zone(
        ui,
        seek,
        Layout::left_to_right(Align::Center),
        |ui| match bar.spotify {
            Some(playback) => {
                let duration = spotify_duration(playback);
                let position = spotify_position_ms(playback, Instant::now());
                let seekable = playback.state.item.is_some() && duration > 0;
                let token = item_token(playback);
                if let Some(target) =
                    seek_scrub(ui, bar.tr, bar.palette, token, position, duration, seekable)
                {
                    out.push(PlayerAction::Spotify(SpotifyRequest::Seek(target)));
                }
            }
            None => seek_row(ui, bar, out),
        },
    );
}

/// A number that changes when Spotify moves to another item, for the seek bar's memory.
fn item_token(playback: &SpotifyPlayback) -> u64 {
    let mut hasher = DefaultHasher::new();
    match &playback.state.item {
        Some(NowItem::Track(track)) => (&track.uri, &track.name).hash(&mut hasher),
        Some(NowItem::Other { name }) => name.hash(&mut hasher),
        None => 0u8.hash(&mut hasher),
    }
    hasher.finish().max(1)
}

/// What the transport shows and does, from the local player or from Spotify.
struct Transport {
    loaded: bool,
    playing: bool,
    shuffle: bool,
    /// The icon and the key naming the repeat mode, and whether it is on.
    repeat: (&'static str, &'static str, bool),
}

impl Transport {
    fn of(bar: &PlayerBar<'_>) -> Transport {
        match bar.spotify {
            Some(playback) => {
                let state = &playback.state;
                Transport {
                    loaded: state.item.is_some(),
                    playing: state.playing,
                    shuffle: state.shuffle,
                    repeat: match state.repeat {
                        RepeatState::Off => (icons::REPEAT, "player-repeat-off", false),
                        RepeatState::Context => (icons::REPEAT, "player-repeat-all", true),
                        RepeatState::Track => (icons::REPEAT_ONCE, "player-repeat-one", true),
                    },
                }
            }
            None => Transport {
                loaded: bar.view.current.is_some(),
                playing: bar.view.playing,
                shuffle: bar.settings.shuffle,
                repeat: match bar.settings.repeat {
                    RepeatMode::Off => (icons::REPEAT, "player-repeat-off", false),
                    RepeatMode::All => (icons::REPEAT, "player-repeat-all", true),
                    RepeatMode::One => (icons::REPEAT_ONCE, "player-repeat-one", true),
                },
            },
        }
    }
}

/// The action for a transport button: the local one, or Spotify's request.
pub(crate) fn transport_action(bar: &PlayerBar<'_>, local: PlayerAction) -> PlayerAction {
    let Some(playback) = bar.spotify else {
        return local;
    };
    let state = &playback.state;
    let request = match local {
        PlayerAction::TogglePlay => SpotifyRequest::TogglePlay,
        PlayerAction::Next => SpotifyRequest::Next,
        PlayerAction::Previous => SpotifyRequest::Previous,
        PlayerAction::ToggleShuffle => SpotifyRequest::SetShuffle(!state.shuffle),
        PlayerAction::CycleRepeat => SpotifyRequest::SetRepeat(match state.repeat {
            RepeatState::Off => RepeatState::Context,
            RepeatState::Context => RepeatState::Track,
            RepeatState::Track => RepeatState::Off,
        }),
        other => return other,
    };
    PlayerAction::Spotify(request)
}

/// The transport's buttons, centered in the row they are in.
fn transport(ui: &mut Ui, bar: &PlayerBar<'_>, out: &mut Vec<PlayerAction>, sides: bool) {
    let (tr, p) = (bar.tr, bar.palette);
    let shown = Transport::of(bar);
    let loaded = shown.loaded;
    let gap = SPACE[1];
    let [side, side_icon] = SMALL_BUTTON;
    let [button, icon] = BUTTON;
    let [play, play_icon] = PLAY_BUTTON;
    let width = if sides {
        side * 2.0 + button * 2.0 + play + gap * 4.0
    } else {
        button * 2.0 + play + gap * 2.0
    };
    ui.spacing_mut().item_spacing.x = gap;
    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));

    if sides {
        let shuffle = tr.tr("player-shuffle");
        let control = IconButton::new(icons::SHUFFLE, &shuffle, p)
            .size(side, side_icon)
            .selected(shown.shuffle);
        if ui.add(control).clicked() {
            out.push(transport_action(bar, PlayerAction::ToggleShuffle));
        }
    }

    let previous = tr.tr("player-previous");
    let control = IconButton::new(icons::SKIP_BACK, &previous, p).size(button, icon);
    if ui.add_enabled(loaded, control).clicked() {
        out.push(transport_action(bar, PlayerAction::Previous));
    }

    let (glyph, key) = if shown.playing {
        (icons::PAUSE, "player-pause")
    } else {
        (icons::PLAY, "player-play")
    };
    let name = tr.tr(key);
    let control = IconButton::new(glyph, &name, p)
        .size(play, play_icon)
        .prominent();
    if ui.add_enabled(loaded, control).clicked() {
        out.push(transport_action(bar, PlayerAction::TogglePlay));
    }

    let next = tr.tr("player-next");
    let control = IconButton::new(icons::SKIP_FORWARD, &next, p).size(button, icon);
    if ui.add_enabled(loaded, control).clicked() {
        out.push(transport_action(bar, PlayerAction::Next));
    }

    if sides {
        let (glyph, key, on) = shown.repeat;
        let name = tr.tr(key);
        let control = IconButton::new(glyph, &name, p)
            .size(side, side_icon)
            .selected(on);
        if ui.add(control).clicked() {
            out.push(transport_action(bar, PlayerAction::CycleRepeat));
        }
    }
}

/// The elapsed time, the seek bar and the length.
fn seek_row(ui: &mut Ui, bar: &PlayerBar<'_>, out: &mut Vec<PlayerAction>) {
    let duration = bar.view.duration_ms;
    let seekable = bar.view.current.is_some() && duration > 0;
    let (tr, p, token, position) = (bar.tr, bar.palette, bar.view.token, bar.position_ms);
    if let Some(target) = seek_scrub(ui, tr, p, token, position, duration, seekable) {
        out.push(PlayerAction::Seek(target));
    }
}

/// The seek bar with its times; returns the position the user seeked to.
fn seek_scrub(
    ui: &mut Ui,
    tr: &Localizer,
    p: &Palette,
    token: u64,
    position_ms: u64,
    duration: u64,
    seekable: bool,
) -> Option<u64> {
    let mut sought = None;
    let row = ui.max_rect();
    let gap = SPACE[1];
    let elapsed = Rect::from_min_size(row.min, vec2(TIME_WIDTH, row.height()));
    let total = Rect::from_min_size(
        pos2(row.right() - TIME_WIDTH, row.top()),
        vec2(TIME_WIDTH, row.height()),
    );
    let track = Rect::from_min_max(
        pos2(elapsed.right() + gap, row.top()),
        pos2(total.left() - gap, row.bottom()),
    );

    // A seek just made is shown, and keyboard steps start from it, until the clock has caught
    // up: the engine answers a few frames later, and the bar must not jump back meanwhile.
    let memory = ui.make_persistent_id("aria_seek_pending");
    let now = ui.input(|input| input.time);
    let pending = ui
        .data(|data| data.get_temp::<PendingSeek>(memory))
        .filter(|pending| !pending.settled(token, position_ms, now));
    let base_ms = pending.map_or(position_ms, |pending| pending.target_ms);
    let fraction = if duration > 0 {
        (base_ms as f64 / duration as f64).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let label = tr.tr("player-seek");
    let step = if duration > 0 {
        (SEEK_STEP_MS as f64 / duration as f64) as f32
    } else {
        0.0
    };
    let scrub = Scrub {
        id: "aria_seek",
        rect: track,
        value: fraction,
        enabled: seekable,
        label: &label,
        step,
        full: 1.0,
    };
    let moved = scrub.show(ui, p);
    // While dragging, the bar and the time show where the pointer is.
    let shown_ms = match moved.dragging {
        Some(at) if seekable => at_ms(at, duration),
        _ => base_ms,
    };
    let mut pending = pending;
    if let (Some(at), true) = (moved.committed, seekable) {
        let target_ms = at_ms(at, duration);
        sought = Some(target_ms);
        pending = Some(PendingSeek {
            token,
            from_ms: base_ms,
            target_ms,
            since: now,
        });
    }
    if pending.is_some() {
        // Even paused, nothing else would draw the clock's answer.
        ui.ctx().request_repaint_after(SETTLE_REPAINT);
    }
    ui.data_mut(|data| {
        if let Some(pending) = pending {
            data.insert_temp(memory, pending);
        } else {
            data.remove::<PendingSeek>(memory);
        }
    });

    let elapsed_text = format_time(shown_ms);
    let total_text = if duration > 0 {
        format_time(duration)
    } else {
        UNKNOWN_TIME.to_owned()
    };
    zone(ui, elapsed, Layout::right_to_left(Align::Center), |ui| {
        ui.add(time_label(&elapsed_text, p));
    });
    zone(ui, total, Layout::left_to_right(Align::Center), |ui| {
        ui.add(time_label(&total_text, p));
    });
    sought
}

/// A seek the user made, which the clock has not reported yet.
#[derive(Clone, Copy)]
struct PendingSeek {
    /// The track it was made in.
    token: u64,
    /// Where the track was when it was made.
    from_ms: u64,
    target_ms: u64,
    /// When it was made, in egui's time.
    since: f64,
}

impl PendingSeek {
    /// Whether it is over: the track changed, the clock got to the target (from the side it
    /// started on), or it took too long.
    fn settled(&self, token: u64, position_ms: u64, now: f64) -> bool {
        let reached = if self.target_ms >= self.from_ms {
            // Forward: at the target, or already past it as playback goes on.
            position_ms + SEEK_SETTLE_MS >= self.target_ms
        } else {
            position_ms <= self.target_ms + SEEK_SETTLE_MS
        };
        self.token != token || now - self.since > SEEK_SETTLE_TIMEOUT || reached
    }
}

fn time_label(text: &str, p: &Palette) -> Label {
    Label::new(RichText::new(text).small().color(p.text_muted)).selectable(false)
}

/// What a time shows when the length is not known.
const UNKNOWN_TIME: &str = "--:--";

/// `m:ss`, or `h:mm:ss` from one hour on.
fn format_time(ms: u64) -> String {
    let secs = ms / 1_000;
    let (h, m, s) = (secs / 3_600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The position `fraction` of the way through a track of `duration` ms.
fn at_ms(fraction: f32, duration: u64) -> u64 {
    let ms = f64::from(fraction.clamp(0.0, 1.0)) * duration as f64;
    (ms.round() as u64).min(duration)
}

/// Mute, volume, devices, queue and lyrics, laid out from the right edge.
fn extras(ui: &mut Ui, bar: &PlayerBar<'_>, out: &mut Vec<PlayerAction>) {
    let (tr, p) = (bar.tr, bar.palette);
    ui.spacing_mut().item_spacing.x = SPACE[0];
    let [button, icon] = BUTTON;

    let full = tr.tr("player-full-screen");
    let control = IconButton::new(icons::CORNERS_OUT, &full, p).size(button, icon);
    if ui.add(control).clicked() {
        out.push(PlayerAction::ToggleFullScreen);
    }

    let mini = tr.tr("player-mini-player");
    let control = IconButton::new(icons::PICTURE_IN_PICTURE, &mini, p)
        .size(button, icon)
        .selected(bar.mini_open);
    if ui.add(control).clicked() {
        out.push(PlayerAction::ToggleMiniPlayer);
    }

    let lyrics = tr.tr("player-lyrics");
    let control = IconButton::new(icons::QUOTES, &lyrics, p).size(button, icon);
    pending(ui, control, &lyrics, LYRICS_PHASE, tr, p);

    if bar.spotify.is_some() {
        let name = tr.tr("player-sleep");
        let control = IconButton::new(icons::MOON, &name, p).size(button, icon);
        not_for_spotify(ui, control, &name, tr, p);
    } else {
        sleep_timer(ui, bar, out);
    }

    let queue = tr.tr("player-queue");
    let control = IconButton::new(icons::QUEUE, &queue, p)
        .size(button, icon)
        .selected(bar.queue_open);
    if ui.add(control).clicked() {
        out.push(PlayerAction::ToggleQueue);
    }

    devices_button(ui, bar, out);

    microphone(ui, bar, out);

    volume_controls(ui, bar, out);
}

/// The volume bar and mute (Spotify's volume while the bar shows Spotify), laid out from the
/// right in a right-to-left `ui`.
pub(crate) fn volume_controls(ui: &mut Ui, bar: &PlayerBar<'_>, out: &mut Vec<PlayerAction>) {
    let (tr, p) = (bar.tr, bar.palette);
    let [button, icon] = BUTTON;
    if let Some(playback) = bar.spotify {
        return spotify_volume(ui, bar, playback, out);
    }

    let (rect, _) = ui.allocate_exact_size(vec2(VOLUME_WIDTH, SEEK_ROW), Sense::hover());
    let label = tr.tr("player-volume");
    let locked = bar.view.volume_locked;
    let volume = shown_volume(bar.view, bar.settings.volume);
    let scrub = Scrub {
        id: "aria_volume",
        rect,
        value: volume,
        enabled: !locked,
        label: &label,
        step: VOLUME_STEP,
        full: 1.0,
    };
    let moved = scrub.show(ui, p);
    // Every move is a change to apply; the app skips the ones that change nothing.
    if let Some(at) = moved.dragging.or(moved.committed) {
        out.push(PlayerAction::SetVolume(at));
    }
    if locked {
        ui.interact(
            rect,
            ui.scope_id().with("aria_volume_locked"),
            Sense::hover(),
        )
        .on_hover_text(tr.tr("player-volume-locked"));
    }

    let muted = bar.settings.muted;
    let (glyph, key) = if muted {
        (icons::SPEAKER_X, "player-unmute")
    } else if volume < 0.5 {
        (icons::SPEAKER_LOW, "player-mute")
    } else {
        (icons::SPEAKER_HIGH, "player-mute")
    };
    let name = tr.tr(key);
    let control = IconButton::new(glyph, &name, p)
        .size(button, icon)
        .selected(muted);
    if ui.add(control).clicked() {
        out.push(PlayerAction::ToggleMute);
    }
}

/// Spotify's volume, the device's own, sent when the drag ends; and mute, which Spotify does
/// not have here.
fn spotify_volume(
    ui: &mut Ui,
    bar: &PlayerBar<'_>,
    playback: &SpotifyPlayback,
    out: &mut Vec<PlayerAction>,
) {
    let (tr, p) = (bar.tr, bar.palette);
    let [button, icon] = BUTTON;
    let device = playback.state.device.as_ref();
    let supported = device.is_some_and(|d| d.supports_volume && d.volume.is_some());
    let reported = device
        .and_then(|d| d.volume)
        .map_or(0.0, |v| f32::from(v.min(100)) / 100.0);
    // What was just set or is being dragged is shown until the device reports it, or a while.
    let memory = ui.make_persistent_id("aria_spotify_volume");
    let now = ui.input(|input| input.time);
    let held = ui
        .data(|data| data.get_temp::<(f32, f64)>(memory))
        .filter(|(_, since)| now - since < VOLUME_SETTLE.as_secs_f64());
    let shown = held.map_or(reported, |(value, _)| value);
    let (rect, _) = ui.allocate_exact_size(vec2(VOLUME_WIDTH, SEEK_ROW), Sense::hover());
    let label = tr.tr("player-volume");
    let scrub = Scrub {
        id: "aria_volume",
        rect,
        value: shown,
        enabled: supported,
        label: &label,
        step: VOLUME_STEP,
        full: 1.0,
    };
    let moved = scrub.show(ui, p);
    if let Some(at) = moved.dragging {
        ui.data_mut(|data| data.insert_temp(memory, (at, now)));
    }
    if let Some(at) = moved.committed {
        ui.data_mut(|data| data.insert_temp(memory, (at, now)));
        let percent = (at.clamp(0.0, 1.0) * 100.0).round() as u8;
        out.push(PlayerAction::Spotify(SpotifyRequest::SetVolume(percent)));
    }
    if held.is_some_and(|(value, _)| (value - reported).abs() < 0.015) && moved.dragging.is_none() {
        ui.data_mut(|data| data.remove::<(f32, f64)>(memory));
    }
    let glyph = if shown < 0.5 {
        icons::SPEAKER_LOW
    } else {
        icons::SPEAKER_HIGH
    };
    let name = tr.tr("player-mute");
    let control = IconButton::new(glyph, &name, p).size(button, icon);
    not_for_spotify(ui, control, &name, tr, p);
}

/// Adds `control` disabled, saying it does nothing for Spotify's playback.
fn not_for_spotify(ui: &mut Ui, control: IconButton<'_>, name: &str, tr: &Localizer, p: &Palette) {
    let hint = tr.tr("player-spotify-not-here");
    let response = ui.add_enabled(false, control);
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_description(&*hint));
    response.on_disabled_hover_ui(|ui| {
        ui.label(RichText::new(name).color(p.text));
        ui.label(RichText::new(&*hint).color(p.text_muted));
    });
}

/// The devices button: Spotify's devices while signed in, to choose the one to play on.
/// Signed out, it is disabled and says where to connect.
fn devices_button(ui: &mut Ui, bar: &PlayerBar<'_>, out: &mut Vec<PlayerAction>) {
    let (tr, p) = (bar.tr, bar.palette);
    let [button, icon] = BUTTON;
    let name = tr.tr("player-devices");
    let control = IconButton::new(icons::DEVICES, &name, p).size(button, icon);
    let Some(devices) = bar.spotify_devices else {
        let hint = tr.tr("player-devices-connect");
        let response = ui.add_enabled(false, control);
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_description(&*hint));
        response.on_disabled_hover_ui(|ui| {
            ui.label(RichText::new(&*name).color(p.text));
            ui.label(RichText::new(&*hint).color(p.text_muted));
        });
        return;
    };
    let response = ui.add(control);
    if response.clicked() {
        out.push(PlayerAction::Spotify(SpotifyRequest::ListDevices));
    }
    let chosen = bar.settings.spotify_device.as_deref();
    let picked = eframe::egui::Popup::menu(&response).show(|ui| {
        let mut picked = None;
        let active = tr.tr("settings-spotify-active-device");
        if ui.selectable_label(chosen.is_none(), &*active).clicked() {
            picked = Some(None);
        }
        if bar.settings.spotify_play_in_aria {
            let aria = tr.tr("spotify-device-aria");
            let is_aria = chosen == Some(aria_core::spotify_player::ARIA_DEVICE);
            if ui.selectable_label(is_aria, &*aria).clicked() {
                picked = Some(Some(aria_core::spotify_player::ARIA_DEVICE.to_owned()));
            }
        }
        for device in devices {
            let Some(id) = &device.id else {
                continue;
            };
            let selected = chosen == Some(id.as_str());
            let control = eframe::egui::Button::selectable(selected, &device.name);
            if ui.add_enabled(!device.restricted, control).clicked() {
                picked = Some(Some(id.clone()));
            }
        }
        picked
    });
    if let Some(id) = picked.and_then(|shown| shown.inner)
        && id.as_deref() != chosen
    {
        out.push(PlayerAction::SpotifyDevice(id));
    }
}

/// While the microphone is open, its icon: the voice goes to the second device, and a click
/// turns it off. Crossed out, it could not be opened, and a click turns the switch off. Closed,
/// nothing.
fn microphone(ui: &mut Ui, bar: &PlayerBar<'_>, out: &mut Vec<PlayerAction>) {
    let (tr, p) = (bar.tr, bar.palette);
    let [button, icon] = BUTTON;
    let (glyph, key) = match bar.view.microphone {
        MicState::Off => return,
        MicState::On => (icons::MICROPHONE, "player-mic-on"),
        MicState::Failed => (icons::MICROPHONE_SLASH, "player-mic-failed"),
    };
    let name = tr.tr(key);
    // Which microphone it is, for the tooltip and for assistive technology.
    let device = bar
        .view
        .microphone_name
        .as_deref()
        .filter(|_| bar.view.microphone == MicState::On)
        .map(|device| {
            let mut args = FluentArgs::new();
            args.set("name", device);
            tr.tr_args("player-mic-device", &args).into_owned()
        });
    let control = IconButton::new(glyph, &name, p)
        .size(button, icon)
        .selected(bar.view.microphone == MicState::On);
    let response = ui.add(control);
    if let Some(device) = &device {
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_description(device.as_str()));
    }
    let tooltip = match &device {
        Some(device) => format!("{name}\n{device}"),
        None => name.into_owned(),
    };
    if response.on_hover_text(tooltip).clicked() {
        out.push(PlayerAction::ToggleMicrophone);
    }
}

/// The level the volume slider shows. While the device is held exclusively the level is the
/// device's, so the slider shows full and cannot move; otherwise, after a refused exclusive open
/// too, it shows the stored volume, which is the level heard.
fn shown_volume(view: &PlayerView, stored: f32) -> f32 {
    if view.volume_locked { 1.0 } else { stored }
}

/// Adds `control` disabled, with a tooltip naming it and the phase that brings it.
fn pending(
    ui: &mut Ui,
    control: IconButton<'_>,
    name: &str,
    phase: u8,
    tr: &Localizer,
    p: &Palette,
) {
    let mut args = FluentArgs::new();
    args.set("phase", phase);
    let hint = tr.tr_args("player-coming-in", &args);
    let response = ui.add_enabled(false, control);
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_description(&*hint));
    response.on_disabled_hover_ui(|ui| {
        ui.label(RichText::new(name).color(p.text));
        ui.label(RichText::new(&*hint).color(p.text_muted));
    });
}

/// The sleep timer: a moon that opens its menu, lit while it runs, with what is left beside it.
fn sleep_timer(ui: &mut Ui, bar: &PlayerBar<'_>, out: &mut Vec<PlayerAction>) {
    let (tr, p) = (bar.tr, bar.palette);
    let [button, icon] = BUTTON;
    let left = match bar.view.sleep {
        SleepState::Off => None,
        SleepState::EndOfTrack => Some(tr.tr("player-sleep-end-of-track").into_owned()),
        SleepState::Ends(at) => {
            let secs = at
                .saturating_duration_since(std::time::Instant::now())
                .as_secs();
            let mut args = FluentArgs::new();
            args.set("minutes", secs.div_ceil(60));
            Some(tr.tr_args("player-sleep-left", &args).into_owned())
        }
    };
    let name = tr.tr("player-sleep");
    let control = IconButton::new(icons::MOON, &name, p)
        .size(button, icon)
        .selected(left.is_some());
    let response = ui.add(control);
    if let Some(left) = &left {
        let text = RichText::new(left).small().color(p.text_muted);
        ui.add(Label::new(text).selectable(false));
    }
    let chosen = eframe::egui::Popup::menu(&response).show(|ui| {
        let mut chosen = None;
        for minutes in SLEEP_MINUTES {
            if ui
                .button(tr.tr(&format!("player-sleep-{minutes}")))
                .clicked()
            {
                chosen = Some(SleepTimer::After(std::time::Duration::from_secs(
                    minutes * 60,
                )));
            }
        }
        if ui.button(tr.tr("player-sleep-end-of-track")).clicked() {
            chosen = Some(SleepTimer::EndOfTrack);
        }
        if left.is_some() && ui.button(tr.tr("player-sleep-off")).clicked() {
            chosen = Some(SleepTimer::Off);
        }
        chosen
    });
    if let Some(timer) = chosen.and_then(|shown| shown.inner) {
        out.push(PlayerAction::SetSleepTimer(timer));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn format(codec: &str, bits: u16, rate: u32, quality: Quality) -> String {
        format_label(&TrackFormat {
            codec: Arc::from(codec),
            sample_rate: rate,
            bits_per_sample: bits,
            quality,
        })
    }

    #[test]
    fn format_labels() {
        assert_eq!(format("flac", 24, 96_000, Quality::HiRes), "FLAC 24/96");
        assert_eq!(
            format("flac", 16, 44_100, Quality::Lossless),
            "FLAC 16/44.1"
        );
        assert_eq!(format("alac", 24, 192_000, Quality::HiRes), "ALAC 24/192");
        assert_eq!(format("mp3", 0, 44_100, Quality::Lossy), "MP3 44.1");
        assert_eq!(format("flac", 24, 88_200, Quality::HiRes), "FLAC 24/88.2");
    }

    #[test]
    fn an_unreadable_rate_shows_no_junk() {
        assert_eq!(format("flac", 24, 0, Quality::Lossless), "FLAC");
        assert_eq!(format("unknown", 0, 0, Quality::Lossy), "");
        assert_eq!(format("unknown", 0, 44_100, Quality::Lossy), "44.1");
    }

    fn view(output: Option<OutputPath>, volume_locked: bool) -> PlayerView {
        PlayerView {
            current: Some(TrackId(1)),
            output: output.map(|path| aria_core::OutputState {
                path,
                device_rate: 44_100,
                device_bits: 16,
            }),
            volume_locked,
            ..PlayerView::default()
        }
    }

    #[test]
    fn the_bar_follows_the_output_actually_opened() {
        // An exclusive open on its way: no path yet, and the slider is the device's.
        let opening = view(None, true);
        assert_eq!(path_key(&opening), None);
        assert_eq!(shown_volume(&opening, 0.3), 1.0);

        // Refused, before the engine has said more: the system mixer, at the stored volume.
        let refused = view(None, false);
        assert_eq!(path_key(&refused), Some("output-system-mixer"));
        assert_eq!(shown_volume(&refused, 0.3), 0.3);

        let shared = view(Some(OutputPath::SystemMixer), false);
        assert_eq!(path_key(&shared), Some("output-system-mixer"));
        assert_eq!(shown_volume(&shared, 0.3), 0.3);

        // Held exclusively again.
        let held = view(Some(OutputPath::BitPerfect), true);
        assert_eq!(path_key(&held), Some("output-bit-perfect"));
        assert_eq!(shown_volume(&held, 0.3), 1.0);
    }

    #[test]
    fn controls_without_sides_have_no_shuffle_or_repeat() {
        use egui_kittest::{Harness, kittest::Queryable as _};
        let tr = Localizer::new(aria_core::settings::Language::En).unwrap();
        let en = |key: &str| tr.tr(key).into_owned();
        let view = PlayerView {
            current: Some(TrackId(1)),
            duration_ms: 60_000,
            ..PlayerView::default()
        };
        let harness = Harness::builder()
            .with_size(vec2(260.0, 60.0))
            .with_context_setup(crate::ui::theme::fonts::install)
            .build_ui(|ui| {
                let palette = Palette::dark();
                let settings = Settings::default();
                let collections = Collections::default();
                let mut bar = PlayerBar {
                    tr: &tr,
                    palette: &palette,
                    view: &view,
                    now: None,
                    position_ms: 0,
                    settings: &settings,
                    queue_open: false,
                    mini_open: false,
                    artwork: None,
                    collections: &collections,
                    spotify: None,
                    spotify_devices: None,
                    covers: None,
                };
                let mut out = Vec::new();
                let column = ui.max_rect();
                controls(ui, column, &mut bar, &mut out, false);
            });
        for key in ["player-previous", "player-play", "player-next"] {
            assert!(harness.query_by_label(&en(key)).is_some(), "{key}");
        }
        for key in ["player-shuffle", "player-repeat-off"] {
            assert!(harness.query_by_label(&en(key)).is_none(), "{key}");
        }
    }

    #[test]
    fn a_lossy_file_never_shows_a_depth() {
        // lofty can report 16 for AAC.
        assert_eq!(format("aac", 16, 44_100, Quality::Lossy), "AAC 44.1");
    }
}
