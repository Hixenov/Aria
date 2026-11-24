//! The Your Library page: Liked Songs (your saved Spotify tracks) and your Spotify playlists.
//! A playlist you own or collaborate on opens here, with its tracks.

use aria_core::i18n::FluentArgs;
use aria_core::spotify_service::SpotifyRequest;
use aria_core::{PlayRequest, SpPlaylist};
use eframe::egui::{Button, Label, RichText, TextStyle, Ui};

use super::spotify_lists::{self, Row};
use super::{PageAction, PageCtx};
use crate::nav::Route;
use crate::ui::theme::fonts::semibold;
use crate::ui::theme::visuals::SPACE;
use crate::ui::widgets::empty_state;

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    if !spotify_lists::ready(ui, ctx, Route::Library) {
        return;
    }
    if let Some(id) = spotify_lists::open_playlist(ui) {
        // Only while it is still one of the user's playlists they may change.
        let open = ctx
            .spotify
            .library
            .playlists
            .iter()
            .find(|p| p.id == id && p.editable)
            .cloned();
        match open {
            Some(playlist) => {
                playlist_view(ui, ctx, &playlist);
                return;
            }
            None => spotify_lists::set_open_playlist(ui, None),
        }
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
        rows.extend(library.playlists.iter().map(Row::LibraryPlaylist));
    }
    spotify_lists::draw_rows(ui, ctx, &rows, "aria_spotify_library", &library.tracks);
}

/// The opened playlist: Back, its name and Play, then its tracks (or why there are none).
fn playlist_view(ui: &mut Ui, ctx: &mut PageCtx<'_>, playlist: &SpPlaylist) {
    let p = ctx.palette;
    ui.horizontal(|ui| {
        if ui
            .add(Button::new(ctx.tr.tr("spotify-playlist-back")))
            .clicked()
        {
            spotify_lists::set_open_playlist(ui, None);
        }
        ui.add_space(SPACE[1]);
        let size = TextStyle::Heading.resolve(ui.style()).size;
        let name = RichText::new(&playlist.name)
            .font(semibold(size))
            .color(p.text);
        ui.add(Label::new(name).selectable(false).truncate());
        ui.add_space(SPACE[1]);
        if ui
            .add(Button::new(ctx.tr.tr("spotify-playlist-play")))
            .clicked()
        {
            ctx.out.push(PageAction::Spotify(SpotifyRequest::Play(
                PlayRequest::Context {
                    uri: playlist.uri.clone(),
                    offset_uri: None,
                },
            )));
        }
    });
    ui.add_space(SPACE[2]);
    let loaded = ctx
        .spotify
        .playlist
        .clone()
        .filter(|open| open.id == playlist.id);
    let key = match loaded.as_deref() {
        None => Some("spotify-playlist-loading"),
        Some(open) if open.tracks.is_empty() => Some(if open.loading {
            "spotify-playlist-loading"
        } else if open.failed {
            "spotify-playlist-failed"
        } else {
            "spotify-playlist-empty"
        }),
        Some(_) => None,
    };
    if let Some(key) = key {
        empty_state(ui, Route::Library.icon(), &ctx.tr.tr(key), "", &p);
        return;
    }
    let Some(open) = loaded else {
        return;
    };
    let rows: Vec<Row<'_>> = open
        .tracks
        .iter()
        .map(|track| Row::PlaylistTrack { track, playlist })
        .collect();
    spotify_lists::draw_rows(ui, ctx, &rows, "aria_spotify_open_playlist", &open.tracks);
}
