//! Headless tests of the Soundboard page and the Soundboard card of Settings. Each page is drawn
//! on its own, with the sounds and devices handed in; what it asks of the app is read from the
//! requests it leaves behind.

use aria_app::{
    app::{AppDeps, AriaApp},
    nav::Route,
    ui::{
        pages::{self, LibraryView, PageAction, PageCtx},
        theme::{Palette, fonts},
    },
};
use aria_core::{
    DeviceId, DeviceInfo,
    hotkey::Hotkey,
    i18n::{FluentArgs, Localizer},
    settings::{Language, Settings},
    soundboard::{SoundTarget, SoundView, SoundboardState},
};
use aria_library::SoundId;
use eframe::egui::{self, Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT as _, Queryable as _},
};

/// Room for the whole Settings page.
const SIZE: egui::Vec2 = egui::vec2(1000.0, 1800.0);
/// Where the volume bar's track starts, from the bar's left edge, and its knob room.
const KNOB: f32 = 5.0;

/// What a page is given, and what it left behind.
struct Page {
    route: Route,
    tr: Localizer,
    settings: Settings,
    modal_open: bool,
    soundboard: SoundboardState,
    devices: Vec<DeviceInfo>,
    actions: Vec<PageAction>,
}

fn en(key: &str) -> String {
    english().tr(key).into_owned()
}

fn english() -> Localizer {
    Localizer::new(Language::En).unwrap_or_else(|err| panic!("English locale: {err}"))
}

fn sound(id: i64, name: &str) -> SoundView {
    SoundView {
        id: SoundId(id),
        name: name.into(),
        volume: 1.0,
        hotkey: None,
        missing: false,
        playing: false,
    }
}

fn hotkey(text: &str) -> Hotkey {
    text.parse()
        .unwrap_or_else(|err| panic!("{text} is a hotkey: {err}"))
}

fn device(id: &str, name: &str) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.into()),
        name: name.into(),
        is_default: false,
    }
}

fn page(route: Route, settings: Settings, sounds: Vec<SoundView>) -> Harness<'static, Page> {
    let page = Page {
        route,
        tr: english(),
        settings,
        modal_open: false,
        soundboard: SoundboardState { sounds },
        devices: vec![device("a", "Speakers"), device("b", "Cable")],
        actions: Vec::new(),
    };
    Harness::builder()
        .with_size(SIZE)
        .with_context_setup(fonts::install)
        .build_ui_state(
            |ui, page: &mut Page| {
                let library = LibraryView::default();
                let mut ctx = PageCtx {
                    tr: &page.tr,
                    palette: Palette::dark(),
                    settings: &page.settings,
                    paths_available: true,
                    exclusive_supported: true,
                    modal_open: &mut page.modal_open,
                    out: &mut page.actions,
                    library: &library,
                    collections: &Default::default(),
                    soundboard: &page.soundboard,
                    inactive_hotkeys: &[],
                    devices: &page.devices,
                    input_devices: &[],
                    artwork: None,
                    spotify: &Default::default(),
                    covers: &mut Default::default(),
                };
                pages::show(page.route, ui, &mut ctx);
            },
            page,
        )
}

fn soundboard(sounds: Vec<SoundView>) -> Harness<'static, Page> {
    page(Route::Soundboard, Settings::default(), sounds)
}

fn settings_page(settings: Settings) -> Harness<'static, Page> {
    page(Route::Settings, settings, Vec::new())
}

fn click_button(harness: &mut Harness<'_, Page>, label: &str) {
    harness.get_by_role_and_label(Role::Button, label).click();
    harness.run();
}

fn open_menu(harness: &mut Harness<'_, Page>, tile: &str) {
    harness
        .get_by_role_and_label(Role::Button, tile)
        .click_secondary();
    harness.run();
}

fn actions(harness: &Harness<'_, Page>) -> Vec<PageAction> {
    harness.state().actions.clone()
}

// The Soundboard page.

#[test]
fn no_sounds_shows_the_empty_state_and_the_add_button() {
    let mut harness = soundboard(vec![]);
    harness.get_by_label(&en("soundboard-empty"));
    harness.get_by_label(&en("soundboard-empty-hint"));
    assert!(
        harness
            .query_by_role_and_label(Role::Button, &en("soundboard-stop-all"))
            .is_none(),
        "Stop all with nothing to stop"
    );
    click_button(&mut harness, &en("soundboard-add"));
    assert_eq!(actions(&harness), [PageAction::AddSounds]);
}

#[test]
fn the_add_button_stays_while_there_are_sounds() {
    let mut harness = soundboard(vec![sound(1, "Airhorn")]);
    click_button(&mut harness, &en("soundboard-add"));
    assert_eq!(actions(&harness), [PageAction::AddSounds]);
}

#[test]
fn tiles_show_name_hotkey_and_playing_state() {
    let mut playing = sound(1, "Airhorn");
    playing.hotkey = Some(hotkey("Ctrl+1"));
    playing.playing = true;
    let idle = sound(2, "Rimshot");
    let harness = soundboard(vec![playing, idle]);

    let airhorn = harness.get_by_role_and_label(Role::Button, "Airhorn");
    let node = airhorn.accesskit_node();
    assert_eq!(node.description().as_deref(), Some("Ctrl+1"));
    assert_eq!(node.toggled(), Some(egui::accesskit::Toggled::True));

    let rimshot = harness.get_by_role_and_label(Role::Button, "Rimshot");
    let node = rimshot.accesskit_node();
    assert_eq!(node.description(), None);
    assert_eq!(node.toggled(), Some(egui::accesskit::Toggled::False));
}

#[test]
fn a_click_on_a_tile_plays_it() {
    let mut harness = soundboard(vec![sound(1, "Airhorn"), sound(2, "Rimshot")]);
    click_button(&mut harness, "Rimshot");
    assert_eq!(actions(&harness), [PageAction::PlaySound(SoundId(2))]);
}

#[test]
fn a_missing_sound_is_greyed_and_still_clickable() {
    let mut gone = sound(1, "Airhorn");
    gone.missing = true;
    let mut harness = soundboard(vec![gone]);
    let tile = harness.get_by_role_and_label(Role::Button, "Airhorn");
    let node = tile.accesskit_node();
    assert!(!node.is_disabled(), "a missing sound must stay clickable");
    assert_eq!(
        node.description().as_deref(),
        Some(en("sound-missing").as_str())
    );
    click_button(&mut harness, "Airhorn");
    assert_eq!(actions(&harness), [PageAction::PlaySound(SoundId(1))]);
}

#[test]
fn stop_all_sends_stop_sounds() {
    let mut harness = soundboard(vec![sound(1, "Airhorn")]);
    click_button(&mut harness, &en("soundboard-stop-all"));
    assert_eq!(actions(&harness), [PageAction::StopSounds]);
}

#[test]
fn the_tile_menu_offers_rename_hotkey_volume_and_remove() {
    let mut harness = soundboard(vec![sound(1, "Airhorn")]);
    open_menu(&mut harness, "Airhorn");
    for key in ["sound-rename", "sound-hotkey", "sound-remove"] {
        harness.get_by_role_and_label(Role::Button, &en(key));
    }
    harness.get_by_role_and_label(Role::Slider, &en("sound-volume"));
    harness.get_by_label(&en("sound-hotkey-none"));
    assert!(harness.state().actions.is_empty(), "opening the menu acted");

    click_button(&mut harness, &en("sound-hotkey"));
    assert_eq!(actions(&harness), [PageAction::AskSoundHotkey(SoundId(1))]);
}

#[test]
fn the_tile_menu_shows_the_current_hotkey() {
    let mut keyed = sound(1, "Airhorn");
    keyed.hotkey = Some(hotkey("Alt+F5"));
    let mut harness = soundboard(vec![keyed]);
    open_menu(&mut harness, "Airhorn");
    harness.get_by_label("Alt+F5");
    assert!(harness.query_by_label(&en("sound-hotkey-none")).is_none());
}

#[test]
fn rename_asks_for_a_name() {
    let mut harness = soundboard(vec![sound(1, "Airhorn"), sound(2, "Rimshot")]);
    open_menu(&mut harness, "Rimshot");
    click_button(&mut harness, &en("sound-rename"));
    assert_eq!(actions(&harness), [PageAction::AskSoundName(SoundId(2))]);
}

#[test]
fn remove_asks_first() {
    let mut harness = soundboard(vec![sound(1, "Airhorn")]);
    open_menu(&mut harness, "Airhorn");
    click_button(&mut harness, &en("sound-remove"));
    assert!(harness.state().actions.is_empty(), "removed without asking");
    assert!(harness.state().modal_open);
    let mut args = FluentArgs::new();
    args.set("name", "Airhorn");
    let body = english()
        .tr_args("sound-remove-confirm", &args)
        .into_owned();
    harness.get_by_label(&body);

    click_button(&mut harness, &en("settings-confirm"));
    assert_eq!(actions(&harness), [PageAction::RemoveSound(SoundId(1))]);
    assert!(!harness.state().modal_open);
}

#[test]
fn cancelling_the_remove_sends_nothing() {
    let mut harness = soundboard(vec![sound(1, "Airhorn")]);
    open_menu(&mut harness, "Airhorn");
    click_button(&mut harness, &en("sound-remove"));
    click_button(&mut harness, &en("settings-cancel"));
    assert!(harness.state().actions.is_empty());
    assert!(!harness.state().modal_open);
}

#[test]
fn a_sound_removed_while_the_question_is_open_closes_it() {
    let mut harness = soundboard(vec![sound(1, "Airhorn")]);
    open_menu(&mut harness, "Airhorn");
    click_button(&mut harness, &en("sound-remove"));
    assert!(harness.state().modal_open);
    harness.state_mut().soundboard = SoundboardState::default();
    harness.run();
    assert!(!harness.state().modal_open, "the page kept the app locked");
}

fn pointer(harness: &mut Harness<'_, Page>, events: Vec<egui::Event>) {
    harness.input_mut().events.extend(events);
    harness.step();
}

fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn the_volume_slider_sends_set_sound_volume_on_release() {
    let mut harness = soundboard(vec![sound(1, "Airhorn")]);
    open_menu(&mut harness, "Airhorn");
    let bar = harness
        .get_by_role_and_label(Role::Slider, &en("sound-volume"))
        .rect();
    let line = bar.width() - 2.0 * KNOB;
    let at = |fraction: f32| egui::pos2(bar.left() + KNOB + line * fraction, bar.center().y);

    pointer(&mut harness, vec![egui::Event::PointerMoved(at(0.95))]);
    pointer(&mut harness, vec![press(at(0.95), true)]);
    pointer(&mut harness, vec![egui::Event::PointerMoved(at(0.7))]);
    pointer(&mut harness, vec![egui::Event::PointerMoved(at(0.5))]);
    assert!(
        harness.state().actions.is_empty(),
        "a volume was sent while the knob was still held: {:?}",
        harness.state().actions
    );

    pointer(&mut harness, vec![press(at(0.5), false)]);
    harness.run();
    let [PageAction::SetSoundVolume { id, volume }] = harness.state().actions.as_slice() else {
        panic!(
            "expected one SetSoundVolume, got {:?}",
            harness.state().actions
        );
    };
    assert_eq!(*id, SoundId(1));
    assert!((volume - 0.5).abs() < 0.02, "volume {volume}");
}

// The Soundboard card of Settings.

/// Opens the combo box `combo` and picks `option`.
fn choose(harness: &mut Harness<'_, Page>, combo: &str, option: &str) {
    harness.get_by_role_and_label(Role::ComboBox, combo).click();
    harness.run();
    harness
        .get_by_role(Role::ListBox)
        .get_by_role_and_label(Role::Button, option)
        .click();
    harness.run();
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

#[test]
fn the_second_device_list_includes_none() {
    let mut harness = settings_page(Settings::default());
    let label = en("settings-sound-device");
    harness
        .get_by_role_and_label(Role::ComboBox, &label)
        .click();
    harness.run();
    let options: Vec<String> = harness
        .get_by_role(Role::ListBox)
        .query_all_by_role(Role::Button)
        .filter_map(|node| node.accesskit_node().label())
        .collect();
    let none = en("settings-sound-device-none");
    assert_eq!(options, [none.as_str(), "Speakers", "Cable"]);
}

#[test]
fn choosing_the_second_device_updates_settings() {
    let mut harness = settings_page(Settings::default());
    choose(&mut harness, &en("settings-sound-device"), "Cable");
    let settings = saved(&harness).unwrap_or_else(|| panic!("no settings update"));
    assert_eq!(settings.soundboard_device, Some(DeviceId("b".into())));

    harness.state_mut().settings = settings;
    harness.state_mut().actions.clear();
    choose(
        &mut harness,
        &en("settings-sound-device"),
        &en("settings-sound-device-none"),
    );
    let settings = saved(&harness).unwrap_or_else(|| panic!("no settings update"));
    assert_eq!(settings.soundboard_device, None);
}

#[test]
fn choosing_a_target_updates_settings() {
    let mut harness = settings_page(Settings::default());
    choose(
        &mut harness,
        &en("settings-sound-target"),
        &en("settings-sound-target-both"),
    );
    let settings = saved(&harness).unwrap_or_else(|| panic!("no settings update"));
    assert_eq!(settings.soundboard_target, SoundTarget::Both);
}

#[test]
fn the_target_has_three_choices_and_starts_on_my_device() {
    let mut harness = settings_page(Settings::default());
    let label = en("settings-sound-target");
    let combo = harness.get_by_role_and_label(Role::ComboBox, &label);
    assert_eq!(
        combo.accesskit_node().value().as_deref(),
        Some(en("settings-sound-target-mine").as_str())
    );
    combo.click();
    harness.run();
    let options: Vec<String> = harness
        .get_by_role(Role::ListBox)
        .query_all_by_role(Role::Button)
        .filter_map(|node| node.accesskit_node().label())
        .collect();
    assert_eq!(
        options,
        [
            en("settings-sound-target-mine"),
            en("settings-sound-target-second"),
            en("settings-sound-target-both"),
        ]
    );
}

#[test]
fn the_target_note_shows_without_a_second_device() {
    let note = en("settings-sound-target-note");
    let with = |device: Option<&str>, target| {
        settings_page(Settings {
            soundboard_device: device.map(|id| DeviceId(id.into())),
            soundboard_target: target,
            ..Settings::default()
        })
    };
    for target in [SoundTarget::Second, SoundTarget::Both] {
        with(None, target).get_by_label(&note);
        assert!(with(Some("b"), target).query_by_label(&note).is_none());
    }
    assert!(
        with(None, SoundTarget::Mine)
            .query_by_label(&note)
            .is_none(),
        "the note says sounds go to the listener's device, which is what Mine says already"
    );
}

#[test]
fn the_global_hotkeys_switch_updates_settings() {
    let mut harness = settings_page(Settings::default());
    assert!(Settings::default().global_hotkeys, "on by default");
    harness
        .get_by_role_and_label(Role::CheckBox, &en("settings-global-hotkeys"))
        .click();
    harness.run();
    let settings = saved(&harness).unwrap_or_else(|| panic!("no settings update"));
    assert!(!settings.global_hotkeys);
}

#[test]
fn the_stop_hotkey_button_shows_it_and_asks_for_a_new_one() {
    let settings = Settings {
        stop_sounds_hotkey: Some(hotkey("Ctrl+Alt+S")),
        ..Settings::default()
    };
    let mut harness = settings_page(settings);
    let label = en("settings-stop-hotkey");
    let node = harness.get_by_role_and_label(Role::Button, &label);
    assert_eq!(
        node.accesskit_node().description().as_deref(),
        Some("Ctrl+Alt+S")
    );
    click_button(&mut harness, &label);
    assert_eq!(actions(&harness), [PageAction::AskStopHotkey]);
}

#[test]
fn the_voice_chat_note_is_shown() {
    let harness = settings_page(Settings::default());
    harness.get_by_label(&en("settings-voice-chat-hint"));
}

// The shell.

#[test]
fn ctrl_9_is_settings_and_ctrl_0_is_soundboard() {
    let app = AriaApp::new(
        Settings {
            language: Some(Language::En),
            ..Settings::default()
        },
        english(),
        AppDeps::none(),
        vec![],
    );
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num0);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Soundboard);
    harness.get_by_role_and_label(Role::Heading, &en("nav-soundboard"));
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num9);
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Settings);
}

#[test]
fn the_sidebar_has_a_soundboard_entry() {
    let app = AriaApp::new(
        Settings {
            language: Some(Language::En),
            ..Settings::default()
        },
        english(),
        AppDeps::none(),
        vec![],
    );
    let mut harness = Harness::builder()
        .with_size(SIZE)
        .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app);
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, &en("nav-soundboard"))
        .click();
    harness.run();
    assert_eq!(harness.state().current_route(), Route::Soundboard);
}
