//! Color tokens. The only place color literals are allowed.

use aria_core::settings::{Rgb, Settings};
use eframe::egui::{self, Color32};

/// Semantic color tokens for one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub surface_raised: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub focus: Color32,
    pub danger: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Share of the way `accent_hover` moves toward white (dark) or black (light).
const HOVER_MIX: f32 = 0.12;

impl Palette {
    /// Base dark palette.
    pub const fn dark() -> Self {
        let accent = rgb(0x2E, 0xC4, 0xB6);
        Self {
            bg: rgb(0x12, 0x14, 0x17),
            surface: rgb(0x1A, 0x1D, 0x21),
            surface_raised: rgb(0x23, 0x27, 0x2D),
            border: rgb(0x2E, 0x33, 0x3A),
            text: rgb(0xE8, 0xEA, 0xED),
            text_muted: rgb(0x9A, 0xA0, 0xA6),
            accent,
            accent_hover: accent,
            focus: accent,
            danger: rgb(0xE5, 0x48, 0x4D),
        }
    }

    /// Base light palette.
    pub const fn light() -> Self {
        let accent = rgb(0x13, 0x89, 0x7F);
        Self {
            bg: rgb(0xF7, 0xF8, 0xFA),
            surface: rgb(0xFF, 0xFF, 0xFF),
            surface_raised: rgb(0xEE, 0xF0, 0xF3),
            border: rgb(0xD9, 0xDD, 0xE3),
            text: rgb(0x16, 0x18, 0x1C),
            text_muted: rgb(0x5F, 0x66, 0x70),
            accent,
            accent_hover: accent,
            focus: accent,
            danger: rgb(0xC6, 0x28, 0x28),
        }
    }

    /// Applies an optional accent override and derives `accent_hover` and `focus`.
    ///
    /// Dark versus light is decided by the relative luminance of `bg` (below 0.5 is dark).
    /// Hover mixes 12% toward white in dark mode and 12% toward black in light mode.
    pub fn with_accent(mut self, accent: Option<Rgb>) -> Self {
        if let Some(a) = accent {
            self.accent = rgb(a.r, a.g, a.b);
        }
        let target = if luminance(self.bg) < 0.5 {
            Color32::WHITE
        } else {
            Color32::BLACK
        };
        self.accent_hover = mix(self.accent, target, HOVER_MIX);
        self.focus = self.accent;
        self
    }

    /// Palette for a theme with the user's per-mode accent override applied.
    pub fn for_theme(theme: egui::Theme, s: &Settings) -> Self {
        match theme {
            egui::Theme::Dark => Self::dark().with_accent(s.accent_dark),
            egui::Theme::Light => Self::light().with_accent(s.accent_light),
        }
    }

    /// Black or white, whichever contrasts more with `accent`.
    pub fn on_accent(&self) -> Color32 {
        if contrast_ratio(Color32::BLACK, self.accent)
            >= contrast_ratio(Color32::WHITE, self.accent)
        {
            Color32::BLACK
        } else {
            Color32::WHITE
        }
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let ch = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

/// WCAG relative luminance of an sRGB color.
fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio between two colors (1.0 to 21.0).
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::visuals::visuals;

    #[test]
    fn text_contrast_meets_aa() {
        for p in [Palette::dark(), Palette::light()] {
            assert!(contrast_ratio(p.text, p.bg) >= 7.0);
            assert!(contrast_ratio(p.text_muted, p.bg) >= 4.5);
            assert!(contrast_ratio(p.text, p.surface) >= 4.5);
        }
    }
    #[test]
    fn accent_is_visible_against_bg() {
        for p in [Palette::dark(), Palette::light()] {
            assert!(contrast_ratio(p.accent, p.bg) >= 3.0);
        }
    }
    #[test]
    fn on_accent_is_readable() {
        for p in [Palette::dark(), Palette::light()] {
            assert!(contrast_ratio(p.on_accent(), p.accent) >= 4.5);
        }
    }
    #[test]
    fn accent_override_applies_per_mode() {
        let s = Settings {
            accent_dark: Some(Rgb {
                r: 255,
                g: 0,
                b: 128,
            }),
            ..Settings::default()
        };
        assert_eq!(
            Palette::for_theme(egui::Theme::Dark, &s).accent,
            Color32::from_rgb(255, 0, 128)
        );
        assert_eq!(
            Palette::for_theme(egui::Theme::Light, &s).accent,
            Palette::light().accent
        );
    }
    #[test]
    fn hover_moves_away_from_bg_direction() {
        let d = Palette::dark().with_accent(None);
        assert!(luminance(d.accent_hover) > luminance(d.accent));
        let l = Palette::light().with_accent(None);
        assert!(luminance(l.accent_hover) < luminance(l.accent));
    }
    #[test]
    fn visuals_follow_palette() {
        let v = visuals(&Palette::dark(), true);
        assert!(v.dark_mode);
        assert_eq!(v.panel_fill, Palette::dark().surface);
        assert_eq!(v.window_fill, Palette::dark().surface_raised);
    }
    #[test]
    fn text_follows_widget_state() {
        for (p, dark) in [(Palette::dark(), true), (Palette::light(), false)] {
            let p = p.with_accent(None);
            let v = visuals(&p, dark);
            assert!(v.override_text_color.is_none());
            assert_eq!(v.widgets.active.fg_stroke.color, p.on_accent());
            assert_eq!(v.widgets.noninteractive.fg_stroke.color, p.text_muted);
        }
    }
}
