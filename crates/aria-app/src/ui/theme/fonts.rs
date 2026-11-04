//! Bundled fonts: Inter (Latin, Cyrillic), Noto Sans Armenian, Phosphor icons.

use std::sync::Arc;

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};

/// Name of the semibold font family (`FontFamily::Name("semibold")`).
pub const SEMIBOLD: &str = "semibold";

/// Embedded font faces as `(name, bytes)`; also used by the glyph coverage test.
pub const FONT_FILES: [(&str, &[u8]); 4] = [
    (
        "inter",
        include_bytes!("../../../../../assets/fonts/Inter-Regular.ttf"),
    ),
    (
        "inter-semibold",
        include_bytes!("../../../../../assets/fonts/Inter-SemiBold.ttf"),
    ),
    (
        "noto-armenian",
        include_bytes!("../../../../../assets/fonts/NotoSansArmenian-Regular.ttf"),
    ),
    (
        "phosphor",
        include_bytes!("../../../../../assets/fonts/Phosphor.ttf"),
    ),
];

/// Font definitions with Inter, Noto Armenian and Phosphor at the front of Proportional.
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in FONT_FILES {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    for (i, name) in ["inter", "noto-armenian", "phosphor"]
        .into_iter()
        .enumerate()
    {
        proportional.insert(i, name.to_owned());
    }
    fonts.families.insert(
        FontFamily::Name(SEMIBOLD.into()),
        ["inter-semibold", "noto-armenian", "phosphor"]
            .map(str::to_owned)
            .to_vec(),
    );
    fonts
}

/// Installs the font chain on the context.
pub fn install(ctx: &Context) {
    ctx.set_fonts(definitions());
}
