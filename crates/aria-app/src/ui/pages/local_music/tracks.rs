//! The track table: a sortable header over rows of which only the visible ones are drawn.

use std::sync::Arc;

use aria_library::LibrarySnapshot;
use eframe::egui::{Button, Label, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use super::{
    PageAction, PageCtx,
    view::{SortKey, TrackView},
};
use crate::ui::theme::{
    Palette,
    fonts::{icon_font, semibold},
    icons,
    visuals::SPACE,
};

/// Height of a header and of a row.
const ROW_HEIGHT: f32 = 28.0;
/// Width of the number column.
const NUMBER_WIDTH: f32 = 48.0;
/// Width of the duration column.
const DURATION_WIDTH: f32 = 80.0;
/// Starting width of the title and artist columns.
const TEXT_WIDTH: f32 = 260.0;
/// Narrowest a text column gets.
const MIN_TEXT_WIDTH: f32 = 100.0;
/// Size of the arrow that marks the sorted column.
const ARROW_SIZE: f32 = 12.0;

/// Draws the table of `rows` (indexes into `snapshot.tracks`) and queues what the user does
/// with it. Returns the column whose header was clicked, if any.
pub fn show(
    ui: &mut Ui,
    ctx: &mut PageCtx<'_>,
    snapshot: &Arc<LibrarySnapshot>,
    view: &mut TrackView,
) -> Option<SortKey> {
    let (tr, p) = (ctx.tr, ctx.palette);
    let current = ctx.library.current;
    let sort = view.sort();
    let rows = view.rows(snapshot);
    let mut clicked = None;

    // Handed to the closures whole, so they borrow nothing else of `ctx`.
    let out = &mut *ctx.out;
    let visible_ids = || {
        rows.iter()
            .map(|&i| snapshot.tracks[i].id)
            .collect::<Vec<_>>()
    };

    TableBuilder::new(ui)
        .id_salt("aria_local_tracks")
        .striped(true)
        .sense(Sense::click())
        .column(Column::exact(NUMBER_WIDTH))
        .column(
            Column::initial(TEXT_WIDTH)
                .at_least(MIN_TEXT_WIDTH)
                .clip(true),
        )
        .column(
            Column::initial(TEXT_WIDTH)
                .at_least(MIN_TEXT_WIDTH)
                .clip(true),
        )
        .column(Column::remainder().at_least(MIN_TEXT_WIDTH).clip(true))
        .column(Column::exact(DURATION_WIDTH))
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.add(Label::new(
                    RichText::new(tr.tr("local-col-number")).color(p.text_muted),
                ));
            });
            let sortable = [
                ("local-col-title", SortKey::Title),
                ("local-col-artist", SortKey::Artist),
                ("local-col-album", SortKey::Album),
                ("local-col-duration", SortKey::Duration),
            ];
            for (key, sort_key) in sortable {
                header.col(|ui| {
                    if header_button(ui, &p, &tr.tr(key), sort_key, sort) {
                        clicked = Some(sort_key);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, rows.len(), |mut row| {
                let index = row.index();
                let track = &snapshot.tracks[rows[index]];
                let playing = current == Some(track.id);
                let main = if playing { p.accent } else { p.text };
                let muted = if playing { p.accent } else { p.text_muted };
                row.col(|ui| cell(ui, &(index + 1).to_string(), muted));
                row.col(|ui| cell(ui, &track.title, main));
                row.col(|ui| {
                    let artist = track.artist.as_deref();
                    cell(
                        ui,
                        &super::or_unknown(tr, artist, "library-unknown-artist"),
                        muted,
                    );
                });
                row.col(|ui| {
                    let album = track.album.as_deref();
                    cell(
                        ui,
                        &super::or_unknown(tr, album, "library-unknown-album"),
                        muted,
                    );
                });
                row.col(|ui| cell(ui, &format_duration(track.duration_ms), muted));

                let response = row.response();
                if response.double_clicked() {
                    out.push(PageAction::PlayContext {
                        ids: visible_ids(),
                        start: index,
                    });
                }
                response.context_menu(|ui| {
                    if ui.button(tr.tr("local-play")).clicked() {
                        out.push(PageAction::PlayContext {
                            ids: visible_ids(),
                            start: index,
                        });
                    }
                    if ui.button(tr.tr("local-play-next")).clicked() {
                        out.push(PageAction::PlayNext(track.id));
                    }
                    if ui.button(tr.tr("local-add-to-queue")).clicked() {
                        out.push(PageAction::AddToQueue(track.id));
                    }
                });
            });
        });
    clicked
}

/// A header that sorts by `key` when clicked, with an arrow while it is the sort. Whether it
/// was clicked.
fn header_button(
    ui: &mut Ui,
    p: &Palette,
    text: &str,
    key: SortKey,
    sort: (SortKey, bool),
) -> bool {
    let active = sort.0 == key;
    let color = if active { p.text } else { p.text_muted };
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE[0];
        let label = RichText::new(text).font(semibold(14.0)).color(color);
        clicked = ui.add(Button::new(label).frame(false)).clicked();
        if active {
            let arrow = if sort.1 {
                icons::CARET_UP
            } else {
                icons::CARET_DOWN
            };
            let arrow = RichText::new(arrow)
                .font(icon_font(ARROW_SIZE))
                .color(p.text_muted);
            ui.add(Label::new(arrow).selectable(false));
        }
    });
    clicked
}

/// One line of text that is cut off at the cell's edge. Clicks pass through to the row.
fn cell(ui: &mut Ui, text: &str, color: eframe::egui::Color32) {
    ui.add(
        Label::new(RichText::new(text).color(color))
            .truncate()
            .selectable(false),
    );
}

/// `m:ss`, or `h:mm:ss` from an hour on. A length of 0 is one nobody knows yet.
pub fn format_duration(ms: u64) -> String {
    if ms == 0 {
        return "--:--".to_owned();
    }
    let total = ms / 1000;
    let (hours, minutes, seconds) = (total / 3600, total % 3600 / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_clock_time() {
        assert_eq!(format_duration(0), "--:--");
        assert_eq!(format_duration(999), "0:00");
        assert_eq!(format_duration(61_000), "1:01");
        assert_eq!(format_duration(337_500), "5:37");
        assert_eq!(format_duration(3_600_000), "1:00:00");
        assert_eq!(format_duration(36_000_000 + 59_000), "10:00:59");
    }
}
