//! The Settings page: General, Playback, Library, Advanced and About, each a card of rows, and
//! the dialog that confirms a reset, a folder's removal or clearing the artwork cache.

mod library;
mod playback;

use aria_core::{
    i18n::Localizer,
    settings::{Language, LogLevel, Rgb, Settings, Theme},
};
use aria_library::FolderId;
use eframe::egui::{
    Align, Button, Checkbox, Color32, ComboBox, Direction, Frame, Id, Label, Layout, Margin, Rect,
    RichText, ScrollArea, Separator, Shape, Sides, Stroke, StrokeKind, TextStyle, Ui, WidgetInfo,
    accesskit::Role, epaint::RectShape, vec2,
};

use super::{PageAction, PageCtx};
use crate::{
    nav::Route,
    ui::{
        theme::{
            Palette, fonts,
            visuals::{SPACE, TITLE_SIZE},
        },
        widgets::{Confirm, confirm_dialog, section_header},
    },
};

/// Aria's version, from its manifest.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Aria's license, from its manifest: the SPDX expression `MIT OR Apache-2.0`.
const LICENSE: &str = env!("CARGO_PKG_LICENSE");

/// Widest the cards get, so a row stays easy to scan in a wide window.
const MAX_WIDTH: f32 = 640.0;
/// Below this card width, a row's controls go under its label instead of beside it.
const STACK_BELOW: f32 = 420.0;
/// Height of a row.
const ROW_HEIGHT: f32 = 40.0;
/// Corner radius of a card, and of the theme switch.
const CARD_RADIUS: u8 = 8;
/// Gap around and between the theme switch's buttons.
const SEGMENT_GAP: i8 = 2;
/// Width of the theme switch's outline.
const TRACK_STROKE: f32 = 1.0;
/// Width of a combo box.
const COMBO_WIDTH: f32 = 180.0;
/// Id of the confirmation dialog.
const DIALOG_ID: &str = "aria_reset_dialog";
/// Where the open dialog's kind is kept between frames.
const DIALOG_KIND_ID: &str = "aria_settings_dialog_kind";

/// What the confirmation dialog asks about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialog {
    /// Restore the default settings.
    Reset,
    /// Take a folder out of the library.
    RemoveFolder(FolderId),
    /// Delete the saved artwork thumbnails.
    ClearArtwork,
    /// Forget every recorded play.
    ClearHistory,
}

/// Opens the confirmation dialog for `dialog`.
fn open_dialog(ui: &Ui, modal_open: &mut bool, dialog: Dialog) {
    ui.data_mut(|data| data.insert_temp(Id::unique(DIALOG_KIND_ID), dialog));
    *modal_open = true;
}

/// The theme choices, in the order shown.
const THEMES: [Theme; 3] = [Theme::Dark, Theme::Light, Theme::System];
/// The language choices: the OS language, then each language by its own name.
const LANGUAGES: [Option<Language>; 4] = [
    None,
    Some(Language::En),
    Some(Language::Ru),
    Some(Language::Hy),
];
/// The log levels, least verbose first.
const LOG_LEVELS: [LogLevel; 5] = [
    LogLevel::Error,
    LogLevel::Warn,
    LogLevel::Info,
    LogLevel::Debug,
    LogLevel::Trace,
];

/// Draws the page, and the reset dialog while it is open.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    section_header(ui, &ctx.tr.tr(Route::Settings.label_key()));
    ui.add_space(SPACE[3]);

    let mut settings = ctx.settings.clone();
    ScrollArea::vertical()
        .id_salt("aria_settings")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // `set_max_width` can also widen a `Ui`: never past the page.
            ui.set_max_width(ui.available_width().min(MAX_WIDTH));
            general(ui, ctx.tr, &ctx.palette, &mut settings);
            playback::show(ui, ctx, &mut settings);
            library::show(ui, ctx);
            advanced(ui, ctx, &mut settings.log_level);
            about(ui, ctx.tr, &ctx.palette);
        });
    if settings != *ctx.settings {
        ctx.out.push(PageAction::UpdateSettings(settings));
    }

    if *ctx.modal_open {
        settings_dialog(ui, ctx);
    }
}

/// Language, theme, accents and the tray.
fn general(ui: &mut Ui, tr: &Localizer, p: &Palette, s: &mut Settings) {
    group(ui, p, &tr.tr("settings-general"), |ui| {
        let language = tr.tr("settings-language");
        row(ui, p, &language, |ui| {
            language_combo(ui, tr, &language, &mut s.language);
        });
        divider(ui);
        row(ui, p, &tr.tr("settings-theme"), |ui| {
            theme_switch(ui, tr, p, &mut s.theme);
        });
        divider(ui);
        let dark = Palette::dark().accent;
        accent_row(ui, tr, p, "settings-accent-dark", dark, &mut s.accent_dark);
        divider(ui);
        let light = Palette::light().accent;
        accent_row(
            ui,
            tr,
            p,
            "settings-accent-light",
            light,
            &mut s.accent_light,
        );
        divider(ui);
        let tray = tr.tr("settings-minimize-to-tray");
        let hint = tr.tr("settings-minimize-to-tray-hint");
        row(ui, p, &tray, |ui| {
            let checkbox = Checkbox::without_text(&mut s.minimize_to_tray);
            let response = ui.add(checkbox).accessible_name(&*tray);
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_description(&*hint));
        });
        caption(ui, p, &hint, TextStyle::Small);
    });
}

/// The log level, the folders and the reset.
fn advanced(ui: &mut Ui, ctx: &mut PageCtx<'_>, log_level: &mut LogLevel) {
    let tr = ctx.tr;
    let p = ctx.palette;
    group(ui, &p, &tr.tr("settings-advanced"), |ui| {
        let label = tr.tr("settings-log-level");
        row(ui, &p, &label, |ui| {
            log_level_combo(ui, tr, &label, log_level)
        });
        divider(ui);
        buttons_row(ui, |ui| {
            let folders = [
                ("settings-open-config", PageAction::OpenConfigDir),
                ("settings-open-logs", PageAction::OpenLogDir),
            ];
            for (key, action) in folders {
                let button = Button::new(tr.tr(key));
                if ui.add_enabled(ctx.paths_available, button).clicked() {
                    ctx.out.push(action);
                }
            }
        });
        divider(ui);
        buttons_row(ui, |ui| {
            if ui.button(tr.tr("settings-reset")).clicked() {
                open_dialog(ui, ctx.modal_open, Dialog::Reset);
            }
        });
    });
}

/// Version, license and third-party notices.
fn about(ui: &mut Ui, tr: &Localizer, p: &Palette) {
    group(ui, p, &tr.tr("settings-about"), |ui| {
        for (key, value) in [("settings-version", VERSION), ("settings-license", LICENSE)] {
            row(ui, p, &tr.tr(key), |ui| {
                let text = RichText::new(value).color(p.text_muted);
                ui.add(Label::new(text).selectable(false));
            });
            divider(ui);
        }
        ui.add_space(SPACE[2]);
        caption(ui, p, &tr.tr("settings-third-party"), TextStyle::Body);
    });
}

/// The language combo box, named `name` for assistive technology.
fn language_combo(ui: &mut Ui, tr: &Localizer, name: &str, language: &mut Option<Language>) {
    let text = |choice: Option<Language>| {
        let key = match choice {
            None => "settings-language-system",
            Some(Language::En) => "lang-en",
            Some(Language::Ru) => "lang-ru",
            Some(Language::Hy) => "lang-hy",
        };
        tr.tr(key).into_owned()
    };
    ComboBox::from_id_salt("aria_language")
        .width(COMBO_WIDTH)
        .selected_text(text(*language))
        .show_ui(ui, |ui| {
            for choice in LANGUAGES {
                ui.selectable_value(language, choice, text(choice));
            }
        })
        .response
        .accessible_name(name);
}

/// The log level combo box, named `name` for assistive technology.
fn log_level_combo(ui: &mut Ui, tr: &Localizer, name: &str, level: &mut LogLevel) {
    let text = |level: LogLevel| {
        let key = match level {
            LogLevel::Error => "settings-log-level-error",
            LogLevel::Warn => "settings-log-level-warn",
            LogLevel::Info => "settings-log-level-info",
            LogLevel::Debug => "settings-log-level-debug",
            LogLevel::Trace => "settings-log-level-trace",
        };
        tr.tr(key).into_owned()
    };
    ComboBox::from_id_salt("aria_log_level")
        .width(COMBO_WIDTH)
        .selected_text(text(*level))
        .show_ui(ui, |ui| {
            for choice in LOG_LEVELS {
                ui.selectable_value(level, choice, text(choice));
            }
        })
        .response
        .accessible_name(name);
}

/// Dark, Light and System side by side, the current one filled with the accent.
fn theme_switch(ui: &mut Ui, tr: &Localizer, p: &Palette, theme: &mut Theme) {
    let mut choices = THEMES;
    if ui.layout().main_dir() == Direction::RightToLeft {
        // Added from the right edge: reverse them, so they still read left to right.
        choices.reverse();
    }
    // The track is painted behind the buttons once their size is known. A frame around them
    // would take the row's full height, since the row centers its items vertically.
    let track = ui.painter().add(Shape::Noop);
    let gap = f32::from(SEGMENT_GAP);
    let mut buttons = Rect::NOTHING;
    ui.scope(|ui| {
        let spacing = ui.spacing_mut();
        spacing.item_spacing.x = gap;
        // Shorter buttons, so the track is as tall as a plain button.
        spacing.button_padding.y = (spacing.button_padding.y - gap - TRACK_STROKE).max(0.0);
        for choice in choices {
            let key = match choice {
                Theme::Dark => "settings-theme-dark",
                Theme::Light => "settings-theme-light",
                Theme::System => "settings-theme-system",
            };
            let response = ui.selectable_value(theme, choice, tr.tr(key).into_owned());
            buttons = buttons.union(response.rect);
        }
    });
    let shape = RectShape::new(
        buttons.expand(gap + TRACK_STROKE),
        CARD_RADIUS,
        p.bg,
        Stroke::new(TRACK_STROKE, p.border),
        StrokeKind::Inside,
    );
    ui.painter().set(track, shape);
}

/// The row for one theme's accent: a color well, which shows `default` while there is no
/// override, and a button that removes the override.
fn accent_row(
    ui: &mut Ui,
    tr: &Localizer,
    p: &Palette,
    key: &str,
    default: Color32,
    accent: &mut Option<Rgb>,
) {
    let label = tr.tr(key);
    row(ui, p, &label, |ui| {
        let mut srgb = accent.map_or([default.r(), default.g(), default.b()], |c| [c.r, c.g, c.b]);
        // As tall as the button beside it.
        let padding = ui.spacing().button_padding.y;
        ui.spacing_mut().interact_size.y = ui.text_style_height(&TextStyle::Button) + 2.0 * padding;
        let well = ui
            .color_edit_button_srgb(&mut srgb)
            .accessible_name(&*label);
        if well.changed() {
            let [r, g, b] = srgb;
            *accent = Some(Rgb { r, g, b });
        }
        let reset = Button::new(tr.tr("settings-accent-reset"));
        let reset = ui.add_enabled(accent.is_some(), reset);
        // Both accent rows have this button: say which accent it resets.
        ui.ctx()
            .accesskit_node_builder(reset.id, |node| node.set_description(&*label));
        if reset.clicked() {
            *accent = None;
        }
    });
}

/// A titled card of rows.
fn group(ui: &mut Ui, p: &Palette, title_text: &str, add: impl FnOnce(&mut Ui)) {
    title(ui, p, title_text);
    ui.add_space(SPACE[1]);
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::symmetric(SPACE[3] as i8, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
    ui.add_space(SPACE[4]);
}

/// A row of a card: `label` on the left, and the controls `add` lays out from the right
/// edge. On a narrow card the controls go under the label instead, from the left.
fn row(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui)) {
    let text = RichText::new(label).color(p.text);
    if ui.available_width() < STACK_BELOW {
        ui.add_space(SPACE[1]);
        ui.add(Label::new(text).selectable(false).wrap());
        ui.add_space(SPACE[0]);
        ui.horizontal_wrapped(add);
        ui.add_space(SPACE[1]);
        return;
    }
    // `Sides` takes only the height of what it holds; the frame keeps every row as tall.
    let size = vec2(ui.available_width(), ROW_HEIGHT);
    ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
        ui.set_min_height(ROW_HEIGHT);
        Sides::new()
            .height(ROW_HEIGHT)
            .spacing(SPACE[3])
            .shrink_left()
            .truncate()
            .show(
                ui,
                |ui| {
                    ui.add(Label::new(text).selectable(false));
                },
                add,
            );
    });
}

/// A row of buttons, from the left; they wrap on a narrow card.
fn buttons_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(SPACE[1]);
    ui.horizontal_wrapped(add);
    ui.add_space(SPACE[1]);
}

/// Muted text across the card, under a row.
fn caption(ui: &mut Ui, p: &Palette, text: &str, style: TextStyle) {
    let text = RichText::new(text).color(p.text_muted).text_style(style);
    ui.add(Label::new(text).selectable(false).wrap());
    ui.add_space(SPACE[2]);
}

/// The line between two rows.
fn divider(ui: &mut Ui) {
    ui.add(Separator::default().spacing(1.0));
}

/// A card or dialog title, which assistive technology sees as a heading.
fn title(ui: &mut Ui, p: &Palette, text: &str) {
    let rich = RichText::new(text)
        .font(fonts::semibold(TITLE_SIZE))
        .color(p.text);
    let response = ui.add(Label::new(rich).selectable(false).wrap());
    response.widget_info(|| WidgetInfo::labeled(Role::Heading, true, text));
}

/// The title and body keys of the dialog asking about `dialog`.
fn dialog_text(dialog: Dialog) -> (&'static str, &'static str) {
    match dialog {
        Dialog::Reset => (
            "settings-reset-confirm-title",
            "settings-reset-confirm-body",
        ),
        Dialog::RemoveFolder(_) => (
            "settings-remove-folder-title",
            "settings-remove-folder-body",
        ),
        Dialog::ClearArtwork => (
            "settings-clear-artwork-title",
            "settings-clear-artwork-body",
        ),
        Dialog::ClearHistory => (
            "settings-history-clear-confirm",
            "settings-history-clear-body",
        ),
    }
}

/// Asks before a change that cannot be taken back. Cancel, Escape and a click outside close
/// it; Confirm closes it and makes the request.
fn settings_dialog(ui: &Ui, ctx: &mut PageCtx<'_>) {
    let tr = ctx.tr;
    let p = ctx.palette;
    // Without a kind there is nothing to confirm, and guessing could do the wrong thing: close.
    let Some(dialog) = ui.data(|data| data.get_temp::<Dialog>(Id::unique(DIALOG_KIND_ID))) else {
        *ctx.modal_open = false;
        return;
    };
    // A folder that has left the library since the dialog opened has nothing to remove.
    if let Dialog::RemoveFolder(id) = dialog
        && !ctx.library.snapshot.folders.iter().any(|f| f.id == id)
    {
        *ctx.modal_open = false;
        ui.data_mut(|data| data.remove::<Dialog>(Id::unique(DIALOG_KIND_ID)));
        return;
    }
    let (title_key, body_key) = dialog_text(dialog);
    let text = Confirm {
        title: &tr.tr(title_key),
        body: &tr.tr(body_key),
        confirm: &tr.tr("settings-confirm"),
        cancel: &tr.tr("settings-cancel"),
    };
    let confirmed = confirm_dialog(ui, &p, Id::unique(DIALOG_ID), &text, |ui| {
        if let Dialog::RemoveFolder(id) = dialog
            && let Some(folder) = ctx.library.snapshot.folders.iter().find(|f| f.id == id)
        {
            // The user's own folder list, where paths are shown.
            let path = RichText::new(folder.path.display().to_string()).color(p.text);
            ui.add_space(SPACE[2]);
            ui.add(Label::new(path).selectable(false).wrap());
        }
    });
    if confirmed.is_some() {
        *ctx.modal_open = false;
        ui.data_mut(|data| data.remove::<Dialog>(Id::unique(DIALOG_KIND_ID)));
    }
    if confirmed == Some(true) {
        ctx.out.push(match dialog {
            Dialog::Reset => PageAction::ResetSettings,
            Dialog::RemoveFolder(id) => PageAction::RemoveFolder(id),
            Dialog::ClearArtwork => PageAction::ClearArtwork,
            Dialog::ClearHistory => PageAction::ClearHistory,
        });
    }
}

#[cfg(test)]
mod tests {
    use aria_library::Collections;
    use eframe::egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;

    use super::{
        super::{
            LibraryView,
            test_support::{World, harness},
        },
        *,
    };

    fn page() -> egui_kittest::Harness<'static, World> {
        let world = World::new(Collections::default(), LibraryView::default());
        harness(world, |ui, world| world.page(ui, show))
    }

    #[test]
    fn the_history_switch_updates_settings() {
        let mut harness = page();
        let label = harness.state().english("settings-history");
        harness
            .get_by_role_and_label(Role::CheckBox, &label)
            .click();
        harness.run();
        let [PageAction::UpdateSettings(settings)] = harness.state().out.as_slice() else {
            panic!("expected one UpdateSettings, got {:?}", harness.state().out);
        };
        assert!(!settings.history_enabled);
    }

    #[test]
    fn clear_history_asks_first() {
        let mut harness = page();
        let clear = harness.state().english("settings-history-clear");
        harness.get_by_role_and_label(Role::Button, &clear).click();
        harness.run();
        assert!(harness.state().out.is_empty(), "cleared without asking");
        assert!(harness.state().modal_open);

        let confirm = harness.state().english("settings-confirm");
        harness
            .get_by_role_and_label(Role::Button, &confirm)
            .click();
        harness.run();
        assert_eq!(harness.state().out, [PageAction::ClearHistory]);
        assert!(!harness.state().modal_open);
    }

    #[test]
    fn cancelling_clear_history_sends_nothing() {
        let mut harness = page();
        let clear = harness.state().english("settings-history-clear");
        harness.get_by_role_and_label(Role::Button, &clear).click();
        harness.run();
        let cancel = harness.state().english("settings-cancel");
        harness.get_by_role_and_label(Role::Button, &cancel).click();
        harness.run();
        assert!(harness.state().out.is_empty());
        assert!(!harness.state().modal_open);
    }
}
