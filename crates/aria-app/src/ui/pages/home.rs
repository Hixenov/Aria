//! The Home page: what was played lately, and what was added lately.

use std::sync::Arc;

use aria_library::{LibrarySnapshot, Track, TrackId};
use eframe::egui::{
    Align2, CursorIcon, Id, Label, RichText, ScrollArea, Sense, TextStyle, Ui, WidgetInfo,
    accesskit::Role, pos2, vec2,
};

use super::{PageAction, PageCtx};
use crate::{
    artwork::paint_cover,
    nav::Route,
    ui::{
        theme::{
            Palette,
            fonts::{icon_font, semibold},
            icons,
            visuals::{SPACE, TITLE_SIZE},
        },
        widgets::{empty_state, section_header, track_menu},
    },
};

/// How many tracks "Recently added" shows.
const ADDED_LIMIT: usize = 20;
/// Height of a row.
const ROW_HEIGHT: f32 = 56.0;
/// Edge of the square the cover, or else the icon, sits on.
const BADGE_SIZE: f32 = 40.0;
/// Size of the placeholder icon on it.
const ICON_SIZE: f32 = 20.0;
/// Corner radius of the badge and of a hovered row.
const RADIUS: u8 = 6;

/// The newest `limit` tracks by when they were added, newest first. Tracks added in the same
/// second come higher id first.
pub fn recently_added(snapshot: &LibrarySnapshot, limit: usize) -> Vec<TrackId> {
    let mut tracks: Vec<&Track> = snapshot.tracks.iter().collect();
    tracks.sort_by(|a, b| b.added_at.cmp(&a.added_at).then(b.id.cmp(&a.id)));
    tracks.into_iter().take(limit).map(|t| t.id).collect()
}

/// What the page remembers between frames: the snapshot "Recently added" was made from, and
/// that list.
#[derive(Clone, Default)]
struct State {
    snapshot: Option<Arc<LibrarySnapshot>>,
    added: Vec<TrackId>,
}

impl State {
    /// Makes the list again when `snapshot` is not the one it was made from.
    fn follow(&mut self, snapshot: &Arc<LibrarySnapshot>) {
        let same = self
            .snapshot
            .as_ref()
            .is_some_and(|known| Arc::ptr_eq(known, snapshot));
        if !same {
            self.added = recently_added(snapshot, ADDED_LIMIT);
            self.snapshot = Some(Arc::clone(snapshot));
        }
    }
}

/// The tracks of `ids` that the snapshot knows, in order. The collections and the snapshot
/// arrive separately, so an id can name a track that is not known yet, or not any more.
fn known<'a>(snapshot: &'a LibrarySnapshot, ids: &[TrackId]) -> Vec<&'a Track> {
    ids.iter().filter_map(|&id| snapshot.track(id)).collect()
}

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    // Taken out of egui's memory while it is used: memory stays locked while borrowed from
    // there, and drawing must not hold that.
    let id = Id::unique("aria_home");
    let mut state: State = ui.data_mut(|data| data.remove_temp(id)).unwrap_or_default();
    draw(ui, ctx, &mut state);
    ui.data_mut(|data| data.insert_temp(id, state));
}

fn draw(ui: &mut Ui, ctx: &mut PageCtx<'_>, state: &mut State) {
    let snapshot = Arc::clone(&ctx.library.snapshot);
    state.follow(&snapshot);
    section_header(ui, &ctx.tr.tr(Route::Home.label_key()));
    ui.add_space(SPACE[1]);

    if snapshot.tracks.is_empty() {
        let (title, hint) = (ctx.tr.tr("home-empty"), ctx.tr.tr("home-empty-hint"));
        empty_state(ui, icons::HOUSE, &title, &hint, &ctx.palette);
        return;
    }
    let recent = known(&snapshot, &ctx.collections.recent);
    let added = known(&snapshot, &state.added);
    ScrollArea::vertical()
        .id_salt("aria_home")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let recent_title = ctx.tr.tr("home-recent");
            let empty = ctx.tr.tr("home-recent-empty");
            section(ui, ctx, "recent", &recent_title, &empty, &recent);
            ui.add_space(SPACE[3]);
            let added_title = ctx.tr.tr("home-added");
            section(ui, ctx, "added", &added_title, "", &added);
        });
}

/// A heading, then `tracks` as rows, or `empty` in muted text when there are none. A click on
/// a row plays the section from there.
fn section(
    ui: &mut Ui,
    ctx: &mut PageCtx<'_>,
    salt: &str,
    title: &str,
    empty: &str,
    tracks: &[&Track],
) {
    let p = ctx.palette;
    heading(ui, &p, title);
    ui.add_space(SPACE[0]);
    if tracks.is_empty() {
        let text = RichText::new(empty).color(p.text_muted);
        ui.add(Label::new(text).selectable(false).wrap());
        return;
    }
    let ids: Vec<TrackId> = tracks.iter().map(|t| t.id).collect();
    for (index, track) in tracks.iter().enumerate() {
        ui.push_id((salt, index), |ui| row(ui, ctx, track, &ids, index));
    }
}

/// A section's heading, which assistive technology sees as one.
fn heading(ui: &mut Ui, p: &Palette, text: &str) {
    let rich = RichText::new(text).font(semibold(TITLE_SIZE)).color(p.text);
    let response = ui.add(Label::new(rich).selectable(false).wrap());
    response.widget_info(|| WidgetInfo::labeled(Role::Heading, true, text));
}

/// One row: the cover, the title and the artist. `ids` is the section and `index` the row's
/// place in it.
fn row(ui: &mut Ui, ctx: &mut PageCtx<'_>, track: &Track, ids: &[TrackId], index: usize) {
    let p = ctx.palette;
    let size = vec2(ui.available_width(), ROW_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let artist = track.artist.as_deref().map_or_else(
        || ctx.tr.tr("library-unknown-artist").into_owned(),
        str::to_owned,
    );
    let name = format!("{} — {artist}", track.title);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), &name));
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(rect);
        if response.hovered() {
            painter.rect_filled(rect, RADIUS, p.surface_raised);
        }
        let badge = eframe::egui::Rect::from_center_size(
            rect.left_center() + vec2(SPACE[1] + BADGE_SIZE / 2.0, 0.0),
            vec2(BADGE_SIZE, BADGE_SIZE),
        );
        painter.rect_filled(badge, RADIUS, p.surface_raised);
        // Only a row that is drawn gets here, so only those covers are asked for.
        let cover = track
            .artwork
            .as_ref()
            .and_then(|id| ctx.artwork.as_deref_mut()?.get(id));
        match cover {
            Some(texture) => paint_cover(&painter, badge, RADIUS, texture),
            None => {
                painter.text(
                    badge.center(),
                    Align2::CENTER_CENTER,
                    icons::MUSIC_NOTES,
                    icon_font(ICON_SIZE),
                    p.text_muted,
                );
            }
        }
        let left = badge.right() + SPACE[2];
        let body = TextStyle::Body.resolve(ui.style());
        let small = TextStyle::Small.resolve(ui.style());
        painter.text(
            pos2(left, rect.center().y - SPACE[1] / 2.0),
            Align2::LEFT_BOTTOM,
            &track.title,
            semibold(body.size),
            p.text,
        );
        painter.text(
            pos2(left, rect.center().y + SPACE[1] / 2.0),
            Align2::LEFT_TOP,
            &artist,
            small,
            p.text_muted,
        );
    }
    let play = PageAction::PlayContext {
        ids: ids.to_vec(),
        start: index,
    };
    if response.clicked() {
        ctx.out.push(play.clone());
    }
    response.context_menu(|ui| {
        if ui.button(ctx.tr.tr("local-play")).clicked() {
            ctx.out.push(play);
        }
        if ui.button(ctx.tr.tr("local-play-next")).clicked() {
            ctx.out.push(PageAction::PlayNext(track.id));
        }
        if ui.button(ctx.tr.tr("local-add-to-queue")).clicked() {
            ctx.out.push(PageAction::AddToQueue(track.id));
        }
        track_menu::items(ui, ctx.tr, ctx.collections, track.id, ctx.out);
    });
}

#[cfg(test)]
mod tests {
    use aria_library::{Collections, FolderId};
    use eframe::egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable as _};

    use super::{
        super::{
            LibraryView,
            test_support::{World, harness},
        },
        *,
    };

    fn track(id: i64, added_at: i64) -> Track {
        Track {
            id: TrackId(id),
            folder: FolderId(1),
            path: format!("{id}.flac").into(),
            title: format!("Song {id}"),
            artist: None,
            album: None,
            album_artist: None,
            track_no: None,
            disc_no: None,
            year: None,
            genre: None,
            duration_ms: 1000,
            sample_rate: 44_100,
            channels: 2,
            codec: Arc::from("flac"),
            artwork: None,
            added_at,
        }
    }

    fn library(tracks: Vec<Track>) -> LibraryView {
        LibraryView {
            snapshot: Arc::new(LibrarySnapshot {
                tracks,
                ..LibrarySnapshot::default()
            }),
            ..LibraryView::default()
        }
    }

    fn home(world: World) -> Harness<'static, World> {
        harness(world, |ui, world| {
            let id = Id::unique("test_home");
            let mut state: State = ui.data_mut(|data| data.remove_temp(id)).unwrap_or_default();
            world.page(ui, |ui, ctx| draw(ui, ctx, &mut state));
            ui.data_mut(|data| data.insert_temp(id, state));
        })
    }

    #[test]
    fn recently_added_takes_the_newest_twenty() {
        // Ids 1..=25; ids 20 to 25 share a second, so the tie order matters.
        let tracks = (1..=25)
            .map(|id| track(id, if id >= 20 { 20 } else { id }))
            .collect();
        let snapshot = LibrarySnapshot {
            tracks,
            ..LibrarySnapshot::default()
        };
        let ids: Vec<i64> = recently_added(&snapshot, 20)
            .into_iter()
            .map(|id| id.0)
            .collect();
        let mut expected: Vec<i64> = vec![25, 24, 23, 22, 21, 20];
        expected.extend((6..=19).rev());
        assert_eq!(ids, expected);
        assert_eq!(ids.len(), 20);
    }

    #[test]
    fn home_rows_skip_unknown_ids() {
        let snapshot = LibrarySnapshot {
            tracks: vec![track(5, 1)],
            ..LibrarySnapshot::default()
        };
        let rows = known(&snapshot, &[TrackId(5), TrackId(99)]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, TrackId(5));
    }

    #[test]
    fn an_empty_library_points_to_settings() {
        let world = World::new(Collections::default(), LibraryView::default());
        let harness = home(world);
        let title = harness.state().english("home-empty");
        harness.get_by_label(&title);
        let hint = harness.state().english("home-empty-hint");
        harness.get_by_label(&hint);
    }

    #[test]
    fn a_click_plays_the_section_from_that_row() {
        let recent = vec![TrackId(3), TrackId(1), TrackId(2)];
        let collections = Collections::new(vec![], vec![], recent.clone());
        let tracks = vec![track(1, 1), track(2, 2), track(3, 3)];
        let mut harness = home(World::new(collections, library(tracks)));
        // Every track is in both sections; Recently played is the upper one.
        let mut rows: Vec<_> = harness
            .get_all_by_role_and_label(Role::Button, "Song 1 — Unknown artist")
            .collect();
        rows.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
        rows[0].click();
        harness.run();
        assert_eq!(
            harness.state().out,
            [PageAction::PlayContext {
                ids: recent,
                start: 1,
            }]
        );
    }

    #[test]
    fn an_empty_section_says_so() {
        let world = World::new(Collections::default(), library(vec![track(1, 1)]));
        let harness = home(world);
        let text = harness.state().english("home-recent-empty");
        harness.get_by_label(&text);
        let empty = harness.state().english("home-empty");
        assert!(harness.query_by_label(&empty).is_none());
    }
}
