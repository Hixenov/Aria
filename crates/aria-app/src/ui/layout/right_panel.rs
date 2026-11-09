//! The right panel, toggled from the player bar's queue button and Ctrl+Alt+R: what is
//! playing, the queue the user built, and what plays after it from the list.
//!
//! The panel draws what it is given and returns what the user asked for as [`QueueAction`]s;
//! the app turns them into commands.

use aria_core::{
    i18n::{FluentArgs, Localizer},
    playback_service::PlayerView,
};
use aria_library::{LibrarySnapshot, TrackId};
use eframe::egui::{
    Align, Align2, Button, DragAndDrop, Label, Layout, Panel, Rect, RichText, ScrollArea, Sense,
    Stroke, TextStyle, Ui, UiBuilder, Vec2, pos2, vec2,
};

use super::{player_bar::NowPlaying, surface_frame};
use crate::{
    artwork::{ArtworkTextures, paint_cover},
    ui::{
        theme::{
            Palette, fonts, icons,
            visuals::{SPACE, TITLE_SIZE},
        },
        widgets::{IconButton, empty_state},
    },
};

/// Panel id, unique among the root panels.
const ID: &str = "aria_right_panel";
/// Width of the panel.
pub const WIDTH: f32 = 300.0;
/// Height of every row of the list, the section headings included, which `show_rows` needs.
const ROW: f32 = 48.0;
/// Edge length of the cover, and its corner radius.
const COVER: [f32; 2] = [36.0, 4.0];
/// Edge length of the remove button, and its icon size.
const REMOVE_BUTTON: [f32; 2] = [28.0, 14.0];
/// Width of the grip that shows a row can be dragged, and the radius of its dots.
const GRIP: [f32; 2] = [10.0, 1.2];
/// Width of the line that shows where a dragged row would land.
const DROP_LINE: f32 = 2.0;

/// What the user did in the panel. The indices are positions in the user queue, which is the
/// row's place among the "Next in queue" rows, not among all the rows shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueAction {
    /// Take the entry at this index out of the user queue.
    Remove(usize),
    /// Move an entry of the user queue so that it ends up at `to`.
    Move { from: usize, to: usize },
    /// Empty the user queue.
    Clear,
}

/// Everything the panel draws from.
pub struct QueuePanel<'a> {
    /// What is playing and what is next.
    pub view: &'a PlayerView,
    /// Where the titles and artists come from.
    pub snapshot: &'a LibrarySnapshot,
    /// Where covers come from. Without it the placeholder shows.
    pub artwork: Option<&'a mut ArtworkTextures>,
}

/// The index of the row being dragged, in the user queue.
struct DragFrom(usize);

/// One line of the list.
enum Row {
    /// A section's heading, as a message id.
    Heading(&'static str),
    /// The current track.
    Now(TrackId),
    /// An upcoming track; `user` is its index in the user queue, for the user's own items.
    Next { track: TrackId, user: Option<usize> },
}

/// Shows the panel while `open`, sliding it in and out as that changes, and returns what the
/// user did in it this frame.
pub fn show(
    ui: &mut Ui,
    tr: &Localizer,
    p: &Palette,
    open: bool,
    queue: &mut QueuePanel<'_>,
) -> Vec<QueueAction> {
    let mut actions = Vec::new();
    let mut expanded = open;
    Panel::right(ID)
        .resizable(false)
        .exact_size(WIDTH)
        .frame(surface_frame(p, SPACE[3]))
        .show_collapsible(ui, &mut expanded, |ui| {
            let rows = rows(queue.view);
            let has_user = rows
                .iter()
                .any(|row| matches!(row, Row::Next { user: Some(_), .. }));
            title(ui, tr, p, has_user, &mut actions);
            if rows.is_empty() {
                empty_state(ui, icons::QUEUE, &tr.tr("empty-right-panel"), "", p);
            } else {
                list(ui, tr, p, queue, &rows, &mut actions);
            }
        });
    actions
}

/// The rows to show: the current track, then the user's items, then the list's, each group
/// under its heading. A group with nothing in it has no heading. Nothing at all when nothing
/// is playing and nothing is next.
fn rows(view: &PlayerView) -> Vec<Row> {
    let mut rows = Vec::with_capacity(view.upcoming.len() + 3);
    if let Some(track) = view.current {
        rows.push(Row::Heading("queue-now-playing"));
        rows.push(Row::Now(track));
    }
    let mut user = 0;
    let mut headed = [false; 2];
    for next in &view.upcoming {
        let (group, heading) = if next.from_user {
            (0, "queue-next-user")
        } else {
            (1, "queue-next-list")
        };
        if !headed[group] {
            headed[group] = true;
            rows.push(Row::Heading(heading));
        }
        let index = next.from_user.then(|| {
            user += 1;
            user - 1
        });
        rows.push(Row::Next {
            track: next.track,
            user: index,
        });
    }
    rows
}

/// What a drop of user row `from` on user row `to` asks for. Dropping a row where it is asks
/// for nothing.
fn drop_action(from: usize, to: usize) -> Option<QueueAction> {
    (from != to).then_some(QueueAction::Move { from, to })
}

/// The panel's title, with the button that empties the user queue when it has anything.
fn title(ui: &mut Ui, tr: &Localizer, p: &Palette, has_user: bool, out: &mut Vec<QueueAction>) {
    // A row as tall as a page heading, so the two titles line up.
    let row = vec2(
        ui.available_width(),
        ui.text_style_height(&TextStyle::Heading),
    );
    ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        let font = fonts::semibold(TITLE_SIZE);
        let title = RichText::new(tr.tr("player-queue"))
            .font(font)
            .color(p.text);
        ui.add(Label::new(title).selectable(false).truncate());
        if has_user {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let text = RichText::new(tr.tr("queue-clear")).color(p.accent);
                if ui.add(Button::new(text).frame(false)).clicked() {
                    out.push(QueueAction::Clear);
                }
            });
        }
    });
}

/// The scrolling list. Only the rows in view are built.
fn list(
    ui: &mut Ui,
    tr: &Localizer,
    p: &Palette,
    queue: &mut QueuePanel<'_>,
    rows: &[Row],
    out: &mut Vec<QueueAction>,
) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ScrollArea::vertical()
        .id_salt("aria_queue_rows")
        .auto_shrink(false)
        .show_rows(ui, ROW, rows.len(), |ui, range| {
            for row in &rows[range] {
                match row {
                    Row::Heading(key) => heading(ui, p, &tr.tr(key)),
                    Row::Now(track) => {
                        let now = NowPlaying::track(*track, queue.snapshot, tr);
                        track_row(ui, p, queue.artwork.as_deref_mut(), &now, None, out, tr);
                    }
                    Row::Next { track, user } => {
                        let now = NowPlaying::track(*track, queue.snapshot, tr);
                        track_row(ui, p, queue.artwork.as_deref_mut(), &now, *user, out, tr);
                    }
                }
            }
        });
}

/// A section's heading, at the bottom of a row of exactly `ROW` height.
fn heading(ui: &mut Ui, p: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::bottom_up(Align::LEFT)),
    );
    row.add_space(SPACE[1]);
    let font = fonts::semibold(TextStyle::Small.resolve(row.style()).size);
    let text = RichText::new(text).font(font).color(p.text_muted);
    row.add(Label::new(text).selectable(false).truncate());
}

/// One track: its cover, title and artist. A row of the user queue (`user` is its index there)
/// also has a grip, can be dragged onto another of them, and has a remove button.
fn track_row(
    ui: &mut Ui,
    p: &Palette,
    artwork: Option<&mut ArtworkTextures>,
    now: &NowPlaying,
    user: Option<usize>,
    out: &mut Vec<QueueAction>,
    tr: &Localizer,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
    // The row's contents go in a child, so that placing them does not move `ui`'s cursor
    // again: `show_rows` relies on every row being exactly `ROW` tall.
    let mut row = ui.new_child(UiBuilder::new().max_rect(rect));
    let Some(index) = user else {
        draw_body(&mut row, p, artwork, now, rect.size(), false);
        return;
    };

    let [button, _] = REMOVE_BUTTON;
    let body = Rect::from_min_max(
        rect.min,
        pos2(rect.right() - button - SPACE[1], rect.bottom()),
    );
    let source = ui.make_persistent_id(("aria_queue_row", index));
    row.scope_builder(UiBuilder::new().max_rect(body), |ui| {
        ui.dnd_drag_source(source, DragFrom(index), |ui| {
            draw_body(ui, p, artwork, now, body.size(), true);
        });
    });
    let remove = Rect::from_center_size(
        pos2(rect.right() - button / 2.0, rect.center().y),
        Vec2::splat(button),
    );
    row.scope_builder(UiBuilder::new().max_rect(remove), |ui| {
        let mut args = FluentArgs::new();
        args.set("title", now.title.as_str());
        let label = tr.tr_args("queue-remove", &args);
        let [size, icon] = REMOVE_BUTTON;
        let button = IconButton::new(icons::X, &label, p).size(size, icon);
        if ui.add(button).clicked() {
            out.push(QueueAction::Remove(index));
        }
    });

    // Where the pointer is, not `Response::contains_pointer`: that is false while a widget
    // inside the row covers the pointer.
    let over = ui
        .input(|input| input.pointer.interact_pos())
        .is_some_and(|pos| rect.intersect(ui.clip_rect()).contains(pos));
    if !over {
        return;
    }
    if ui.input(|input| input.pointer.any_released()) {
        if let Some(from) = DragAndDrop::take_payload::<DragFrom>(ui.ctx()) {
            out.extend(drop_action(from.0, index));
        }
    } else if let Some(from) = DragAndDrop::payload::<DragFrom>(ui.ctx())
        && from.0 != index
    {
        // Moving up lands above this row, moving down below it.
        let y = if index < from.0 {
            rect.top()
        } else {
            rect.bottom()
        };
        ui.painter()
            .hline(rect.x_range(), y, Stroke::new(DROP_LINE, p.accent));
    }
}

/// The grip, cover, title and artist, in a block of `size`.
fn draw_body(
    ui: &mut Ui,
    p: &Palette,
    artwork: Option<&mut ArtworkTextures>,
    now: &NowPlaying,
    size: Vec2,
    grip: bool,
) {
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_size(size);
        if grip {
            draw_grip(ui, p);
        }
        let [edge, radius] = COVER;
        let (art, _) = ui.allocate_exact_size(Vec2::splat(edge), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(art, radius, p.surface_raised);
        let cover = now.artwork.as_ref().and_then(|id| artwork?.get(id));
        match cover {
            Some(texture) => paint_cover(painter, art, radius as u8, texture),
            None => {
                painter.text(
                    art.center(),
                    Align2::CENTER_CENTER,
                    icons::MUSIC_NOTES,
                    fonts::icon_font(edge / 2.0),
                    p.text_muted,
                );
            }
        }
        ui.add_space(SPACE[1]);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let body = TextStyle::Body.resolve(ui.style()).size;
            let title = RichText::new(&now.title)
                .font(fonts::semibold(body))
                .color(p.text);
            ui.add(Label::new(title).selectable(false).truncate());
            let artist = RichText::new(&now.artist).small().color(p.text_muted);
            ui.add(Label::new(artist).selectable(false).truncate());
        });
    });
}

/// Two columns of three dots.
fn draw_grip(ui: &mut Ui, p: &Palette) {
    let [width, dot] = GRIP;
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
    for column in [-1.0, 1.0] {
        for line in [-1.0, 0.0, 1.0] {
            let at = rect.center() + vec2(column * width / 4.0, line * 5.0);
            ui.painter().circle_filled(at, dot, p.text_muted);
        }
    }
    ui.add_space(SPACE[0]);
}

#[cfg(test)]
mod tests {
    use aria_core::playback_service::PlayerView;
    use aria_core::player::queue::UpNext;

    use super::*;

    fn up(id: i64, from_user: bool) -> UpNext {
        UpNext {
            track: TrackId(id),
            from_user,
        }
    }

    #[test]
    fn dropping_a_row_where_it_is_asks_for_nothing() {
        assert_eq!(drop_action(2, 2), None);
        assert_eq!(
            drop_action(0, 2),
            Some(QueueAction::Move { from: 0, to: 2 })
        );
    }

    #[test]
    fn rows_count_user_items_apart_from_the_list() {
        let view = PlayerView {
            current: Some(TrackId(1)),
            upcoming: vec![up(2, true), up(3, true), up(4, false)],
            ..PlayerView::default()
        };
        let users: Vec<_> = rows(&view)
            .iter()
            .filter_map(|row| match row {
                Row::Next { track, user } => Some((track.0, *user)),
                _ => None,
            })
            .collect();
        assert_eq!(users, [(2, Some(0)), (3, Some(1)), (4, None)]);
    }

    #[test]
    fn nothing_playing_and_nothing_next_has_no_rows() {
        assert!(rows(&PlayerView::default()).is_empty());
    }
}
