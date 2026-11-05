//! The right panel, toggled from the player bar's queue button and Ctrl+Alt+R.
//!
//! There is no queue yet, so it shows an empty state.

use aria_core::i18n::Localizer;
use eframe::egui::{
    Align, FontFamily, FontId, Label, Layout, Panel, RichText, TextStyle, Ui, vec2,
};

use super::surface_frame;
use crate::ui::{
    theme::{Palette, fonts::SEMIBOLD, icons, visuals::SPACE},
    widgets::empty_state,
};

/// Panel id, unique among the root panels.
const ID: &str = "aria_right_panel";
/// Width of the panel.
pub const WIDTH: f32 = 300.0;
/// Text size of the panel title.
const TITLE_SIZE: f32 = 16.0;

/// Shows the panel while `open`, sliding it in and out as that changes.
pub fn show(ui: &mut Ui, tr: &Localizer, p: &Palette, open: bool) {
    let mut expanded = open;
    Panel::right(ID)
        .resizable(false)
        .exact_size(WIDTH)
        .frame(surface_frame(p, SPACE[3]))
        .show_collapsible(ui, &mut expanded, |ui| {
            // A row as tall as a page heading, so the two titles line up.
            let row = vec2(
                ui.available_width(),
                ui.text_style_height(&TextStyle::Heading),
            );
            ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                let font = FontId::new(TITLE_SIZE, FontFamily::Name(SEMIBOLD.into()));
                let title = RichText::new(tr.tr("player-queue"))
                    .font(font)
                    .color(p.text);
                ui.add(Label::new(title).selectable(false).truncate());
            });
            empty_state(ui, icons::QUEUE, &tr.tr("empty-right-panel"), "", p);
        });
}
