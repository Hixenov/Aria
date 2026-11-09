//! The Albums and Artists tabs: a list of each, of which only the visible rows are drawn.

use aria_core::i18n::{FluentArgs, Localizer};
use aria_library::LibrarySnapshot;
use eframe::egui::{
    Align2, CursorIcon, ScrollArea, Sense, TextStyle, Ui, WidgetInfo, accesskit::Role, vec2,
};

use crate::ui::theme::{
    Palette,
    fonts::{icon_font, semibold},
    icons,
    visuals::SPACE,
};

/// Height of a row.
const ROW_HEIGHT: f32 = 56.0;
/// Edge of the square the icon sits on. Task 12 puts the thumbnail there.
const BADGE_SIZE: f32 = 40.0;
/// Size of the icon on it.
const ICON_SIZE: f32 = 20.0;
/// Corner radius of the badge and of a hovered row.
const RADIUS: u8 = 6;

/// One row of a list.
struct Entry {
    icon: &'static str,
    title: String,
    subtitle: String,
    count: String,
}

/// Draws the albums. Returns the index in `snapshot.albums` of the one clicked.
pub fn albums(
    ui: &mut Ui,
    tr: &Localizer,
    p: &Palette,
    snapshot: &LibrarySnapshot,
) -> Option<usize> {
    list(ui, p, "aria_local_albums", snapshot.albums.len(), |i| {
        let album = &snapshot.albums[i];
        Entry {
            icon: icons::VINYL_RECORD,
            title: super::or_unknown(tr, album.title.as_deref(), "library-unknown-album")
                .into_owned(),
            subtitle: super::or_unknown(tr, album.artist.as_deref(), "library-unknown-artist")
                .into_owned(),
            count: track_count(tr, album.tracks.len()),
        }
    })
}

/// Draws the artists. Returns the index in `snapshot.artists` of the one clicked.
pub fn artists(
    ui: &mut Ui,
    tr: &Localizer,
    p: &Palette,
    snapshot: &LibrarySnapshot,
) -> Option<usize> {
    list(ui, p, "aria_local_artists", snapshot.artists.len(), |i| {
        let artist = &snapshot.artists[i];
        Entry {
            icon: icons::MICROPHONE_STAGE,
            title: super::or_unknown(tr, artist.name.as_deref(), "library-unknown-artist")
                .into_owned(),
            subtitle: String::new(),
            count: track_count(tr, artist.tracks.len()),
        }
    })
}

fn track_count(tr: &Localizer, count: usize) -> String {
    let mut args = FluentArgs::new();
    args.set("count", u64::try_from(count).unwrap_or(u64::MAX));
    tr.tr_args("local-track-count", &args).into_owned()
}

/// `len` rows, made by `entry` only for those in view. Returns the index of the clicked one.
fn list(
    ui: &mut Ui,
    p: &Palette,
    id_salt: &str,
    len: usize,
    entry: impl Fn(usize) -> Entry,
) -> Option<usize> {
    let mut clicked = None;
    ScrollArea::vertical()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, len, |ui, range| {
            for i in range {
                if row(ui, p, &entry(i)) {
                    clicked = Some(i);
                }
            }
        });
    clicked
}

/// One row. Whether it was clicked.
fn row(ui: &mut Ui, p: &Palette, entry: &Entry) -> bool {
    let size = vec2(ui.available_width(), ROW_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), &entry.title));
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if !ui.is_rect_visible(rect) {
        return response.clicked();
    }

    let painter = ui.painter().with_clip_rect(rect);
    if response.hovered() {
        painter.rect_filled(rect, RADIUS, p.surface_raised);
    }
    let badge = eframe::egui::Rect::from_center_size(
        rect.left_center() + vec2(SPACE[1] + BADGE_SIZE / 2.0, 0.0),
        vec2(BADGE_SIZE, BADGE_SIZE),
    );
    painter.rect_filled(badge, RADIUS, p.surface_raised);
    painter.text(
        badge.center(),
        Align2::CENTER_CENTER,
        entry.icon,
        icon_font(ICON_SIZE),
        p.text_muted,
    );

    let left = badge.right() + SPACE[2];
    let body = TextStyle::Body.resolve(ui.style());
    let small = TextStyle::Small.resolve(ui.style());
    let count = painter.text(
        rect.right_center() - vec2(SPACE[2], 0.0),
        Align2::RIGHT_CENTER,
        &entry.count,
        small.clone(),
        p.text_muted,
    );
    // The texts end before the count does.
    let text_painter = painter.with_clip_rect(eframe::egui::Rect::from_min_max(
        rect.min,
        eframe::egui::pos2(count.left() - SPACE[1], rect.bottom()),
    ));
    if entry.subtitle.is_empty() {
        text_painter.text(
            eframe::egui::pos2(left, rect.center().y),
            Align2::LEFT_CENTER,
            &entry.title,
            semibold(body.size),
            p.text,
        );
    } else {
        text_painter.text(
            eframe::egui::pos2(left, rect.center().y - SPACE[1] / 2.0),
            Align2::LEFT_BOTTOM,
            &entry.title,
            semibold(body.size),
            p.text,
        );
        text_painter.text(
            eframe::egui::pos2(left, rect.center().y + SPACE[1] / 2.0),
            Align2::LEFT_TOP,
            &entry.subtitle,
            small,
            p.text_muted,
        );
    }
    response.clicked()
}
