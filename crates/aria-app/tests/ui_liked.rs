//! Headless tests of the Liked Songs page, driven through AccessKit queries.

use std::sync::Arc;

use aria_app::{
    nav::Route,
    ui::{
        pages::{self, LibraryView, PageAction, PageCtx},
        theme::{Palette, fonts},
    },
};
use aria_core::{
    i18n::Localizer,
    settings::{Language, Settings},
};
use aria_library::{Collections, Folder, FolderId, LibrarySnapshot, Track, TrackId};
use eframe::egui;
use egui_kittest::{Harness, kittest::Queryable as _};

const SIZE: egui::Vec2 = egui::vec2(1000.0, 700.0);

struct Page {
    tr: Localizer,
    settings: Settings,
    modal_open: bool,
    library: LibraryView,
    collections: Collections,
    actions: Vec<PageAction>,
}

fn english() -> Localizer {
    Localizer::new(Language::En).unwrap_or_else(|err| panic!("English locale: {err}"))
}

fn en(key: &str) -> String {
    english().tr(key).into_owned()
}

fn track(id: i64, title: &str) -> Track {
    Track {
        id: TrackId(id),
        folder: FolderId(1),
        path: format!("music/{id}.flac").into(),
        title: title.to_owned(),
        artist: None,
        album: None,
        album_artist: None,
        track_no: None,
        disc_no: None,
        year: None,
        genre: None,
        duration_ms: 1000,
        sample_rate: 44_100,
        channels: 2,
        codec: Arc::from("flac"),
        artwork: None,
        added_at: id,
    }
}

/// Tracks 1 "Alpha", 2 "Zulu", 3 "Mike", of which `favorites` (newest first) are liked.
fn harness(favorites: &[i64]) -> Harness<'static, Page> {
    let snapshot = LibrarySnapshot {
        tracks: vec![track(1, "Alpha"), track(2, "Zulu"), track(3, "Mike")],
        folders: vec![Folder {
            id: FolderId(1),
            path: "music".into(),
            available: true,
        }],
        ..LibrarySnapshot::default()
    };
    let page = Page {
        tr: english(),
        settings: Settings::default(),
        modal_open: false,
        library: LibraryView {
            snapshot: Arc::new(snapshot),
            ..LibraryView::default()
        },
        collections: collections(favorites),
        actions: Vec::new(),
    };
    Harness::builder()
        .with_size(SIZE)
        .with_context_setup(fonts::install)
        .build_ui_state(
            |ui, page: &mut Page| {
                let mut ctx = PageCtx {
                    tr: &page.tr,
                    palette: Palette::dark(),
                    settings: &page.settings,
                    paths_available: true,
                    modal_open: &mut page.modal_open,
                    out: &mut page.actions,
                    library: &page.library,
                    collections: &page.collections,
                    devices: &[],
                    artwork: None,
                };
                pages::show(Route::Liked, ui, &mut ctx);
            },
            page,
        )
}

fn collections(favorites: &[i64]) -> Collections {
    Collections::new(
        favorites.iter().map(|&id| TrackId(id)).collect(),
        vec![],
        vec![],
    )
}

#[test]
fn no_favorites_shows_the_empty_state() {
    let mut harness = harness(&[]);
    harness.run();
    assert!(harness.query_by_label(&en("liked-empty")).is_some());
    assert!(harness.query_by_label(&en("liked-empty-hint")).is_some());
}

#[test]
fn favorites_are_listed_newest_first() {
    // Zulu was liked after Alpha; by title Alpha would come first.
    let mut harness = harness(&[2, 1]);
    harness.run();
    let zulu = harness.get_by_label("Zulu").rect().top();
    let alpha = harness.get_by_label("Alpha").rect().top();
    assert!(zulu < alpha, "the newer favorite is not above");
    assert!(harness.query_by_label("Mike").is_none(), "not a favorite");
    assert!(harness.query_by_label(&en("liked-empty")).is_none());
}

#[test]
fn a_new_collections_value_replaces_the_rows() {
    let mut harness = harness(&[2, 1]);
    harness.run();
    harness.state_mut().collections = collections(&[1]);
    harness.run();
    assert!(harness.query_by_label("Zulu").is_none(), "still listed");
    assert!(harness.query_by_label("Alpha").is_some());
}

#[test]
fn the_heart_unfavorites_a_row() {
    let mut harness = harness(&[1]);
    harness.run();
    harness.get_by_label(&en("track-unfavorite")).click();
    harness.run();
    assert_eq!(
        harness.state().actions,
        [PageAction::SetFavorite {
            track: TrackId(1),
            favorite: false
        }]
    );
}
