//! The Local Music page: the user's own tracks, albums and artists, from the library the
//! backend keeps.
//!
//! Nothing here waits for the library. It draws the snapshot it is given, which starts empty,
//! so a library that is off, still loading or empty all show the empty state until a
//! snapshot arrives.

mod groups;
mod tracks;
mod view;

use std::{borrow::Cow, sync::Arc};

use aria_core::i18n::{FluentArgs, Localizer};
use aria_library::{LibrarySnapshot, ScanProgress, TrackId};
use eframe::egui::{Align, Button, Id, Label, Layout, RichText, TextEdit, Ui, accesskit::Role};

pub use self::{
    tracks::show as show_tracks,
    view::{Scope, SortKey, Tab, TrackView},
};
use super::{PageAction, PageCtx};
use crate::{
    nav::Route,
    ui::{
        theme::{Palette, fonts::semibold, icons, visuals::SPACE},
        widgets::{IconButton, empty_state, empty_state_with, section_header},
    },
};

/// Height of the empty state's button, for centering the block it sits in.
const EMPTY_BUTTON_HEIGHT: f32 = 32.0;
/// Width of the search field.
const SEARCH_WIDTH: f32 = 260.0;
/// Thickness of the line under the open tab.
const TAB_LINE: f32 = 2.0;

/// What the page shows of the library.
#[derive(Clone, Debug, Default)]
pub struct LibraryView {
    /// The latest library. Empty until the backend sends one.
    pub snapshot: Arc<LibrarySnapshot>,
    /// The running scan, if there is one.
    pub scan: Option<ScanProgress>,
    /// The track that is playing, to mark its row.
    pub current: Option<TrackId>,
}

/// Which album or artist the Tracks tab is limited to, by name. A snapshot numbers its
/// groups by their order, and a new one can renumber them, so the number alone cannot say
/// which group is open. Names are lowercase, as the library groups them.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Open {
    /// The album's title, and the album artist (else the artist).
    Album(Option<String>, Option<String>),
    Artist(Option<String>),
}

impl Open {
    /// The identity of the group `scope` points at in `snapshot`.
    fn of(scope: &Scope, snapshot: &LibrarySnapshot) -> Option<Self> {
        let lower = |s: &Option<Arc<str>>| s.as_deref().map(str::to_lowercase);
        match scope {
            Scope::All | Scope::Ids(_) => None,
            Scope::Album(i) => snapshot
                .albums
                .get(*i)
                .map(|a| Self::Album(lower(&a.title), lower(&a.artist))),
            Scope::Artist(i) => snapshot
                .artists
                .get(*i)
                .map(|a| Self::Artist(lower(&a.name))),
        }
    }

    /// Where the group is in `snapshot`, if it still is.
    fn find(&self, snapshot: &LibrarySnapshot) -> Option<Scope> {
        let lower = |s: &Option<Arc<str>>| s.as_deref().map(str::to_lowercase);
        match self {
            Self::Album(title, artist) => snapshot
                .albums
                .iter()
                .position(|a| lower(&a.title) == *title && lower(&a.artist) == *artist)
                .map(Scope::Album),
            Self::Artist(name) => snapshot
                .artists
                .iter()
                .position(|a| lower(&a.name) == *name)
                .map(Scope::Artist),
        }
    }
}

/// What the page remembers between frames.
#[derive(Clone, Default)]
struct State {
    tab: Tab,
    /// What is typed in the search field.
    query: String,
    view: TrackView,
    /// The group the view is limited to, so it can be found again in a new snapshot.
    open: Option<Open>,
    /// The snapshot `open` was last looked up in.
    resolved_in: std::sync::Weak<LibrarySnapshot>,
}

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    // The state is taken out of egui's memory while it is used: memory stays locked for as
    // long as it is borrowed from there, and drawing must not hold that.
    let id = Id::unique("aria_local_music");
    let mut state: State = ui.data_mut(|data| data.remove_temp(id)).unwrap_or_default();
    draw(ui, ctx, &mut state);
    ui.data_mut(|data| data.insert_temp(id, state));
}

fn draw(ui: &mut Ui, ctx: &mut PageCtx<'_>, state: &mut State) {
    let snapshot = Arc::clone(&ctx.library.snapshot);
    state.follow_open_group(&snapshot);
    let has_folders = !snapshot.folders.is_empty();

    header(ui, ctx, has_folders);
    if !has_folders {
        // The library can scan before a snapshot lists its folder, as at startup.
        if let Some(scan) = ctx.library.scan {
            scanning(ui, ctx.tr, &ctx.palette, scan);
        }
        no_folders(ui, ctx);
        return;
    }
    controls(ui, ctx, state);
    if let Some(scan) = ctx.library.scan {
        scanning(ui, ctx.tr, &ctx.palette, scan);
    }
    ui.add_space(SPACE[1]);

    match state.tab {
        Tab::Tracks => tracks_tab(ui, ctx, state, &snapshot),
        Tab::Albums if snapshot.albums.is_empty() => no_tracks(ui, ctx),
        Tab::Albums => {
            if let Some(i) = groups::albums(
                ui,
                ctx.tr,
                &ctx.palette,
                &snapshot,
                ctx.artwork.as_deref_mut(),
            ) {
                state.open(Scope::Album(i), &snapshot);
            }
        }
        Tab::Artists if snapshot.artists.is_empty() => no_tracks(ui, ctx),
        Tab::Artists => {
            if let Some(i) = groups::artists(
                ui,
                ctx.tr,
                &ctx.palette,
                &snapshot,
                ctx.artwork.as_deref_mut(),
            ) {
                state.open(Scope::Artist(i), &snapshot);
            }
        }
    }
}

impl State {
    /// Shows the tracks of `scope` in `snapshot`, whatever was searched for before.
    fn open(&mut self, scope: Scope, snapshot: &Arc<LibrarySnapshot>) {
        self.query.clear();
        self.open = Open::of(&scope, snapshot);
        self.resolved_in = Arc::downgrade(snapshot);
        self.view.set_scope(scope);
        self.tab = Tab::Tracks;
    }

    /// Goes back to every track.
    fn show_all(&mut self) {
        self.open = None;
        self.view.set_scope(Scope::All);
    }

    /// Keeps the view on the group that was opened when a new snapshot renumbers the groups,
    /// and goes back to every track if the group is gone.
    fn follow_open_group(&mut self, snapshot: &Arc<LibrarySnapshot>) {
        if self.resolved_in.ptr_eq(&Arc::downgrade(snapshot)) {
            return;
        }
        self.resolved_in = Arc::downgrade(snapshot);
        let Some(open) = &self.open else {
            return;
        };
        match open.find(snapshot) {
            Some(scope) => self.view.move_scope(scope),
            None => self.show_all(),
        }
    }
}

/// The heading, and the buttons that act on the library as a whole.
fn header(ui: &mut Ui, ctx: &mut PageCtx<'_>, has_folders: bool) {
    ui.horizontal(|ui| {
        section_header(ui, &ctx.tr.tr(Route::LocalMusic.label_key()));
        // With no folders the empty state has the Add folder button, and there is nothing to
        // rescan.
        if !has_folders {
            return;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(ctx.tr.tr("local-add-folder")).clicked() {
                ctx.out.push(PageAction::PickFolder);
            }
            if ui.button(ctx.tr.tr("local-rescan")).clicked() {
                ctx.out.push(PageAction::Rescan);
            }
        });
    });
}

/// The tabs, and on the first one the search field.
fn controls(ui: &mut Ui, ctx: &mut PageCtx<'_>, state: &mut State) {
    ui.horizontal(|ui| {
        let tabs = [
            (Tab::Tracks, "local-tab-tracks"),
            (Tab::Albums, "local-tab-albums"),
            (Tab::Artists, "local-tab-artists"),
        ];
        for (tab, key) in tabs {
            if tab_button(ui, ctx, &ctx.tr.tr(key), state.tab == tab) {
                if tab == Tab::Tracks {
                    // The tab is the way back to every track.
                    state.show_all();
                }
                state.tab = tab;
            }
        }
        if state.tab == Tab::Tracks {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let hint = ctx.tr.tr("local-search");
                let field = TextEdit::singleline(&mut state.query)
                    .hint_text(&*hint)
                    .desired_width(SEARCH_WIDTH);
                ui.add(field).accessible_name(&*hint);
            });
        }
    });
    state.view.set_query(&state.query);
}

/// A tab. Whether it was clicked.
fn tab_button(ui: &mut Ui, ctx: &PageCtx<'_>, text: &str, selected: bool) -> bool {
    let p = &ctx.palette;
    let color = if selected { p.text } else { p.text_muted };
    let label = RichText::new(text).font(semibold(14.0)).color(color);
    let response = ui.add(Button::new(label).frame(false));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(Role::Tab);
        node.set_selected(selected);
    });
    if selected {
        let rect = response.rect;
        let y = rect.bottom() + SPACE[0] / 2.0;
        ui.painter().line_segment(
            [
                eframe::egui::pos2(rect.left(), y),
                eframe::egui::pos2(rect.right(), y),
            ],
            eframe::egui::Stroke::new(TAB_LINE, p.accent),
        );
    }
    response.clicked()
}

/// The progress of a running scan.
fn scanning(ui: &mut Ui, tr: &Localizer, p: &Palette, scan: ScanProgress) {
    let mut args = FluentArgs::new();
    args.set("count", scan.seen);
    let text = tr.tr_args("library-scanning", &args);
    ui.add(Label::new(RichText::new(&*text).color(p.text_muted)).selectable(false));
}

/// The Tracks tab: the scope's header when there is one, then the table.
fn tracks_tab(
    ui: &mut Ui,
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    snapshot: &Arc<LibrarySnapshot>,
) {
    if let Some(name) = scope_name(ctx.tr, state.view.scope(), snapshot) {
        ui.horizontal(|ui| {
            let back = ctx.tr.tr("local-back");
            if ui
                .add(IconButton::new(icons::CARET_LEFT, &back, &ctx.palette))
                .clicked()
            {
                state.show_all();
            }
            ui.add(
                Label::new(
                    RichText::new(name)
                        .font(semibold(16.0))
                        .color(ctx.palette.text),
                )
                .selectable(false),
            );
        });
        ui.add_space(SPACE[0]);
    }

    let filtered = state.view.is_filtered();
    if state.view.rows(snapshot).is_empty() {
        if filtered {
            let title = ctx.tr.tr("local-no-results");
            empty_state(ui, icons::MAGNIFYING_GLASS, &title, "", &ctx.palette);
        } else {
            no_tracks(ui, ctx);
        }
        return;
    }
    if let Some(key) = tracks::show(ui, "aria_local_tracks", ctx, snapshot, &mut state.view) {
        state.view.sort_by_header(key);
    }
}

/// The name of the album or artist the Tracks tab is limited to.
fn scope_name(tr: &Localizer, scope: &Scope, snapshot: &LibrarySnapshot) -> Option<String> {
    match scope {
        Scope::All | Scope::Ids(_) => None,
        Scope::Album(i) => snapshot.albums.get(*i).map(|album| {
            or_unknown(tr, album.title.as_deref(), "library-unknown-album").into_owned()
        }),
        Scope::Artist(i) => snapshot.artists.get(*i).map(|artist| {
            or_unknown(tr, artist.name.as_deref(), "library-unknown-artist").into_owned()
        }),
    }
}

/// The empty state of a library that has no folders: it offers to add one.
fn no_folders(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    let title = ctx.tr.tr("local-empty-title");
    let body = ctx.tr.tr("local-empty-body");
    let button = ctx.tr.tr("local-add-folder");
    empty_state_with(
        ui,
        icons::FOLDER_SIMPLE,
        &title,
        &body,
        &ctx.palette,
        EMPTY_BUTTON_HEIGHT,
        |ui| {
            let button =
                Button::new(&*button).min_size(eframe::egui::vec2(0.0, EMPTY_BUTTON_HEIGHT));
            if ui.add(button).clicked() {
                ctx.out.push(PageAction::PickFolder);
            }
        },
    );
}

/// The empty state of a library whose folders have not given any tracks yet.
fn no_tracks(ui: &mut Ui, ctx: &PageCtx<'_>) {
    let title = ctx.tr.tr("local-no-tracks-title");
    let body = ctx.tr.tr("local-no-tracks-body");
    empty_state(ui, icons::MUSIC_NOTES, &title, &body, &ctx.palette);
}

/// `value`, or the localized `unknown` message when the tag is missing.
fn or_unknown<'a>(tr: &'a Localizer, value: Option<&'a str>, unknown: &str) -> Cow<'a, str> {
    value.map_or_else(|| tr.tr(unknown), Cow::Borrowed)
}

#[cfg(test)]
mod tests {
    use aria_library::{Album, Artist};

    use super::*;

    fn album(title: &str, artist: &str) -> Album {
        Album {
            title: Some(Arc::from(title)),
            artist: Some(Arc::from(artist)),
            tracks: Vec::new(),
            artwork: None,
            year: None,
        }
    }

    fn snapshot(albums: Vec<Album>, artists: &[&str]) -> Arc<LibrarySnapshot> {
        Arc::new(LibrarySnapshot {
            albums,
            artists: artists
                .iter()
                .map(|name| Artist {
                    name: Some(Arc::from(*name)),
                    tracks: Vec::new(),
                })
                .collect(),
            ..LibrarySnapshot::default()
        })
    }

    #[test]
    fn an_open_album_is_followed_when_a_snapshot_renumbers_the_albums() {
        let before = snapshot(vec![album("Kind of Blue", "Miles Davis")], &[]);
        let mut state = State::default();
        state.follow_open_group(&before);
        state.open(Scope::Album(0), &before);

        // A new album sorts in front, so the open one is now at index 1.
        let after = snapshot(
            vec![
                album("Abbey Road", "Beatles"),
                album("KIND OF BLUE", "miles davis"),
            ],
            &[],
        );
        state.follow_open_group(&after);
        assert_eq!(state.view.scope(), &Scope::Album(1));
        assert_eq!(state.tab, Tab::Tracks);
    }

    #[test]
    fn an_open_artist_is_followed_and_a_sort_survives() {
        let before = snapshot(Vec::new(), &["Miles Davis"]);
        let mut state = State::default();
        state.open(Scope::Artist(0), &before);
        state.view.set_sort(SortKey::Album, false);

        let after = snapshot(Vec::new(), &["Beatles", "Miles Davis"]);
        state.follow_open_group(&after);
        assert_eq!(state.view.scope(), &Scope::Artist(1));
        assert_eq!(state.view.sort(), Some((SortKey::Album, false)));
    }

    #[test]
    fn a_group_that_is_gone_returns_to_every_track() {
        let before = snapshot(vec![album("Kind of Blue", "Miles Davis")], &[]);
        let mut state = State::default();
        state.open(Scope::Album(0), &before);

        let after = snapshot(vec![album("Abbey Road", "Beatles")], &[]);
        state.follow_open_group(&after);
        assert_eq!(state.view.scope(), &Scope::All);
        assert_eq!(state.open, None);
        // And it stays there when the next snapshot brings the album back.
        state.follow_open_group(&before);
        assert_eq!(state.view.scope(), &Scope::All);
    }

    #[test]
    fn showing_all_forgets_the_open_group() {
        let snap = snapshot(vec![album("A", "B")], &[]);
        let mut state = State::default();
        state.open(Scope::Album(0), &snap);
        state.show_all();
        let other = snapshot(vec![album("Z", "Y"), album("A", "B")], &[]);
        state.follow_open_group(&other);
        assert_eq!(state.view.scope(), &Scope::All);
    }
}
