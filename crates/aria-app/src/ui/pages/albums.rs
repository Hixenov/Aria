//! The Albums page: your saved Spotify albums. A click plays the album.

use eframe::egui::Ui;

use super::PageCtx;
use super::spotify_lists::{self, Row};
use crate::nav::Route;

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    if !spotify_lists::ready(ui, ctx, Route::Albums) {
        return;
    }
    let library = &*ctx.spotify.library;
    if spotify_lists::loading(ui, ctx, Route::Albums, library.albums.is_empty()) {
        return;
    }
    let rows: Vec<Row<'_>> = library.albums.iter().map(Row::Album).collect();
    spotify_lists::draw_rows(ui, ctx, &rows, "aria_spotify_albums", &[]);
}
