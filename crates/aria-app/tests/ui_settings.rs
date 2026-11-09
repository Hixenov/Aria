//! Headless tests of the Settings page, the reset dialog, toasts and the shortcuts the dialog
//! suspends, driven through AccessKit queries.
//!
//! Page tests start on Home and reach Settings with Ctrl+9, as a user would.

use std::{path::Path, sync::Arc, time::Duration};

use aria_app::{
    app::{AppDeps, AriaApp},
    nav::Route,
    ui::{
        pages::{self, LibraryView, PageAction, PageCtx},
        theme::{Palette, fonts},
        widgets::Toasts,
    },
};
use aria_core::{
    DeviceId, DeviceInfo, FakeFactory, OutputFormat,
    i18n::{Localizer, os_language},
    notice::{Notice, NoticeLevel},
    paths::AppPaths,
    runtime::{Backend, BackendConfig, BackendServices, Waker},
    settings::{Language, LogLevel, Rgb, Settings, Theme, store::SettingsStore},
};
use aria_library::{Folder, FolderId, LibrarySnapshot};
use eframe::egui::{self, Key, Modifiers, PointerButton, ViewportId, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT as _, Queryable as _},
};

/// The default window size, tall enough that every card of the Settings page is on screen.
const SIZE: egui::Vec2 = egui::vec2(1200.0, 1400.0);
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
    Notice::new(level, key)
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

// Persistence through the real backend and settings file.

/// The Settings page, driven by a real backend that saves to `file`.
fn persisting_settings_page(file: &Path) -> Harness<'static, AriaApp> {
    let waker: Waker = Arc::new(|| {});
    let store = SettingsStore::new(file.to_path_buf());
    let services = BackendServices {
        library: None,
        output: Box::new(FakeFactory::new(OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        })),
        settings: Settings::default(),
    };
    let backend = Backend::start(
        Some(Box::new(store)),
        services,
        waker,
        BackendConfig::default(),
    )
    .unwrap_or_else(|err| panic!("the backend did not start: {err}"));
    let deps = AppDeps {
        backend: Some(backend),
        ..AppDeps::none()
    };
    settings_page_with(english(), deps)
}

/// Closes the app as eframe does, which stops the backend once it has saved.
fn exit(harness: &mut Harness<'_, AriaApp>) {
    eframe::App::on_exit(harness.state_mut(), None);
}

#[test]
fn chosen_theme_is_on_disk_after_exit() {
    let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let file = dir.path().join("settings.toml");
    let mut harness = persisting_settings_page(&file);
    click_button(&mut harness, &en("settings-theme-dark"));
    exit(&mut harness);

    let loaded = SettingsStore::new(file).load();
    assert_eq!(loaded.settings.theme, Theme::Dark);
    assert_eq!(loaded.settings.language, Some(Language::En));
    assert_eq!(loaded.notice, None);
}

#[test]
fn reset_defaults_are_on_disk_after_exit() {
    let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let file = dir.path().join("settings.toml");
    // A file that differs from the defaults, so only a written reset can restore them.
    let stored = Settings {
        theme: Theme::Light,
        log_level: LogLevel::Debug,
        ..english()
    };
    SettingsStore::new(file.clone())
        .save(&stored)
        .unwrap_or_else(|err| panic!("seed the settings file: {err}"));

    let mut harness = persisting_settings_page(&file);
    click_button(&mut harness, &en("settings-theme-dark"));
    click_button(&mut harness, &en("settings-reset"));
    click_button(&mut harness, &en("settings-confirm"));
    exit(&mut harness);

    let loaded = SettingsStore::new(file).load();
    assert_eq!(loaded.settings, Settings::default());
    assert_eq!(loaded.notice, None);
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
fn notice_args_reach_the_translation() {
    let skipped = notice(NoticeLevel::Warning, "library-scan-skipped").with_arg("count", "42");
    let harness = app_harness(english(), AppDeps::none(), vec![skipped]);
    assert!(
        harness
            .query_by_label("42 files or folders couldn't be read and were skipped.")
            .is_some()
    );
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

#[test]
fn narrowest_russian_theme_switch_stays_on_the_page() {
    // The narrowest page: the smallest window, the widest sidebar and the queue open. The
    // Russian theme names do not fit on one line there, so the switch wraps.
    let settings = Settings {
        right_panel_open: true,
        sidebar_width: aria_core::settings::SIDEBAR_MAX,
        language: Some(Language::Ru),
        ..Default::default()
    };
    let app = AriaApp::new(settings, localizer(Language::Ru), AppDeps::none(), vec![]);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num9);
    harness.run();

    let ru = localizer(Language::Ru);
    let page_left = aria_core::settings::SIDEBAR_MAX;
    let page_right = 900.0 - aria_app::ui::layout::right_panel::WIDTH;
    let rects: Vec<egui::Rect> = [
        "settings-theme-dark",
        "settings-theme-light",
        "settings-theme-system",
    ]
    .iter()
    .map(|key| {
        harness
            .get_by_role_and_label(Role::Button, &ru.tr(key))
            .rect()
    })
    .collect();
    for (n, rect) in rects.iter().enumerate() {
        assert!(
            rect.min.x >= page_left && rect.max.x <= page_right,
            "theme button {n} at {rect:?} leaves the page {page_left}..{page_right}"
        );
        for other in &rects[n + 1..] {
            let overlap = rect.intersect(*other);
            assert!(
                overlap.width() <= 0.0 || overlap.height() <= 0.0,
                "{rect:?} overlaps {other:?}"
            );
        }
    }
}

// Playback and Library sections, drawn on their own with the devices and library handed in:
// what the page asks of the app is read from the actions it leaves behind.

/// Tall enough that no card needs scrolling to.
const TALL: egui::Vec2 = egui::vec2(1000.0, 1600.0);

/// What the page is given, and what it left behind.
struct Page {
    tr: Localizer,
    settings: Settings,
    modal_open: bool,
    devices: Vec<DeviceInfo>,
    library: LibraryView,
    actions: Vec<PageAction>,
}

fn device(id: &str, name: &str) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.into()),
        name: name.into(),
        is_default: false,
    }
}

fn folder(id: i64, path: &str, available: bool) -> Folder {
    Folder {
        id: FolderId(id),
        path: path.into(),
        available,
    }
}

fn settings_only(
    settings: Settings,
    devices: Vec<DeviceInfo>,
    folders: Vec<Folder>,
) -> Harness<'static, Page> {
    let page = Page {
        tr: localizer(Language::En),
        settings,
        modal_open: false,
        devices,
        library: LibraryView {
            snapshot: Arc::new(LibrarySnapshot {
                folders,
                ..LibrarySnapshot::default()
            }),
            ..LibraryView::default()
        },
        actions: Vec::new(),
    };
    Harness::builder()
        .with_size(TALL)
        .with_context_setup(fonts::install)
        .build_ui_state(
            |ui, page: &mut Page| {
                let mut ctx = PageCtx {
                    tr: &page.tr,
                    palette: Palette::dark(),
                    settings: &page.settings,
                    paths_available: true,
                    modal_open: &mut page.modal_open,
                    out: &mut page.actions,
                    library: &page.library,
                    devices: &page.devices,
                    artwork: None,
                };
                pages::show(Route::Settings, ui, &mut ctx);
            },
            page,
        )
}

/// The settings the page asked to save last, if it asked.
fn saved(harness: &Harness<'_, Page>) -> Option<Settings> {
    harness
        .state()
        .actions
        .iter()
        .rev()
        .find_map(|action| match action {
            PageAction::UpdateSettings(settings) => Some(settings.clone()),
            _ => None,
        })
}

/// Opens the combo box `combo` and picks `option`, in a page-only harness.
fn choose_in(harness: &mut Harness<'_, Page>, combo: &str, option: &str) {
    harness.get_by_role_and_label(Role::ComboBox, combo).click();
    harness.run();
    harness
        .get_by_role(Role::ListBox)
        .get_by_role_and_label(Role::Button, option)
        .click();
    harness.run();
}

#[test]
fn device_list_starts_with_system_default() {
    let devices = vec![device("a", "Speakers"), device("b", "Headphones")];
    let mut harness = settings_only(english(), devices, vec![]);
    let label = en("settings-output-device");
    harness
        .get_by_role_and_label(Role::ComboBox, &label)
        .click();
    harness.run();
    let options: Vec<String> = harness
        .get_by_role(Role::ListBox)
        .query_all_by_role(Role::Button)
        .filter_map(|node| node.accesskit_node().label())
        .collect();
    let first = en("settings-device-system-default");
    assert_eq!(options, [first.as_str(), "Speakers", "Headphones"]);
}

#[test]
fn choosing_a_device_updates_settings() {
    let devices = vec![device("a", "Speakers"), device("b", "Headphones")];
    let mut harness = settings_only(english(), devices, vec![]);
    choose_in(&mut harness, &en("settings-output-device"), "Headphones");
    let settings = saved(&harness).unwrap_or_else(|| panic!("no settings update"));
    assert_eq!(settings.output_device, Some(DeviceId("b".into())));
    // And back to the system default.
    harness.state_mut().settings = settings;
    harness.state_mut().actions.clear();
    choose_in(
        &mut harness,
        &en("settings-output-device"),
        &en("settings-device-system-default"),
    );
    let settings = saved(&harness).unwrap_or_else(|| panic!("no settings update"));
    assert_eq!(settings.output_device, None);
}

#[test]
fn missing_saved_device_stays_selected() {
    let settings = Settings {
        output_device: Some(DeviceId("gone-private-id".into())),
        ..english()
    };
    let mut harness = settings_only(settings, vec![device("a", "Speakers")], vec![]);
    let unavailable = en("settings-device-unavailable");
    // Shown as unavailable, never by its id, and drawing it changes nothing.
    let name = en("settings-output-device");
    let combo = harness.get_by_role_and_label(Role::ComboBox, &name);
    assert_eq!(combo.accesskit_node().value(), Some(unavailable.clone()));
    assert!(harness.query_by_label_contains("gone-private-id").is_none());
    assert_eq!(saved(&harness), None);
    // The entry is in the list, and picking it keeps the setting.
    choose_in(&mut harness, &en("settings-output-device"), &unavailable);
    assert_eq!(
        saved(&harness),
        None,
        "keeping the entry changed the setting"
    );
    assert!(harness.query_by_label_contains("gone-private-id").is_none());
}

#[test]
fn gapless_toggle_updates_settings() {
    let mut harness = settings_only(english(), vec![], vec![]);
    let label = en("settings-gapless");
    harness
        .get_by_role_and_label(Role::CheckBox, &label)
        .click();
    harness.run();
    let settings = saved(&harness).unwrap_or_else(|| panic!("no settings update"));
    assert!(!settings.gapless);
}

#[test]
fn folders_are_listed_with_status() {
    let folders = vec![folder(1, "music-a", true), folder(2, "music-b", false)];
    let harness = settings_only(english(), vec![], folders);
    assert!(harness.query_by_label("music-a").is_some());
    assert!(harness.query_by_label("music-b").is_some());
    // Only the folder that cannot be reached carries the warning.
    let warning = en("settings-folder-unavailable");
    assert_eq!(harness.query_all_by_label(&warning).count(), 1);
}

#[test]
fn an_empty_library_still_shows_the_section() {
    let harness = settings_only(english(), vec![], vec![]);
    assert!(
        harness
            .query_by_label(&en("settings-folders-empty"))
            .is_some()
    );
    for key in ["local-add-folder", "local-rescan", "settings-clear-artwork"] {
        assert!(harness.query_by_label(&en(key)).is_some(), "{key}");
    }
}

#[test]
fn remove_folder_asks_first() {
    let mut harness = settings_only(english(), vec![], vec![folder(7, "music-a", true)]);
    click_button(&mut harness, "Remove music-a");
    assert!(harness.state().actions.is_empty(), "removed without asking");
    assert!(harness.state().modal_open);
    // The dialog says the files stay on disk.
    let body = en("settings-remove-folder-body");
    assert!(body.contains("stay on disk"));
    assert!(harness.query_by_label(&body).is_some());
    // Cancel keeps the folder.
    click_button(&mut harness, &en("settings-cancel"));
    assert!(harness.state().actions.is_empty());
    assert!(!harness.state().modal_open);
    // Confirm removes it.
    click_button(&mut harness, "Remove music-a");
    click_button(&mut harness, &en("settings-confirm"));
    assert_eq!(
        harness.state().actions,
        [PageAction::RemoveFolder(FolderId(7))]
    );
    assert!(!harness.state().modal_open);
}

#[test]
fn clear_artwork_asks_first() {
    let mut harness = settings_only(english(), vec![], vec![]);
    click_button(&mut harness, &en("settings-clear-artwork"));
    assert!(harness.state().actions.is_empty(), "cleared without asking");
    let body = en("settings-clear-artwork-body");
    assert!(harness.query_by_label(&body).is_some());
    click_button(&mut harness, &en("settings-confirm"));
    assert_eq!(harness.state().actions, [PageAction::ClearArtwork]);
}

#[test]
fn add_folder_requests_the_dialog() {
    let mut harness = settings_only(english(), vec![], vec![]);
    click_button(&mut harness, &en("local-add-folder"));
    assert_eq!(harness.state().actions, [PageAction::PickFolder]);
}

#[test]
fn rescan_button_sends_rescan() {
    let mut harness = settings_only(english(), vec![], vec![folder(1, "music-a", true)]);
    click_button(&mut harness, &en("local-rescan"));
    assert_eq!(harness.state().actions, [PageAction::Rescan]);
}

#[test]
fn reset_keeps_folders() {
    let mut harness = settings_only(english(), vec![], vec![folder(1, "music-a", true)]);
    click_button(&mut harness, &en("settings-reset"));
    click_button(&mut harness, &en("settings-confirm"));
    // Only the settings are reset: nothing asks for a folder to go, and the list stays.
    assert_eq!(harness.state().actions, [PageAction::ResetSettings]);
    assert!(harness.query_by_label("music-a").is_some());
}

#[test]
fn each_remove_button_names_its_folder() {
    let folders = vec![folder(1, "music-a", true), folder(2, "music-b", true)];
    let harness = settings_only(english(), vec![], folders);
    for name in ["Remove music-a", "Remove music-b"] {
        assert!(
            harness
                .query_by_role_and_label(Role::Button, name)
                .is_some(),
            "{name}"
        );
    }
}

#[test]
fn removal_follows_the_folder_when_the_list_changes_under_the_dialog() {
    let folders = vec![folder(1, "music-a", true), folder(2, "music-b", true)];
    let mut harness = settings_only(english(), vec![], folders);
    click_button(&mut harness, "Remove music-b");
    // A new snapshot arrives with the folders the other way round.
    let swapped = vec![folder(2, "music-b", true), folder(1, "music-a", true)];
    harness.state_mut().library.snapshot = Arc::new(LibrarySnapshot {
        folders: swapped,
        ..LibrarySnapshot::default()
    });
    harness.run();
    click_button(&mut harness, &en("settings-confirm"));
    assert_eq!(
        harness.state().actions,
        [PageAction::RemoveFolder(FolderId(2))]
    );
}

#[test]
fn the_dialog_closes_when_its_folder_is_gone() {
    let mut harness = settings_only(english(), vec![], vec![folder(1, "music-a", true)]);
    click_button(&mut harness, "Remove music-a");
    assert!(harness.state().modal_open);
    harness.state_mut().library.snapshot = Arc::new(LibrarySnapshot::default());
    harness.run();
    assert!(!harness.state().modal_open, "the dialog stayed open");
    assert!(harness.state().actions.is_empty(), "sent a removal");
    assert!(harness.query_by_label(&en("settings-confirm")).is_none());
}

#[test]
fn a_dialog_of_no_known_kind_closes_instead_of_resetting() {
    let mut harness = settings_only(english(), vec![], vec![]);
    harness.state_mut().modal_open = true;
    harness.run();
    assert!(!harness.state().modal_open);
    assert!(harness.query_by_label(&en("settings-confirm")).is_none());
    assert!(harness.state().actions.is_empty());
}

#[test]
fn the_page_scrolls_to_reset_at_a_normal_height() {
    let app = AriaApp::new(english(), localizer(Language::En), AppDeps::none(), vec![]);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 780.0))
        .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num9);
    harness.run();
    let reset = en("settings-reset");
    harness
        .get_by_role_and_label(Role::Button, &reset)
        .scroll_to_me();
    harness.run();
    click_button(&mut harness, &reset);
    let tr = localizer(Language::En);
    assert!(dialog_open(&harness, &tr), "Reset was not reachable");
}
