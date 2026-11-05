//! Headless tests of the Settings page, the reset dialog, toasts and the shortcuts the dialog
//! suspends, driven through AccessKit queries.
//!
//! Page tests start on Home and reach Settings with Ctrl+9, as a user would.

use std::time::Duration;

use aria_app::{
    app::{AppDeps, AriaApp},
    nav::Route,
    ui::{
        theme::{Palette, fonts},
        widgets::Toasts,
    },
};
use aria_core::{
    i18n::{Localizer, os_language},
    notice::{Notice, NoticeLevel},
    paths::AppPaths,
    settings::{Language, LogLevel, Rgb, Settings, Theme},
};
use eframe::egui::{self, Key, Modifiers, PointerButton, ViewportId, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT as _, Queryable as _},
};

/// The default window size.
const SIZE: egui::Vec2 = egui::vec2(1200.0, 780.0);
/// AltGr as Windows reports it: Ctrl and Alt together.
const ALTGR: Modifiers = Modifiers {
    alt: true,
    ctrl: true,
    shift: false,
    mac_cmd: false,
    command: true,
};
/// Ctrl alone, as Windows reports it (Ctrl is also the command key there).
const CTRL: Modifiers = Modifiers {
    alt: false,
    ctrl: true,
    shift: false,
    mac_cmd: false,
    command: true,
};

fn english() -> Settings {
    Settings {
        language: Some(Language::En),
        ..Default::default()
    }
}

/// A localizer for `language`. The bundled locales always parse.
fn localizer(language: Language) -> Localizer {
    Localizer::new(language).unwrap_or_else(|err| panic!("{language:?} locale: {err}"))
}

/// The English text of `key`.
fn en(key: &str) -> String {
    localizer(Language::En).tr(key).into_owned()
}

fn app_harness(
    settings: Settings,
    deps: AppDeps,
    notices: Vec<Notice>,
) -> Harness<'static, AriaApp> {
    let language = settings.language.unwrap_or(Language::En);
    let app = AriaApp::new(settings, localizer(language), deps, notices);
    Harness::builder()
        .with_size(SIZE)
        .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app)
}

/// The app with `settings`, moved from Home to the Settings page with Ctrl+9.
fn settings_page(settings: Settings) -> Harness<'static, AriaApp> {
    settings_page_with(settings, AppDeps::none())
}

fn settings_page_with(settings: Settings, deps: AppDeps) -> Harness<'static, AriaApp> {
    let mut harness = app_harness(settings, deps, vec![]);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num9);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Settings);
    harness
}

/// Clicks the button labelled `label` and lets the UI settle.
fn click_button<S>(harness: &mut Harness<'_, S>, label: &str) {
    harness.get_by_role_and_label(Role::Button, label).click();
    harness.run();
}

/// Opens the combo box labelled `combo` and picks its option `option`.
fn choose(harness: &mut Harness<'_, AriaApp>, combo: &str, option: &str) {
    harness.get_by_role_and_label(Role::ComboBox, combo).click();
    harness.run();
    harness
        .get_by_role(Role::ListBox)
        .get_by_role_and_label(Role::Button, option)
        .click();
    harness.run();
}

fn is_disabled(harness: &Harness<'_, AriaApp>, label: &str) -> bool {
    harness
        .get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

/// Whether the reset dialog is on screen, judged by its title in `tr`'s language.
fn dialog_open(harness: &Harness<'_, AriaApp>, tr: &Localizer) -> bool {
    harness
        .query_by_role_and_label(Role::Heading, &tr.tr("settings-reset-confirm-title"))
        .is_some()
}

/// How long after the last frame egui asked to be repainted; `Duration::MAX` means never.
fn repaint_delay<S>(harness: &Harness<'_, S>) -> Duration {
    harness
        .output()
        .viewport_output
        .get(&ViewportId::ROOT)
        .map_or(Duration::MAX, |viewport| viewport.repaint_delay)
}

/// A harness that shows only `toasts`, in English on the dark palette.
fn toast_harness(toasts: Toasts) -> Harness<'static, Toasts> {
    let tr = localizer(Language::En);
    let palette = Palette::dark();
    Harness::builder()
        .with_size(SIZE)
        .with_context_setup(fonts::install)
        .build_ui_state(
            move |ui, toasts: &mut Toasts| toasts.show(ui.ctx(), &tr, &palette),
            toasts,
        )
}

fn notice(level: NoticeLevel, key: &'static str) -> Notice {
    Notice { level, key }
}

// Theme and language.

#[test]
fn choosing_dark_theme_switches_visuals() {
    let mut harness = settings_page(Settings {
        theme: Theme::Light,
        ..english()
    });
    assert_eq!(harness.ctx.theme(), egui::Theme::Light);

    click_button(&mut harness, &en("settings-theme-dark"));
    assert_eq!(harness.ctx.theme(), egui::Theme::Dark);
    assert!(harness.ctx.global_style().visuals.dark_mode);
    assert_eq!(harness.state().settings().theme, Theme::Dark);
}

#[test]
fn choosing_light_theme_switches_visuals() {
    let mut harness = settings_page(Settings {
        theme: Theme::Dark,
        ..english()
    });
    assert_eq!(harness.ctx.theme(), egui::Theme::Dark);

    click_button(&mut harness, &en("settings-theme-light"));
    assert_eq!(harness.ctx.theme(), egui::Theme::Light);
    assert!(!harness.ctx.global_style().visuals.dark_mode);
    assert_eq!(harness.state().settings().theme, Theme::Light);
}

#[test]
fn switching_language_relabels_sidebar() {
    let mut harness = settings_page(english());
    choose(&mut harness, &en("settings-language"), &en("lang-ru"));

    let ru = localizer(Language::Ru);
    assert_eq!(harness.state().settings().language, Some(Language::Ru));
    assert!(
        harness
            .query_by_role_and_label(Role::Button, &en("nav-settings"))
            .is_none(),
        "the sidebar still says Settings"
    );
    assert!(
        harness
            .query_by_role_and_label(Role::Button, &ru.tr("nav-settings"))
            .is_some()
    );
}

#[test]
fn system_language_follows_the_os_at_once() {
    // Start in a language the OS does not use, so the switch is visible.
    let start = if os_language() == Language::Hy {
        Language::En
    } else {
        Language::Hy
    };
    let tr = localizer(start);
    let mut harness = settings_page(Settings {
        language: Some(start),
        ..Default::default()
    });
    // The options read like the theme's System button in some languages, so look in the list.
    choose(
        &mut harness,
        &tr.tr("settings-language"),
        &tr.tr("settings-language-system"),
    );

    assert_eq!(harness.state().settings().language, None);
    let os = localizer(os_language());
    assert!(
        harness
            .query_by_role_and_label(Role::Button, &os.tr("nav-settings"))
            .is_some(),
        "the sidebar is not in the OS language"
    );
}

// Other settings.

#[test]
fn log_level_choice_is_applied() {
    let mut harness = settings_page(english());
    choose(
        &mut harness,
        &en("settings-log-level"),
        &en("settings-log-level-debug"),
    );
    assert_eq!(harness.state().settings().log_level, LogLevel::Debug);
}

#[test]
fn minimize_to_tray_toggles() {
    let mut harness = settings_page(english());
    harness
        .get_by_role_and_label(Role::CheckBox, &en("settings-minimize-to-tray"))
        .click();
    harness.run();
    assert!(harness.state().settings().minimize_to_tray);
}

#[test]
fn accent_reset_returns_to_the_default() {
    let custom = Rgb {
        r: 255,
        g: 0,
        b: 128,
    };
    let mut harness = settings_page(Settings {
        accent_dark: Some(custom),
        ..english()
    });
    // Both color wells are named after their rows.
    for row in ["settings-accent-dark", "settings-accent-light"] {
        assert!(
            harness
                .query_by_role_and_label(Role::ColorWell, &en(row))
                .is_some(),
            "no color well named {row}"
        );
    }

    // The two reset buttons share a label; their description names the row.
    let reset = en("settings-accent-reset");
    let in_row = |harness: &Harness<'_, AriaApp>, row: &str| {
        harness
            .get_all_by_role_and_label(Role::Button, &reset)
            .find(|button| button.accesskit_node().description() == Some(en(row)))
            .map(|button| button.accesskit_node().is_disabled())
    };
    assert_eq!(in_row(&harness, "settings-accent-light"), Some(true));
    assert_eq!(in_row(&harness, "settings-accent-dark"), Some(false));

    harness
        .get_all_by_role_and_label(Role::Button, &reset)
        .find(|button| button.accesskit_node().description() == Some(en("settings-accent-dark")))
        .unwrap_or_else(|| panic!("no reset button for the dark accent"))
        .click();
    harness.run();
    assert_eq!(harness.state().settings().accent_dark, None);
    assert_eq!(in_row(&harness, "settings-accent-dark"), Some(true));
}

#[test]
fn folder_buttons_need_known_paths() {
    let harness = settings_page(english());
    for key in ["settings-open-config", "settings-open-logs"] {
        assert!(is_disabled(&harness, &en(key)), "{key} should be disabled");
    }

    let paths = AppPaths {
        config_dir: "config".into(),
        data_dir: "data".into(),
        cache_dir: "cache".into(),
        log_dir: "logs".into(),
    };
    let deps = AppDeps {
        paths: Some(paths),
        ..AppDeps::none()
    };
    let harness = settings_page_with(english(), deps);
    for key in ["settings-open-config", "settings-open-logs"] {
        assert!(!is_disabled(&harness, &en(key)), "{key} should be enabled");
    }
}

#[test]
fn about_shows_version_and_license() {
    let harness = settings_page(english());
    for text in [
        env!("CARGO_PKG_VERSION").to_owned(),
        "MIT OR Apache-2.0".to_owned(),
        en("settings-third-party"),
    ] {
        assert!(harness.query_by_label(&text).is_some(), "no {text:?}");
    }
}

#[test]
fn narrow_page_keeps_every_control_beside_the_right_panel() {
    // The smallest window with the queue open leaves the page about 330 points wide.
    let settings = Settings {
        right_panel_open: true,
        language: Some(Language::Ru),
        ..Default::default()
    };
    let language = settings.language.unwrap_or(Language::En);
    let app = AriaApp::new(settings, localizer(language), AppDeps::none(), vec![]);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num9);
    harness.run();

    let panel_left = 900.0 - aria_app::ui::layout::right_panel::WIDTH;
    let ru = localizer(Language::Ru);
    let mut controls: Vec<(Role, String)> = [
        "settings-theme-system",
        "settings-open-logs",
        "settings-reset",
    ]
    .iter()
    .map(|key| (Role::Button, ru.tr(key).into_owned()))
    .collect();
    controls.push((Role::ComboBox, ru.tr("settings-language").into_owned()));
    controls.push((Role::ComboBox, ru.tr("settings-log-level").into_owned()));
    controls.push((Role::ColorWell, ru.tr("settings-accent-light").into_owned()));
    controls.push((
        Role::CheckBox,
        ru.tr("settings-minimize-to-tray").into_owned(),
    ));
    for (role, label) in controls {
        let rect = harness.get_by_role_and_label(role, &label).rect();
        assert!(rect.max.x <= panel_left, "{label} reaches {rect:?}");
    }
}

// The reset dialog.

#[test]
fn reset_cancel_keeps_settings() {
    let mut harness = settings_page(english());
    let tr = localizer(Language::En);
    click_button(&mut harness, &en("settings-theme-dark"));
    click_button(&mut harness, &en("settings-reset"));
    assert!(dialog_open(&harness, &tr));
    assert!(
        harness
            .query_by_label(&en("settings-reset-confirm-body"))
            .is_some()
    );

    click_button(&mut harness, &en("settings-cancel"));
    assert!(!dialog_open(&harness, &tr));
    assert_eq!(harness.state().settings().theme, Theme::Dark);
}

#[test]
fn reset_confirm_restores_defaults() {
    let mut harness = settings_page(english());
    click_button(&mut harness, &en("settings-theme-dark"));
    click_button(&mut harness, &en("settings-reset"));
    click_button(&mut harness, &en("settings-confirm"));

    assert_eq!(harness.state().settings(), &Settings::default());
    // The default language follows the OS.
    assert!(!dialog_open(&harness, &localizer(os_language())));
}

#[test]
fn escape_closes_reset_dialog() {
    let mut harness = settings_page(english());
    let tr = localizer(Language::En);
    click_button(&mut harness, &en("settings-reset"));
    assert!(dialog_open(&harness, &tr));

    harness.key_press(Key::Escape);
    harness.run();
    assert!(!dialog_open(&harness, &tr));
    assert_eq!(harness.state().settings(), &english());
}

#[test]
fn reset_dialog_suspends_navigation_and_panel_shortcuts() {
    let mut harness = settings_page(english());
    let tr = localizer(Language::En);
    // Home, Settings, Albums, then back: both history directions now have somewhere to go.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num6);
    harness.run();
    harness.key_press_modifiers(Modifiers::ALT, Key::ArrowLeft);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Settings);

    click_button(&mut harness, &en("settings-reset"));
    assert!(dialog_open(&harness, &tr));
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num1);
    harness.key_press_modifiers(Modifiers::ALT, Key::ArrowLeft);
    harness.key_press_modifiers(Modifiers::ALT, Key::ArrowRight);
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::R);
    harness.run();
    let title = tr.tr("settings-reset-confirm-title");
    for button in [PointerButton::Extra1, PointerButton::Extra2] {
        harness
            .get_by_role_and_label(Role::Heading, &title)
            .click_button(button);
        harness.run();
    }
    assert_eq!(harness.state().current_route(), Route::Settings);
    assert!(!harness.state().settings().right_panel_open);
    assert!(dialog_open(&harness, &tr), "a shortcut closed the dialog");

    // Once the dialog is closed, the shortcuts work again.
    click_button(&mut harness, &en("settings-cancel"));
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num1);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Home);
}

#[test]
fn altgr_r_is_left_for_typing() {
    let mut harness = app_harness(english(), AppDeps::none(), vec![]);
    // Windows reports AltGr as a left Ctrl press followed by the right Alt key.
    harness.key_down_modifiers(CTRL, Key::ControlLeft);
    harness.key_down_modifiers(ALTGR, Key::AltRight);
    harness.key_press_modifiers(ALTGR, Key::R);
    harness.key_up(Key::AltRight);
    harness.key_up(Key::ControlLeft);
    harness.run();
    assert!(
        !harness.state().settings().right_panel_open,
        "AltGr+R toggled the right panel"
    );

    // The left Ctrl and Alt keys still toggle it.
    harness.key_down_modifiers(CTRL, Key::ControlLeft);
    harness.key_down_modifiers(ALTGR, Key::AltLeft);
    harness.key_press_modifiers(ALTGR, Key::R);
    harness.key_up(Key::AltLeft);
    harness.key_up(Key::ControlLeft);
    harness.run();
    assert!(harness.state().settings().right_panel_open);
}

// Toasts.

#[test]
fn startup_notice_shows_toast() {
    let recovered = notice(NoticeLevel::Warning, "notice-settings-recovered");
    let harness = app_harness(english(), AppDeps::none(), vec![recovered]);
    assert!(
        harness
            .query_by_label(&en("notice-settings-recovered"))
            .is_some()
    );
    assert!(harness.state().pending_notices().is_empty());
}

#[test]
fn error_toast_persists_until_dismissed() {
    let mut toasts = Toasts::default();
    toasts.push(
        notice(NoticeLevel::Error, "notice-settings-save-failed"),
        0.0,
    );
    let mut harness = toast_harness(toasts);

    harness.input_mut().time = Some(6.5);
    harness.run();
    assert_eq!(harness.state().len(), 1);
    assert!(
        harness
            .query_by_label(&en("notice-settings-save-failed"))
            .is_some()
    );

    click_button(&mut harness, &en("notice-dismiss"));
    assert_eq!(harness.state().len(), 0);
    assert!(
        harness
            .query_by_label(&en("notice-settings-save-failed"))
            .is_none()
    );
}

#[test]
fn info_toast_expires() {
    let mut toasts = Toasts::default();
    toasts.push(notice(NoticeLevel::Info, "notice-settings-recovered"), 0.0);
    let mut harness = toast_harness(toasts);
    assert_eq!(harness.state().len(), 1);

    harness.input_mut().time = Some(6.5);
    harness.step();
    assert_eq!(harness.state().len(), 0);
}

#[test]
fn toast_timer_repaints_only_while_a_toast_can_expire() {
    let mut toasts = Toasts::default();
    toasts.push(
        notice(NoticeLevel::Warning, "notice-settings-recovered"),
        0.0,
    );
    let mut harness = toast_harness(toasts);
    let delay = repaint_delay(&harness);
    assert!(
        delay > Duration::ZERO && delay <= Duration::from_secs(6),
        "no repaint scheduled for the expiry: {delay:?}"
    );

    harness.input_mut().time = Some(6.5);
    harness.run();
    assert_eq!(harness.state().len(), 0);
    assert_eq!(repaint_delay(&harness), Duration::MAX);

    // An error never expires, so it schedules nothing either.
    let mut toasts = Toasts::default();
    toasts.push(
        notice(NoticeLevel::Error, "notice-settings-save-failed"),
        0.0,
    );
    let harness = toast_harness(toasts);
    assert_eq!(harness.state().len(), 1);
    assert_eq!(repaint_delay(&harness), Duration::MAX);
}

#[test]
fn idle_app_schedules_no_repaints() {
    let harness = settings_page(english());
    assert_eq!(repaint_delay(&harness), Duration::MAX);
}
