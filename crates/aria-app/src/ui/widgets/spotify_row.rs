//! One row of a Spotify list: its cover, title and muted subtitle. A click on it is the
//! response's click.

use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Rect, Response, Sense, TextStyle, Ui, Vec2, WidgetInfo,
    accesskit::Role,
    pos2,
    text::{LayoutJob, TextFormat, TextWrapping},
    vec2,
};

use crate::artwork::paint_cover;
use crate::ui::pages::PageCtx;
use crate::ui::theme::fonts::{icon_font, semibold};
use crate::ui::theme::{icons, visuals::SPACE};

/// Height of a row, which `show_rows` needs every row to be.
pub const SPOTIFY_ROW: f32 = 48.0;
/// Edge length of the cover.
const COVER: f32 = 40.0;
const COVER_RADIUS: u8 = 4;
const PLACEHOLDER_ICON: f32 = 18.0;
/// Between the title and the subtitle.
const LINE_GAP: f32 = 2.0;

/// A row with the cover at `image` (or a placeholder), `title` and `subtitle`. A row that
/// cannot be played (`enabled: false`) is drawn muted and does not respond to clicks.
pub fn spotify_row(
    ui: &mut Ui,
    ctx: &mut PageCtx<'_>,
    image: Option<&str>,
    title: &str,
    subtitle: &str,
    enabled: bool,
) -> Response {
    let p = ctx.palette;
    let size = vec2(ui.available_width(), SPOTIFY_ROW);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, title));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(COVER_RADIUS), p.surface_raised);
    }
    let cover = Rect::from_min_size(
        pos2(rect.left() + SPACE[1], rect.center().y - COVER / 2.0),
        Vec2::splat(COVER),
    );
    let texture = image.and_then(|url| ctx.covers.get(url, ctx.out).cloned());
    let painter = ui.painter();
    painter.rect_filled(cover, COVER_RADIUS, p.surface_raised);
    match texture {
        Some(texture) => paint_cover(painter, cover, COVER_RADIUS, &texture),
        None => {
            painter.text(
                cover.center(),
                Align2::CENTER_CENTER,
                icons::MUSIC_NOTES,
                icon_font(PLACEHOLDER_ICON),
                p.text_muted,
            );
        }
    }
    let left = cover.right() + SPACE[2];
    let width = (rect.right() - left - SPACE[1]).max(0.0);
    let body = TextStyle::Body.resolve(ui.style()).size;
    let title_color = if enabled { p.text } else { p.text_muted };
    let title = one_line(ui, title, semibold(body), title_color, width);
    let subtitle = (!subtitle.is_empty()).then(|| {
        let small = TextStyle::Small.resolve(ui.style());
        one_line(ui, subtitle, small, p.text_muted, width)
    });
    let total = title.size().y + subtitle.as_ref().map_or(0.0, |s| LINE_GAP + s.size().y);
    let top = rect.center().y - total / 2.0;
    let painter = ui.painter();
    let title_height = title.size().y;
    painter.galley(pos2(left, top), title, title_color);
    if let Some(subtitle) = subtitle {
        painter.galley(
            pos2(left, top + title_height + LINE_GAP),
            subtitle,
            p.text_muted,
        );
    }
    response
}

/// `text` on one line, cut with an ellipsis at `width`.
fn one_line(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<eframe::egui::Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap = TextWrapping::truncate_at_width(width);
    ui.painter().layout_job(job)
}
