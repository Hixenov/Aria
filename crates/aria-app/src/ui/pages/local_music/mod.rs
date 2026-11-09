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

pub use self::view::{Scope, SortKey, Tab, TrackView};
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

/// What the page remembers between frames.
#[derive(Clone, Default)]
struct State {
    tab: Tab,
    /// What is typed in the search field.
    query: String,
    view: TrackView,
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
    state.drop_stale_scope(&snapshot);
    let has_folders = !snapshot.folders.is_empty();

    header(ui, ctx, has_folders);
    if !has_folders {
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
            if let Some(i) = groups::albums(ui, ctx.tr, &ctx.palette, &snapshot) {
                state.open(Scope::Album(i));
            }
        }
        Tab::Artists if snapshot.artists.is_empty() => no_tracks(ui, ctx),
        Tab::Artists => {
            if let Some(i) = groups::artists(ui, ctx.tr, &ctx.palette, &snapshot) {
                state.open(Scope::Artist(i));
            }
        }
    }
}

impl State {
    /// Shows the tracks of `scope`, whatever was searched for before.
    fn open(&mut self, scope: Scope) {
        self.query.clear();
        self.view.set_scope(scope);
        self.tab = Tab::Tracks;
    }

    /// A new snapshot can have fewer albums or artists than the one the scope pointed into.
    fn drop_stale_scope(&mut self, snapshot: &LibrarySnapshot) {
        let gone = match self.view.scope() {
            Scope::All => false,
            Scope::Album(i) => i >= snapshot.albums.len(),
            Scope::Artist(i) => i >= snapshot.artists.len(),
        };
        if gone {
            self.view.set_scope(Scope::All);
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
                    state.view.set_scope(Scope::All);
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
                state.view.set_scope(Scope::All);
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
    if let Some(key) = tracks::show(ui, ctx, snapshot, &mut state.view) {
        let (current, ascending) = state.view.sort();
        // Another column starts ascending; the same one turns around.
        state.view.set_sort(key, key != current || !ascending);
    }
}

/// The name of the album or artist the Tracks tab is limited to.
fn scope_name(tr: &Localizer, scope: Scope, snapshot: &LibrarySnapshot) -> Option<String> {
    match scope {
        Scope::All => None,
        Scope::Album(i) => snapshot.albums.get(i).map(|album| {
            or_unknown(tr, album.title.as_deref(), "library-unknown-album").into_owned()
        }),
        Scope::Artist(i) => snapshot.artists.get(i).map(|artist| {
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
