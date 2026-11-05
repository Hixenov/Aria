//! Every character the bundled locales use must resolve to a glyph in the bundled fonts.

use aria_app::ui::theme::{
    fonts::{self, FONT_FILES},
    icons,
};
use aria_core::i18n::SOURCES;

fn faces() -> Vec<ttf_parser::Face<'static>> {
    FONT_FILES
        .iter()
        .map(|(name, bytes)| {
            ttf_parser::Face::parse(bytes, 0).unwrap_or_else(|e| panic!("{name}: {e}"))
        })
        .collect()
}

#[test]
fn every_locale_char_has_a_glyph() {
    let faces = faces();
    for (lang, src) in SOURCES {
        for c in src.chars() {
            if c.is_whitespace() || c.is_control() {
                continue;
            }
            assert!(
                faces.iter().any(|f| f.glyph_index(c).is_some()),
                "{lang:?}: no glyph for {c:?} (U+{:04X})",
                c as u32
            );
        }
    }
}

#[test]
fn every_icon_exists_in_phosphor() {
    let faces = faces();
    let (_, bytes) = FONT_FILES
        .iter()
        .find(|(name, _)| *name == "phosphor")
        .expect("phosphor font is bundled");
    let phosphor = ttf_parser::Face::parse(bytes, 0).expect("phosphor parses");
    assert_eq!(faces.len(), FONT_FILES.len());
    for icon in icons::ALL {
        let mut chars = icon.chars();
        let c = chars.next().expect("icon is non-empty");
        assert!(
            chars.next().is_none(),
            "icon {icon:?} must be one codepoint"
        );
        assert!(
            phosphor.glyph_index(c).is_some(),
            "icon U+{:04X} missing from Phosphor",
            c as u32
        );
    }
}

#[test]
fn icons_are_drawn_from_phosphor_alone() {
    // Inter has glyphs in the Private Use Area too, and it leads the Proportional chain, so
    // there it hides some Phosphor icons. The icon family must hold Phosphor alone.
    let families = fonts::definitions().families;
    let icons = &families[&eframe::egui::FontFamily::Name(fonts::ICONS.into())];
    assert_eq!(icons, &["phosphor"]);
    assert_eq!(
        fonts::icon_font(18.0).family,
        eframe::egui::FontFamily::Name(fonts::ICONS.into())
    );
}
