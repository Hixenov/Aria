//! The Playlists page: the playlists on one side, the selected one on the other.
//!
//! The entries are the ones the library can show: a track the snapshot does not know, or one
//! whose folder is gone, has no row. Every request names an entry by its stored position,
//! which is not the row's place when entries are missing.

use aria_core::i18n::{FluentArgs, Localizer};
use aria_library::{Collections, LibrarySnapshot, Playlist, PlaylistId, Track, TrackId};
use eframe::egui::{
    Align, Button, Color32, DragAndDrop, Frame, Id, Label, Layout, Margin, Modal, Rect, RichText,
    ScrollArea, Sense, Stroke, Ui, UiBuilder, WidgetInfo, accesskit::Role, vec2,
};
use egui_extras::{Size, StripBuilder};

use super::{
    NameFor, PageAction, PageCtx,
    local_music::{format_duration, or_unknown},
};
use crate::{
    nav::Route,
    ui::{
        theme::{
            Palette,
            fonts::semibold,
            icons,
            visuals::{SPACE, TITLE_SIZE},
        },
        widgets::{empty_state, empty_state_with, section_header, track_menu},
    },
};

/// Width of the list of playlists.
const LIST_WIDTH: f32 = 240.0;
/// Height of a row of the list of playlists.
const LIST_ROW: f32 = 48.0;
/// Height of an entry's row, which `show_rows` needs.
const ROW: f32 = 36.0;
/// Width of the grip that shows an entry can be dragged, and the radius of its dots.
const GRIP: [f32; 2] = [10.0, 1.2];
/// Width of the line that shows where a dragged entry would land.
const DROP_LINE: f32 = 2.0;
/// Width of the duration column.
const DURATION_WIDTH: f32 = 64.0;
/// Share of the text columns' width that goes to the title; the artist gets the rest.
const TITLE_SHARE: f32 = 0.55;
/// Height of the empty state's button, for centering the block it sits in.
const EMPTY_BUTTON_HEIGHT: f32 = 32.0;
/// Corner radius of a row and of the dialog.
const RADIUS: u8 = 8;
/// Id of the delete confirmation.
const DIALOG_ID: &str = "aria_playlist_delete_dialog";
/// Width of the delete confirmation's text.
const DIALOG_WIDTH: f32 = 360.0;
/// Least width of the delete confirmation's buttons.
const DIALOG_BUTTON_WIDTH: f32 = 88.0;

/// What the page remembers between frames.
#[derive(Clone, Default)]
struct State {
    /// The playlist shown on the right. When it is gone, or nothing is chosen yet, the first
    /// one is.
    selected: Option<PlaylistId>,
    /// The playlist the open delete confirmation asks about.
    deleting: Option<PlaylistId>,
}

/// The stored position of the entry being dragged.
struct DragFrom(u32);

/// One entry the page can show.
struct Entry<'a> {
    /// Its stored position, which requests use.
    position: u32,
    track: &'a Track,
}

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    // Taken out of egui's memory while it is used, as Local Music does: memory stays locked
    // while borrowed from there, and drawing must not hold that.
    let id = Id::unique("aria_playlists");
    let mut state: State = ui.data_mut(|data| data.remove_temp(id)).unwrap_or_default();
    draw(ui, ctx, &mut state);
    ui.data_mut(|data| data.insert_temp(id, state));
}

fn draw(ui: &mut Ui, ctx: &mut PageCtx<'_>, state: &mut State) {
    let collections = ctx.collections;
    section_header(ui, &ctx.tr.tr(Route::Playlists.label_key()));
    ui.add_space(SPACE[1]);

    state.selected = resolve(state.selected, &collections.playlists);
    // A playlist deleted since the confirmation opened has nothing left to confirm.
    if state
        .deleting
        .is_some_and(|id| find(collections, id).is_none())
    {
        state.deleting = None;
        *ctx.modal_open = false;
    }
    let Some(selected) = state.selected.and_then(|id| find(collections, id)) else {
        none_yet(ui, ctx);
        return;
    };

    StripBuilder::new(ui)
        .size(Size::exact(LIST_WIDTH))
        .size(Size::exact(SPACE[4]))
        .size(Size::remainder())
        .horizontal(|mut strip| {
            strip.cell(|ui| list(ui, ctx, &mut state.selected));
            strip.empty();
            strip.cell(|ui| detail(ui, ctx, selected, &mut state.deleting));
        });

    if let Some(id) = state.deleting
        && let Some(playlist) = find(collections, id)
    {
        confirm_delete(ui, ctx, playlist, &mut state.deleting);
    }
}

/// The playlist the page shows: `selected` while it exists, else the first.
fn resolve(selected: Option<PlaylistId>, playlists: &[Playlist]) -> Option<PlaylistId> {
    selected
        .filter(|id| playlists.iter().any(|playlist| playlist.id == *id))
        .or_else(|| playlists.first().map(|playlist| playlist.id))
}

fn find(collections: &Collections, id: PlaylistId) -> Option<&Playlist> {
    collections
        .playlists
        .iter()
        .find(|playlist| playlist.id == id)
}

/// The empty state of a page with no playlists, with the button that makes the first.
fn none_yet(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    let title = ctx.tr.tr("playlists-empty");
    let hint = ctx.tr.tr("playlists-empty-hint");
    let button = ctx.tr.tr("playlist-new");
    empty_state_with(
        ui,
        icons::PLAYLIST,
        &title,
        &hint,
        &ctx.palette,
        EMPTY_BUTTON_HEIGHT,
        |ui| {
            let button = Button::new(&*button).min_size(vec2(0.0, EMPTY_BUTTON_HEIGHT));
            if ui.add(button).clicked() {
                ctx.out
                    .push(PageAction::AskPlaylistName(NameFor::New { first: None }));
            }
        },
    );
}

/// "New playlist", then one row per playlist.
fn list(ui: &mut Ui, ctx: &mut PageCtx<'_>, selected: &mut Option<PlaylistId>) {
    let new = ctx.tr.tr("playlist-new");
    if ui.button(&*new).clicked() {
        ctx.out
            .push(PageAction::AskPlaylistName(NameFor::New { first: None }));
    }
    ui.add_space(SPACE[1]);
    ScrollArea::vertical()
        .id_salt("aria_playlist_list")
        .auto_shrink(false)
        .show(ui, |ui| {
            for playlist in &ctx.collections.playlists {
                let chosen = *selected == Some(playlist.id);
                if list_row(ui, ctx.tr, &ctx.palette, playlist, chosen) {
                    *selected = Some(playlist.id);
                }
            }
        });
}

/// A playlist's name over its track count. Whether it was clicked.
fn list_row(ui: &mut Ui, tr: &Localizer, p: &Palette, playlist: &Playlist, chosen: bool) -> bool {
    let size = vec2(ui.available_width(), LIST_ROW);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, chosen, &playlist.name));
    if chosen || response.hovered() {
        let fill = if chosen { p.surface_raised } else { p.surface };
        ui.painter().rect_filled(rect, RADIUS, fill);
    }
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(SPACE[2], 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    row.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let name = RichText::new(&playlist.name)
            .font(semibold(14.0))
            .color(p.text);
        ui.add(Label::new(name).selectable(false).truncate());
        let count = count_text(tr, "playlist-track-count", playlist.items.len());
        let count = RichText::new(count).small().color(p.text_muted);
        ui.add(Label::new(count).selectable(false).truncate());
    });
    response.clicked()
}

/// `key` with `count` filled in.
fn count_text(tr: &Localizer, key: &str, count: usize) -> String {
    let mut args = FluentArgs::new();
    args.set("count", count);
    tr.tr_args(key, &args).into_owned()
}

/// The selected playlist: its name and buttons, the unavailable line, and its entries.
fn detail(
    ui: &mut Ui,
    ctx: &mut PageCtx<'_>,
    playlist: &Playlist,
    deleting: &mut Option<PlaylistId>,
) {
    let (tr, p) = (ctx.tr, ctx.palette);
    ui.horizontal(|ui| {
        let name = RichText::new(&playlist.name)
            .font(semibold(TITLE_SIZE))
            .color(p.text);
        ui.add(Label::new(name).selectable(false).truncate());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(tr.tr("playlist-delete")).clicked() {
                *deleting = Some(playlist.id);
                *ctx.modal_open = true;
            }
            if ui.button(tr.tr("playlist-rename")).clicked() {
                ctx.out
                    .push(PageAction::AskPlaylistName(NameFor::Rename(playlist.id)));
            }
        });
    });
    if playlist.unavailable > 0 {
        let text = count_text(tr, "playlist-unavailable", playlist.unavailable);
        ui.add(Label::new(RichText::new(text).small().color(p.text_muted)).selectable(false));
    }
    ui.add_space(SPACE[1]);

    if playlist.items.is_empty() {
        empty_state(ui, icons::PLAYLIST, &tr.tr("playlist-empty"), "", &p);
        return;
    }
    let entries = entries(playlist, &ctx.library.snapshot);
    let table = Table {
        tr,
        p,
        collections: ctx.collections,
        current: ctx.library.current,
        playlist: playlist.id,
        ids: entries.iter().map(|entry| entry.track.id).collect(),
        entries,
    };
    ui.spacing_mut().item_spacing.y = 0.0;
    ScrollArea::vertical()
        .id_salt("aria_playlist_entries")
        .auto_shrink(false)
        .show_rows(ui, ROW, table.entries.len(), |ui, range| {
            for index in range {
                table.row(ui, index, ctx.out);
            }
        });
}

/// The entries of `playlist` that `snapshot` knows, in order, each with its stored position.
fn entries<'a>(playlist: &Playlist, snapshot: &'a LibrarySnapshot) -> Vec<Entry<'a>> {
    playlist
        .items
        .iter()
        .filter_map(|item| {
            Some(Entry {
                position: item.position,
                track: snapshot.track(item.track)?,
            })
        })
        .collect()
}

/// What dropping the entry at stored position `from` on the one at `to` asks for. Dropping
/// an entry where it is asks for nothing.
fn drop_move(from: u32, to: u32) -> Option<(u32, u32)> {
    (from != to).then_some((from, to))
}

/// The request that removes the entry at `position`.
fn remove_action(playlist: PlaylistId, position: u32) -> PageAction {
    PageAction::RemoveFromPlaylist { playlist, position }
}

/// What the rows of the entries are drawn from.
struct Table<'a> {
    tr: &'a Localizer,
    p: Palette,
    collections: &'a Collections,
    current: Option<TrackId>,
    playlist: PlaylistId,
    entries: Vec<Entry<'a>>,
    /// The tracks of `entries`, in order: what playing from a row plays.
    ids: Vec<TrackId>,
}

impl Table<'_> {
    /// Entry `index`: grip, heart, title, artist and duration.
    fn row(&self, ui: &mut Ui, index: usize, out: &mut Vec<PageAction>) {
        let (tr, p) = (self.tr, &self.p);
        let entry = &self.entries[index];
        let track = entry.track;
        let size = vec2(ui.available_width(), ROW);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(rect, RADIUS, p.surface);
        }
        // The row's contents go in a child, so that placing them does not move `ui`'s cursor
        // again: `show_rows` relies on every row being exactly `ROW` tall.
        let mut row = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let gap = SPACE[1];
        row.spacing_mut().item_spacing.x = gap;
        let source = ui.make_persistent_id(("aria_playlist_entry", entry.position));
        row.dnd_drag_source(source, DragFrom(entry.position), |ui| draw_grip(ui, p));
        out.extend(track_menu::heart(
            &mut row,
            tr,
            p,
            self.collections,
            track.id,
        ));

        let fixed = GRIP[0] + track_menu::HEART_BUTTON[0] + DURATION_WIDTH + 4.0 * gap;
        let text = (rect.width() - fixed).max(0.0);
        let playing = self.current == Some(track.id);
        let main = if playing { p.accent } else { p.text };
        let muted = if playing { p.accent } else { p.text_muted };
        let artist = or_unknown(tr, track.artist.as_deref(), "library-unknown-artist");
        let title_width = text * TITLE_SHARE;
        text_cell(&mut row, title_width, &track.title, main, Align::Min);
        text_cell(&mut row, text - title_width, &artist, muted, Align::Min);
        let duration = format_duration(track.duration_ms);
        text_cell(&mut row, DURATION_WIDTH, &duration, muted, Align::Max);

        if response.double_clicked() {
            out.push(self.play_from(index));
        }
        response.context_menu(|ui| {
            if ui.button(tr.tr("local-play")).clicked() {
                out.push(self.play_from(index));
            }
            if ui.button(tr.tr("local-play-next")).clicked() {
                out.push(PageAction::PlayNext(track.id));
            }
            if ui.button(tr.tr("local-add-to-queue")).clicked() {
                out.push(PageAction::AddToQueue(track.id));
            }
            track_menu::items(ui, tr, self.collections, track.id, out);
            if ui.button(tr.tr("playlist-remove-entry")).clicked() {
                out.push(remove_action(self.playlist, entry.position));
            }
        });
        self.drop_target(ui, rect, entry.position, out);
    }

    /// Plays the playlist's entries from row `index`.
    fn play_from(&self, index: usize) -> PageAction {
        PageAction::PlayContext {
            ids: self.ids.clone(),
            start: index,
        }
    }

    /// Takes an entry dropped on the row at `rect`, and shows where one being dragged would
    /// land.
    fn drop_target(&self, ui: &Ui, rect: Rect, position: u32, out: &mut Vec<PageAction>) {
        // Where the pointer is, not `Response::contains_pointer`: that is false while a widget
        // inside the row covers the pointer.
        let over = ui
            .input(|input| input.pointer.interact_pos())
            .is_some_and(|pos| rect.intersect(ui.clip_rect()).contains(pos));
        if !over {
            return;
        }
        if ui.input(|input| input.pointer.any_released()) {
            if let Some(from) = DragAndDrop::take_payload::<DragFrom>(ui.ctx())
                && let Some((from, to)) = drop_move(from.0, position)
            {
                out.push(PageAction::MoveInPlaylist {
                    playlist: self.playlist,
                    from,
                    to,
                });
            }
        } else if let Some(from) = DragAndDrop::payload::<DragFrom>(ui.ctx())
            && from.0 != position
        {
            // Moving up lands above this row, moving down below it.
            let y = if position < from.0 {
                rect.top()
            } else {
                rect.bottom()
            };
            ui.painter()
                .hline(rect.x_range(), y, Stroke::new(DROP_LINE, self.p.accent));
        }
    }
}

/// One line of text, cut off at the cell's edge, in a cell `width` wide. Clicks pass through
/// to the row.
fn text_cell(ui: &mut Ui, width: f32, text: &str, color: Color32, align: Align) {
    let layout = Layout::left_to_right(Align::Center).with_main_align(align);
    ui.allocate_ui_with_layout(vec2(width, ROW), layout, |ui| {
        let text = RichText::new(text).color(color);
        ui.add(Label::new(text).truncate().selectable(false));
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
}

/// Asks before a playlist is deleted. Cancel, Escape and a click outside close it; Confirm
/// closes it and makes the request.
fn confirm_delete(
    ui: &Ui,
    ctx: &mut PageCtx<'_>,
    playlist: &Playlist,
    deleting: &mut Option<PlaylistId>,
) {
    let (tr, p) = (ctx.tr, ctx.palette);
    let frame = Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(SPACE[4] as i8))
        .shadow(ui.visuals().window_shadow);
    let result = Modal::new(Id::unique(DIALOG_ID))
        .frame(frame)
        .show(ui.ctx(), |ui| {
            ui.set_width(DIALOG_WIDTH);
            let title = RichText::new(tr.tr("playlist-delete"))
                .font(semibold(TITLE_SIZE))
                .color(p.text);
            ui.add(Label::new(title).selectable(false).wrap());
            ui.add_space(SPACE[1]);
            let mut args = FluentArgs::new();
            args.set("name", playlist.name.as_str());
            let body = tr.tr_args("playlist-delete-confirm", &args);
            let body = RichText::new(body).color(p.text_muted);
            ui.add(Label::new(body).selectable(false).wrap());
            ui.add_space(SPACE[4]);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let min_size = vec2(DIALOG_BUTTON_WIDTH, 0.0);
                let confirm = RichText::new(tr.tr("settings-confirm")).color(p.on_accent());
                let confirm = Button::new(confirm).fill(p.accent).min_size(min_size);
                if ui.add(confirm).clicked() {
                    return Some(true);
                }
                let cancel = Button::new(tr.tr("settings-cancel")).min_size(min_size);
                ui.add(cancel).clicked().then_some(false)
            })
            .inner
        });
    let confirmed = result.inner;
    if confirmed.is_some() || result.should_close() {
        *deleting = None;
        *ctx.modal_open = false;
    }
    if confirmed == Some(true) {
        ctx.out.push(PageAction::DeletePlaylist(playlist.id));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use aria_library::{FolderId, PlaylistItem};
    use egui_kittest::kittest::Queryable as _;

    use super::{
        super::{
            LibraryView,
            test_support::{World, harness},
        },
        *,
    };

    fn track(id: i64, title: &str) -> Track {
        Track {
            id: TrackId(id),
            folder: FolderId(1),
            path: format!("{id}.flac").into(),
            title: title.to_owned(),
            artist: Some(Arc::from("Artist")),
            album: None,
            album_artist: None,
            track_no: None,
            disc_no: None,
            year: None,
            genre: None,
            duration_ms: 61_000,
            sample_rate: 44_100,
            channels: 2,
            codec: Arc::from("flac"),
            artwork: None,
            added_at: id,
        }
    }

    fn playlist(id: i64, name: &str, items: &[(u32, i64)], unavailable: usize) -> Playlist {
        Playlist {
            id: PlaylistId(id),
            name: name.to_owned(),
            items: items
                .iter()
                .map(|&(position, track)| PlaylistItem {
                    position,
                    track: TrackId(track),
                })
                .collect(),
            unavailable,
        }
    }

    fn library() -> LibraryView {
        LibraryView {
            snapshot: Arc::new(LibrarySnapshot {
                tracks: vec![track(1, "Alpha song"), track(2, "Beta song")],
                ..LibrarySnapshot::default()
            }),
            ..LibraryView::default()
        }
    }

    /// The page over `playlists`, drawn once, with its state.
    fn page(playlists: Vec<Playlist>) -> egui_kittest::Harness<'static, (World, State)> {
        let world = World::new(Collections::new(vec![], playlists, vec![]), library());
        harness((world, State::default()), |ui, (world, state)| {
            world.page(ui, |ui, ctx| draw(ui, ctx, state));
        })
    }

    #[test]
    fn dropping_a_row_where_it_is_asks_for_nothing() {
        assert_eq!(drop_move(2, 2), None);
        assert_eq!(drop_move(0, 2), Some((0, 2)));
    }

    #[test]
    fn rows_use_stored_positions() {
        // Position 1 is an entry the library cannot show.
        let list = playlist(7, "Mix", &[(0, 1), (2, 2)], 1);
        let library = library();
        let entries = entries(&list, &library.snapshot);
        let positions: Vec<_> = entries.iter().map(|entry| entry.position).collect();
        assert_eq!(positions, [0, 2]);
        assert_eq!(
            remove_action(list.id, entries[1].position),
            PageAction::RemoveFromPlaylist {
                playlist: PlaylistId(7),
                position: 2,
            }
        );
    }

    #[test]
    fn entries_the_snapshot_does_not_know_have_no_row() {
        let list = playlist(7, "Mix", &[(0, 1), (1, 99), (2, 2)], 0);
        let library = library();
        let ids: Vec<_> = entries(&list, &library.snapshot)
            .iter()
            .map(|entry| entry.track.id.0)
            .collect();
        assert_eq!(ids, [1, 2]);
    }

    #[test]
    fn no_playlists_shows_the_empty_state_and_the_new_button() {
        let mut harness = page(vec![]);
        let title = harness.state().0.english("playlists-empty");
        harness.get_by_label(&title);
        let new = harness.state().0.english("playlist-new");
        harness.get_by_role_and_label(Role::Button, &new).click();
        harness.run();
        assert_eq!(
            harness.state().0.out,
            [PageAction::AskPlaylistName(NameFor::New { first: None })]
        );
    }

    #[test]
    fn clicking_new_asks_for_a_name() {
        let mut harness = page(vec![playlist(1, "Mix", &[(0, 1)], 0)]);
        let new = harness.state().0.english("playlist-new");
        harness.get_by_role_and_label(Role::Button, &new).click();
        harness.run();
        assert_eq!(
            harness.state().0.out,
            [PageAction::AskPlaylistName(NameFor::New { first: None })]
        );
    }

    #[test]
    fn the_selected_playlist_shows_its_tracks_and_the_unavailable_line() {
        let harness = page(vec![playlist(1, "Mix", &[(0, 1), (2, 2)], 1)]);
        harness.get_by_label("Alpha song");
        harness.get_by_label("Beta song");
        let mut args = FluentArgs::new();
        args.set("count", 1);
        let line = harness
            .state()
            .0
            .english_with("playlist-unavailable", &args);
        harness.get_by_label(&line);
    }

    #[test]
    fn a_playlist_without_entries_says_so() {
        let harness = page(vec![playlist(1, "Mix", &[], 0)]);
        let empty = harness.state().0.english("playlist-empty");
        harness.get_by_label(&empty);
    }

    #[test]
    fn delete_asks_first() {
        let mut harness = page(vec![playlist(1, "Mix", &[(0, 1)], 0)]);
        let delete = harness.state().0.english("playlist-delete");
        harness.get_by_role_and_label(Role::Button, &delete).click();
        harness.run();
        assert!(harness.state().0.out.is_empty(), "asked nothing yet");
        assert!(harness.state().0.modal_open, "the dialog holds the app");

        let confirm = harness.state().0.english("settings-confirm");
        harness
            .get_by_role_and_label(Role::Button, &confirm)
            .click();
        harness.run();
        assert_eq!(
            harness.state().0.out,
            [PageAction::DeletePlaylist(PlaylistId(1))]
        );
        assert!(!harness.state().0.modal_open);
    }

    #[test]
    fn cancelling_the_delete_sends_nothing() {
        let mut harness = page(vec![playlist(1, "Mix", &[(0, 1)], 0)]);
        let delete = harness.state().0.english("playlist-delete");
        harness.get_by_role_and_label(Role::Button, &delete).click();
        harness.run();
        let cancel = harness.state().0.english("settings-cancel");
        harness.get_by_role_and_label(Role::Button, &cancel).click();
        harness.run();
        assert!(harness.state().0.out.is_empty());
        assert!(!harness.state().0.modal_open);
    }

    #[test]
    fn double_click_plays_the_playlist_from_that_row() {
        let mut harness = page(vec![playlist(1, "Mix", &[(0, 1), (2, 2)], 0)]);
        let pos = harness.get_by_label("Beta song").rect().center();
        harness.hover_at(pos);
        harness.run();
        for _ in 0..2 {
            for pressed in [true, false] {
                harness.event(eframe::egui::Event::PointerButton {
                    pos,
                    button: eframe::egui::PointerButton::Primary,
                    pressed,
                    modifiers: eframe::egui::Modifiers::NONE,
                });
                harness.step();
            }
        }
        harness.run();
        assert_eq!(
            harness.state().0.out,
            [PageAction::PlayContext {
                ids: vec![TrackId(1), TrackId(2)],
                start: 1,
            }]
        );
    }

    #[test]
    fn a_deleted_selection_falls_back_to_the_first_playlist() {
        let first = playlist(1, "First", &[(0, 1)], 0);
        let second = playlist(2, "Second", &[(0, 2)], 0);
        let mut harness = page(vec![first.clone(), second.clone()]);
        harness.state_mut().1.selected = Some(PlaylistId(2));
        harness.run();
        harness.get_by_label("Beta song");

        // The second is deleted, as the backend reports.
        harness.state_mut().0.collections = Collections::new(vec![], vec![first], vec![]);
        harness.run();
        assert_eq!(harness.state().1.selected, Some(PlaylistId(1)));
        harness.get_by_label("Alpha song");
    }

    #[test]
    fn the_selection_is_kept_while_it_exists() {
        let playlists = [playlist(1, "A", &[], 0), playlist(2, "B", &[], 0)];
        assert_eq!(
            resolve(Some(PlaylistId(2)), &playlists),
            Some(PlaylistId(2))
        );
        assert_eq!(
            resolve(Some(PlaylistId(9)), &playlists),
            Some(PlaylistId(1))
        );
        assert_eq!(resolve(None, &playlists), Some(PlaylistId(1)));
        assert_eq!(resolve(Some(PlaylistId(1)), &[]), None);
    }
}
