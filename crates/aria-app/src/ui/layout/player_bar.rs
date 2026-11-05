//! The player bar along the bottom of the window.
//!
//! Nothing can play yet, so the transport, volume, device and lyrics controls are disabled
//! and their tooltips say which phase brings them. Only the queue button works: it toggles
//! the right panel.

use aria_core::i18n::{FluentArgs, Localizer};
use eframe::egui::{
    Align, Align2, Label, Layout, Margin, Panel, Rect, RichText, Sense, Ui, UiBuilder, Vec2, pos2,
};

use super::surface_frame;
use crate::ui::{
    theme::{Palette, fonts::icon_font, icons, visuals::SPACE},
    widgets::IconButton,
};

/// Panel id, unique among the root panels.
const ID: &str = "aria_player_bar";
/// Height of the bar.
pub const HEIGHT: f32 = 72.0;
/// Edge length of the artwork placeholder, and its icon size.
const ARTWORK: [f32; 2] = [48.0, 22.0];
/// Edge length of the play button, and its icon size.
const PLAY_BUTTON: [f32; 2] = [40.0, 20.0];
/// Edge length of the other buttons, and their icon size.
const BUTTON: [f32; 2] = [32.0, 18.0];
/// Share of the bar's width taken by each side; the transport is centered between them.
const SIDE_SHARE: f32 = 0.3;

/// Phase that brings playback: the transport and volume controls.
const PLAYBACK_PHASE: u8 = 2;
/// Phase that brings output device selection.
const DEVICES_PHASE: u8 = 3;
/// Phase that brings lyrics.
const LYRICS_PHASE: u8 = 5;

/// Shows the bar. `queue_open` marks the queue button as on; returns whether it was
/// clicked.
pub fn show(ui: &mut Ui, tr: &Localizer, p: &Palette, queue_open: bool) -> bool {
    let margin = Margin::symmetric(SPACE[3] as i8, SPACE[1] as i8);
    Panel::bottom(ID)
        .exact_size(HEIGHT)
        .frame(surface_frame(p, margin))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let side = rect.width() * SIDE_SHARE;
            let left = Rect::from_min_max(rect.min, pos2(rect.left() + side, rect.bottom()));
            let right = Rect::from_min_max(pos2(rect.right() - side, rect.top()), rect.max);
            zone(ui, left, Layout::left_to_right(Align::Center), |ui| {
                now_playing(ui, tr, p);
            });
            transport(ui, rect, tr, p);
            let mut clicked = false;
            zone(ui, right, Layout::right_to_left(Align::Center), |ui| {
                clicked = extras(ui, tr, p, queue_open);
            });
            clicked
        })
        .inner
}

/// Runs `add` in a child [`Ui`] confined to `rect`.
fn zone(ui: &mut Ui, rect: Rect, layout: Layout, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(layout), add);
}

/// The artwork placeholder and the "Nothing playing" line.
fn now_playing(ui: &mut Ui, tr: &Localizer, p: &Palette) {
    let [size, icon] = ARTWORK;
    let (art, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(art) {
        let painter = ui.painter();
        painter.rect_filled(
            art,
            ui.visuals().widgets.inactive.corner_radius,
            p.surface_raised,
        );
        painter.text(
            art.center(),
            Align2::CENTER_CENTER,
            icons::MUSIC_NOTES,
            icon_font(icon),
            p.text_muted,
        );
    }
    ui.add_space(SPACE[0]);
    let text = RichText::new(tr.tr("empty-nothing-playing")).color(p.text_muted);
    ui.add(Label::new(text).selectable(false).truncate());
}

/// Previous, play and next, centered in `bar`.
fn transport(ui: &mut Ui, bar: Rect, tr: &Localizer, p: &Palette) {
    let gap = SPACE[2];
    let width = BUTTON[0] * 2.0 + PLAY_BUTTON[0] + gap * 2.0;
    let rect = Rect::from_center_size(bar.center(), Vec2::new(width, bar.height()));
    zone(ui, rect, Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let [button, icon] = BUTTON;
        let [play, play_icon] = PLAY_BUTTON;
        let controls = [
            (icons::SKIP_BACK, "player-previous", button, icon, false),
            (icons::PLAY, "player-play", play, play_icon, true),
            (icons::SKIP_FORWARD, "player-next", button, icon, false),
        ];
        for (glyph, key, size, icon_size, prominent) in controls {
            let name = tr.tr(key);
            let mut control = IconButton::new(glyph, &name, p).size(size, icon_size);
            if prominent {
                control = control.prominent();
            }
            pending(ui, control, &name, PLAYBACK_PHASE, tr, p);
        }
    });
}

/// Lyrics, queue, devices and volume, laid out from the right edge.
fn extras(ui: &mut Ui, tr: &Localizer, p: &Palette, queue_open: bool) -> bool {
    ui.spacing_mut().item_spacing.x = SPACE[0];
    let [button, icon] = BUTTON;
    let volume = tr.tr("player-volume");
    let control = IconButton::new(icons::SPEAKER_HIGH, &volume, p).size(button, icon);
    pending(ui, control, &volume, PLAYBACK_PHASE, tr, p);

    let devices = tr.tr("player-devices");
    let control = IconButton::new(icons::DEVICES, &devices, p).size(button, icon);
    pending(ui, control, &devices, DEVICES_PHASE, tr, p);

    let queue = tr.tr("player-queue");
    let control = IconButton::new(icons::QUEUE, &queue, p)
        .size(button, icon)
        .selected(queue_open);
    let clicked = ui.add(control).clicked();

    let lyrics = tr.tr("player-lyrics");
    let control = IconButton::new(icons::QUOTES, &lyrics, p).size(button, icon);
    pending(ui, control, &lyrics, LYRICS_PHASE, tr, p);
    clicked
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
