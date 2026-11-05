//! The placeholder shown where content will appear.

use eframe::egui::{Align2, FontId, Label, RichText, Sense, TextStyle, Ui, Vec2};

use crate::ui::theme::{
    Palette,
    fonts::{icon_font, semibold},
    visuals::{SPACE, TITLE_SIZE},
};

/// Diameter of the badge behind the icon.
const BADGE_SIZE: f32 = 72.0;
/// Size of the icon.
const ICON_SIZE: f32 = 32.0;
/// Widest a line of text gets before it wraps.
const MAX_TEXT_WIDTH: f32 = 360.0;

/// A block centered in the remaining space: `icon` on a round badge, `title` in primary
/// text, then `body` in muted text. An empty `body` is left out.
pub fn empty_state(ui: &mut Ui, icon: &str, title: &str, body: &str, p: &Palette) {
    let title_font = semibold(TITLE_SIZE);
    let body_font = TextStyle::Body.resolve(ui.style());
    let text_width = ui.available_width().min(MAX_TEXT_WIDTH);

    // Measure the text first, so the whole block can be centered vertically.
    let measure = |text: &str, font: &FontId| {
        ui.painter()
            .layout(text.to_owned(), font.clone(), p.text, text_width)
            .size()
            .y
    };
    let mut height = BADGE_SIZE + SPACE[3] + measure(title, &title_font);
    if !body.is_empty() {
        height += SPACE[0] + measure(body, &body_font);
    }
    let top = ((ui.available_height() - height) / 2.0).max(0.0);

    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(top);
        let (badge, _) = ui.allocate_exact_size(Vec2::splat(BADGE_SIZE), Sense::hover());
        if ui.is_rect_visible(badge) {
            let painter = ui.painter();
            painter.circle_filled(badge.center(), BADGE_SIZE / 2.0, p.surface_raised);
            painter.text(
                badge.center(),
                Align2::CENTER_CENTER,
                icon,
                icon_font(ICON_SIZE),
                p.text_muted,
            );
        }
        ui.add_space(SPACE[3]);
        ui.set_max_width(text_width);
        ui.add(Label::new(RichText::new(title).font(title_font).color(p.text)).selectable(false));
        if !body.is_empty() {
            ui.add_space(SPACE[0]);
            ui.add(
                Label::new(RichText::new(body).font(body_font).color(p.text_muted))
                    .selectable(false),
            );
        }
    });
}
