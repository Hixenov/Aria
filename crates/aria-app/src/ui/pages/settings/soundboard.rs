//! The Soundboard card: the second device, where sounds go, the global hotkeys and the
//! voice chat note.

use aria_core::{i18n::Localizer, settings::Settings, soundboard::SoundTarget};
use eframe::egui::{Button, Checkbox, ComboBox, TextStyle, Ui};

use super::{
    PageAction, PageCtx, caption, divider, group,
    playback::{DeviceChoice, device_combo},
    row,
};

/// Width of the "Play sounds to" combo box.
const TARGET_COMBO_WIDTH: f32 = 180.0;
/// The places sounds can go, in the order shown.
const TARGETS: [SoundTarget; 3] = [SoundTarget::Mine, SoundTarget::Second, SoundTarget::Both];

/// The second device, the target, the hotkeys switch and the Stop all hotkey.
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
        row(ui, &p, &stop, |ui| {
            let current = s.stop_sounds_hotkey.as_ref().map_or_else(
                || tr.tr("sound-hotkey-none").into_owned(),
                ToString::to_string,
            );
            let response = ui.add(Button::new(&current)).accessible_name(&*stop);
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_description(current.as_str()));
            if response.clicked() {
                ctx.out.push(PageAction::AskStopHotkey);
            }
        });
        divider(ui);
        ui.add_space(super::SPACE[2]);
        caption(ui, &p, &tr.tr("settings-voice-chat-hint"), TextStyle::Small);
    });
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
