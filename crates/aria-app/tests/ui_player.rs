//! Headless tests of the player bar. The bar's own behavior (what it returns, what it
//! disables) is driven through a harness around `player_bar::show`; what the app does with
//! the answers (settings, titles from the library) through the whole app.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use aria_app::{
    app::{AppDeps, AriaApp},
    ui::{
        layout::{
            player_bar::{self, NowPlaying, PlayerAction, PlayerBar},
            right_panel::{self, QueueAction, QueuePanel},
        },
        theme::{Palette, fonts},
    },
};
use aria_core::{
    ClockReading, PlaybackClock,
    event::Event,
    i18n::{FluentArgs, Localizer},
    playback_service::PlayerView,
    player::queue::{RepeatMode, UpNext},
    settings::{Language, Settings},
};
use aria_library::{FolderId, LibrarySnapshot, Track, TrackId};
use eframe::egui::{self, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT as _, Queryable as _},
};

const SIZE: egui::Vec2 = egui::vec2(1200.0, 780.0);
const SEEK_KNOB: f32 = 5.0;

fn localizer() -> Localizer {
    Localizer::new(Language::En).unwrap_or_else(|err| panic!("English locale: {err}"))
}

fn en(key: &str) -> String {
    localizer().tr(key).into_owned()
}

fn english() -> Settings {
    Settings {
        language: Some(Language::En),
        ..Settings::default()
    }
}

fn track(id: i64, title: &str, artist: Option<&str>) -> Track {
    Track {
        id: TrackId(id),
        folder: FolderId(1),
        path: PathBuf::from("/secret/folder/file.flac"),
        title: title.to_owned(),
        artist: artist.map(Arc::from),
        album: None,
        album_artist: None,
        track_no: None,
        disc_no: None,
        year: None,
        genre: None,
        duration_ms: 200_000,
        sample_rate: 44_100,
        channels: 2,
        codec: Arc::from("flac"),
        artwork: None,
        added_at: 0,
    }
}

fn view(current: Option<i64>, playing: bool, duration_ms: u64) -> PlayerView {
    PlayerView {
        current: current.map(TrackId),
        token: current.map_or(0, |_| 7),
        playing,
        duration_ms,
        upcoming: Vec::new(),
    }
}

/// A bar on its own: what it is given, and what it returned over the frames so far.
struct Bar {
    tr: Localizer,
    view: PlayerView,
    now: Option<NowPlaying>,
    position_ms: u64,
    settings: Settings,
    actions: Vec<PlayerAction>,
}

fn bar_harness(view: PlayerView, position_ms: u64) -> Harness<'static, Bar> {
    let state = Bar {
        tr: localizer(),
        now: view.current.map(|_| NowPlaying {
            title: "Song".into(),
            artist: "Singer".into(),
            artwork: None,
        }),
        view,
        position_ms,
        settings: english(),
        actions: Vec::new(),
    };
    Harness::builder()
        .with_size(SIZE)
        .with_context_setup(fonts::install)
        .build_ui_state(
            |ui, bar: &mut Bar| {
                let palette = Palette::dark();
                let mut shown = PlayerBar {
                    tr: &bar.tr,
                    palette: &palette,
                    view: &bar.view,
                    now: bar.now.as_ref(),
                    position_ms: bar.position_ms,
                    settings: &bar.settings,
                    queue_open: false,
                    artwork: None,
                };
                let actions = player_bar::show(ui, &mut shown);
                bar.actions.extend(actions);
            },
            state,
        )
}

fn app_harness() -> Harness<'static, AriaApp> {
    let app = AriaApp::new(english(), localizer(), AppDeps::none(), vec![]);
    Harness::builder()
        .with_size(SIZE)
        .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app)
}

fn is_disabled<S>(harness: &Harness<'_, S>, role: Role, label: &str) -> bool {
    harness
        .get_by_role_and_label(role, label)
        .accesskit_node()
        .is_disabled()
}

/// Where along a bar `fraction` of its length lies.
fn point_on<S>(harness: &Harness<'_, S>, label: &str, fraction: f32) -> egui::Pos2 {
    let rect = harness.get_by_role_and_label(Role::Slider, label).rect();
    let width = rect.width() - 2.0 * SEEK_KNOB;
    egui::pos2(rect.left() + SEEK_KNOB + width * fraction, rect.center().y)
}

fn pointer<S>(harness: &mut Harness<'_, S>, events: Vec<egui::Event>) {
    harness.input_mut().events.extend(events);
    harness.step();
}

fn moved(pos: egui::Pos2) -> egui::Event {
    egui::Event::PointerMoved(pos)
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Presses and releases at `pos`, in one frame.
fn click_at<S>(harness: &mut Harness<'_, S>, pos: egui::Pos2) {
    let events = vec![moved(pos), button(pos, true), button(pos, false)];
    pointer(harness, events);
    harness.run();
}

#[test]
fn shows_title_and_artist() {
    let mut harness = app_harness();
    let ctx = harness.ctx.clone();
    let snapshot = LibrarySnapshot {
        tracks: vec![track(1, "So What", Some("Miles Davis"))],
        ..LibrarySnapshot::default()
    };
    harness
        .state_mut()
        .handle_event(&ctx, Event::LibraryUpdated(Arc::new(snapshot)));
    harness
        .state_mut()
        .handle_event(&ctx, Event::Player(view(Some(1), false, 200_000)));
    harness.run();
    assert!(harness.query_by_label("So What").is_some());
    assert!(harness.query_by_label("Miles Davis").is_some());
    assert!(
        harness
            .query_by_label(&en("empty-nothing-playing"))
            .is_none()
    );
}

#[test]
fn a_track_without_a_row_shows_unknown_never_a_path() {
    let mut harness = app_harness();
    let ctx = harness.ctx.clone();
    // No snapshot at all, and then one without the track.
    for snapshot in [None, Some(LibrarySnapshot::default())] {
        if let Some(snapshot) = snapshot {
            harness
                .state_mut()
                .handle_event(&ctx, Event::LibraryUpdated(Arc::new(snapshot)));
        }
        harness
            .state_mut()
            .handle_event(&ctx, Event::Player(view(Some(9), false, 0)));
        harness.run();
        assert!(
            harness
                .query_by_label(&en("player-unknown-title"))
                .is_some()
        );
        assert!(
            harness
                .query_by_label(&en("library-unknown-artist"))
                .is_some()
        );
    }
}

#[test]
fn a_known_track_with_a_row_but_no_artist_says_unknown_artist() {
    let snapshot = LibrarySnapshot {
        tracks: vec![track(1, "Untitled live", None)],
        ..LibrarySnapshot::default()
    };
    let now = NowPlaying::of(&view(Some(1), false, 0), &snapshot, &localizer());
    let Some(now) = now else {
        panic!("a current track gives a line");
    };
    assert_eq!(now.title, "Untitled live");
    assert_eq!(now.artist, en("library-unknown-artist"));
    assert!(NowPlaying::of(&view(None, false, 0), &snapshot, &localizer()).is_none());
}

#[test]
fn position_belongs_to_the_clock_only_for_the_current_track() {
    let clock = Arc::new(PlaybackClock::default());
    clock.publish(ClockReading {
        token: 7,
        position_ms: 42_000,
        playing: true,
    });
    let shown = view(Some(1), true, 200_000);
    assert_eq!(player_bar::position_ms(clock.read(), &shown), 42_000);

    // The clock still holds the last track while the new one loads.
    let next = PlayerView {
        token: 8,
        ..shown.clone()
    };
    assert_eq!(player_bar::position_ms(clock.read(), &next), 0);
    // No track, no position; and token 0 never matches an idle clock.
    let none = view(None, false, 0);
    assert_eq!(player_bar::position_ms(ClockReading::default(), &none), 0);
}

#[test]
fn play_button_sends_toggle() {
    let mut harness = bar_harness(view(Some(1), false, 200_000), 0);
    harness
        .get_by_role_and_label(Role::Button, &en("player-play"))
        .click();
    harness.run();
    assert_eq!(harness.state().actions, [PlayerAction::TogglePlay]);

    // While playing the same button pauses, and next and previous send theirs.
    let mut harness = bar_harness(view(Some(1), true, 200_000), 0);
    for (key, action) in [
        ("player-pause", PlayerAction::TogglePlay),
        ("player-next", PlayerAction::Next),
        ("player-previous", PlayerAction::Previous),
    ] {
        harness.state_mut().actions.clear();
        harness
            .get_by_role_and_label(Role::Button, &en(key))
            .click();
        harness.run();
        assert_eq!(harness.state().actions, [action], "{key}");
    }
}

#[test]
fn transport_is_disabled_with_nothing_loaded() {
    let mut harness = bar_harness(view(None, false, 0), 0);
    for key in ["player-play", "player-previous", "player-next"] {
        let label = en(key);
        assert!(is_disabled(&harness, Role::Button, &label), "{key}");
        // Nothing says a phase brings it: playback is here.
        let node = harness.get_by_role_and_label(Role::Button, &label);
        assert_eq!(node.accesskit_node().description(), None, "{key}");
        node.click();
    }
    assert!(is_disabled(&harness, Role::Slider, &en("player-seek")));
    harness.run();
    assert!(harness.state().actions.is_empty());
}

#[test]
fn seek_sends_once_on_release() {
    let mut harness = bar_harness(view(Some(1), true, 100_000), 10_000);
    let label = en("player-seek");
    let (from, to) = (
        point_on(&harness, &label, 0.1),
        point_on(&harness, &label, 0.75),
    );

    pointer(&mut harness, vec![moved(from)]);
    pointer(&mut harness, vec![button(from, true)]);
    let mid = point_on(&harness, &label, 0.4);
    pointer(&mut harness, vec![moved(mid)]);
    pointer(&mut harness, vec![moved(to)]);
    harness.run();
    assert!(
        harness.state().actions.is_empty(),
        "dragging sought: {:?}",
        harness.state().actions
    );
    // The time shows where the drag is, not where the track is.
    assert!(harness.query_by_label("1:15").is_some());

    pointer(&mut harness, vec![button(to, false)]);
    harness.run();
    let [PlayerAction::Seek(ms)] = harness.state().actions[..] else {
        panic!("expected one seek, got {:?}", harness.state().actions);
    };
    assert!((74_000..=76_000).contains(&ms), "sought to {ms}");
}

#[test]
fn unknown_duration_disables_seek() {
    let mut harness = bar_harness(view(Some(1), true, 0), 5_000);
    let label = en("player-seek");
    assert!(is_disabled(&harness, Role::Slider, &label));
    assert!(harness.query_by_label("--:--").is_some());
    let pos = point_on(&harness, &label, 0.5);
    click_at(&mut harness, pos);
    assert!(harness.state().actions.is_empty());
}

#[test]
fn volume_change_is_saved_in_settings() {
    let mut harness = app_harness();
    assert_eq!(harness.state().settings().volume, 1.0);
    let label = en("player-volume");
    let pos = point_on(&harness, &label, 0.5);
    click_at(&mut harness, pos);
    let volume = harness.state().settings().volume;
    assert!((0.45..=0.55).contains(&volume), "volume is {volume}");

    // Muting keeps the volume; raising it while muted unmutes.
    harness
        .get_by_role_and_label(Role::Button, &en("player-mute"))
        .click();
    harness.run();
    assert!(harness.state().settings().muted);
    assert_eq!(harness.state().settings().volume, volume);
    let pos = point_on(&harness, &label, 0.9);
    click_at(&mut harness, pos);
    assert!(!harness.state().settings().muted);
    assert!(harness.state().settings().volume > volume);
}

#[test]
fn repeat_cycles_through_three_modes() {
    let mut harness = app_harness();
    for (shown, then) in [
        ("player-repeat-off", RepeatMode::All),
        ("player-repeat-all", RepeatMode::One),
        ("player-repeat-one", RepeatMode::Off),
    ] {
        harness
            .get_by_role_and_label(Role::Button, &en(shown))
            .click();
        harness.run();
        assert_eq!(harness.state().settings().repeat, then, "after {shown}");
    }
    harness
        .get_by_role_and_label(Role::Button, &en("player-shuffle"))
        .click();
    harness.run();
    assert!(harness.state().settings().shuffle);
}

#[test]
fn repaints_only_while_playing() {
    let delays = |playing: bool| {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut harness = bar_harness(view(Some(1), playing, 100_000), 0);
        let sink = Arc::clone(&seen);
        harness.ctx.set_request_repaint_callback(move |info| {
            if let Ok(mut seen) = sink.lock() {
                seen.push(info.delay);
            }
        });
        harness.step();
        seen.lock().map(|seen| seen.clone()).unwrap_or_default()
    };
    assert!(delays(true).contains(&Duration::from_millis(250)));
    assert!(!delays(false).contains(&Duration::from_millis(250)));
}

/// Collects the delays of every repaint request made while `step` runs.
fn repaint_delays<S>(
    harness: &mut Harness<'_, S>,
    step: impl FnOnce(&mut Harness<'_, S>),
) -> Vec<Duration> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    harness.ctx.set_request_repaint_callback(move |info| {
        if let Ok(mut seen) = sink.lock() {
            seen.push(info.delay);
        }
    });
    step(harness);
    seen.lock().map(|seen| seen.clone()).unwrap_or_default()
}

const SOON: Duration = Duration::from_millis(50);

#[test]
fn a_seek_is_shown_until_the_clock_catches_up() {
    let mut harness = bar_harness(view(Some(1), true, 100_000), 10_000);
    let label = en("player-seek");
    let to = point_on(&harness, &label, 0.75);
    click_at(&mut harness, to);
    let [PlayerAction::Seek(target)] = harness.state().actions[..] else {
        panic!("expected one seek, got {:?}", harness.state().actions);
    };
    assert!((74_000..=76_000).contains(&target));

    // The clock still says 0:10, which is where the track was. The bar keeps the target.
    harness.run();
    assert!(
        harness.query_by_label("0:10").is_none(),
        "the bar jumped back"
    );
    assert!(harness.query_by_label("1:15").is_some());

    // Once the clock answers, it is the clock's position that shows.
    harness.state_mut().position_ms = 80_000;
    harness.run();
    assert!(harness.query_by_label("1:20").is_some());
}

#[test]
fn a_seek_while_paused_repaints_until_the_clock_answers() {
    let mut harness = bar_harness(view(Some(1), false, 100_000), 10_000);
    let to = point_on(&harness, &en("player-seek"), 0.5);
    let delays = repaint_delays(&mut harness, |harness| click_at(harness, to));
    assert!(
        delays.contains(&SOON),
        "no quick repaint after the seek: {delays:?}"
    );

    harness.state_mut().position_ms = 50_000;
    harness.step();
    let delays = repaint_delays(&mut harness, |harness| harness.step());
    assert!(
        !delays.contains(&SOON),
        "still repainting after the answer: {delays:?}"
    );
}

#[test]
fn previous_while_paused_repaints_for_a_moment() {
    let mut harness = app_harness();
    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .handle_event(&ctx, Event::Player(view(Some(1), false, 100_000)));
    harness.run();
    let idle = repaint_delays(&mut harness, |harness| harness.step());
    assert!(!idle.contains(&SOON), "an idle bar repaints: {idle:?}");

    let delays = repaint_delays(&mut harness, |harness| {
        harness
            .get_by_role_and_label(Role::Button, &en("player-previous"))
            .click();
        harness.run();
    });
    assert!(
        delays.contains(&SOON),
        "no quick repaint after Previous: {delays:?}"
    );
}

#[test]
fn held_arrow_keys_accumulate() {
    let mut harness = bar_harness(view(Some(1), true, 100_000), 10_000);
    harness
        .get_by_role_and_label(Role::Slider, &en("player-seek"))
        .focus();
    // A few frames with the focus, as there are in real use, before the keys come.
    harness.run();
    harness.step();
    // The clock does not move between the presses, as when keys repeat faster than the engine
    // answers.
    for _ in 0..3 {
        harness.key_press(egui::Key::ArrowRight);
        harness.step();
    }
    assert_eq!(
        harness.state().actions,
        [
            PlayerAction::Seek(15_000),
            PlayerAction::Seek(20_000),
            PlayerAction::Seek(25_000),
        ]
    );
}

/// The right panel on its own: what it is given, and what it returned over the frames so far.
struct Panel {
    tr: Localizer,
    view: PlayerView,
    snapshot: LibrarySnapshot,
    actions: Vec<QueueAction>,
}

fn up(id: i64, from_user: bool) -> UpNext {
    UpNext {
        track: TrackId(id),
        from_user,
    }
}

/// Tracks 1 to 5 are known by the titles "Song 1" to "Song 5".
fn panel_harness(current: Option<i64>, upcoming: Vec<UpNext>) -> Harness<'static, Panel> {
    let state = Panel {
        tr: localizer(),
        view: PlayerView {
            upcoming,
            ..view(current, true, 200_000)
        },
        snapshot: LibrarySnapshot {
            tracks: (1..=5)
                .map(|id| track(id, &format!("Song {id}"), Some("Singer")))
                .collect(),
            ..LibrarySnapshot::default()
        },
        actions: Vec::new(),
    };
    Harness::builder()
        .with_size(SIZE)
        .with_context_setup(fonts::install)
        .build_ui_state(
            |ui, panel: &mut Panel| {
                let palette = Palette::dark();
                let mut queue = QueuePanel {
                    view: &panel.view,
                    snapshot: &panel.snapshot,
                    artwork: None,
                };
                let actions = right_panel::show(ui, &panel.tr, &palette, true, &mut queue);
                panel.actions.extend(actions);
            },
            state,
        )
}

/// Track 1 is current, 2 and 3 are the user's, 4 and 5 come from the list.
fn mixed() -> Harness<'static, Panel> {
    let mut harness = panel_harness(
        Some(1),
        vec![up(2, true), up(3, true), up(4, false), up(5, false)],
    );
    harness.run();
    harness
}

fn remove_label(title: &str) -> String {
    let mut args = FluentArgs::new();
    args.set("title", title);
    localizer().tr_args("queue-remove", &args).into_owned()
}

#[test]
fn queue_lists_user_then_context() {
    let harness = mixed();
    let y = |label: &str| harness.get_by_label(label).rect().top();
    let order = [
        y(&en("queue-now-playing")),
        y("Song 1"),
        y(&en("queue-next-user")),
        y("Song 2"),
        y("Song 3"),
        y(&en("queue-next-list")),
        y("Song 4"),
        y("Song 5"),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
}

#[test]
fn remove_sends_the_index() {
    let mut harness = mixed();
    harness.get_by_label(&remove_label("Song 3")).click();
    harness.run();
    assert_eq!(harness.state().actions, [QueueAction::Remove(1)]);
}

#[test]
fn context_rows_have_no_remove_button() {
    let harness = mixed();
    assert!(harness.query_by_label(&remove_label("Song 2")).is_some());
    assert!(harness.query_by_label(&remove_label("Song 4")).is_none());
    assert!(harness.query_by_label(&remove_label("Song 5")).is_none());
    assert!(harness.query_by_label(&remove_label("Song 1")).is_none());
}

#[test]
fn clear_is_offered_only_with_user_items() {
    let mut harness = mixed();
    harness.get_by_label(&en("queue-clear")).click();
    harness.run();
    assert_eq!(harness.state().actions, [QueueAction::Clear]);

    let mut harness = panel_harness(Some(1), vec![up(4, false)]);
    harness.run();
    assert!(harness.query_by_label(&en("queue-clear")).is_none());
}

#[test]
fn empty_queue_shows_empty_state() {
    let mut harness = panel_harness(None, Vec::new());
    harness.run();
    assert!(harness.query_by_label(&en("empty-right-panel")).is_some());
    assert!(harness.query_by_label(&en("queue-clear")).is_none());
}

#[test]
fn a_queued_track_without_a_row_shows_unknown_never_a_path() {
    let mut harness = panel_harness(Some(1), vec![up(99, true)]);
    harness.run();
    assert!(
        harness
            .query_by_label(&en("player-unknown-title"))
            .is_some()
    );
    assert!(
        harness
            .query_by_label(&en("library-unknown-artist"))
            .is_some()
    );
    assert!(harness.query_by_label_contains("secret").is_none());
}

#[test]
fn dragging_a_row_onto_another_moves_it() {
    let mut harness = mixed();
    let from = harness.get_by_label("Song 2").rect().center();
    let to = harness.get_by_label("Song 3").rect().center();
    pointer(&mut harness, vec![moved(from), button(from, true)]);
    pointer(&mut harness, vec![moved(from + egui::vec2(0.0, 12.0))]);
    pointer(&mut harness, vec![moved(to)]);
    pointer(&mut harness, vec![button(to, false)]);
    harness.run();
    assert_eq!(
        harness.state().actions,
        [QueueAction::Move { from: 0, to: 1 }]
    );
}

#[test]
fn dropping_a_row_where_it_is_or_on_the_list_sends_nothing() {
    let mut harness = mixed();
    let from = harness.get_by_label("Song 2").rect().center();
    let nudged = from + egui::vec2(0.0, 6.0);
    pointer(&mut harness, vec![moved(from), button(from, true)]);
    pointer(&mut harness, vec![moved(nudged)]);
    pointer(&mut harness, vec![button(nudged, false)]);
    harness.run();
    assert!(harness.state().actions.is_empty());

    let onto_list = harness.get_by_label("Song 4").rect().center();
    let from = harness.get_by_label("Song 3").rect().center();
    pointer(&mut harness, vec![moved(from), button(from, true)]);
    pointer(&mut harness, vec![moved(from + egui::vec2(0.0, 12.0))]);
    pointer(&mut harness, vec![moved(onto_list)]);
    pointer(&mut harness, vec![button(onto_list, false)]);
    harness.run();
    assert!(harness.state().actions.is_empty());
}

/// Drags the row titled `from` onto the one titled `to` and returns what the panel asked for.
fn drag(harness: &mut Harness<'_, Panel>, from: &str, to: &str) {
    let start = harness.get_by_label(from).rect().center();
    let end = harness.get_by_label(to).rect().center();
    pointer(harness, vec![moved(start), button(start, true)]);
    pointer(harness, vec![moved(start + egui::vec2(0.0, -12.0))]);
    pointer(harness, vec![moved(end)]);
    pointer(harness, vec![button(end, false)]);
    harness.run();
}

fn three_user_rows() -> Harness<'static, Panel> {
    let mut harness = panel_harness(
        Some(1),
        vec![up(2, true), up(3, true), up(4, true), up(5, false)],
    );
    harness.run();
    harness
}

#[test]
fn dragging_a_row_up_moves_it_to_the_target_index() {
    let mut harness = three_user_rows();
    drag(&mut harness, "Song 3", "Song 2");
    assert_eq!(
        harness.state().actions,
        [QueueAction::Move { from: 1, to: 0 }]
    );
    harness.state_mut().actions.clear();
    drag(&mut harness, "Song 4", "Song 2");
    assert_eq!(
        harness.state().actions,
        [QueueAction::Move { from: 2, to: 0 }]
    );
}

#[test]
fn a_capped_queue_builds_only_the_rows_in_view() {
    // The backend caps `upcoming` at 200; all of them are the user's here.
    let upcoming = (1..=200).map(|id| up(id + 1000, true)).collect();
    let mut harness = panel_harness(Some(1), upcoming);
    harness.run();
    let unknown = en("player-unknown-title");
    let shown = harness.query_all_by_label(&unknown).count();
    assert!(shown > 0 && shown < 40, "{shown} rows were built");
    let first = remove_label(&unknown);
    if let Some(button) = harness.get_all_by_label(&first).next() {
        button.click();
    }
    harness.run();
    assert_eq!(harness.state().actions, [QueueAction::Remove(0)]);
}

#[test]
fn every_row_and_heading_is_the_same_height() {
    let harness = mixed();
    let top = |label: &str| harness.get_by_label(label).rect().top();
    // User row to user row: one row. Current row to the first user row: the current row
    // and the "Next in queue" heading.
    assert_eq!(top("Song 3") - top("Song 2"), 48.0);
    assert_eq!(top("Song 2") - top("Song 1"), 96.0);
    assert_eq!(top("Song 4") - top("Song 3"), 96.0);
}
