//! Headless tests of the app shell (sidebar, shortcuts, player bar, right panel), driven
//! through AccessKit queries.
//!
//! A page heading and its sidebar entry share a label, so queries name the role:
//! sidebar entries are buttons and page headings are headings.

use aria_app::{
    app::{AppDeps, AriaApp},
    nav::Route,
};
use aria_core::{
    i18n::{FluentArgs, Localizer},
    settings::{Language, SIDEBAR_MAX, SIDEBAR_MIN, Settings, Theme},
};
use eframe::egui::{self, Key, Modifiers, PointerButton, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT as _, Queryable as _},
};

/// The default window size.
const SIZE: egui::Vec2 = egui::vec2(1200.0, 780.0);
/// The minimum window size.
const MIN_SIZE: egui::Vec2 = egui::vec2(900.0, 600.0);

fn english() -> Settings {
    Settings {
        language: Some(Language::En),
        ..Default::default()
    }
}

fn harness(size: egui::Vec2) -> Harness<'static, AriaApp> {
    harness_with(size, english())
}

fn harness_with(size: egui::Vec2, settings: Settings) -> Harness<'static, AriaApp> {
    let language = settings.language.unwrap_or(Language::En);
    let app = AriaApp::new(settings, localizer(language), AppDeps::none(), vec![]);
    Harness::builder()
        .with_size(size)
        .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app)
}

/// A localizer for `language`. The bundled locales always parse.
fn localizer(language: Language) -> Localizer {
    Localizer::new(language).unwrap_or_else(|err| panic!("{language:?} locale: {err}"))
}

/// The English text of `key`.
fn en(key: &str) -> String {
    localizer(Language::En).tr(key).into_owned()
}

/// The English text of `key` with its `$phase` placeable filled.
fn en_phase(key: &str, phase: u8) -> String {
    let mut args = FluentArgs::new();
    args.set("phase", phase);
    localizer(Language::En).tr_args(key, &args).into_owned()
}

/// Clicks the button labelled `label` and lets the UI settle.
fn click_button(harness: &mut Harness<'_, AriaApp>, label: &str) {
    harness.get_by_role_and_label(Role::Button, label).click();
    harness.run();
}

fn button_rect(harness: &Harness<'_, AriaApp>, label: &str) -> egui::Rect {
    harness.get_by_role_and_label(Role::Button, label).rect()
}

fn is_disabled(harness: &Harness<'_, AriaApp>, label: &str) -> bool {
    harness
        .get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

const DIGITS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

#[test]
fn every_sidebar_entry_changes_heading() {
    let mut harness = harness(SIZE);
    for route in Route::ALL {
        let label = en(route.label_key());
        click_button(&mut harness, &label);
        assert_eq!(harness.state().current_route(), route);
        assert!(
            harness
                .query_by_role_and_label(Role::Heading, &label)
                .is_some(),
            "no {label:?} heading on {route:?}"
        );
    }
}

#[test]
fn ctrl_number_jumps_to_route() {
    let mut harness = harness(SIZE);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num9);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Settings);

    for (route, key) in Route::ALL.into_iter().zip(DIGITS) {
        harness.key_press_modifiers(Modifiers::COMMAND, key);
        harness.run();
        assert_eq!(harness.state().current_route(), route, "{key:?}");
    }
}

#[test]
fn ctrl_alt_number_is_left_for_typing() {
    // AltGr arrives as Ctrl+Alt on Windows, and AltGr+digit types a character on many layouts.
    let mut harness = harness(SIZE);
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::Num2);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Home);
}

#[test]
fn alt_left_goes_back() {
    let mut harness = harness(SIZE);
    click_button(&mut harness, &en("nav-search"));

    harness.key_press_modifiers(Modifiers::ALT, Key::ArrowLeft);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Home);

    harness.key_press_modifiers(Modifiers::ALT, Key::ArrowRight);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Search);
}

#[test]
fn mouse_side_buttons_walk_history() {
    let mut harness = harness(SIZE);
    let search = en("nav-search");
    click_button(&mut harness, &search);

    harness
        .get_by_role_and_label(Role::Heading, &search)
        .click_button(PointerButton::Extra1);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Home);

    harness
        .get_by_role_and_label(Role::Heading, &en("nav-home"))
        .click_button(PointerButton::Extra2);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Search);
}

#[test]
fn history_buttons_follow_history() {
    let mut harness = harness(SIZE);
    let (back, forward) = (en("nav-back"), en("nav-forward"));
    assert!(is_disabled(&harness, &back));
    assert!(is_disabled(&harness, &forward));

    click_button(&mut harness, &en("nav-albums"));
    assert!(!is_disabled(&harness, &back));

    click_button(&mut harness, &back);
    assert_eq!(harness.state().current_route(), Route::Home);
    assert!(!is_disabled(&harness, &forward));

    click_button(&mut harness, &forward);
    assert_eq!(harness.state().current_route(), Route::Albums);
}

#[test]
fn pending_pages_say_which_phase() {
    let mut harness = harness(SIZE);
    let local = en(Route::LocalMusic.label_key());
    click_button(&mut harness, &local);
    assert!(
        harness
            .query_by_role_and_label(Role::Heading, &local)
            .is_some(),
        "no {local:?} heading"
    );
    // Local Music has its own content now, so it no longer promises a phase.
    assert!(
        harness
            .query_by_label(&en_phase("empty-phase-pending", 2))
            .is_none()
    );

    for route in Route::ALL {
        let Some(phase) = route.phase() else {
            continue;
        };
        let title = en(route.label_key());
        click_button(&mut harness, &title);
        let pending = en_phase("empty-phase-pending", phase);
        assert!(
            harness.query_by_label(&pending).is_some(),
            "{route:?} should say {pending:?}"
        );
        // The heading names the page once; the empty state does not repeat it.
        assert!(
            harness
                .query_by_role_and_label(Role::Label, &title)
                .is_none(),
            "{route:?} shows its title twice"
        );
    }
}

#[test]
fn player_controls_are_disabled() {
    let harness = harness(SIZE);
    for key in [
        "player-play",
        "player-previous",
        "player-next",
        "player-volume",
        "player-devices",
        "player-lyrics",
    ] {
        assert!(is_disabled(&harness, &en(key)), "{key} should be disabled");
    }
    assert!(!is_disabled(&harness, &en("player-queue")));
}

#[test]
fn disabled_controls_say_when_they_arrive() {
    let mut harness = harness(SIZE);
    for (key, phase) in [
        ("player-play", 2),
        ("player-devices", 3),
        ("player-lyrics", 5),
    ] {
        let name = en(key);
        let hint = en_phase("player-coming-in", phase);
        let control = harness.get_by_role_and_label(Role::Button, &name);
        assert_eq!(control.accesskit_node().description(), Some(hint.clone()));
        control.hover();
        harness.run_steps(4);
        assert!(
            harness.query_by_label(&hint).is_some(),
            "hovering {name} should say {hint:?}"
        );
    }
}

#[test]
fn queue_button_toggles_right_panel() {
    let mut harness = harness(SIZE);
    let (queue, empty) = (en("player-queue"), en("empty-right-panel"));
    assert!(harness.query_by_label(&empty).is_none());

    click_button(&mut harness, &queue);
    assert!(harness.query_by_label(&empty).is_some());
    assert!(harness.state().settings().right_panel_open);

    click_button(&mut harness, &queue);
    assert!(harness.query_by_label(&empty).is_none());
    assert!(!harness.state().settings().right_panel_open);
}

#[test]
fn ctrl_alt_r_toggles_right_panel() {
    let mut harness = harness(SIZE);
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::R);
    harness.run();
    assert!(harness.state().settings().right_panel_open);
    assert!(harness.query_by_label(&en("empty-right-panel")).is_some());
}

#[test]
fn layout_fits_minimum_window() {
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, MIN_SIZE);
    let widest = Settings {
        sidebar_width: SIDEBAR_MAX,
        right_panel_open: true,
        ..english()
    };
    for settings in [english(), widest] {
        let harness = harness_with(MIN_SIZE, settings);
        let play = button_rect(&harness, &en("player-play"));
        assert!(window.contains_rect(play), "Play at {play:?}");
        for route in Route::ALL {
            let label = en(route.label_key());
            let rect = button_rect(&harness, &label);
            assert!(window.contains_rect(rect), "{label} at {rect:?}");
            assert!(rect.max.y <= play.min.y, "{label} overlaps the player bar");
        }
        let heading = harness
            .get_by_role_and_label(Role::Heading, &en("nav-home"))
            .rect();
        assert!(window.contains_rect(heading), "heading at {heading:?}");
    }
}

#[test]
fn narrow_sidebar_truncates_long_labels() {
    let settings = Settings {
        language: Some(Language::Ru),
        sidebar_width: SIDEBAR_MIN,
        ..Default::default()
    };
    let ru = localizer(Language::Ru);
    let mut harness = harness_with(SIZE, settings.clone());
    // The resize handle sits where it does for the short English labels at the same width:
    // the panel did not grow to fit the Russian ones.
    let english = harness_with(
        SIZE,
        Settings {
            language: Some(Language::En),
            ..settings
        },
    );
    let (edge, expected) = (sidebar_edge(&harness).x, sidebar_edge(&english).x);
    assert!(
        (edge - expected).abs() <= 0.5,
        "the sidebar's edge is at {edge}, not {expected}"
    );
    assert_eq!(
        harness.state().settings().sidebar_width,
        SIDEBAR_MIN,
        "the sidebar must not grow to fit its labels"
    );

    // The longest label is elided, so hovering it shows the whole label.
    let longest = ru.tr(Route::LocalMusic.label_key());
    harness
        .get_by_role_and_label(Role::Button, &longest)
        .hover();
    harness.run_steps(4);
    assert!(
        harness
            .query_by_role_and_label(Role::Label, &longest)
            .is_some(),
        "no tooltip with the full label"
    );
}

/// The center of the sidebar's resize handle, the only splitter in the shell.
fn sidebar_edge(harness: &Harness<'_, AriaApp>) -> egui::Pos2 {
    harness.get_by_role(Role::Splitter).rect().center()
}

/// Presses at `from` and moves to `to`, leaving the button held.
fn start_drag(harness: &mut Harness<'_, AriaApp>, from: egui::Pos2, to: egui::Pos2) {
    harness.hover_at(from);
    harness.run();
    harness.drag_at(from);
    harness.run();
    harness.hover_at(to);
    harness.run();
}

#[test]
fn sidebar_resize_is_saved_on_release() {
    let mut harness = harness(SIZE);
    let start = harness.state().settings().sidebar_width;
    let home = en("nav-home");
    let home_width = button_rect(&harness, &home).width();

    let grab = sidebar_edge(&harness);
    let to = grab + egui::vec2(40.0, 0.0);
    start_drag(&mut harness, grab, to);
    assert!(
        button_rect(&harness, &home).width() > home_width,
        "no live resize"
    );
    assert_eq!(
        harness.state().settings().sidebar_width,
        start,
        "saved before the drag ended"
    );

    harness.drop_at(to);
    harness.run();
    let saved = harness.state().settings().sidebar_width;
    assert!((saved - (start + 40.0)).abs() <= 1.0, "saved {saved}");

    // Dragging past the maximum stops at the maximum.
    let grab = sidebar_edge(&harness);
    let to = grab + egui::vec2(SIDEBAR_MAX, 0.0);
    start_drag(&mut harness, grab, to);
    harness.drop_at(to);
    harness.run();
    assert_eq!(harness.state().settings().sidebar_width, SIDEBAR_MAX);
}

#[test]
fn narrow_window_does_not_overwrite_sidebar_width() {
    // Only reachable when zoomed in far, but a squeezed sidebar is not a resize.
    let settings = Settings {
        sidebar_width: 300.0,
        ..english()
    };
    let mut harness = harness_with(egui::vec2(260.0, 600.0), settings);
    assert_eq!(harness.state().settings().sidebar_width, 300.0);

    harness.set_size(SIZE);
    harness.run();
    assert_eq!(harness.state().settings().sidebar_width, 300.0);
    let home = button_rect(&harness, &en("nav-home"));
    assert!(home.width() > 250.0, "the saved width is back: {home:?}");
}

#[test]
fn first_frame_applies_theme_from_settings() {
    let harness = harness_with(
        SIZE,
        Settings {
            theme: Theme::Light,
            ..english()
        },
    );
    assert_eq!(harness.ctx.theme(), egui::Theme::Light);
    assert!(!harness.ctx.global_style().visuals.dark_mode);
}
