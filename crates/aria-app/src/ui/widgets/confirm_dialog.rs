//! The dialog that asks before something that cannot be taken back.

use eframe::egui::{
    Align, Button, Frame, Id, Label, Layout, Margin, Modal, RichText, Stroke, Ui, WidgetInfo,
    accesskit::Role, vec2,
};

use crate::ui::theme::{
    Palette, fonts,
    visuals::{SPACE, TITLE_SIZE},
};

/// Corner radius of the dialog.
const RADIUS: u8 = 8;
/// Width of the dialog's text.
const WIDTH: f32 = 360.0;
/// Least width of the dialog's buttons.
const BUTTON_WIDTH: f32 = 88.0;

/// What the dialog says and how its buttons read.
pub struct Confirm<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub confirm: &'a str,
    pub cancel: &'a str,
}

/// Shows the dialog under `id`, with `extra` between the body and the buttons. Cancel, Escape
/// and a click outside answer `Some(false)`, Confirm answers `Some(true)`; `None` while it is
/// still open.
pub fn confirm_dialog(
    ui: &Ui,
    p: &Palette,
    id: Id,
    text: &Confirm<'_>,
    extra: impl FnOnce(&mut Ui),
) -> Option<bool> {
    let frame = Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(SPACE[4] as i8))
        .shadow(ui.visuals().window_shadow);
    let result = Modal::new(id).frame(frame).show(ui.ctx(), |ui| {
        ui.set_width(WIDTH);
        let title = RichText::new(text.title)
            .font(fonts::semibold(TITLE_SIZE))
            .color(p.text);
        let response = ui.add(Label::new(title).selectable(false).wrap());
        response.widget_info(|| WidgetInfo::labeled(Role::Heading, true, text.title));
        ui.add_space(SPACE[1]);
        let body = RichText::new(text.body).color(p.text_muted);
        ui.add(Label::new(body).selectable(false).wrap());
        extra(ui);
        ui.add_space(SPACE[4]);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let min_size = vec2(BUTTON_WIDTH, 0.0);
            let confirm = RichText::new(text.confirm).color(p.on_accent());
            let confirm = Button::new(confirm).fill(p.accent).min_size(min_size);
            if ui.add(confirm).clicked() {
                return Some(true);
            }
            let cancel = Button::new(text.cancel).min_size(min_size);
            ui.add(cancel).clicked().then_some(false)
        })
        .inner
    });
    result
        .inner
        .or_else(|| result.should_close().then_some(false))
}
