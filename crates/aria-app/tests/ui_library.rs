//! Headless tests of the Local Music page, driven through AccessKit queries.
//!
//! The page is drawn on its own, with a library handed to it, so no backend or folder dialog
//! is involved: what the page asks of the app is read from the actions it leaves behind.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

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
use aria_library::{Album, Folder, FolderId, LibrarySnapshot, ScanProgress, Track, TrackId};
use eframe::egui::{self, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable as _};

/// A window the table fills with a few dozen rows.
const SIZE: egui::Vec2 = egui::vec2(1000.0, 700.0);

/// What the page is given, and what it left behind.
struct Page {
    tr: Localizer,
    settings: Settings,
    modal_open: bool,
    library: LibraryView,
    actions: Vec<PageAction>,
}

fn english() -> Localizer {
    Localizer::new(Language::En).unwrap_or_else(|err| panic!("English locale: {err}"))
}

/// The English text of `key`.
fn en(key: &str) -> String {
    english().tr(key).into_owned()
}

fn track(id: i64, title: &str, artist: Option<&str>, album: Option<&str>, ms: u64) -> Track {
    Track {
        id: TrackId(id),
        folder: FolderId(1),
        path: format!("music/{id}.flac").into(),
        title: title.to_owned(),
        artist: artist.map(Arc::from),
        album: album.map(Arc::from),
        album_artist: None,
        track_no: None,
        disc_no: None,
        year: None,
        genre: None,
        duration_ms: ms,
        sample_rate: 44_100,
        channels: 2,
        codec: Arc::from("flac"),
        artwork: None,
        added_at: id,
    }
}

fn folder() -> Folder {
    Folder {
        id: FolderId(1),
        path: "music".into(),
        available: true,
    }
}

fn snapshot(tracks: Vec<Track>) -> LibrarySnapshot {
    LibrarySnapshot {
        tracks,
        folders: vec![folder()],
        ..LibrarySnapshot::default()
    }
}

fn harness(snapshot: LibrarySnapshot) -> Harness<'static, Page> {
    let page = Page {
        tr: english(),
        settings: Settings::default(),
        modal_open: false,
        library: LibraryView {
            snapshot: Arc::new(snapshot),
            ..LibraryView::default()
        },
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
                };
                pages::show(Route::LocalMusic, ui, &mut ctx);
            },
            page,
        )
}

/// Types `text` into the search field.
fn search(harness: &mut Harness<'_, Page>, text: &str) {
    let label = en("local-search");
    let field = harness.get_by_role_and_label(Role::TextInput, &label);
    field.focus();
    field.type_text(text);
    harness.run();
}

/// Double-clicks the node labelled `label`. Both clicks go into one frame: the harness runs a
/// frame per event, which spreads them over more time than a double click may take.
fn double_click(harness: &mut Harness<'_, Page>, label: &str) {
    let pos = harness.get_by_label(label).rect().center();
    let events = &mut harness.input_mut().events;
    events.push(egui::Event::PointerMoved(pos));
    for pressed in [true, false, true, false] {
        events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.step();
    harness.run();
}

#[test]
fn empty_library_offers_add_folder() {
    let mut harness = harness(LibrarySnapshot::default());
    assert!(
        harness.query_by_label(&en("local-empty-title")).is_some(),
        "no empty state"
    );
    harness
        .get_by_role_and_label(Role::Button, &en("local-add-folder"))
        .click();
    harness.run();
    assert_eq!(harness.state().actions, [PageAction::PickFolder]);
}

#[test]
fn tracks_appear_from_a_snapshot() {
    let harness = harness(snapshot(vec![
        track(
            1,
            "Blue in Green",
            Some("Miles Davis"),
            Some("Kind of Blue"),
            337_000,
        ),
        track(2, "Untagged", None, None, 0),
    ]));
    for text in [
        "Blue in Green",
        "Miles Davis",
        "Kind of Blue",
        "5:37",
        "Untagged",
    ] {
        assert!(
            harness.query_by_label(text).is_some(),
            "no {text:?} on the page"
        );
    }
    assert!(
        harness
            .query_by_label(&en("library-unknown-artist"))
            .is_some()
    );
    assert!(
        harness
            .query_by_label(&en("library-unknown-album"))
            .is_some()
    );
    assert!(harness.query_by_label("--:--").is_some());
}

#[test]
fn double_click_plays_the_visible_list() {
    let mut harness = harness(snapshot(vec![
        track(1, "Alpha Two", Some("A"), None, 1000),
        track(2, "Beta", Some("B"), None, 1000),
        track(3, "Alpha One", Some("C"), None, 1000),
        track(4, "Gamma", Some("D"), None, 1000),
    ]));
    search(&mut harness, "ALPHA");
    assert!(
        harness.query_by_label("Beta").is_none(),
        "the filter did not apply"
    );

    // The visible list is sorted by title: Alpha One (3), Alpha Two (1).
    double_click(&mut harness, "Alpha Two");
    assert_eq!(
        harness.state().actions,
        [PageAction::PlayContext {
            ids: vec![TrackId(3), TrackId(1)],
            start: 1,
        }]
    );
}

#[test]
fn context_menu_queues_a_track() {
    let mut harness = harness(snapshot(vec![
        track(7, "First", Some("A"), None, 1000),
        track(9, "Second", Some("B"), None, 1000),
    ]));
    harness.get_by_label("Second").click_secondary();
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, &en("local-add-to-queue"))
        .click();
    harness.run();
    assert_eq!(
        harness.state().actions,
        [PageAction::AddToQueue(TrackId(9))]
    );

    harness.state_mut().actions.clear();
    harness.get_by_label("First").click_secondary();
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, &en("local-play-next"))
        .click();
    harness.run();
    assert_eq!(harness.state().actions, [PageAction::PlayNext(TrackId(7))]);
}

#[test]
fn a_header_click_sorts_and_a_second_click_reverses() {
    let mut harness = harness(snapshot(vec![
        track(1, "First", Some("Zed"), None, 1000),
        track(2, "Second", Some("Abe"), None, 1000),
        track(3, "Third", None, None, 1000),
    ]));
    // The order shows in what a double click sends: the visible list.
    let order = |harness: &mut Harness<'_, Page>| {
        harness.state_mut().actions.clear();
        double_click(harness, "First");
        match harness.state().actions.as_slice() {
            [PageAction::PlayContext { ids, .. }] => ids.iter().map(|id| id.0).collect::<Vec<_>>(),
            other => panic!("expected one PlayContext, got {other:?}"),
        }
    };
    assert_eq!(order(&mut harness), [1, 2, 3], "by title to begin with");

    let artist = en("local-col-artist");
    harness.get_by_role_and_label(Role::Button, &artist).click();
    harness.run();
    assert_eq!(
        order(&mut harness),
        [2, 1, 3],
        "ascending, the untagged last"
    );

    harness.get_by_role_and_label(Role::Button, &artist).click();
    harness.run();
    assert_eq!(
        order(&mut harness),
        [1, 2, 3],
        "descending, the untagged still last"
    );
}

#[test]
fn no_results_message() {
    let mut harness = harness(snapshot(vec![track(1, "Blue", Some("A"), None, 1000)]));
    assert!(harness.query_by_label(&en("local-no-results")).is_none());
    search(&mut harness, "zzz");
    assert!(harness.query_by_label(&en("local-no-results")).is_some());
    assert!(harness.query_by_label("Blue").is_none());
}

#[test]
fn a_scan_shows_its_count() {
    let mut harness = harness(snapshot(vec![track(1, "Blue", Some("A"), None, 1000)]));
    harness.state_mut().library.scan = Some(ScanProgress {
        seen: 1234,
        changed: 3,
    });
    harness.run();
    let tr = english();
    let mut args = aria_core::i18n::FluentArgs::new();
    args.set("count", 1234_u64);
    let text = tr.tr_args("library-scanning", &args);
    assert!(harness.query_by_label(&text).is_some(), "no {text:?}");
}

#[test]
fn an_album_opens_its_tracks_and_back_returns() {
    let mut snap = snapshot(vec![
        track(1, "On Album", Some("A"), Some("Record"), 1000),
        track(2, "Elsewhere", Some("B"), None, 1000),
    ]);
    snap.albums = vec![Album {
        title: Some(Arc::from("Record")),
        artist: Some(Arc::from("A")),
        tracks: vec![TrackId(1)],
        artwork: None,
        year: None,
    }];
    let mut harness = harness(snap);

    harness
        .get_by_role_and_label(Role::Tab, &en("local-tab-albums"))
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, "Record")
        .click();
    harness.run();
    assert!(harness.query_by_label("On Album").is_some());
    assert!(
        harness.query_by_label("Elsewhere").is_none(),
        "the scope did not apply"
    );

    harness
        .get_by_role_and_label(Role::Button, &en("local-back"))
        .click();
    harness.run();
    assert!(harness.query_by_label("Elsewhere").is_some());
}

#[test]
fn large_snapshot_draws_only_visible_rows() {
    let tracks: Vec<Track> = (1..=50_000)
        .map(|i| {
            track(
                i,
                &format!("Song {i:05}"),
                Some("Artist"),
                Some("Album"),
                200_000,
            )
        })
        .collect();
    let mut harness = harness(LibrarySnapshot::default());
    harness.state_mut().library.snapshot = Arc::new(snapshot(tracks));

    // The first frame with the snapshot caches, filters and sorts all of it.
    let started = Instant::now();
    harness.step();
    let first = started.elapsed();
    assert!(
        first < Duration::from_millis(500),
        "the frame took {first:?}"
    );

    // Later frames reuse that work.
    let started = Instant::now();
    harness.step();
    let again = started.elapsed();
    assert!(
        again < Duration::from_millis(500),
        "the next frame took {again:?}"
    );

    let drawn = harness.query_all_by_label_contains("Song ").count();
    assert!(
        (1..200).contains(&drawn),
        "{drawn} of 50000 rows were drawn"
    );
}
