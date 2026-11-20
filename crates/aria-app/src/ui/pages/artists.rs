//! The Artists page: the artists you follow on Spotify. A click plays the artist, with the
//! tracks Spotify picks.

use eframe::egui::Ui;

use super::PageCtx;
use super::spotify_lists::{self, Row};
use crate::nav::Route;

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    if !spotify_lists::ready(ui, ctx, Route::Artists) {
        return;
    }
    let library = &*ctx.spotify.library;
    if spotify_lists::loading(ui, ctx, Route::Artists, library.artists.is_empty()) {
        return;
    }
    let rows: Vec<Row<'_>> = library.artists.iter().map(Row::Artist).collect();
    spotify_lists::draw_rows(ui, ctx, &rows, "aria_spotify_artists", &[]);
}
