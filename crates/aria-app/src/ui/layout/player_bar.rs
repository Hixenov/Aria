//! The player bar along the bottom of the window: what is playing, the transport and seek
//! bar, and the volume, queue, device and lyrics controls.
//!
//! The bar draws what it is given and returns what the user asked for as [`PlayerAction`]s;
//! the app turns them into commands and settings. The device and lyrics buttons stay disabled
//! and say which phase brings them.

use std::time::Duration;

use aria_core::{
    ClockReading,
    i18n::{FluentArgs, Localizer},
    playback_service::PlayerView,
    player::queue::RepeatMode,
    settings::Settings,
};
use aria_library::{ArtworkId, Collections, LibrarySnapshot, TrackId};
use eframe::egui::{
    Align, Align2, Label, Layout, Margin, Panel, Rect, RichText, Sense, TextStyle, Ui, UiBuilder,
    Vec2, pos2, vec2,
};

use super::surface_frame;
use crate::{
    artwork::{ArtworkTextures, paint_cover},
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

/// Phase that brings output device selection.
const DEVICES_PHASE: u8 = 3;
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
}

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
    /// Where covers come from. Without it the placeholder shows.
    pub artwork: Option<&'a mut ArtworkTextures>,
    /// Which tracks are favorites, for the heart.
    pub collections: &'a Collections,
}

/// Shows the bar and returns what the user did on it this frame.
pub fn show(ui: &mut Ui, bar: &mut PlayerBar<'_>) -> Vec<PlayerAction> {
    let margin = Margin::symmetric(SPACE[3] as i8, SPACE[1] as i8);
    let mut actions = Vec::new();
    if bar.view.playing {
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
        });
    });
    if let Some(PageAction::SetFavorite { track, favorite }) =
        track_menu::heart(ui, bar.tr, p, bar.collections, track)
    {
        out.push(PlayerAction::SetFavorite { track, favorite });
    }
}

/// Shuffle, previous, play, next and repeat, with the seek bar under them, centered in `bar`.
fn center(ui: &mut Ui, area: Rect, bar: &mut PlayerBar<'_>, out: &mut Vec<PlayerAction>) {
    let width = (area.width() * (1.0 - 2.0 * SIDE_SHARE)).min(CENTER_MAX_WIDTH);
    let height = PLAY_BUTTON[0] + SEEK_ROW;
    let column = Rect::from_center_size(area.center(), vec2(width, height));
    let buttons = Rect::from_min_size(column.min, vec2(width, PLAY_BUTTON[0]));
    let seek = Rect::from_min_size(
        pos2(column.left(), column.bottom() - SEEK_ROW),
        vec2(width, SEEK_ROW),
    );
    zone(ui, buttons, Layout::left_to_right(Align::Center), |ui| {
        transport(ui, bar, out);
    });
    zone(ui, seek, Layout::left_to_right(Align::Center), |ui| {
        seek_row(ui, bar, out);
    });
}

/// The transport's buttons, centered in the row they are in.
fn transport(ui: &mut Ui, bar: &PlayerBar<'_>, out: &mut Vec<PlayerAction>) {
    let (tr, p) = (bar.tr, bar.palette);
    let loaded = bar.view.current.is_some();
    let gap = SPACE[1];
    let [side, side_icon] = SMALL_BUTTON;
    let [button, icon] = BUTTON;
    let [play, play_icon] = PLAY_BUTTON;
    let width = side * 2.0 + button * 2.0 + play + gap * 4.0;
    ui.spacing_mut().item_spacing.x = gap;
    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));

    let shuffle = tr.tr("player-shuffle");
    let control = IconButton::new(icons::SHUFFLE, &shuffle, p)
        .size(side, side_icon)
        .selected(bar.settings.shuffle);
    if ui.add(control).clicked() {
        out.push(PlayerAction::ToggleShuffle);
    }

    let previous = tr.tr("player-previous");
    let control = IconButton::new(icons::SKIP_BACK, &previous, p).size(button, icon);
    if ui.add_enabled(loaded, control).clicked() {
        out.push(PlayerAction::Previous);
    }

    let (glyph, key) = if bar.view.playing {
        (icons::PAUSE, "player-pause")
    } else {
        (icons::PLAY, "player-play")
    };
    let name = tr.tr(key);
    let control = IconButton::new(glyph, &name, p)
        .size(play, play_icon)
        .prominent();
    if ui.add_enabled(loaded, control).clicked() {
        out.push(PlayerAction::TogglePlay);
    }

    let next = tr.tr("player-next");
    let control = IconButton::new(icons::SKIP_FORWARD, &next, p).size(button, icon);
    if ui.add_enabled(loaded, control).clicked() {
        out.push(PlayerAction::Next);
    }

    let (glyph, key) = match bar.settings.repeat {
        RepeatMode::Off => (icons::REPEAT, "player-repeat-off"),
        RepeatMode::All => (icons::REPEAT, "player-repeat-all"),
        RepeatMode::One => (icons::REPEAT_ONCE, "player-repeat-one"),
    };
    let name = tr.tr(key);
    let control = IconButton::new(glyph, &name, p)
        .size(side, side_icon)
        .selected(bar.settings.repeat != RepeatMode::Off);
    if ui.add(control).clicked() {
        out.push(PlayerAction::CycleRepeat);
    }
}

/// The elapsed time, the seek bar and the length.
fn seek_row(ui: &mut Ui, bar: &PlayerBar<'_>, out: &mut Vec<PlayerAction>) {
    let p = bar.palette;
    let duration = bar.view.duration_ms;
    let seekable = bar.view.current.is_some() && duration > 0;
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
        .filter(|pending| !pending.settled(bar.view.token, bar.position_ms, now));
    let base_ms = pending.map_or(bar.position_ms, |pending| pending.target_ms);
    let fraction = if duration > 0 {
        (base_ms as f64 / duration as f64).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let label = bar.tr.tr("player-seek");
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
        out.push(PlayerAction::Seek(target_ms));
        pending = Some(PendingSeek {
            token: bar.view.token,
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

    let lyrics = tr.tr("player-lyrics");
    let control = IconButton::new(icons::QUOTES, &lyrics, p).size(button, icon);
    pending(ui, control, &lyrics, LYRICS_PHASE, tr, p);

    let queue = tr.tr("player-queue");
    let control = IconButton::new(icons::QUEUE, &queue, p)
        .size(button, icon)
        .selected(bar.queue_open);
    if ui.add(control).clicked() {
        out.push(PlayerAction::ToggleQueue);
    }

    let devices = tr.tr("player-devices");
    let control = IconButton::new(icons::DEVICES, &devices, p).size(button, icon);
    pending(ui, control, &devices, DEVICES_PHASE, tr, p);

    let (rect, _) = ui.allocate_exact_size(vec2(VOLUME_WIDTH, SEEK_ROW), Sense::hover());
    let label = tr.tr("player-volume");
    let volume = bar.settings.volume;
    let scrub = Scrub {
        id: "aria_volume",
        rect,
        value: volume,
        enabled: true,
        label: &label,
        step: VOLUME_STEP,
    };
    let moved = scrub.show(ui, p);
    // Every move is a change to apply; the app skips the ones that change nothing.
    if let Some(at) = moved.dragging.or(moved.committed) {
        out.push(PlayerAction::SetVolume(at));
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
