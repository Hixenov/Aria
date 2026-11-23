//! The full-screen "now playing" mode: the playing track as a record turning on a turntable.
//!
//! The app owns a [`VinylMode`], opens and closes it, and while it is open draws only it (and
//! the toasts). The mode draws the record from what the player bar is given, and its controls
//! are the player bar's own, so they act the same on local and on Spotify playback.

pub mod cover;
pub mod draw;
pub mod finish;
pub mod layout;
pub mod motion;

use std::path::Path;

use aria_core::{NowItem, TrackId, settings::VinylFinish};
use eframe::egui::{
    self, Align, Color32, CursorIcon, Id, Label, Layout as EguiLayout, Pos2, Rect, RichText, Sense,
    TextureHandle, Ui, UiBuilder, ViewportCommand, WidgetInfo, accesskit::Role, pos2, vec2,
};

use self::cover::LocalCover;
use self::draw::{
    ArmColours, FinishCache, Label as RecordLabel, draw_arm, draw_record, draw_sleeve,
};
use self::finish::{CoverColours, Finish, render, resolve};
use self::layout::{ARTIST_BELOW, STRIP, layout, volume_rect};
use self::motion::{RPM_MARKS, Turntable, snap_rpm};
use crate::ui::layout::player_bar::{self, PlayerAction, PlayerBar};
use crate::ui::pages::PageAction;
use crate::ui::theme::fonts::{icon_font, semibold};
use crate::ui::theme::{Palette, icons};
use crate::ui::widgets::IconButton;

/// Seconds without pointer movement before the controls hide.
pub const HIDE_AFTER: f64 = 3.0;
/// How long the controls take to fade.
const FADE: f32 = 0.3;
/// Size of a finish swatch in the panel.
const SWATCH: f32 = 56.0;

/// What the user did in the mode. The app carries it out.
#[derive(Clone, Debug, PartialEq)]
pub enum VinylAction {
    Player(PlayerAction),
    Close,
    SetFinish(VinylFinish),
    /// Turn at this speed, in turns per minute.
    SetRpm(f32),
}

/// What the mode needs beside the player bar.
pub struct VinylInput<'a> {
    pub finish: VinylFinish,
    /// The record's speed, in turns per minute.
    pub rpm: f32,
    pub dark: bool,
    /// The current local track and its file, for its large cover.
    pub local: Option<(TrackId, &'a Path)>,
}

#[derive(Default)]
pub struct VinylMode {
    open: bool,
    /// Whether the window was in OS full screen when the mode opened.
    was_fullscreen: Option<bool>,
    /// The OS has reported full screen since the mode opened.
    seen_fullscreen: bool,
    /// The last full-screen command sent, for tests.
    requested: Option<bool>,
    turntable: Turntable,
    panel_open: bool,
    last_move: f64,
    /// Where the pointer was last frame, to notice it moving.
    last_pointer: Option<Pos2>,
    hovering: bool,
    finishes: FinishCache,
    local_cover: LocalCover,
    /// The panel's swatches, rendered for these colours.
    swatches: Option<(CoverColours, Vec<TextureHandle>)>,
    /// The window's first reported state has been looked at.
    start_checked: bool,
    /// The speed on the slider while it is dragged, before it is saved.
    dragged_rpm: Option<f32>,
}

impl VinylMode {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The last full-screen command the mode sent: `Some(true)` on opening, `Some(false)` on a
    /// closing that leaves full screen.
    pub fn requested_fullscreen(&self) -> Option<bool> {
        self.requested
    }

    /// Leaves an OS full screen the window starts in. Aria is full screen only in the mode, and
    /// the window is saved as it is on quitting (or every 30 s), so a quit or a crash while the
    /// mode was open would bring the next start back full screen, with no way out.
    pub fn leave_restored_fullscreen(&mut self, ctx: &egui::Context) {
        if self.start_checked {
            return;
        }
        let Some(fullscreen) = ctx.input(|i| i.viewport().fullscreen) else {
            return;
        };
        self.start_checked = true;
        if fullscreen && !self.open {
            self.requested = Some(false);
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
    }

    pub fn open(&mut self, ctx: &egui::Context) {
        if self.open {
            return;
        }
        self.open = true;
        self.was_fullscreen = ctx.input(|i| i.viewport().fullscreen);
        self.seen_fullscreen = self.was_fullscreen == Some(true);
        self.panel_open = false;
        self.last_move = ctx.input(|i| i.time);
        self.requested = Some(true);
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
    }

    /// Closes the mode, leaving OS full screen unless the window was in it before.
    pub fn close(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        self.open = false;
        self.panel_open = false;
        if self.was_fullscreen != Some(true) {
            self.requested = Some(false);
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        self.finishes.clear();
        self.local_cover.clear();
        self.swatches = None;
        ctx.set_cursor_icon(CursorIcon::Default);
    }

    pub fn toggle(&mut self, ctx: &egui::Context) {
        if self.open {
            self.close(ctx);
        } else {
            self.open(ctx);
        }
    }

    /// Esc: closes the finish panel if it is open (`true`), else leaves it to the caller.
    pub fn escape(&mut self) -> bool {
        std::mem::replace(&mut self.panel_open, false)
    }

    /// Closes the mode if the window left OS full screen another way. Returns whether it did.
    pub fn follow_os(&mut self, ctx: &egui::Context) -> bool {
        if !self.open {
            return false;
        }
        match ctx.input(|i| i.viewport().fullscreen) {
            Some(true) => self.seen_fullscreen = true,
            Some(false) if self.seen_fullscreen => {
                // Already windowed: nothing to undo.
                self.was_fullscreen = Some(true);
                self.close(ctx);
                return true;
            }
            _ => {}
        }
        false
    }

    /// Draws the mode over `ui`, and returns what the user did.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        bar: &mut PlayerBar<'_>,
        input: &VinylInput<'_>,
    ) -> Vec<VinylAction> {
        let ctx = ui.ctx().clone();
        let screen = ui.max_rect();
        let now = ui.input(|i| i.time);
        let dt = ui.input(|i| i.stable_dt).clamp(0.0, 0.1);
        let playing = bar.spotify.map_or(bar.view.playing, |p| p.state.playing);
        let rpm = self.dragged_rpm.unwrap_or(input.rpm);
        self.turntable.step(dt, playing, rpm);
        let p = *bar.palette;
        let mut out = Vec::new();
        let mut player = Vec::new();
        ui.painter()
            .rect_filled(screen, 0.0, background(input.dark));

        let shown = self.what_plays(&ctx, bar, input, &mut player);
        let finish = resolve(input.finish, &shown.identity);
        let key = draw::finish_key(finish, &shown.colours);
        self.finishes.want(&ctx, finish, shown.colours, key);
        let finish_texture = self.finishes.texture(&ctx).cloned();

        let place = layout(screen);
        let cover = shown.large.as_ref().or(shown.small.as_ref());
        let painter = ui.painter().clone();
        if let Some(sleeve) = place.sleeve {
            draw_sleeve(&painter, sleeve, cover, p.surface_raised);
        }
        let [r, g, b] = shown.colours.main;
        let label = match cover {
            Some(texture) => RecordLabel::Cover(texture),
            None => RecordLabel::Plain {
                colour: Color32::from_rgb(r, g, b),
                title: &shown.title,
                artist: &shown.artist,
            },
        };
        let sheen = Color32::from_white_alpha(if input.dark { 26 } else { 36 });
        draw_record(
            &painter,
            place.record,
            self.turntable.angle,
            finish_texture.as_ref(),
            label,
            sheen,
        );
        draw_arm(
            &painter,
            &place,
            self.turntable.arm,
            ArmColours::for_theme(input.dark),
        );
        self.title_block(ui, &p, place.title, screen, &shown, bar);

        // The controls show on pointer movement and fade after a while.
        let (pointer, pressed) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.any_pressed()));
        let moved = pressed || (pointer.is_some() && pointer != self.last_pointer);
        self.last_pointer = pointer;
        if moved {
            self.last_move = now;
        }
        let idle = now - self.last_move;
        let visible = idle < HIDE_AFTER || self.panel_open || self.hovering;
        let alpha = ctx.animate_bool_with_time(Id::unique("aria_vinyl_controls"), visible, FADE);
        if alpha > 0.0 {
            let mut parts = Parts {
                bar,
                input,
                shown: &shown,
                player: &mut player,
                out: &mut out,
            };
            self.controls(ui, screen, alpha, &mut parts);
        } else {
            self.hovering = false;
            ctx.set_cursor_icon(CursorIcon::None);
        }
        out.extend(player.into_iter().map(VinylAction::Player));

        if self.turntable.busy(playing) || (alpha > 0.0 && alpha < 1.0) {
            ctx.request_repaint();
        } else if visible && idle < HIDE_AFTER {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(HIDE_AFTER - idle));
        }
        out
    }

    /// The title, the artist, and the cover the mode shows.
    fn what_plays(
        &mut self,
        ctx: &egui::Context,
        bar: &mut PlayerBar<'_>,
        input: &VinylInput<'_>,
        player: &mut Vec<PlayerAction>,
    ) -> Shown {
        let mut shown = Shown::default();
        let mut asked = Vec::new();
        if let Some(playback) = bar.spotify {
            match &playback.state.item {
                Some(NowItem::Track(track)) => {
                    shown.title.clone_from(&track.name);
                    shown.artist = track.artists.join(", ");
                    shown.identity = track.uri.clone().unwrap_or_else(|| track.name.clone());
                    if let Some(covers) = bar.covers.as_deref_mut() {
                        if let Some(url) = &track.image_large {
                            shown.large = covers.get_large(url, &mut asked).cloned();
                            shown.colours = covers.colours(url).unwrap_or(shown.colours);
                        }
                        if let Some(url) = &track.image {
                            shown.small = covers.get(url, &mut asked).cloned();
                            if track.image_large.is_none() {
                                shown.colours = covers.colours(url).unwrap_or(shown.colours);
                            }
                        }
                    }
                }
                Some(NowItem::Other { name }) => {
                    shown.title.clone_from(name);
                    shown.artist = "Spotify".to_owned();
                    shown.identity.clone_from(name);
                }
                None => {}
            }
        } else if let (Some(now), Some(id)) = (bar.now, bar.view.current) {
            shown.title.clone_from(&now.title);
            shown.artist.clone_from(&now.artist);
            match input.local {
                Some((key, path)) if key == id => {
                    shown.identity = path.to_string_lossy().into_owned();
                    self.local_cover.request(ctx, key, path.to_path_buf());
                    shown.large = self.local_cover.texture(ctx, key).cloned();
                    shown.colours = self.local_cover.colours(key).unwrap_or(shown.colours);
                }
                _ => shown.identity = format!("{}", id.0),
            }
            shown.small = now
                .artwork
                .as_ref()
                .and_then(|art| bar.artwork.as_deref_mut()?.get(art).cloned());
        }
        player.extend(asked.into_iter().filter_map(|action| match action {
            PageAction::Spotify(request) => Some(PlayerAction::Spotify(request)),
            _ => None,
        }));
        shown
    }

    fn title_block(
        &self,
        ui: &mut Ui,
        p: &Palette,
        at: Pos2,
        screen: Rect,
        shown: &Shown,
        bar: &PlayerBar<'_>,
    ) {
        let width = (screen.width() * 0.6).max(200.0);
        let (title, muted) = if shown.title.is_empty() {
            (bar.tr.tr("empty-nothing-playing").into_owned(), true)
        } else {
            (shown.title.clone(), false)
        };
        let colour = if muted { p.text_muted } else { p.text };
        let title_rect = Rect::from_min_size(pos2(at.x, at.y - 46.0), vec2(width, 46.0));
        ui.put(
            title_rect,
            Label::new(RichText::new(title).font(semibold(36.0)).color(colour))
                .truncate()
                .selectable(false),
        );
        if !shown.artist.is_empty() {
            let artist_rect =
                Rect::from_min_size(pos2(at.x, at.y + 4.0), vec2(width, ARTIST_BELOW - 4.0));
            ui.put(
                artist_rect,
                Label::new(RichText::new(&shown.artist).size(20.0).color(p.text_muted))
                    .truncate()
                    .selectable(false),
            );
        }
    }

    fn controls(&mut self, ui: &mut Ui, screen: Rect, alpha: f32, parts: &mut Parts<'_, '_, '_>) {
        let p = *parts.bar.palette;
        let dark = parts.input.dark;
        let strip = Rect::from_min_max(pos2(screen.left(), screen.bottom() - STRIP), screen.max);
        let top = Rect::from_min_max(
            pos2(screen.right() - 140.0, screen.top()),
            pos2(screen.right(), screen.top() + 64.0),
        );
        ui.scope_builder(UiBuilder::new().max_rect(screen), |ui| {
            ui.set_opacity(alpha);
            ui.painter().rect_filled(
                strip,
                0.0,
                p.bg.gamma_multiply(if dark { 0.75 } else { 0.85 }),
            );
            player_bar::center(ui, strip, parts.bar, parts.player);
            let volume = volume_rect(strip);
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(volume)
                    .layout(EguiLayout::right_to_left(Align::Center)),
                |ui| player_bar::volume_controls(ui, parts.bar, parts.player),
            );
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(top.shrink(12.0))
                    .layout(EguiLayout::right_to_left(Align::Center)),
                |ui| {
                    let exit = parts.bar.tr.tr("player-exit-full-screen");
                    if ui
                        .add(IconButton::new(icons::CORNERS_IN, &exit, &p).size(40.0, 22.0))
                        .clicked()
                    {
                        parts.out.push(VinylAction::Close);
                    }
                    let name = parts.bar.tr.tr("vinyl-finish");
                    let button = IconButton::new(icons::PALETTE, &name, &p)
                        .size(40.0, 22.0)
                        .selected(self.panel_open);
                    if ui.add(button).clicked() {
                        self.panel_open = !self.panel_open;
                    }
                },
            );
            if self.panel_open {
                self.panel(ui, screen, &p, parts);
            }
        });
        self.hovering = ui.rect_contains_pointer(strip) || ui.rect_contains_pointer(top);
    }

    fn panel(&mut self, ui: &mut Ui, screen: Rect, p: &Palette, parts: &mut Parts<'_, '_, '_>) {
        let (tr, shown) = (parts.bar.tr, parts.shown);
        let width = 5.0 * (SWATCH + 16.0) + 32.0;
        let rect = Rect::from_center_size(
            pos2(screen.center().x, screen.bottom() - STRIP - 110.0),
            vec2(width, 190.0),
        );
        ui.painter().rect_filled(rect, 16.0, p.surface);
        let title = tr.tr("vinyl-finish");
        ui.put(
            Rect::from_min_size(rect.min + vec2(20.0, 8.0), vec2(width - 40.0, 22.0)),
            Label::new(RichText::new(&*title).color(p.text_muted)).selectable(false),
        );
        if self
            .swatches
            .as_ref()
            .is_none_or(|(c, _)| *c != shown.colours)
        {
            let textures = [
                Finish::Black,
                Finish::Tinted,
                Finish::Splatter,
                Finish::Marble,
            ]
            .iter()
            .map(|finish| {
                let image = render(*finish, &shown.colours, (SWATCH * 2.0) as u32);
                let size = [image.width() as usize, image.height() as usize];
                let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                ui.ctx()
                    .load_texture("aria-vinyl-swatch", pixels, egui::TextureOptions::LINEAR)
            })
            .collect();
            self.swatches = Some((shown.colours, textures));
        }
        let Some((_, swatches)) = &self.swatches else {
            return;
        };
        for (i, finish) in VinylFinish::ALL.iter().enumerate() {
            let centre = pos2(
                rect.left() + 16.0 + (SWATCH + 16.0) * i as f32 + SWATCH / 2.0 + 8.0,
                rect.top() + 36.0 + SWATCH / 2.0,
            );
            let area = Rect::from_center_size(centre, vec2(SWATCH, SWATCH));
            let name = tr.tr(finish_key(*finish));
            let response = ui.interact(area, Id::unique(("aria_vinyl_swatch", i)), Sense::click());
            response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &*name));
            let painter = ui.painter();
            let index = match finish {
                VinylFinish::Black => Some(0),
                VinylFinish::Tinted => Some(1),
                VinylFinish::Splatter => Some(2),
                VinylFinish::Marble => Some(3),
                VinylFinish::Random => None,
            };
            match index.and_then(|i| swatches.get(i)) {
                Some(texture) => {
                    painter.circle_filled(centre, SWATCH / 2.0, Color32::from_rgb(14, 14, 16));
                    crate::artwork::paint_cover(painter, area, (SWATCH / 2.0) as u8, texture);
                }
                None => {
                    painter.circle_filled(centre, SWATCH / 2.0, p.surface_raised);
                    painter.text(
                        centre,
                        egui::Align2::CENTER_CENTER,
                        icons::SHUFFLE,
                        icon_font(22.0),
                        p.text,
                    );
                }
            }
            if *finish == parts.input.finish {
                painter.circle_stroke(centre, SWATCH / 2.0 + 3.0, egui::Stroke::new(2.5, p.text));
            } else if response.hovered() {
                painter.circle_stroke(
                    centre,
                    SWATCH / 2.0 + 3.0,
                    egui::Stroke::new(1.5, p.text_muted),
                );
            }
            if response.clicked() {
                parts.out.push(VinylAction::SetFinish(*finish));
            }
        }
        let speed = Rect::from_min_max(
            pos2(rect.left() + 20.0, rect.top() + 110.0),
            pos2(rect.right() - 20.0, rect.bottom() - 10.0),
        );
        self.speed_slider(ui, speed, p, parts);
    }

    /// The speed: a slider from the slowest to the fastest, its marks named under it. It is
    /// saved when let go, and turns the record as it is dragged.
    fn speed_slider(
        &mut self,
        ui: &mut Ui,
        area: Rect,
        p: &Palette,
        parts: &mut Parts<'_, '_, '_>,
    ) {
        let tr = parts.bar.tr;
        let mut rpm = self.dragged_rpm.unwrap_or(parts.input.rpm);
        let name = tr.tr("vinyl-speed");
        let head = Rect::from_min_size(area.min, vec2(area.width(), 20.0));
        ui.put(
            head,
            Label::new(RichText::new(&*name).color(p.text_muted)).selectable(false),
        );
        let mut args = aria_core::i18n::FluentArgs::new();
        args.set("rpm", rpm_text(rpm));
        let value = tr.tr_args("vinyl-rpm", &args);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(head)
                .layout(EguiLayout::right_to_left(Align::Center)),
            |ui| ui.add(Label::new(RichText::new(&*value).color(p.text)).selectable(false)),
        );
        let track = Rect::from_min_size(
            pos2(area.left(), head.bottom() + 4.0),
            vec2(area.width(), 24.0),
        );
        let range = aria_core::settings::VINYL_RPM_MIN..=aria_core::settings::VINYL_RPM_MAX;
        let response = ui
            .scope_builder(UiBuilder::new().max_rect(track), |ui| {
                ui.spacing_mut().slider_width = track.width();
                ui.add(
                    egui::Slider::new(&mut rpm, range.clone())
                        .show_value(false)
                        .text(""),
                )
            })
            .inner;
        response.widget_info(|| WidgetInfo::slider(true, f64::from(rpm), &*name));
        let rpm = snap_rpm(rpm);
        if response.dragged() {
            self.dragged_rpm = Some(rpm);
        } else if response.drag_stopped() || response.changed() {
            self.dragged_rpm = None;
            parts.out.push(VinylAction::SetRpm(rpm));
        }
        let (low, high) = (*range.start(), *range.end());
        let painter = ui.painter();
        for mark in RPM_MARKS {
            let x = track.left() + (mark - low) / (high - low) * track.width();
            painter.text(
                pos2(
                    x.clamp(track.left() + 8.0, track.right() - 8.0),
                    track.bottom() + 10.0,
                ),
                egui::Align2::CENTER_CENTER,
                rpm_text(mark),
                egui::FontId::proportional(12.0),
                p.text_muted,
            );
        }
    }
}

/// A speed as the slider names it: whole numbers, and 33⅓ for a third.
fn rpm_text(rpm: f32) -> String {
    if (rpm - 100.0 / 3.0).abs() < 0.01 {
        "33⅓".to_owned()
    } else {
        format!("{}", rpm.round() as i32)
    }
}

/// The Fluent key naming `finish`.
pub fn finish_key(finish: VinylFinish) -> &'static str {
    match finish {
        VinylFinish::Random => "vinyl-finish-random",
        VinylFinish::Black => "vinyl-finish-black",
        VinylFinish::Tinted => "vinyl-finish-tinted",
        VinylFinish::Splatter => "vinyl-finish-splatter",
        VinylFinish::Marble => "vinyl-finish-marble",
    }
}

/// The mode's background, from the theme: a warm paper white, or a dark warm grey.
fn background(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(28, 26, 27)
    } else {
        Color32::from_rgb(240, 237, 232)
    }
}

/// What the controls work with, gathered so they travel together.
struct Parts<'s, 'b, 'i> {
    bar: &'s mut PlayerBar<'b>,
    input: &'s VinylInput<'i>,
    shown: &'s Shown,
    player: &'s mut Vec<PlayerAction>,
    out: &'s mut Vec<VinylAction>,
}

/// What the mode shows of the playing track.
struct Shown {
    title: String,
    artist: String,
    identity: String,
    large: Option<TextureHandle>,
    small: Option<TextureHandle>,
    colours: CoverColours,
}

impl Default for Shown {
    fn default() -> Shown {
        Shown {
            title: String::new(),
            artist: String::new(),
            identity: String::new(),
            large: None,
            small: None,
            colours: CoverColours::neutral(),
        }
    }
}
