//! The Library card: the folders, and adding, rescanning and clearing the artwork cache.

use aria_library::Folder;
use eframe::egui::{Button, Label, RichText, TextStyle, Ui};

use super::{
    Dialog, PageAction, PageCtx, SPACE, buttons_row, caption, divider, group, open_dialog, row,
};
use crate::ui::theme::{fonts, icons};

/// Size of the warning icon.
const ICON_SIZE: f32 = 16.0;

/// The folder list, and the buttons under it. Draws from whatever snapshot there is, which is
/// empty while the library is off or loading, so it never waits.
pub(super) fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    let tr = ctx.tr;
    let p = ctx.palette;
    let snapshot = ctx.library.snapshot.clone();
    group(ui, &p, &tr.tr("settings-library"), |ui| {
        if snapshot.folders.is_empty() {
            ui.add_space(SPACE[2]);
            caption(ui, &p, &tr.tr("settings-folders-empty"), TextStyle::Body);
        }
        for folder in &snapshot.folders {
            folder_row(ui, ctx, folder);
            divider(ui);
        }
        buttons_row(ui, |ui| {
            if ui.button(tr.tr("local-add-folder")).clicked() {
                ctx.out.push(PageAction::PickFolder);
            }
            if ui.button(tr.tr("local-rescan")).clicked() {
                ctx.out.push(PageAction::Rescan);
            }
            if ui.button(tr.tr("settings-clear-artwork")).clicked() {
                open_dialog(ui, ctx.modal_open, Dialog::ClearArtwork);
            }
        });
    });
}

/// One folder: its path, a warning while it cannot be reached, and Remove.
fn folder_row(ui: &mut Ui, ctx: &mut PageCtx<'_>, folder: &Folder) {
    let tr = ctx.tr;
    let p = ctx.palette;
    let path = folder.path.display().to_string();
    row(ui, &p, &path, |ui| {
        let remove = ui.add(Button::new(tr.tr("settings-remove")));
        // Every folder has this button: say which one it removes.
        ui.ctx()
            .accesskit_node_builder(remove.id, |node| node.set_description(&*path));
        if remove.clicked() {
            open_dialog(ui, ctx.modal_open, Dialog::RemoveFolder(folder.id));
        }
        if !folder.available {
            let icon = RichText::new(icons::WARNING)
                .font(fonts::icon_font(ICON_SIZE))
                .color(p.danger);
            ui.add(Label::new(icon).selectable(false));
            let text = RichText::new(tr.tr("settings-folder-unavailable")).color(p.danger);
            ui.add(Label::new(text).selectable(false));
        }
    });
}
