//! The Liked Songs page: the favorites, newest first until the user sorts them.

use std::sync::Arc;

use aria_library::TrackId;
use eframe::egui::{Align, Id, Layout, TextEdit, Ui};

use super::{PageCtx, Scope, TrackView, local_music::show_tracks};
use crate::{
    nav::Route,
    ui::{
        theme::icons,
        theme::visuals::SPACE,
        widgets::{empty_state, section_header},
    },
};

/// Width of the search field.
const SEARCH_WIDTH: f32 = 260.0;

/// What the page remembers between frames.
#[derive(Clone)]
struct State {
    /// What is typed in the search field.
    query: String,
    view: TrackView,
    /// The favorites the view's scope was made from.
    favorites: Vec<TrackId>,
}

impl Default for State {
    /// No favorites yet, and a view that shows nothing rather than the whole library.
    fn default() -> Self {
        let mut view = TrackView::new();
        view.move_scope(Scope::Ids(Arc::from([])));
        Self {
            query: String::new(),
            view,
            favorites: Vec::new(),
        }
    }
}

impl State {
    /// Points the view at `favorites` when they are not the ones it already has. Only the
    /// scope moves: a sort the user chose stays.
    fn follow(&mut self, favorites: &[TrackId]) {
        if self.favorites != favorites {
            self.favorites = favorites.to_vec();
            self.view.move_scope(Scope::Ids(Arc::from(favorites)));
        }
    }
}

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    // Taken out of egui's memory while it is used, as Local Music does: memory stays locked
    // while borrowed from there, and drawing must not hold that.
    let id = Id::unique("aria_liked");
    let mut state: State = ui.data_mut(|data| data.remove_temp(id)).unwrap_or_default();
    draw(ui, ctx, &mut state);
    ui.data_mut(|data| data.insert_temp(id, state));
}

fn draw(ui: &mut Ui, ctx: &mut PageCtx<'_>, state: &mut State) {
    let snapshot = Arc::clone(&ctx.library.snapshot);
    state.follow(&ctx.collections.favorites);

    ui.horizontal(|ui| {
        section_header(ui, &ctx.tr.tr(Route::Liked.label_key()));
        if state.favorites.is_empty() {
            return;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let hint = ctx.tr.tr("local-search");
            let field = TextEdit::singleline(&mut state.query)
                .hint_text(&*hint)
                .desired_width(SEARCH_WIDTH);
            ui.add(field).accessible_name(&*hint);
        });
    });
    state.view.set_query(&state.query);
    ui.add_space(SPACE[1]);

    let filtered = state.view.is_filtered();
    if state.view.rows(&snapshot).is_empty() {
        let p = &ctx.palette;
        if filtered {
            let title = ctx.tr.tr("local-no-results");
            empty_state(ui, icons::MAGNIFYING_GLASS, &title, "", p);
        } else {
            let (title, hint) = (ctx.tr.tr("liked-empty"), ctx.tr.tr("liked-empty-hint"));
            empty_state(ui, icons::HEART, &title, &hint, p);
        }
        return;
    }
    if let Some(key) = show_tracks(ui, ctx, &snapshot, &mut state.view) {
        state.view.sort_by_header(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_favorites_list_moves_the_scope_and_keeps_the_sort() {
        let mut state = State::default();
        state.follow(&[TrackId(2), TrackId(1)]);
        assert_eq!(
            state.view.scope(),
            &Scope::Ids(Arc::from([TrackId(2), TrackId(1)]))
        );
        state.view.set_sort(super::super::SortKey::Artist, false);
        state.follow(&[TrackId(3), TrackId(2), TrackId(1)]);
        assert_eq!(
            state.view.sort(),
            Some((super::super::SortKey::Artist, false))
        );
    }
}
