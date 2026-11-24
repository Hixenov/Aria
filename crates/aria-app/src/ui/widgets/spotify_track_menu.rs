//! What every Spotify track offers: the heart, and the like and playlist items of its context
//! menu.
//!
//! Liking and editing need the session's edit permissions. Without them the heart and the items
//! are greyed out and say to reconnect. A track without a URI (a local file in a playlist) can be
//! neither liked nor edited.

use aria_core::i18n::Localizer;
use aria_core::spotify_service::SpotifyRequest;
use aria_core::{SpPlaylist, SpTrack};
use eframe::egui::{Button, Ui};

use super::IconButton;
use super::track_menu::HEART_BUTTON;
use crate::spotify::SpotifyUi;
use crate::ui::pages::PageAction;
use crate::ui::theme::{Palette, icons};

/// The key of the item that likes or unlikes a track, by whether it is liked now.
pub fn like_key(liked: bool) -> &'static str {
    if liked {
        "spotify-unlike"
    } else {
        "spotify-like"
    }
}

fn like(track: &SpTrack, liked: bool) -> PageAction {
    PageAction::Spotify(SpotifyRequest::Like {
        track: track.clone(),
        liked,
    })
}

/// The heart button for `track`. Returns the action when clicked.
///
/// As with local tracks, the bundled font has only the outline heart, so a liked track shows it
/// in the accent color.
pub fn heart(
    ui: &mut Ui,
    tr: &Localizer,
    p: &Palette,
    spotify: &SpotifyUi,
    track: &SpTrack,
) -> Option<PageAction> {
    let [size, icon] = HEART_BUTTON;
    let liked = track
        .uri
        .as_deref()
        .is_some_and(|uri| spotify.is_liked(uri));
    let label = tr.tr(like_key(liked));
    let button = IconButton::new(icons::HEART, &label, p)
        .size(size, icon)
        .selected(liked)
        .without_dot();
    if track.uri.is_none() {
        ui.add_enabled(false, button);
        return None;
    }
    if !spotify.can_edit() {
        ui.add_enabled(false, button)
            .on_disabled_hover_text(tr.tr("spotify-reconnect-to-edit"));
        return None;
    }
    ui.add(button).clicked().then(|| like(track, !liked))
}

/// The context menu's items for `track`: like or unlike, "Add to playlist" with the playlists
/// the user may change and "New playlist…", and, inside the opened playlist `open`, "Remove
/// from this playlist".
pub fn items(
    ui: &mut Ui,
    tr: &Localizer,
    spotify: &SpotifyUi,
    track: &SpTrack,
    open: Option<&SpPlaylist>,
    out: &mut Vec<PageAction>,
) {
    let Some(uri) = track.uri.as_deref() else {
        return;
    };
    let can = spotify.can_edit();
    let reconnect = tr.tr("spotify-reconnect-to-edit");
    let liked = spotify.is_liked(uri);
    let response = ui.add_enabled(can, Button::new(tr.tr(like_key(liked))));
    if response.clicked() {
        out.push(like(track, !liked));
    }
    response.on_disabled_hover_text(&*reconnect);
    let add = ui.add_enabled_ui(can, |ui| {
        ui.menu_button(tr.tr("track-add-to-playlist"), |ui| {
            let mut any = false;
            for playlist in spotify.library.playlists.iter().filter(|p| p.editable) {
                any = true;
                if ui.button(&playlist.name).clicked() {
                    out.push(PageAction::Spotify(SpotifyRequest::AddToPlaylist {
                        playlist: playlist.id.clone(),
                        name: playlist.name.clone(),
                        uri: uri.to_owned(),
                    }));
                }
            }
            if any {
                ui.separator();
            }
            if ui.button(tr.tr("playlist-new")).clicked() {
                out.push(PageAction::AskSpotifyPlaylistName {
                    first: Some(uri.to_owned()),
                });
            }
        })
    });
    add.response.on_disabled_hover_text(&*reconnect);
    if let Some(open) = open {
        let response = ui.add_enabled(can, Button::new(tr.tr("spotify-remove-from-playlist")));
        if response.clicked() {
            out.push(PageAction::Spotify(SpotifyRequest::RemoveFromPlaylist {
                playlist: open.id.clone(),
                uri: uri.to_owned(),
            }));
        }
        if can {
            response.on_hover_text(tr.tr("spotify-remove-every-copy"));
        } else {
            response.on_disabled_hover_text(&*reconnect);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use aria_core::spotify_service::{
        SpotifyAccount, SpotifyEvent, SpotifyLibrary, SpotifyRequest,
    };
    use aria_core::{SpPlaylist, SpTrack, i18n::Localizer, settings::Language};
    use eframe::egui::accesskit::Role;
    use egui_kittest::{
        Harness,
        kittest::{NodeT as _, Queryable as _},
    };

    use super::*;
    use crate::spotify::SpotifyUi;
    use crate::ui::pages::PageAction;
    use crate::ui::theme::Palette;

    const URI: &str = "spotify:track:t1";

    fn track(uri: Option<&str>) -> SpTrack {
        SpTrack {
            uri: uri.map(str::to_owned),
            name: "Song".to_owned(),
            artists: Vec::new(),
            album: String::new(),
            album_uri: None,
            duration_ms: 1000,
            image: None,
            image_large: None,
        }
    }

    fn playlist(id: &str, name: &str, editable: bool) -> SpPlaylist {
        SpPlaylist {
            uri: format!("spotify:playlist:{id}"),
            name: name.to_owned(),
            owner: "Ann".to_owned(),
            image: None,
            id: id.to_owned(),
            owner_id: "ann".to_owned(),
            collaborative: false,
            snapshot: None,
            editable,
        }
    }

    fn spotify(can_edit: bool, liked: &[&str], playlists: Vec<SpPlaylist>) -> SpotifyUi {
        let mut ui = SpotifyUi::default();
        ui.apply(SpotifyEvent::Account(SpotifyAccount::SignedIn {
            name: "Ann".to_owned(),
            can_edit,
        }));
        ui.apply(SpotifyEvent::Library(Arc::new(SpotifyLibrary {
            tracks: liked.iter().map(|uri| track(Some(uri))).collect(),
            playlists,
            ..SpotifyLibrary::default()
        })));
        ui
    }

    fn en(key: &str) -> String {
        Localizer::new(Language::En).unwrap().tr(key).into_owned()
    }

    fn menu(
        spotify: SpotifyUi,
        track: SpTrack,
        open: Option<SpPlaylist>,
    ) -> Harness<'static, Vec<PageAction>> {
        let tr = Localizer::new(Language::En).unwrap();
        let mut harness = Harness::new_ui_state(
            move |ui, out: &mut Vec<PageAction>| {
                items(ui, &tr, &spotify, &track, open.as_ref(), out);
            },
            Vec::new(),
        );
        harness.run();
        harness
    }

    fn heart_alone(spotify: SpotifyUi, track: SpTrack) -> Harness<'static, Vec<PageAction>> {
        let tr = Localizer::new(Language::En).unwrap();
        let mut harness = Harness::builder()
            .with_context_setup(crate::ui::theme::fonts::install)
            .build_ui_state(
                move |ui, out: &mut Vec<PageAction>| {
                    if let Some(action) = heart(ui, &tr, &Palette::dark(), &spotify, &track) {
                        out.push(action);
                    }
                },
                Vec::new(),
            );
        harness.run();
        harness
    }

    fn like(liked: bool) -> PageAction {
        PageAction::Spotify(SpotifyRequest::Like {
            track: track(Some(URI)),
            liked,
        })
    }

    #[test]
    fn a_liked_track_shows_a_heart_that_unlikes() {
        let mut harness = heart_alone(spotify(true, &[URI], vec![]), track(Some(URI)));
        harness
            .get_by_role_and_label(Role::Button, &en("spotify-unlike"))
            .click();
        harness.run();
        assert_eq!(harness.state(), &[like(false)]);
    }

    #[test]
    fn a_track_not_liked_shows_a_heart_that_likes() {
        let mut harness = heart_alone(spotify(true, &[], vec![]), track(Some(URI)));
        harness
            .get_by_role_and_label(Role::Button, &en("spotify-like"))
            .click();
        harness.run();
        assert_eq!(harness.state(), &[like(true)]);
    }

    #[test]
    fn the_heart_says_to_reconnect_without_the_permissions() {
        let mut harness = heart_alone(spotify(false, &[], vec![]), track(Some(URI)));
        let label = en("spotify-like");
        let heart = harness.get_by_role_and_label(Role::Button, &label);
        assert!(heart.accesskit_node().is_disabled());
        heart.click();
        harness.run();
        assert!(harness.state().is_empty());
        let item = menu(spotify(false, &[], vec![]), track(Some(URI)), None);
        let like_item = item.get_by_role_and_label(Role::Button, &label);
        assert!(like_item.accesskit_node().is_disabled());
    }

    #[test]
    fn a_track_without_a_uri_offers_no_heart_and_no_edits() {
        let harness = heart_alone(spotify(true, &[], vec![]), track(None));
        let label = en("spotify-like");
        let heart = harness.get_by_role_and_label(Role::Button, &label);
        assert!(heart.accesskit_node().is_disabled());
        let open = playlist("mine", "Mine", true);
        let harness = menu(
            spotify(true, &[], vec![open.clone()]),
            track(None),
            Some(open),
        );
        for key in [
            "spotify-like",
            "track-add-to-playlist",
            "spotify-remove-from-playlist",
        ] {
            assert!(harness.query_by_label(&en(key)).is_none(), "{key}");
        }
    }

    #[test]
    fn the_like_item_likes() {
        let mut harness = menu(spotify(true, &[], vec![]), track(Some(URI)), None);
        harness
            .get_by_role_and_label(Role::Button, &en("spotify-like"))
            .click();
        harness.run();
        assert_eq!(harness.state(), &[like(true)]);
    }

    fn open_add_menu(harness: &mut Harness<'static, Vec<PageAction>>) {
        harness
            .get_by_role_and_label(Role::Button, &en("track-add-to-playlist"))
            .click();
        harness.run();
    }

    #[test]
    fn the_menu_lists_only_editable_playlists_then_new() {
        let playlists = vec![
            playlist("mine", "Mine", true),
            playlist("theirs", "Theirs", false),
        ];
        let mut harness = menu(spotify(true, &[], playlists), track(Some(URI)), None);
        open_add_menu(&mut harness);
        assert!(harness.query_by_label("Theirs").is_none());
        harness.get_by_label(&en("playlist-new"));
        harness.get_by_role_and_label(Role::Button, "Mine").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &[PageAction::Spotify(SpotifyRequest::AddToPlaylist {
                playlist: "mine".to_owned(),
                name: "Mine".to_owned(),
                uri: URI.to_owned(),
            })]
        );
    }

    #[test]
    fn new_playlist_asks_for_a_name_for_spotify() {
        let mut harness = menu(spotify(true, &[], vec![]), track(Some(URI)), None);
        open_add_menu(&mut harness);
        harness
            .get_by_role_and_label(Role::Button, &en("playlist-new"))
            .click();
        harness.run();
        assert_eq!(
            harness.state(),
            &[PageAction::AskSpotifyPlaylistName {
                first: Some(URI.to_owned())
            }]
        );
    }

    #[test]
    fn remove_from_this_playlist_only_inside_an_open_playlist() {
        let harness = menu(spotify(true, &[], vec![]), track(Some(URI)), None);
        assert!(
            harness
                .query_by_label(&en("spotify-remove-from-playlist"))
                .is_none()
        );
        let open = playlist("mine", "Mine", true);
        let mut harness = menu(
            spotify(true, &[], vec![open.clone()]),
            track(Some(URI)),
            Some(open),
        );
        harness
            .get_by_role_and_label(Role::Button, &en("spotify-remove-from-playlist"))
            .click();
        harness.run();
        assert_eq!(
            harness.state(),
            &[PageAction::Spotify(SpotifyRequest::RemoveFromPlaylist {
                playlist: "mine".to_owned(),
                uri: URI.to_owned(),
            })]
        );
    }
}
