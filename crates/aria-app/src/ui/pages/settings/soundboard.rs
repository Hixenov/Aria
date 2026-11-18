//! The Soundboard card: the second device and the virtual cable, where sounds go, the music and
//! the microphone for a chat, the global hotkeys and the voice chat note.

use aria_core::{
    DeviceId, DeviceInfo,
    chat::{CABLE_GUIDE_URL, CableSide, cable_side, find_virtual_cable, is_virtual_cable},
    hotkey::Hotkey,
    i18n::{FluentArgs, Localizer},
    settings::{MIC_VOLUME_MAX, Settings},
    soundboard::SoundTarget,
};
use eframe::egui::{Button, Checkbox, ComboBox, TextStyle, Ui, vec2};

use super::{
    PageAction, PageCtx, buttons_row, caption, divider, group,
    playback::{DeviceChoice, device_combo},
    row,
};
use crate::ui::{theme::Palette, widgets::Scrub};

/// Width of the "Play sounds to" combo box.
const TARGET_COMBO_WIDTH: f32 = 180.0;
/// The places sounds can go, in the order shown.
const TARGETS: [SoundTarget; 3] = [SoundTarget::Mine, SoundTarget::Second, SoundTarget::Both];
/// Size of a volume bar in a row, knob room included.
const LEVEL_BAR: [f32; 2] = [180.0, 16.0];
/// How far an arrow key moves a volume bar, as a share of its length.
const LEVEL_STEP: f32 = 0.05;

/// The second device and the cable, the target, the chat rows, the hotkeys switch and the
/// hotkeys of Stop all and the microphone.
pub(super) fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>, s: &mut Settings) {
    let tr = ctx.tr;
    let p = ctx.palette;
    group(ui, &p, &tr.tr("settings-soundboard"), |ui| {
        let device = tr.tr("settings-sound-device");
        row(ui, &p, &device, |ui| {
            let choice = DeviceChoice {
                salt: "aria_sound_device",
                none_key: "settings-sound-device-none",
            };
            device_combo(
                ui,
                tr,
                &device,
                ctx.devices,
                &mut s.soundboard_device,
                &choice,
            );
        });
        cable(ui, tr, &p, ctx.devices, s);
        divider(ui);
        let target = tr.tr("settings-sound-target");
        row(ui, &p, &target, |ui| {
            target_combo(ui, tr, &target, &mut s.soundboard_target);
        });
        // Without a second device, the second two behave as the first: say so.
        if s.soundboard_device.is_none() && s.soundboard_target != SoundTarget::Mine {
            caption(
                ui,
                &p,
                &tr.tr("settings-sound-target-note"),
                TextStyle::Small,
            );
        }
        divider(ui);
        chat(ui, ctx, s);
        divider(ui);
        let hotkeys = tr.tr("settings-global-hotkeys");
        let hint = tr.tr("settings-global-hotkeys-hint");
        row(ui, &p, &hotkeys, |ui| {
            let checkbox = Checkbox::without_text(&mut s.global_hotkeys);
            let response = ui.add(checkbox).accessible_name(&*hotkeys);
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_description(&*hint));
        });
        caption(ui, &p, &hint, TextStyle::Small);
        divider(ui);
        let stop = tr.tr("settings-stop-hotkey");
        if hotkey_row(ui, tr, &p, &stop, s.stop_sounds_hotkey) {
            ctx.out.push(PageAction::AskStopHotkey);
        }
        divider(ui);
        ui.add_space(super::SPACE[2]);
        caption(ui, &p, &tr.tr("settings-voice-chat-hint"), TextStyle::Small);
    });
}

/// A row with a button showing `hotkey`, named `name`. Returns whether it was clicked.
fn hotkey_row(
    ui: &mut Ui,
    tr: &Localizer,
    p: &Palette,
    name: &str,
    hotkey: Option<Hotkey>,
) -> bool {
    let mut clicked = false;
    row(ui, p, name, |ui| {
        let current = hotkey.as_ref().map_or_else(
            || tr.tr("sound-hotkey-none").into_owned(),
            ToString::to_string,
        );
        let response = ui.add(Button::new(&current)).accessible_name(name);
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_description(current.as_str()));
        clicked = response.clicked();
    });
    clicked
}

/// Under the second device: a virtual cable found and offered while none is chosen, a guide to
/// getting one when there is none, and, with a cable chosen, what to pick in the chat program.
fn cable(ui: &mut Ui, tr: &Localizer, p: &Palette, devices: &[DeviceInfo], s: &mut Settings) {
    let Some(chosen) = &s.soundboard_device else {
        // The music's own device is never the one to offer, cable or not.
        let others: Vec<DeviceInfo> = devices
            .iter()
            .filter(|device| !is_music_device(device, s.output_device.as_ref()))
            .cloned()
            .collect();
        if let Some(found) = find_virtual_cable(&others) {
            let mut args = FluentArgs::new();
            args.set("name", found.name.as_str());
            let line = tr.tr_args("settings-cable-found", &args);
            caption(ui, p, &line, TextStyle::Small);
            buttons_row(ui, |ui| {
                let response = ui.button(tr.tr("settings-cable-use"));
                ui.ctx()
                    .accesskit_node_builder(response.id, |node| node.set_description(&*line));
                if response.clicked() {
                    s.soundboard_device = Some(found.id.clone());
                }
            });
        } else {
            caption(ui, p, &tr.tr("settings-cable-guide-1"), TextStyle::Small);
            buttons_row(ui, |ui| {
                ui.hyperlink_to(tr.tr("settings-cable-guide-link"), CABLE_GUIDE_URL);
            });
            caption(ui, p, &tr.tr("settings-cable-guide-2"), TextStyle::Small);
            caption(ui, p, &tr.tr("settings-cable-guide-3"), TextStyle::Small);
        }
        return;
    };
    let Some(device) = devices.iter().find(|device| device.id == *chosen) else {
        return;
    };
    if is_virtual_cable(&device.name) {
        let key = match cable_side(&device.name) {
            CableSide::VbCable => "settings-cable-hint-vb",
            CableSide::Other => "settings-cable-hint-other",
        };
        caption(ui, p, &tr.tr(key), TextStyle::Small);
    }
}

/// Whether the microphone chosen, or the system default input for `None`, is a virtual cable.
fn mic_is_cable(inputs: &[DeviceInfo], chosen: Option<&DeviceId>) -> bool {
    inputs
        .iter()
        .find(|input| match chosen {
            Some(id) => input.id == *id,
            None => input.is_default,
        })
        .is_some_and(|input| is_virtual_cable(&input.name))
}

/// Whether `device` is the one the music plays on: the chosen one, or the system default for
/// `None`.
fn is_music_device(device: &DeviceInfo, music: Option<&DeviceId>) -> bool {
    match music {
        Some(id) => device.id == *id,
        None => device.is_default,
    }
}

/// Whether the second device is the one the music plays on, so nothing would reach a chat.
fn second_is_music(devices: &[DeviceInfo], s: &Settings) -> bool {
    let Some(second) = &s.soundboard_device else {
        return false;
    };
    match &s.output_device {
        Some(music) => music == second,
        None => devices
            .iter()
            .any(|device| device.is_default && device.id == *second),
    }
}

/// The music and the microphone for a chat. Without a second device the switches and bars are
/// disabled, with a line saying why; with the second device being the music's, a warning.
fn chat(ui: &mut Ui, ctx: &mut PageCtx<'_>, s: &mut Settings) {
    let (tr, p) = (ctx.tr, ctx.palette);
    let has_second = s.soundboard_device.is_some();
    if !has_second {
        caption(
            ui,
            &p,
            &tr.tr("settings-chat-needs-device"),
            TextStyle::Small,
        );
    } else if second_is_music(ctx.devices, s) {
        caption(
            ui,
            &p,
            &tr.tr("settings-chat-same-device"),
            TextStyle::Small,
        );
    }

    let share = tr.tr("settings-share-music");
    let hint = tr.tr("settings-share-music-hint");
    switch_row(ui, &p, &share, &hint, has_second, &mut s.share_music);
    caption(ui, &p, &hint, TextStyle::Small);
    let level = tr.tr("settings-share-music-volume");
    level_row(
        ui,
        &p,
        ("aria_share_music_volume", &level),
        has_second && s.share_music,
        1.0,
        &mut s.share_music_volume,
    );
    divider(ui);

    let mic = tr.tr("settings-mic-device");
    row(ui, &p, &mic, |ui| {
        let choice = DeviceChoice {
            salt: "aria_mic_device",
            none_key: "settings-mic-default",
        };
        device_combo(ui, tr, &mic, ctx.input_devices, &mut s.mic_device, &choice);
    });
    if mic_is_cable(ctx.input_devices, s.mic_device.as_ref()) {
        caption(ui, &p, &tr.tr("settings-mic-is-cable"), TextStyle::Small);
    }
    divider(ui);
    let send = tr.tr("settings-mic-send");
    let hint = tr.tr("settings-mic-send-hint");
    switch_row(ui, &p, &send, &hint, has_second, &mut s.mic_enabled);
    caption(ui, &p, &hint, TextStyle::Small);
    let level = tr.tr("settings-mic-volume");
    level_row(
        ui,
        &p,
        ("aria_mic_volume", &level),
        has_second && s.mic_enabled,
        MIC_VOLUME_MAX,
        &mut s.mic_volume,
    );
    divider(ui);
    let hotkey = tr.tr("settings-mic-hotkey");
    if hotkey_row(ui, tr, &p, &hotkey, s.mic_toggle_hotkey) {
        ctx.out.push(PageAction::AskMicHotkey);
    }
}

/// A row with a switch named `name`, described by `hint`.
fn switch_row(ui: &mut Ui, p: &Palette, name: &str, hint: &str, enabled: bool, on: &mut bool) {
    row(ui, p, name, |ui| {
        let checkbox = Checkbox::without_text(on);
        let response = ui.add_enabled(enabled, checkbox).accessible_name(name);
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_description(hint));
    });
}

/// A row with a volume bar named `name` over `0.0..=max`, and its percentage. The value changes
/// only when the user lets go of the bar, clicks it or moves it with the keyboard, so dragging
/// sends nothing until then.
fn level_row(
    ui: &mut Ui,
    p: &Palette,
    (id, name): (&'static str, &str),
    enabled: bool,
    max: f32,
    value: &mut f32,
) {
    row(ui, p, name, |ui| {
        let (rect, _) = ui.allocate_exact_size(
            vec2(LEVEL_BAR[0], LEVEL_BAR[1]),
            eframe::egui::Sense::hover(),
        );
        let moved = Scrub {
            id,
            rect,
            value: (*value / max).clamp(0.0, 1.0),
            enabled,
            label: name,
            step: LEVEL_STEP,
            full: max,
        }
        .show(ui, p);
        if let Some(at) = moved.committed {
            *value = at * max;
        }
        let shown = moved.dragging.map_or(*value, |at| at * max);
        ui.label(percent(shown));
    });
}

/// `level` as a whole percentage.
fn percent(level: f32) -> String {
    format!("{}%", (level.max(0.0) * 100.0).round())
}

/// The "Play sounds to" combo box, named `name` for assistive technology.
fn target_combo(ui: &mut Ui, tr: &Localizer, name: &str, target: &mut SoundTarget) {
    let text = |choice: SoundTarget| {
        let key = match choice {
            SoundTarget::Mine => "settings-sound-target-mine",
            SoundTarget::Second => "settings-sound-target-second",
            SoundTarget::Both => "settings-sound-target-both",
        };
        tr.tr(key).into_owned()
    };
    ComboBox::from_id_salt("aria_sound_target")
        .width(TARGET_COMBO_WIDTH)
        .selected_text(text(*target))
        .show_ui(ui, |ui| {
            for choice in TARGETS {
                ui.selectable_value(target, choice, text(choice));
            }
        })
        .response
        .accessible_name(name);
}

#[cfg(test)]
mod tests {
    use aria_core::{DeviceId, DeviceInfo, i18n::FluentArgs, settings::Settings};
    use aria_library::Collections;
    use eframe::egui::{self, accesskit::Role};
    use egui_kittest::{
        Harness,
        kittest::{NodeT as _, Queryable as _},
    };

    use super::{
        super::super::{
            LibraryView, PageAction,
            test_support::{World, harness_sized},
        },
        show,
    };

    const SPEAKERS: &str = "Speakers (Realtek(R) Audio)";
    const VB_CABLE: &str = "CABLE Input (VB-Audio Virtual Cable)";
    const OTHER_CABLE: &str = "Line 1 (Virtual Audio Cable)";

    fn device(id: &str, name: &str, is_default: bool) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(id.into()),
            name: name.into(),
            is_default,
        }
    }

    fn outputs() -> Vec<DeviceInfo> {
        vec![
            device("speakers", SPEAKERS, true),
            device("vb", VB_CABLE, false),
            device("line", OTHER_CABLE, false),
        ]
    }

    /// The Soundboard card with `settings` and output `devices`, tall enough to need no
    /// scrolling. A change to the settings is left as an UpdateSettings, as the page leaves it.
    fn card(settings: Settings, devices: Vec<DeviceInfo>) -> Harness<'static, World> {
        let mut world = World::new(Collections::default(), LibraryView::default());
        world.settings = settings;
        world.devices = devices;
        world.input_devices = vec![device("mic", "Microphone (USB)", true)];
        harness_sized(egui::vec2(1000.0, 1600.0), world, |ui, world| {
            world.page(ui, |ui, ctx| {
                let mut settings = ctx.settings.clone();
                show(ui, ctx, &mut settings);
                if settings != *ctx.settings {
                    ctx.out.push(PageAction::UpdateSettings(settings));
                }
            });
        })
    }

    fn with_cable() -> Settings {
        Settings {
            soundboard_device: Some(DeviceId("vb".into())),
            ..Settings::default()
        }
    }

    fn updated(harness: &Harness<'_, World>) -> Vec<Settings> {
        harness
            .state()
            .out
            .iter()
            .filter_map(|action| match action {
                PageAction::UpdateSettings(settings) => Some(settings.clone()),
                _ => None,
            })
            .collect()
    }

    fn disabled(harness: &Harness<'_, World>, role: Role, key: &str) -> bool {
        let label = harness.state().english(key);
        harness
            .get_by_role_and_label(role, &label)
            .accesskit_node()
            .is_disabled()
    }

    fn shows(harness: &Harness<'_, World>, text: &str) -> bool {
        harness.query_by_label(text).is_some()
    }

    fn shows_key(harness: &Harness<'_, World>, key: &str) -> bool {
        shows(harness, &harness.state().english(key))
    }

    #[test]
    fn the_new_rows_are_disabled_without_a_second_device() {
        let harness = card(Settings::default(), outputs());

        assert!(disabled(&harness, Role::CheckBox, "settings-share-music"));
        assert!(disabled(&harness, Role::CheckBox, "settings-mic-send"));
        assert!(disabled(
            &harness,
            Role::Slider,
            "settings-share-music-volume"
        ));
        assert!(disabled(&harness, Role::Slider, "settings-mic-volume"));
        assert!(shows_key(&harness, "settings-chat-needs-device"));

        let harness = card(with_cable(), outputs());
        assert!(!disabled(&harness, Role::CheckBox, "settings-share-music"));
        assert!(!disabled(&harness, Role::CheckBox, "settings-mic-send"));
        assert!(!shows_key(&harness, "settings-chat-needs-device"));
    }

    #[test]
    fn the_same_device_warning_shows() {
        let same = Settings {
            output_device: Some(DeviceId("vb".into())),
            ..with_cable()
        };
        assert!(shows_key(
            &card(same, outputs()),
            "settings-chat-same-device"
        ));

        // The music on the system default, which the second device is.
        let default = Settings {
            soundboard_device: Some(DeviceId("speakers".into())),
            ..Settings::default()
        };
        assert!(shows_key(
            &card(default, outputs()),
            "settings-chat-same-device"
        ));

        assert!(!shows_key(
            &card(with_cable(), outputs()),
            "settings-chat-same-device"
        ));
    }

    #[test]
    fn the_switches_send_update_settings() {
        let mut harness = card(with_cable(), outputs());
        let share = harness.state().english("settings-share-music");
        harness
            .get_by_role_and_label(Role::CheckBox, &share)
            .click();
        harness.run();
        let send = harness.state().english("settings-mic-send");
        harness.get_by_role_and_label(Role::CheckBox, &send).click();
        harness.run();

        let updates = updated(&harness);
        assert!(updates.iter().any(|s| s.share_music), "{updates:?}");
        assert!(updates.iter().any(|s| s.mic_enabled), "{updates:?}");
    }

    #[test]
    fn assistive_technology_reads_the_volumes_as_shown() {
        let settings = Settings {
            share_music: true,
            share_music_volume: 0.5,
            mic_enabled: true,
            mic_volume: 1.0,
            ..with_cable()
        };
        let harness = card(settings, outputs());
        let value = |key: &str| {
            let label = harness.state().english(key);
            harness
                .get_by_role_and_label(Role::Slider, &label)
                .accesskit_node()
                .numeric_value()
        };
        // 100 % on a bar that runs to 200 % is read as 100, not as half the bar.
        assert_eq!(value("settings-mic-volume"), Some(100.0));
        assert_eq!(value("settings-share-music-volume"), Some(50.0));
    }

    #[test]
    fn the_volume_sliders_send_on_release_only() {
        let settings = Settings {
            share_music: true,
            mic_enabled: true,
            ..with_cable()
        };
        let mut harness = card(settings, outputs());
        let label = harness.state().english("settings-share-music-volume");
        let rect = harness.get_by_role_and_label(Role::Slider, &label).rect();

        // Pressed and dragged, not let go: nothing yet.
        let (from, to) = (rect.center(), rect.left_center() + egui::vec2(8.0, 0.0));
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(from),
            egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        harness.step();
        for step in 1..=5 {
            let at = from + (to - from) * (step as f32 / 5.0);
            harness
                .input_mut()
                .events
                .push(egui::Event::PointerMoved(at));
            harness.step();
        }
        assert!(updated(&harness).is_empty(), "sent while dragging");

        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
        let updates = updated(&harness);
        assert_eq!(updates.len(), 1, "{updates:?}");
        assert!(updates[0].share_music_volume < 0.2, "{updates:?}");

        // The microphone's bar runs to 200 %.
        let label = harness.state().english("settings-mic-volume");
        let rect = harness.get_by_role_and_label(Role::Slider, &label).rect();
        let end = rect.right_center() - egui::vec2(2.0, 0.0);
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(end),
            egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        harness.run();
        let last = updated(&harness).pop().unwrap_or_default();
        assert!(last.mic_volume > 1.9, "{}", last.mic_volume);
        assert!(shows(&harness, "100%"), "the microphone level is not shown");
    }

    #[test]
    fn the_microphone_list_shows_the_input_devices() {
        let mut harness = card(with_cable(), outputs());
        let label = harness.state().english("settings-mic-device");
        harness
            .get_by_role_and_label(Role::ComboBox, &label)
            .click();
        harness.run();
        harness
            .get_by_role(Role::ListBox)
            .get_by_role_and_label(Role::Button, "Microphone (USB)")
            .click();
        harness.run();

        let updates = updated(&harness);
        assert_eq!(
            updates.last().and_then(|s| s.mic_device.clone()),
            Some(DeviceId("mic".into()))
        );
    }

    #[test]
    fn a_cable_as_the_microphone_is_warned() {
        let cable_input = vec![
            device("cable out", "CABLE Output (VB-Audio Virtual Cable)", true),
            device("mic", "Microphone (USB)", false),
        ];
        let with_inputs = |settings: Settings| {
            let mut harness = card(settings, outputs());
            harness.state_mut().input_devices = cable_input.clone();
            harness.run();
            harness
        };

        // The system default input is the cable.
        let harness = with_inputs(with_cable());
        assert!(shows_key(&harness, "settings-mic-is-cable"));
        // The cable chosen by hand.
        let chosen = Settings {
            mic_device: Some(DeviceId("cable out".into())),
            ..with_cable()
        };
        assert!(shows_key(&with_inputs(chosen), "settings-mic-is-cable"));
        // A real microphone chosen.
        let real = Settings {
            mic_device: Some(DeviceId("mic".into())),
            ..with_cable()
        };
        assert!(!shows_key(&with_inputs(real), "settings-mic-is-cable"));
    }

    #[test]
    fn the_hotkey_row_opens_the_dialog() {
        let mut harness = card(with_cable(), outputs());
        let label = harness.state().english("settings-mic-hotkey");
        harness.get_by_role_and_label(Role::Button, &label).click();
        harness.run();

        assert!(harness.state().out.contains(&PageAction::AskMicHotkey));
    }

    #[test]
    fn a_found_cable_is_offered_and_use_it_picks_it() {
        let mut harness = card(Settings::default(), outputs());
        let mut args = FluentArgs::new();
        args.set("name", VB_CABLE);
        let line = harness.state().english_with("settings-cable-found", &args);
        assert!(shows(&harness, &line), "no cable offered");
        assert!(!shows_key(&harness, "settings-cable-guide-1"));

        let use_it = harness.state().english("settings-cable-use");
        harness.get_by_role_and_label(Role::Button, &use_it).click();
        harness.run();

        let updates = updated(&harness);
        assert_eq!(
            updates.last().and_then(|s| s.soundboard_device.clone()),
            Some(DeviceId("vb".into()))
        );
    }

    #[test]
    fn the_music_device_is_never_offered_as_the_cable() {
        // Voicemeeter users often play everything into "Voicemeeter Input".
        let voicemeeter = "VoiceMeeter Input (VB-Audio VoiceMeeter VAIO)";
        let devices = vec![
            device("vm", voicemeeter, true),
            device("vb", VB_CABLE, false),
        ];
        let harness = card(Settings::default(), devices.clone());
        let mut args = FluentArgs::new();
        args.set("name", VB_CABLE);
        let line = harness.state().english_with("settings-cable-found", &args);
        assert!(shows(&harness, &line), "the other cable is not offered");

        // Chosen as the music's device by hand, too.
        let chosen = Settings {
            output_device: Some(DeviceId("vb".into())),
            ..Settings::default()
        };
        let harness = card(chosen, devices[1..].to_vec());
        assert!(!shows_key(&harness, "settings-cable-use"));
        assert!(shows_key(&harness, "settings-cable-guide-1"));
    }

    #[test]
    fn the_guide_shows_when_no_cable_is_found() {
        let harness = card(
            Settings::default(),
            vec![device("speakers", SPEAKERS, true)],
        );

        assert!(shows_key(&harness, "settings-cable-guide-1"));
        assert!(shows_key(&harness, "settings-cable-guide-3"));
        let link = harness.state().english("settings-cable-guide-link");
        harness.get_by_role_and_label(Role::Link, &link);
        assert!(!shows_key(&harness, "settings-cable-use"));
    }

    #[test]
    fn the_chat_hint_names_cable_output() {
        assert!(shows_key(
            &card(with_cable(), outputs()),
            "settings-cable-hint-vb"
        ));

        let other = Settings {
            soundboard_device: Some(DeviceId("line".into())),
            ..Settings::default()
        };
        let harness = card(other, outputs());
        assert!(shows_key(&harness, "settings-cable-hint-other"));
        assert!(!shows_key(&harness, "settings-cable-hint-vb"));

        let speakers = Settings {
            soundboard_device: Some(DeviceId("speakers".into())),
            ..Settings::default()
        };
        let harness = card(speakers, outputs());
        assert!(!shows_key(&harness, "settings-cable-hint-vb"));
        assert!(!shows_key(&harness, "settings-cable-hint-other"));
    }
}
