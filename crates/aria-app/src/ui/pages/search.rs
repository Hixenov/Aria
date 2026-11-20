//! The Search page: one field, and Spotify's results in four groups. A track plays within its
//! album.

use aria_core::spotify_service::SpotifyRequest;
use eframe::egui::{TextEdit, TextStyle, Ui};

use super::spotify_lists::{self, Row};
use super::{PageAction, PageCtx};
use crate::nav::Route;
use crate::ui::theme::visuals::SPACE;
use crate::ui::widgets::empty_state;

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    if !spotify_lists::ready(ui, ctx, Route::Search) {
        return;
    }
    let spotify = ctx.spotify;
    let mut text = spotify.search_text.clone();
    let hint = ctx.tr.tr("spotify-search-hint");
    let field = TextEdit::singleline(&mut text)
        .id_salt("aria_spotify_search_field")
        .hint_text(&*hint)
        .font(TextStyle::Body)
        .desired_width(f32::INFINITY);
    let response = ui.add(field).accessible_name(&*hint);
    if response.changed() {
        ctx.out
            .push(PageAction::Spotify(SpotifyRequest::Search(text.clone())));
    }
    ui.add_space(SPACE[2]);
    let query = text.trim();
    if query.is_empty() {
        return;
    }
    let search = &spotify.search;
    let results = &search.results;
    if search.query.trim() == query && results.is_empty() {
        let none = ctx.tr.tr("spotify-search-none");
        empty_state(ui, Route::Search.icon(), &none, "", &ctx.palette);
        return;
    }
    let mut rows = Vec::new();
    if !results.tracks.is_empty() {
        rows.push(Row::Heading(ctx.tr.tr("spotify-songs").into_owned()));
        rows.extend(
            results
                .tracks
                .iter()
                .enumerate()
                .map(|(index, track)| Row::Track {
                    index,
                    track,
                    from_list: false,
                }),
        );
    }
    if !results.albums.is_empty() {
        rows.push(Row::Heading(ctx.tr.tr("spotify-albums").into_owned()));
        rows.extend(results.albums.iter().map(Row::Album));
    }
    if !results.artists.is_empty() {
        rows.push(Row::Heading(ctx.tr.tr("spotify-artists").into_owned()));
        rows.extend(results.artists.iter().map(Row::Artist));
    }
    if !results.playlists.is_empty() {
        rows.push(Row::Heading(ctx.tr.tr("spotify-playlists").into_owned()));
        rows.extend(results.playlists.iter().map(Row::Playlist));
    }
    spotify_lists::draw_rows(ui, ctx, &rows, "aria_spotify_search", &[]);
}
