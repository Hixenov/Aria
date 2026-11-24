//! What the Spotify pages share: the signed-out and loading states, the rows of their lists,
//! and what a click on each row plays.

use aria_core::i18n::FluentArgs;
use aria_core::spotify_service::{SpotifyAccount, SpotifyRequest};
use aria_core::{PlayRequest, SpAlbum, SpArtist, SpPlaylist, SpTrack};
use eframe::egui::{
    Align2, Button, Id, Rect, Response, ScrollArea, Sense, TextStyle, Ui, UiBuilder, Vec2,
    WidgetInfo, accesskit::Role, pos2, vec2,
};

use super::{PageAction, PageCtx};
use crate::nav::Route;
use crate::ui::theme::fonts::semibold;
use crate::ui::theme::visuals::SPACE;
use crate::ui::widgets::spotify_row::{SPOTIFY_ROW, spotify_row, spotify_row_with_room};
use crate::ui::widgets::spotify_track_menu;
use crate::ui::widgets::track_menu::HEART_BUTTON;
use crate::ui::widgets::{empty_state, empty_state_with, section_header};

/// The most tracks a Liked Songs click hands Spotify.
pub const LIKED_LIMIT: usize = 100;
/// Height of the Connect button, for centering the empty state.
const BUTTON_HEIGHT: f32 = 28.0;

/// One row of a list. Headings are rows too, as tall as the others, so `show_rows` can count.
pub enum Row<'a> {
    Heading(String),
    /// A track; `from_list` plays it with the ones after it in `list`, else within its album.
    Track {
        index: usize,
        track: &'a SpTrack,
        from_list: bool,
    },
    Album(&'a SpAlbum),
    Artist(&'a SpArtist),
    /// A playlist that plays when clicked.
    Playlist(&'a SpPlaylist),
    /// One of the user's playlists on Your Library: it opens when the user may change it, and
    /// otherwise plays, saying why it does not open.
    LibraryPlaylist(&'a SpPlaylist),
    /// A track of the opened playlist: it plays the playlist from there.
    PlaylistTrack {
        track: &'a SpTrack,
        playlist: &'a SpPlaylist,
    },
}

/// Where Your Library keeps the id of the playlist it shows, in egui's memory.
const OPEN_PLAYLIST: &str = "aria_spotify_open_playlist";

/// The id of the playlist Your Library shows, if one is open.
pub fn open_playlist(ui: &Ui) -> Option<String> {
    ui.data(|d| d.get_temp::<String>(Id::unique(OPEN_PLAYLIST)))
}

/// Opens the playlist `id` on Your Library, or (`None`) goes back to the list.
pub fn set_open_playlist(ui: &Ui, id: Option<String>) {
    let key = Id::unique(OPEN_PLAYLIST);
    ui.data_mut(|d| match id {
        Some(id) => {
            d.insert_temp(key, id);
        }
        None => d.remove::<String>(key),
    });
}

/// The page's heading, then, unless signed in, the state that offers to connect. Returns
/// whether the page should draw its content.
pub fn ready(ui: &mut Ui, ctx: &mut PageCtx<'_>, route: Route) -> bool {
    section_header(ui, &ctx.tr.tr(route.label_key()));
    let waiting = match ctx.spotify.account {
        SpotifyAccount::SignedIn { .. } => return true,
        SpotifyAccount::SigningIn => true,
        SpotifyAccount::SignedOut { .. } => false,
    };
    let tr = ctx.tr;
    let p = ctx.palette;
    let title = tr.tr("spotify-page-signed-out");
    let label = if waiting {
        tr.tr("settings-spotify-waiting")
    } else {
        tr.tr("settings-spotify-connect")
    };
    let out = &mut *ctx.out;
    empty_state_with(ui, route.icon(), &title, "", &p, BUTTON_HEIGHT, |ui| {
        if ui.add_enabled(!waiting, Button::new(&*label)).clicked() {
            out.push(PageAction::Spotify(SpotifyRequest::SignIn {
                done_page: String::new(),
                failed_page: String::new(),
            }));
        }
    });
    false
}

/// While the library loads and there is nothing to show yet, says so. Returns whether it did.
pub fn loading(ui: &mut Ui, ctx: &PageCtx<'_>, route: Route, empty: bool) -> bool {
    if !empty {
        return false;
    }
    let library = &ctx.spotify.library;
    let key = if library.loading {
        "spotify-loading"
    } else if library.failed {
        "spotify-load-failed"
    } else {
        "spotify-empty"
    };
    empty_state(ui, route.icon(), &ctx.tr.tr(key), "", &ctx.palette);
    true
}

/// What a click on a track plays: within its album, or with the tracks after it in `list`.
/// `None` for a track that cannot be played (a local file).
pub fn play_track(track: &SpTrack, index: usize, list: Option<&[SpTrack]>) -> Option<PlayRequest> {
    let uri = track.uri.clone()?;
    if let Some(list) = list {
        let uris: Vec<String> = list
            .iter()
            .skip(index)
            .filter_map(|t| t.uri.clone())
            .take(LIKED_LIMIT)
            .collect();
        return Some(PlayRequest::Tracks { uris, start: 0 });
    }
    Some(match &track.album_uri {
        Some(album) => PlayRequest::Context {
            uri: album.clone(),
            offset_uri: Some(uri),
        },
        None => PlayRequest::Tracks {
            uris: vec![uri],
            start: 0,
        },
    })
}

/// Draws `rows`, only those in view. `list` is the list `from_list` tracks play from.
pub fn draw_rows(
    ui: &mut Ui,
    ctx: &mut PageCtx<'_>,
    rows: &[Row<'_>],
    salt: &str,
    list: &[SpTrack],
) {
    ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .show_rows(ui, SPOTIFY_ROW, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in &rows[range] {
                draw_row(ui, ctx, row, list);
            }
        });
}

fn draw_row(ui: &mut Ui, ctx: &mut PageCtx<'_>, row: &Row<'_>, list: &[SpTrack]) {
    let play = match row {
        Row::Heading(text) => {
            heading(ui, ctx, text);
            None
        }
        Row::Track {
            index,
            track,
            from_list,
        } => {
            let subtitle = track_subtitle(track);
            let playable = track.uri.is_some();
            let row = spotify_row_with_room(
                ui,
                ctx,
                track.image.as_deref(),
                &track.name,
                &subtitle,
                playable,
                HEART_ROOM,
            );
            track_extras(ui, ctx, &row, track, None);
            row.clicked()
                .then(|| play_track(track, *index, from_list.then_some(list)))
                .flatten()
        }
        Row::Album(album) => {
            let subtitle = album.artists.join(", ");
            let image = album.image.as_deref();
            spotify_row(ui, ctx, image, &album.name, &subtitle, true)
                .clicked()
                .then(|| context(&album.uri))
        }
        Row::Artist(artist) => {
            let image = artist.image.as_deref();
            spotify_row(ui, ctx, image, &artist.name, "", true)
                .clicked()
                .then(|| context(&artist.uri))
        }
        Row::Playlist(playlist) => {
            let by = by_line(ctx, playlist);
            let image = playlist.image.as_deref();
            spotify_row(ui, ctx, image, &playlist.name, &by, true)
                .clicked()
                .then(|| context(&playlist.uri))
        }
        Row::LibraryPlaylist(playlist) => {
            let by = by_line(ctx, playlist);
            let image = playlist.image.as_deref();
            let row = spotify_row(ui, ctx, image, &playlist.name, &by, true);
            if playlist.editable {
                if row.clicked() {
                    set_open_playlist(ui, Some(playlist.id.clone()));
                    ctx.out
                        .push(PageAction::Spotify(SpotifyRequest::OpenPlaylist(
                            playlist.id.clone(),
                        )));
                }
                None
            } else {
                let row = row.on_hover_text(ctx.tr.tr("spotify-follow-only"));
                row.clicked().then(|| context(&playlist.uri))
            }
        }
        Row::PlaylistTrack { track, playlist } => {
            let subtitle = track_subtitle(track);
            let playable = track.uri.is_some();
            let row = spotify_row_with_room(
                ui,
                ctx,
                track.image.as_deref(),
                &track.name,
                &subtitle,
                playable,
                HEART_ROOM,
            );
            track_extras(ui, ctx, &row, track, Some(playlist));
            row.clicked()
                .then(|| PlayRequest::Context {
                    uri: playlist.uri.clone(),
                    offset_uri: track.uri.clone(),
                })
                .filter(|_| playable)
        }
    };
    if let Some(play) = play {
        ctx.out
            .push(PageAction::Spotify(SpotifyRequest::Play(play)));
    }
}

/// A track's artists, then its album.
fn track_subtitle(track: &SpTrack) -> String {
    let mut subtitle = track.artists.join(", ");
    if !track.album.is_empty() {
        if !subtitle.is_empty() {
            subtitle.push_str(" · ");
        }
        subtitle.push_str(&track.album);
    }
    subtitle
}

/// "by <owner>" for a playlist row.
fn by_line(ctx: &PageCtx<'_>, playlist: &SpPlaylist) -> String {
    let mut args = FluentArgs::new();
    args.set("owner", playlist.owner.as_str());
    ctx.tr.tr_args("spotify-by", &args).into_owned()
}

/// Room kept at a track row's right end for its heart.
const HEART_ROOM: f32 = HEART_BUTTON[0] + SPACE[1];

/// The heart at the right end of a track's `row`, and the row's context menu; `open` is the
/// opened playlist the row belongs to.
pub fn track_extras(
    ui: &mut Ui,
    ctx: &mut PageCtx<'_>,
    row: &Response,
    track: &SpTrack,
    open: Option<&SpPlaylist>,
) {
    let size = HEART_BUTTON[0];
    let center = pos2(
        row.rect.right() - SPACE[1] - size / 2.0,
        row.rect.center().y,
    );
    let rect = Rect::from_center_size(center, Vec2::splat(size));
    let (tr, palette, spotify) = (ctx.tr, ctx.palette, ctx.spotify);
    // A child that takes no room from the list, drawn after the row so it gets the clicks.
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    if let Some(action) = spotify_track_menu::heart(&mut child, tr, &palette, spotify, track) {
        ctx.out.push(action);
    }
    if track.uri.is_some() {
        let out = &mut *ctx.out;
        row.context_menu(|ui| spotify_track_menu::items(ui, tr, spotify, track, open, out));
    }
}

fn context(uri: &str) -> PlayRequest {
    PlayRequest::Context {
        uri: uri.to_owned(),
        offset_uri: None,
    }
}

/// A section's heading, as tall as a row.
fn heading(ui: &mut Ui, ctx: &PageCtx<'_>, text: &str) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), SPOTIFY_ROW), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, text));
    if ui.is_rect_visible(rect) {
        let size = TextStyle::Body.resolve(ui.style()).size;
        ui.painter().text(
            rect.left_bottom() + vec2(0.0, -8.0),
            Align2::LEFT_BOTTOM,
            text,
            semibold(size + 2.0),
            ctx.palette.text,
        );
    }
}
