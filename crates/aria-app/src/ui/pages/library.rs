//! The Your Library page: Liked Songs (your saved Spotify tracks) and your Spotify playlists.

use aria_core::i18n::FluentArgs;
use eframe::egui::Ui;

use super::PageCtx;
use super::spotify_lists::{self, Row};
use crate::nav::Route;

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    if !spotify_lists::ready(ui, ctx, Route::Library) {
        return;
    }
    let library = &ctx.spotify.library;
    let empty = library.tracks.is_empty() && library.playlists.is_empty();
    if spotify_lists::loading(ui, ctx, Route::Library, empty) {
        return;
    }
    let library = &*ctx.spotify.library;
    let mut rows = Vec::with_capacity(library.tracks.len() + library.playlists.len() + 2);
    if !library.tracks.is_empty() {
        let mut args = FluentArgs::new();
        args.set("count", library.tracks.len());
        let heading = ctx.tr.tr_args("spotify-liked", &args).into_owned();
        rows.push(Row::Heading(heading));
        rows.extend(
            library
                .tracks
                .iter()
                .enumerate()
                .map(|(index, track)| Row::Track {
                    index,
                    track,
                    from_list: true,
                }),
        );
    }
    if !library.playlists.is_empty() {
        rows.push(Row::Heading(ctx.tr.tr("spotify-playlists").into_owned()));
        rows.extend(library.playlists.iter().map(Row::Playlist));
    }
    spotify_lists::draw_rows(ui, ctx, &rows, "aria_spotify_library", &library.tracks);
}
