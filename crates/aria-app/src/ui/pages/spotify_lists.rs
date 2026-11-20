//! What the Spotify pages share: the signed-out and loading states, the rows of their lists,
//! and what a click on each row plays.

use aria_core::i18n::FluentArgs;
use aria_core::spotify_service::{SpotifyAccount, SpotifyRequest};
use aria_core::{PlayRequest, SpAlbum, SpArtist, SpPlaylist, SpTrack};
use eframe::egui::{
    Align2, Button, ScrollArea, Sense, TextStyle, Ui, WidgetInfo, accesskit::Role, vec2,
};

use super::{PageAction, PageCtx};
use crate::nav::Route;
use crate::ui::theme::fonts::semibold;
use crate::ui::widgets::spotify_row::{SPOTIFY_ROW, spotify_row};
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
    Playlist(&'a SpPlaylist),
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
            let mut subtitle = track.artists.join(", ");
            if !track.album.is_empty() {
                if !subtitle.is_empty() {
                    subtitle.push_str(" · ");
                }
                subtitle.push_str(&track.album);
            }
            let playable = track.uri.is_some();
            let clicked = spotify_row(
                ui,
                ctx,
                track.image.as_deref(),
                &track.name,
                &subtitle,
                playable,
            )
            .clicked();
            clicked
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
            let mut args = FluentArgs::new();
            args.set("owner", playlist.owner.as_str());
            let by = ctx.tr.tr_args("spotify-by", &args).into_owned();
            let image = playlist.image.as_deref();
            spotify_row(ui, ctx, image, &playlist.name, &by, true)
                .clicked()
                .then(|| context(&playlist.uri))
        }
    };
    if let Some(play) = play {
        ctx.out
            .push(PageAction::Spotify(SpotifyRequest::Play(play)));
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
