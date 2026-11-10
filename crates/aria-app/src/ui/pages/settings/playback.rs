//! The Playback card: the output device and gapless playback.

use aria_core::{DeviceId, DeviceInfo, i18n::Localizer, settings::Settings};
use eframe::egui::{Button, Checkbox, ComboBox, TextStyle, Ui};

use super::{Dialog, PageCtx, buttons_row, caption, divider, group, open_dialog, row};

/// Width of the device combo box, wider than the others since device names are long.
const DEVICE_COMBO_WIDTH: f32 = 260.0;

/// The output device, gapless playback and the playback history.
pub(super) fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>, s: &mut Settings) {
    let tr = ctx.tr;
    let p = ctx.palette;
    group(ui, &p, &tr.tr("settings-playback"), |ui| {
        let device = tr.tr("settings-output-device");
        row(ui, &p, &device, |ui| {
            device_combo(ui, tr, &device, ctx.devices, &mut s.output_device);
        });
        divider(ui);
        let gapless = tr.tr("settings-gapless");
        row(ui, &p, &gapless, |ui| {
            let checkbox = Checkbox::without_text(&mut s.gapless);
            ui.add(checkbox).accessible_name(&*gapless);
        });
        divider(ui);
        let history = tr.tr("settings-history");
        let hint = tr.tr("settings-history-hint");
        row(ui, &p, &history, |ui| {
            let checkbox = Checkbox::without_text(&mut s.history_enabled);
            let response = ui.add(checkbox).accessible_name(&*history);
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_description(&*hint));
        });
        caption(ui, &p, &hint, TextStyle::Small);
        divider(ui);
        buttons_row(ui, |ui| {
            if ui.button(tr.tr("settings-history-clear")).clicked() {
                open_dialog(ui, ctx.modal_open, Dialog::ClearHistory);
            }
        });
    });
}

/// The output device combo box, named `name` for assistive technology: "System default",
/// then each device. A saved device that is not in `devices` stays selected as "Unavailable
/// device", so the setting is kept until the user chooses another. Its id is never shown.
fn device_combo(
    ui: &mut Ui,
    tr: &Localizer,
    name: &str,
    devices: &[DeviceInfo],
    selected: &mut Option<DeviceId>,
) {
    let system_default = tr.tr("settings-device-system-default");
    let unavailable = tr.tr("settings-device-unavailable");
    let listed = |id: &DeviceId| devices.iter().find(|d| d.id == *id);
    let missing = selected.as_ref().is_some_and(|id| listed(id).is_none());
    let text = match selected {
        None => system_default.to_string(),
        Some(id) => listed(id).map_or_else(|| unavailable.to_string(), |d| d.name.clone()),
    };
    ComboBox::from_id_salt("aria_output_device")
        .width(DEVICE_COMBO_WIDTH)
        .truncate()
        .selected_text(text)
        .show_ui(ui, |ui| {
            ui.selectable_value(selected, None, &*system_default);
            for device in devices {
                ui.selectable_value(selected, Some(device.id.clone()), &device.name);
            }
            if missing {
                // Already the value: clicking it changes nothing.
                ui.add(Button::selectable(true, &*unavailable));
            }
        })
        .response
        .accessible_name(name);
}
