//! Page headings.

use eframe::egui::{FontFamily, FontId, Response, Sense, Ui, WidgetInfo, accesskit::Role};

use crate::ui::theme::fonts::SEMIBOLD;

/// Text size of a heading.
const HEADING_SIZE: f32 = 20.0;

/// A heading in the semibold family at size 20, in the primary text color.
///
/// Assistive technology sees it as a heading, which also tells it apart from the sidebar
/// entry with the same text.
pub fn section_header(ui: &mut Ui, text: &str) -> Response {
    let font = FontId::new(HEADING_SIZE, FontFamily::Name(SEMIBOLD.into()));
    let color = ui.visuals().text_color();
    let galley = ui
        .painter()
        .layout(text.to_owned(), font, color, ui.available_width());
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Heading, ui.is_enabled(), text));
    if ui.is_rect_visible(rect) {
        ui.painter().galley(rect.min, galley, color);
    }
    response
}
