//! The modal that asks for a playlist's name.

use aria_core::i18n::Localizer;
use aria_library::playlist_name;
use eframe::egui::{
    Align, Button, Context, Frame, Id, Key, Label, Layout, Margin, Modal, RichText, Stroke,
    TextEdit, WidgetInfo, accesskit::Role, vec2,
};

use crate::ui::theme::{
    Palette, fonts,
    visuals::{SPACE, TITLE_SIZE},
};

/// Width of the dialog's content.
const WIDTH: f32 = 360.0;
/// Least width of its buttons.
const BUTTON_WIDTH: f32 = 88.0;
/// Corner radius of its card.
const RADIUS: u8 = 8;
/// Id of the modal.
const ID: &str = "aria_name_dialog";

/// What the user did with the dialog this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameAnswer {
    /// Still open.
    Open,
    /// Closed by Cancel, Escape or a click outside.
    Cancelled,
    /// Confirmed with this name, already trimmed and within the rules.
    Confirmed(String),
}

/// Text of the dialog.
pub struct NameDialog<'a> {
    /// The heading.
    pub title: &'a str,
    /// The confirm button.
    pub confirm: &'a str,
}

impl NameDialog<'_> {
    /// Draws the dialog editing `text`. The confirm button, and Enter, work only while
    /// `text` is a valid playlist name.
    pub fn show(
        &self,
        ctx: &Context,
        tr: &Localizer,
        p: &Palette,
        text: &mut String,
    ) -> NameAnswer {
        let frame = Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(RADIUS)
            .inner_margin(Margin::same(SPACE[4] as i8))
            .shadow(ctx.global_style().visuals.window_shadow);
        let result = Modal::new(Id::unique(ID)).frame(frame).show(ctx, |ui| {
            ui.set_width(WIDTH);
            let heading = RichText::new(self.title)
                .font(fonts::semibold(TITLE_SIZE))
                .color(p.text);
            let response = ui.add(Label::new(heading).selectable(false).wrap());
            response.widget_info(|| WidgetInfo::labeled(Role::Heading, true, self.title));
            ui.add_space(SPACE[2]);

            let label = tr.tr("playlist-name-label");
            ui.add(Label::new(RichText::new(&*label).color(p.text_muted)).selectable(false));
            let field = ui
                .add(TextEdit::singleline(text).desired_width(f32::INFINITY))
                .accessible_name(&*label);
            // Typing starts at once; a click elsewhere in the dialog gives the field back.
            if ui.memory(|memory| memory.focused().is_none()) {
                field.request_focus();
            }
            let valid = playlist_name(text).is_some();
            let enter = (field.has_focus() || field.lost_focus())
                && ui.input(|input| input.key_pressed(Key::Enter));

            ui.add_space(SPACE[4]);
            let clicked = ui
                .with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let min_size = vec2(BUTTON_WIDTH, 0.0);
                    let confirm = RichText::new(self.confirm).color(p.on_accent());
                    let confirm = Button::new(confirm).fill(p.accent).min_size(min_size);
                    if ui.add_enabled(valid, confirm).clicked() {
                        return Some(true);
                    }
                    let cancel = Button::new(tr.tr("settings-cancel")).min_size(min_size);
                    ui.add(cancel).clicked().then_some(false)
                })
                .inner;
            clicked.or((enter && valid).then_some(true))
        });
        match result.inner {
            Some(true) => match playlist_name(text) {
                Some(name) => NameAnswer::Confirmed(name.to_owned()),
                None => NameAnswer::Open,
            },
            Some(false) => NameAnswer::Cancelled,
            None if result.should_close() => NameAnswer::Cancelled,
            None => NameAnswer::Open,
        }
    }
}
