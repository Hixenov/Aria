//! Turns a [`Palette`] into egui visuals and style.

use aria_core::settings::{Settings, Theme};
use eframe::egui::{
    self, Context, CornerRadius, FontFamily, FontId, Stroke, TextStyle, ThemePreference, Visuals,
};

use super::fonts::SEMIBOLD;
use super::palette::Palette;

/// Spacing scale in points.
pub const SPACE: [f32; 5] = [4.0, 8.0, 12.0, 16.0, 24.0];

/// Corner radius of controls.
const RADIUS_CONTROL: u8 = 6;
/// Corner radius of panels and windows.
const RADIUS_PANEL: u8 = 8;

/// Builds egui visuals from palette tokens only.
pub fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = p.surface;
    v.window_fill = p.surface_raised;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.surface_raised;
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.danger;
    // No dedicated warning token exists, so reuse `danger` instead of adding a color literal.
    v.warn_fg_color = p.danger;
    v.window_corner_radius = CornerRadius::same(RADIUS_PANEL);
    v.menu_corner_radius = CornerRadius::same(RADIUS_PANEL);
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, p.on_accent());
    v.text_cursor.stroke = Stroke::new(2.0, p.focus);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    // Plain labels use primary text; secondary text asks for `text_muted` explicitly.
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);

    w.inactive.bg_fill = p.surface_raised;
    w.inactive.weak_bg_fill = p.surface_raised;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);

    w.hovered.bg_fill = p.surface_raised;
    w.hovered.weak_bg_fill = p.surface_raised;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent_hover);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);

    w.active.bg_fill = p.accent;
    w.active.weak_bg_fill = p.accent;
    w.active.bg_stroke = Stroke::new(1.0, p.accent_hover);
    w.active.fg_stroke = Stroke::new(2.0, p.on_accent());

    w.open.bg_fill = p.surface_raised;
    w.open.weak_bg_fill = p.surface_raised;
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    w.open.fg_stroke = Stroke::new(1.0, p.text);

    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.corner_radius = CornerRadius::same(RADIUS_CONTROL);
    }
    v
}

fn text_styles() -> [(TextStyle, FontId); 5] {
    [
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Name(SEMIBOLD.into())),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ]
}

/// Installs visuals for both themes, the theme preference and the shared style.
///
/// Call at startup and whenever the theme or an accent changes.
pub fn apply(ctx: &Context, s: &Settings) {
    for (theme, dark) in [(egui::Theme::Dark, true), (egui::Theme::Light, false)] {
        ctx.set_visuals_of(theme, visuals(&Palette::for_theme(theme, s), dark));
    }
    ctx.set_theme(match s.theme {
        Theme::Dark => ThemePreference::Dark,
        Theme::Light => ThemePreference::Light,
        Theme::System => ThemePreference::System,
    });
    ctx.all_styles_mut(|style| {
        for (ts, font) in text_styles() {
            style.text_styles.insert(ts, font);
        }
        style.spacing.item_spacing = egui::vec2(SPACE[1], SPACE[0]);
        style.spacing.button_padding = egui::vec2(SPACE[2], SPACE[0]);
        style.spacing.window_margin = egui::Margin::same(SPACE[3] as i8);
    });
}
