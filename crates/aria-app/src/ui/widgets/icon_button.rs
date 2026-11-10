//! Icon-only buttons.

use eframe::egui::{
    Align2, Response, Sense, Stroke, Ui, Vec2, Widget, WidgetInfo, accesskit::Role, pos2,
};

use super::FOCUS_RING_WIDTH;
use crate::ui::theme::{Palette, fonts::icon_font};

/// Default edge length of the button.
const SIZE: f32 = 32.0;
/// Default icon size.
const ICON_SIZE: f32 = 18.0;
/// Radius of the dot under a selected button.
const SELECTED_DOT: f32 = 2.0;

/// A round, icon-only button.
///
/// `label` names it for assistive technology and is its tooltip while enabled; give a
/// disabled button its own tooltip with [`Response::on_disabled_hover_ui`]. Add it with
/// [`Ui::add`], or [`Ui::add_enabled`] to disable it.
#[must_use = "add it with `ui.add`"]
pub struct IconButton<'a> {
    icon: &'a str,
    label: &'a str,
    palette: &'a Palette,
    size: f32,
    icon_size: f32,
    selected: Option<bool>,
    dot: bool,
    prominent: bool,
}

impl<'a> IconButton<'a> {
    /// A plain icon button: a soft fill appears on hover.
    pub fn new(icon: &'a str, label: &'a str, palette: &'a Palette) -> Self {
        Self {
            icon,
            label,
            palette,
            size: SIZE,
            icon_size: ICON_SIZE,
            selected: None,
            dot: true,
            prominent: false,
        }
    }

    /// Sets the edge length of the button and the size of its icon.
    pub fn size(mut self, size: f32, icon_size: f32) -> Self {
        self.size = size;
        self.icon_size = icon_size;
        self
    }

    /// Makes it a toggle; a selected toggle has an accent icon and a dot beneath it.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    /// Leaves out the dot under a selected toggle, where the accent icon says enough.
    pub fn without_dot(mut self) -> Self {
        self.dot = false;
        self
    }

    /// Draws the icon on a filled circle, for the main action of a group.
    pub fn prominent(mut self) -> Self {
        self.prominent = true;
        self
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), Sense::click());
        response.widget_info(|| match self.selected {
            Some(selected) => {
                WidgetInfo::selected(Role::Button, ui.is_enabled(), selected, self.label)
            }
            None => WidgetInfo::labeled(Role::Button, ui.is_enabled(), self.label),
        });

        if ui.is_rect_visible(rect) {
            let p = self.palette;
            let painter = ui.painter();
            let center = rect.center();
            let radius = self.size / 2.0;
            let selected = self.selected == Some(true);
            let icon_color = if self.prominent {
                painter.circle_filled(center, radius, p.text);
                p.bg
            } else {
                let hover = ui
                    .ctx()
                    .animate_bool_responsive(response.id.with("hover"), response.hovered());
                if hover > 0.0 {
                    painter.circle_filled(center, radius, p.surface_raised.gamma_multiply(hover));
                }
                if selected {
                    p.accent
                } else if response.hovered() {
                    p.text
                } else {
                    p.text_muted
                }
            };
            painter.text(
                center,
                Align2::CENTER_CENTER,
                self.icon,
                icon_font(self.icon_size),
                icon_color,
            );
            if selected && self.dot {
                let dot = pos2(center.x, rect.bottom() - SELECTED_DOT);
                painter.circle_filled(dot, SELECTED_DOT, p.accent);
            }
            if response.has_focus() {
                let ring = Stroke::new(FOCUS_RING_WIDTH, p.focus);
                painter.circle_stroke(center, radius - FOCUS_RING_WIDTH / 2.0, ring);
            }
        }

        let label = self.label;
        response.on_hover_ui(|ui| {
            ui.label(label);
        })
    }
}
