//! A sidebar navigation row.

use eframe::egui::{
    Align2, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, TextStyle, Ui, WidgetInfo,
    accesskit::Role, pos2, text::LayoutJob, text::TextWrapping, vec2,
};

use super::FOCUS_RING_WIDTH;
use crate::ui::theme::{Palette, fonts::icon_font, visuals::SPACE};

/// Height of a navigation row.
pub const NAV_ROW_HEIGHT: f32 = 32.0;
/// Size of the row's icon.
const ICON_SIZE: f32 = 18.0;
/// Size of the accent bar on the selected row.
const ACCENT_BAR: [f32; 2] = [3.0, 16.0];
/// Strength of the hover fill, relative to the selected row's fill.
const HOVER_FILL: f32 = 0.6;

/// A full-width navigation row: an icon, then `label`, which ends in an ellipsis when the
/// row is too narrow and then shows in full on hover.
///
/// The selected row has a raised fill, an accent bar on its left edge and an accent icon.
/// Assistive technology sees a button named `label` whose toggled state is `selected`.
pub fn nav_item(ui: &mut Ui, icon: &str, label: &str, selected: bool, p: &Palette) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), NAV_ROW_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, ui.is_enabled(), selected, label));

    let icon_center = pos2(rect.left() + SPACE[2] + ICON_SIZE / 2.0, rect.center().y);
    let text_left = icon_center.x + ICON_SIZE / 2.0 + SPACE[2];
    let text_color = if selected || response.hovered() {
        p.text
    } else {
        p.text_muted
    };
    let mut job = LayoutJob::simple_singleline(
        label.to_owned(),
        TextStyle::Body.resolve(ui.style()),
        text_color,
    );
    job.wrap = TextWrapping::truncate_at_width((rect.right() - SPACE[1] - text_left).max(0.0));
    let galley = ui.painter().layout_job(job);
    let elided = galley.elided;

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let hover = ui
            .ctx()
            .animate_bool_responsive(response.id.with("hover"), response.hovered());
        if selected {
            painter.rect_filled(rect, radius, p.surface_raised);
            let bar = Rect::from_center_size(
                pos2(rect.left() + ACCENT_BAR[0] / 2.0, rect.center().y),
                ACCENT_BAR.into(),
            );
            painter.rect_filled(bar, CornerRadius::from(ACCENT_BAR[0] / 2.0), p.accent);
        } else if hover > 0.0 {
            painter.rect_filled(
                rect,
                radius,
                p.surface_raised.gamma_multiply(HOVER_FILL * hover),
            );
        }
        painter.text(
            icon_center,
            Align2::CENTER_CENTER,
            icon,
            icon_font(ICON_SIZE),
            if selected { p.accent } else { text_color },
        );
        let text_top = rect.center().y - galley.size().y / 2.0;
        painter.galley(pos2(text_left, text_top), galley, text_color);
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(FOCUS_RING_WIDTH, p.focus),
                StrokeKind::Inside,
            );
        }
    }

    if elided {
        response.on_hover_text(label)
    } else {
        response
    }
}
