//! Headless tests of the Spotify pages: Search, Your Library, Albums and Artists.

use std::sync::Arc;

use aria_app::{
    nav::Route,
    spotify::{RemoteCovers, SpotifyUi},
    ui::{
        pages::{self, LibraryView, PageAction, PageCtx},
        theme::{Palette, fonts},
    },
};
use aria_core::{
    PlayRequest, SearchResults, SpAlbum, SpArtist, SpPlaylist, SpTrack,
    i18n::{FluentArgs, Localizer},
    settings::{Language, Settings},
    spotify_service::{SpotifyAccount, SpotifyLibrary, SpotifyRequest, SpotifySearch},
};
use eframe::egui::{self, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable as _};

const SIZE: egui::Vec2 = egui::vec2(1000.0, 700.0);

fn english() -> Localizer {
    Localizer::new(Language::En).unwrap_or_else(|err| panic!("English locale: {err}"))
}

fn en(key: &str) -> String {
    english().tr(key).into_owned()
}

struct Page {
    tr: Localizer,
    settings: Settings,
    modal_open: bool,
    library: LibraryView,
    actions: Vec<PageAction>,
    spotify: SpotifyUi,
    covers: RemoteCovers,
    route: Route,
}

fn harness(route: Route, spotify: SpotifyUi) -> Harness<'static, Page> {
    let page = Page {
        tr: english(),
        settings: Settings::default(),
        modal_open: false,
        library: LibraryView::default(),
        actions: Vec::new(),
        spotify,
        covers: RemoteCovers::default(),
        route,
    };
    Harness::builder()
        .with_size(SIZE)
        .with_context_setup(fonts::install)
        .build_ui_state(
            |ui, page: &mut Page| {
                let before = page.actions.len();
                let mut ctx = PageCtx {
                    tr: &page.tr,
                    palette: Palette::dark(),
                    settings: &page.settings,
                    paths_available: true,
                    exclusive_supported: true,
                    modal_open: &mut page.modal_open,
                    out: &mut page.actions,
                    library: &page.library,
                    collections: &Default::default(),
                    soundboard: &Default::default(),
                    inactive_hotkeys: &[],
                    devices: &[],
                    input_devices: &[],
                    artwork: None,
                    spotify: &page.spotify,
                    covers: &mut page.covers,
                };
                pages::show(page.route, ui, &mut ctx);
                // What the app does with a search: the field keeps the text.
                for action in &page.actions[before..] {
                    if let PageAction::Spotify(SpotifyRequest::Search(text)) = action {
                        page.spotify.search_text.clone_from(text);
                    }
                }
            },
            page,
        )
}

fn track(n: usize, uri: bool) -> SpTrack {
    SpTrack {
        uri: uri.then(|| format!("spotify:track:t{n}")),
        name: format!("Song {n}"),
        artists: vec!["Band".to_owned()],
        album: "Record".to_owned(),
        album_uri: uri.then(|| "spotify:album:a1".to_owned()),
        duration_ms: 180_000,
        image: None,
        image_large: None,
    }
}

fn signed_in() -> SpotifyUi {
    SpotifyUi {
        account: SpotifyAccount::SignedIn {
            name: "Ann".to_owned(),
        },
        ..SpotifyUi::default()
    }
}

fn with_library(library: SpotifyLibrary) -> SpotifyUi {
    SpotifyUi {
        library: Arc::new(library),
        ..signed_in()
    }
}

fn plays(harness: &Harness<'_, Page>) -> Vec<PlayRequest> {
    harness
        .state()
        .actions
        .iter()
        .filter_map(|a| match a {
            PageAction::Spotify(SpotifyRequest::Play(play)) => Some(play.clone()),
            _ => None,
        })
        .collect()
}

fn click(harness: &mut Harness<'_, Page>, label: &str) {
    harness.get_by_role_and_label(Role::Button, label).click();
    harness.run();
}

#[test]
fn signed_out_pages_offer_to_connect() {
    for route in [Route::Search, Route::Library, Route::Albums, Route::Artists] {
        let mut harness = harness(route, SpotifyUi::default());
        harness.get_by_label(&en("spotify-page-signed-out"));
        click(&mut harness, &en("settings-spotify-connect"));
        assert!(
            matches!(
                harness.state().actions.as_slice(),
                [PageAction::Spotify(SpotifyRequest::SignIn { .. })]
            ),
            "{route:?}: {:?}",
            harness.state().actions
        );
    }
}

#[test]
fn typing_in_search_sends_the_query() {
    let mut harness = harness(Route::Search, signed_in());
    let hint = en("spotify-search-hint");
    let field = harness.get_by_role_and_label(Role::TextInput, &hint);
    field.focus();
    field.type_text("daft");
    harness.run();
    let searches: Vec<&str> = harness
        .state()
        .actions
        .iter()
        .filter_map(|a| match a {
            PageAction::Spotify(SpotifyRequest::Search(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(searches.last(), Some(&"daft"), "{searches:?}");
}

fn searched(results: SearchResults, query: &str) -> SpotifyUi {
    SpotifyUi {
        search: Arc::new(SpotifySearch {
            query: query.to_owned(),
            results,
        }),
        search_text: query.to_owned(),
        ..signed_in()
    }
}

#[test]
fn a_search_track_plays_in_its_album() {
    let results = SearchResults {
        tracks: vec![track(1, true)],
        ..SearchResults::default()
    };
    let mut harness = harness(Route::Search, searched(results, "song"));
    harness.get_by_label(&en("spotify-songs"));
    click(&mut harness, "Song 1");
    assert_eq!(
        plays(&harness),
        [PlayRequest::Context {
            uri: "spotify:album:a1".to_owned(),
            offset_uri: Some("spotify:track:t1".to_owned()),
        }]
    );
}

#[test]
fn an_album_artist_or_playlist_plays_as_a_context() {
    let results = SearchResults {
        albums: vec![SpAlbum {
            uri: "spotify:album:x".to_owned(),
            name: "Homework".to_owned(),
            artists: vec!["Daft Punk".to_owned()],
            image: None,
        }],
        artists: vec![SpArtist {
            uri: "spotify:artist:y".to_owned(),
            name: "Daft Punk".to_owned(),
            image: None,
        }],
        playlists: vec![SpPlaylist {
            uri: "spotify:playlist:z".to_owned(),
            name: "Road Trip".to_owned(),
            owner: "Ann".to_owned(),
            image: None,
        }],
        ..SearchResults::default()
    };
    let mut harness = harness(Route::Search, searched(results, "daft"));
    for name in ["Homework", "Daft Punk", "Road Trip"] {
        click(&mut harness, name);
    }
    let mut args = FluentArgs::new();
    args.set("owner", "Ann");
    let by = english().tr_args("spotify-by", &args).into_owned();
    assert!(!by.is_empty());
    let uris: Vec<String> = plays(&harness)
        .into_iter()
        .map(|play| match play {
            PlayRequest::Context {
                uri,
                offset_uri: None,
            } => uri,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        uris,
        ["spotify:album:x", "spotify:artist:y", "spotify:playlist:z"]
    );
}

#[test]
fn a_liked_song_plays_with_the_ones_after_it() {
    let tracks: Vec<SpTrack> = (0..150).map(|n| track(n, true)).collect();
    let mut harness = harness(
        Route::Library,
        with_library(SpotifyLibrary {
            tracks,
            ..SpotifyLibrary::default()
        }),
    );
    let mut args = FluentArgs::new();
    args.set("count", 150);
    harness.get_by_label(
        &Localizer::new(Language::En)
            .unwrap()
            .tr_args("spotify-liked", &args),
    );
    click(&mut harness, "Song 3");
    let played = plays(&harness);
    let [PlayRequest::Tracks { uris, start: 0 }] = played.as_slice() else {
        panic!("{:?}", plays(&harness));
    };
    assert_eq!(uris.len(), 100);
    assert_eq!(uris[0], "spotify:track:t3");
    assert_eq!(uris[99], "spotify:track:t102");
}

#[test]
fn a_local_file_does_not_play() {
    let tracks = vec![track(0, false), track(1, true)];
    let mut harness = harness(
        Route::Library,
        with_library(SpotifyLibrary {
            tracks,
            ..SpotifyLibrary::default()
        }),
    );
    click(&mut harness, "Song 0");
    assert!(plays(&harness).is_empty(), "{:?}", plays(&harness));
    click(&mut harness, "Song 1");
    let played = plays(&harness);
    let [PlayRequest::Tracks { uris, .. }] = played.as_slice() else {
        panic!("{:?}", plays(&harness));
    };
    assert_eq!(uris, &["spotify:track:t1".to_owned()]);
}

#[test]
fn a_long_list_draws_only_what_is_visible() {
    let tracks: Vec<SpTrack> = (0..5000).map(|n| track(n, true)).collect();
    let harness = harness(
        Route::Library,
        with_library(SpotifyLibrary {
            tracks,
            ..SpotifyLibrary::default()
        }),
    );
    let drawn = (0..5000)
        .filter(|n| harness.query_by_label(&format!("Song {n}")).is_some())
        .count();
    assert!(drawn > 0 && drawn < 100, "{drawn} rows drawn");
}

#[test]
fn no_results_says_so() {
    let harness = harness(Route::Search, searched(SearchResults::default(), "zzzz"));
    harness.get_by_label(&en("spotify-search-none"));
}

#[test]
fn an_empty_or_loading_library_says_so() {
    let harness = harness(
        Route::Albums,
        with_library(SpotifyLibrary {
            loading: true,
            ..SpotifyLibrary::default()
        }),
    );
    harness.get_by_label(&en("spotify-loading"));
    let harness = self::harness(Route::Artists, with_library(SpotifyLibrary::default()));
    harness.get_by_label(&en("spotify-empty"));
}

#[test]
fn a_library_that_failed_to_load_says_so() {
    let harness = harness(
        Route::Library,
        with_library(SpotifyLibrary {
            failed: true,
            ..SpotifyLibrary::default()
        }),
    );
    harness.get_by_label(&en("spotify-load-failed"));
}

#[test]
fn the_four_routes_are_no_longer_pending() {
    for route in [Route::Search, Route::Library, Route::Albums, Route::Artists] {
        assert_eq!(route.phase(), None, "{route:?}");
    }
}
