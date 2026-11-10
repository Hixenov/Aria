//! Tests for favorites, plays and the `Collections` view.

use std::path::{Path, PathBuf};

use aria_library::{
    Collections, FolderId, Library, LibraryError, PLAYLIST_NAME_MAX, PLAYS_KEPT, PlaylistId,
    QueueRecord, RECENT_LIMIT, SCHEMA_VERSION, TrackId, playlist_name,
};
use rusqlite::Connection;

/// Unwraps in helpers, which clippy's allow-expect-in-tests does not cover.
fn ok<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("test setup failed: {e:?}"),
    }
}

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: ok(tempfile::tempdir()),
        }
    }
    fn db(&self) -> PathBuf {
        self.dir.path().join("library.db")
    }
    fn art(&self) -> PathBuf {
        self.dir.path().join("art")
    }
    fn open(&self) -> Library {
        ok(Library::open(&self.db(), &self.art())).library
    }
    fn folder(&self, name: &str) -> PathBuf {
        self.dir.path().join("music").join(name)
    }
}

fn insert_track(db: &Path, folder: FolderId, name: &str) -> TrackId {
    let c = ok(Connection::open(db));
    ok(c.execute(
        "INSERT INTO tracks (folder_id, path, size, mtime, title, added_at)
         VALUES (?1, ?2, 1, 1, ?3, 1)",
        (folder.0, name.as_bytes(), name),
    ));
    TrackId(c.last_insert_rowid())
}

fn count(db: &Path, sql: &str) -> i64 {
    let c = ok(Connection::open(db));
    ok(c.query_row(sql, [], |r| r.get(0)))
}

/// A library with one folder and tracks named `names`.
fn with_tracks(env: &Env, names: &[&str]) -> (Library, FolderId, Vec<TrackId>) {
    let mut lib = env.open();
    let folder = ok(lib.add_folder(&env.folder("a")));
    let ids = names
        .iter()
        .map(|n| insert_track(&env.db(), folder, n))
        .collect();
    (lib, folder, ids)
}

#[test]
fn a_0_2_0_database_upgrades_and_keeps_its_data() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a", "b"]);
    let record = QueueRecord {
        context: ids.clone(),
        original: ids.clone(),
        user: vec![],
        current: Some(ids[1]),
        position_ms: 1234,
        shuffle_seed: 7,
        context_index: Some(1),
    };
    ok(lib.save_queue(&record));
    let tracks_before = ok(lib.snapshot()).tracks;
    drop(lib);
    {
        // What a 0.2.0 file looks like: no collection tables, version 1.
        let c = ok(Connection::open(env.db()));
        ok(c.execute_batch(
            "DROP TABLE favorites; DROP TABLE playlist_items; DROP TABLE playlists;
             DROP TABLE plays; PRAGMA user_version = 1;",
        ));
    }

    let lib = env.open();
    assert_eq!(SCHEMA_VERSION, 2);
    assert_eq!(count(&env.db(), "PRAGMA user_version"), 2);
    assert_eq!(ok(lib.snapshot()).tracks, tracks_before);
    assert_eq!(ok(lib.load_queue()), Some(record));
    assert_eq!(ok(lib.collections()), Collections::default());
}

#[test]
fn favorites_are_listed_newest_first_and_marking_twice_is_once() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    assert!(ok(lib.set_favorite(a, true, 10)));
    assert!(ok(lib.set_favorite(b, true, 20)));
    assert!(!ok(lib.set_favorite(a, true, 30)));
    let c = ok(lib.collections());
    assert_eq!(c.favorites, vec![b, a]);
    assert!(c.is_favorite(a));
    assert!(c.is_favorite(b));
    assert!(ok(lib.set_favorite(b, false, 40)));
    assert!(!ok(lib.set_favorite(b, false, 41)));
    let c = ok(lib.collections());
    assert_eq!(c.favorites, vec![a]);
    assert!(!c.is_favorite(b));
}

#[test]
fn an_unknown_track_changes_nothing() {
    let env = Env::new();
    let (mut lib, _, _) = with_tracks(&env, &["a"]);
    assert!(!ok(lib.set_favorite(TrackId(999), true, 1)));
    assert!(!ok(lib.record_play(TrackId(999), 1)));
    assert_eq!(count(&env.db(), "SELECT COUNT(*) FROM favorites"), 0);
    assert_eq!(count(&env.db(), "SELECT COUNT(*) FROM plays"), 0);
}

#[test]
fn favorites_survive_a_folder_removed_and_added_again() {
    let env = Env::new();
    let (mut lib, folder, ids) = with_tracks(&env, &["a", "b"]);
    ok(lib.set_favorite(ids[0], true, 10));
    ok(lib.set_favorite(ids[1], true, 20));
    ok(lib.record_play(ids[0], 5));
    ok(lib.remove_folder(folder));
    let c = ok(lib.collections());
    assert_eq!(c.favorites, vec![]);
    assert_eq!(c.recent, vec![]);

    let folder = ok(lib.add_folder(&env.folder("a")));
    let a2 = insert_track(&env.db(), folder, "a");
    let b2 = insert_track(&env.db(), folder, "b");
    let c = ok(lib.collections());
    assert_eq!(c.favorites, vec![b2, a2]);
    assert_eq!(c.recent, vec![a2]);
}

#[test]
fn recent_lists_each_track_once_newest_first() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    assert!(ok(lib.record_play(a, 1)));
    assert!(ok(lib.record_play(b, 2)));
    assert!(ok(lib.record_play(a, 3)));
    assert_eq!(ok(lib.collections()).recent, vec![a, b]);
}

#[test]
fn recent_holds_at_most_twenty() {
    let env = Env::new();
    let names: Vec<String> = (0..25).map(|i| format!("t{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (mut lib, _, ids) = with_tracks(&env, &refs);
    for (i, id) in ids.iter().enumerate() {
        ok(lib.record_play(*id, i as i64 + 1));
    }
    let recent = ok(lib.collections()).recent;
    assert_eq!(recent.len(), RECENT_LIMIT);
    let expected: Vec<TrackId> = ids.iter().rev().take(RECENT_LIMIT).copied().collect();
    assert_eq!(recent, expected);
}

#[test]
fn plays_are_trimmed_to_the_newest_ten_thousand() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a"]);
    {
        let mut c = ok(Connection::open(env.db()));
        let tx = ok(c.transaction());
        for i in 1..PLAYS_KEPT + 5 {
            ok(tx.execute(
                "INSERT INTO plays (path, played_at) VALUES (?1, ?2)",
                (b"a".as_slice(), i as i64),
            ));
        }
        ok(tx.commit());
    }
    assert_eq!(count(&env.db(), "SELECT COUNT(*) FROM plays"), 10_004);
    assert!(ok(lib.record_play(ids[0], 99_999)));
    assert_eq!(count(&env.db(), "SELECT COUNT(*) FROM plays"), 10_000);
    assert_eq!(count(&env.db(), "SELECT MIN(played_at) FROM plays"), 6);
    assert_eq!(count(&env.db(), "SELECT MAX(played_at) FROM plays"), 99_999);
}

#[test]
fn clear_history_empties_recent_and_keeps_favorites() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a"]);
    ok(lib.set_favorite(ids[0], true, 1));
    ok(lib.record_play(ids[0], 2));
    ok(lib.clear_history());
    let c = ok(lib.collections());
    assert_eq!(c.recent, vec![]);
    assert_eq!(c.favorites, vec![ids[0]]);
    assert_eq!(count(&env.db(), "SELECT COUNT(*) FROM plays"), 0);
}

#[test]
fn favorites_with_equal_added_at_list_the_higher_track_id_first() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a", "b", "c"]);
    for id in &ids {
        assert!(ok(lib.set_favorite(*id, true, 10)));
    }
    assert_eq!(
        ok(lib.collections()).favorites,
        vec![ids[2], ids[1], ids[0]]
    );
}

#[test]
fn recent_with_equal_played_at_lists_the_later_play_first() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a", "b", "c"]);
    ok(lib.record_play(ids[1], 5));
    ok(lib.record_play(ids[2], 5));
    ok(lib.record_play(ids[0], 5));
    assert_eq!(ok(lib.collections()).recent, vec![ids[0], ids[2], ids[1]]);
}

/// The visible entries of a playlist as `(position, track)` pairs.
fn items(lib: &Library, id: PlaylistId) -> Vec<(u32, TrackId)> {
    let c = ok(lib.collections());
    match c.playlist(id) {
        Some(p) => p.items.iter().map(|i| (i.position, i.track)).collect(),
        None => panic!("playlist {id:?} is missing"),
    }
}

fn unavailable(lib: &Library, id: PlaylistId) -> usize {
    match ok(lib.collections()).playlist(id) {
        Some(p) => p.unavailable,
        None => panic!("playlist {id:?} is missing"),
    }
}

/// A playlist holding `tracks` in order.
fn playlist_of(lib: &mut Library, tracks: &[TrackId]) -> PlaylistId {
    let id = ok(lib.create_playlist("p", None, 1));
    for t in tracks {
        assert!(ok(lib.add_to_playlist(id, *t)));
    }
    id
}

#[test]
fn names_are_trimmed_and_measured_in_characters() {
    assert_eq!(PLAYLIST_NAME_MAX, 100);
    assert_eq!(playlist_name("  Road  "), Some("Road"));
    assert_eq!(playlist_name(""), None);
    assert_eq!(playlist_name("   "), None);
    let armenian_100 = "ե".repeat(100);
    let armenian_101 = "ե".repeat(101);
    assert_eq!(playlist_name(&armenian_100), Some(armenian_100.as_str()));
    assert_eq!(playlist_name(&armenian_101), None);
    assert_eq!(playlist_name("🎵 mix"), Some("🎵 mix"));

    let env = Env::new();
    let mut lib = env.open();
    assert!(matches!(
        lib.create_playlist("   ", None, 1),
        Err(LibraryError::InvalidPlaylistName)
    ));
    assert_eq!(count(&env.db(), "SELECT COUNT(*) FROM playlists"), 0);
    let id = ok(lib.create_playlist("  Road ", None, 1));
    assert!(matches!(
        lib.rename_playlist(id, ""),
        Err(LibraryError::InvalidPlaylistName)
    ));
    assert_eq!(ok(lib.collections()).playlists[0].name, "Road");
    assert!(ok(lib.rename_playlist(id, " Trip ")));
    assert_eq!(ok(lib.collections()).playlists[0].name, "Trip");
    assert!(!ok(lib.rename_playlist(PlaylistId(999), "x")));
}

#[test]
fn two_playlists_may_share_a_name_and_sort_by_name_then_id() {
    let env = Env::new();
    let mut lib = env.open();
    let b = ok(lib.create_playlist("b", None, 1));
    let upper = ok(lib.create_playlist("A", None, 2));
    let lower = ok(lib.create_playlist("a", None, 3));
    let c = ok(lib.collections());
    let order: Vec<(PlaylistId, &str)> = c
        .playlists
        .iter()
        .map(|p| (p.id, p.name.as_str()))
        .collect();
    assert_eq!(order, vec![(upper, "A"), (lower, "a"), (b, "b")]);
}

#[test]
fn entries_append_and_may_repeat() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let id = playlist_of(&mut lib, &[a, b, a]);
    assert_eq!(items(&lib, id), vec![(0, a), (1, b), (2, a)]);
    assert!(!ok(lib.add_to_playlist(id, TrackId(999))));
    assert!(!ok(lib.add_to_playlist(PlaylistId(999), a)));
}

#[test]
fn create_with_a_first_track_holds_it() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a"]);
    let id = ok(lib.create_playlist("p", Some(ids[0]), 1));
    assert_eq!(items(&lib, id), vec![(0, ids[0])]);
}

#[test]
fn create_with_an_unknown_first_track_is_empty() {
    let env = Env::new();
    let (mut lib, _, _) = with_tracks(&env, &["a"]);
    let id = ok(lib.create_playlist("p", Some(TrackId(999)), 1));
    assert_eq!(items(&lib, id), vec![]);
}

#[test]
fn removing_an_entry_closes_the_gap() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a", "b", "c"]);
    let id = playlist_of(&mut lib, &ids);
    assert!(ok(lib.remove_from_playlist(id, 1)));
    assert_eq!(items(&lib, id), vec![(0, ids[0]), (1, ids[2])]);
    assert!(!ok(lib.remove_from_playlist(id, 9)));
    assert!(!ok(lib.remove_from_playlist(PlaylistId(999), 0)));
}

#[test]
fn moving_an_entry_lands_it_at_the_target() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a", "b", "c", "d"]);
    let (a, b, c, d) = (ids[0], ids[1], ids[2], ids[3]);
    let id = playlist_of(&mut lib, &ids);
    assert!(ok(lib.move_in_playlist(id, 0, 2)));
    assert_eq!(items(&lib, id), vec![(0, b), (1, c), (2, a), (3, d)]);
    assert!(ok(lib.move_in_playlist(id, 3, 0)));
    assert_eq!(items(&lib, id), vec![(0, d), (1, b), (2, c), (3, a)]);
    assert!(!ok(lib.move_in_playlist(id, 1, 1)));
    assert!(!ok(lib.move_in_playlist(id, 0, 9)));
    assert!(!ok(lib.move_in_playlist(id, 9, 0)));
    assert!(!ok(lib.move_in_playlist(PlaylistId(999), 0, 1)));
    assert_eq!(items(&lib, id), vec![(0, d), (1, b), (2, c), (3, a)]);
}

#[test]
fn entries_hide_and_return_with_their_folder() {
    let env = Env::new();
    let (mut lib, folder, ids) = with_tracks(&env, &["a", "b"]);
    let id = playlist_of(&mut lib, &ids);
    ok(lib.remove_folder(folder));
    assert_eq!(items(&lib, id), vec![]);
    assert_eq!(unavailable(&lib, id), 2);

    let folder = ok(lib.add_folder(&env.folder("a")));
    let a2 = insert_track(&env.db(), folder, "a");
    let b2 = insert_track(&env.db(), folder, "b");
    assert_eq!(items(&lib, id), vec![(0, a2), (1, b2)]);
    assert_eq!(unavailable(&lib, id), 0);
}

#[test]
fn moves_keep_unavailable_entries_in_place() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a", "x", "b", "c"]);
    let (a, x, b, c) = (ids[0], ids[1], ids[2], ids[3]);
    let id = playlist_of(&mut lib, &ids);
    {
        let conn = ok(Connection::open(env.db()));
        ok(conn.execute("DELETE FROM tracks WHERE id = ?1", [x.0]));
    }
    assert_eq!(items(&lib, id), vec![(0, a), (2, b), (3, c)]);
    assert_eq!(unavailable(&lib, id), 1);

    assert!(ok(lib.move_in_playlist(id, 3, 0)));
    assert_eq!(items(&lib, id), vec![(0, c), (1, a), (3, b)]);
    assert_eq!(unavailable(&lib, id), 1);

    assert!(ok(lib.remove_from_playlist(id, 1)));
    assert_eq!(items(&lib, id), vec![(0, c), (2, b)]);
    assert_eq!(unavailable(&lib, id), 1);
}

#[test]
fn deleting_a_playlist_deletes_its_entries() {
    let env = Env::new();
    let (mut lib, _, ids) = with_tracks(&env, &["a", "b"]);
    let id = playlist_of(&mut lib, &ids);
    assert_eq!(count(&env.db(), "SELECT COUNT(*) FROM playlist_items"), 2);
    assert!(ok(lib.delete_playlist(id)));
    assert_eq!(count(&env.db(), "SELECT COUNT(*) FROM playlist_items"), 0);
    assert_eq!(ok(lib.collections()).playlists, vec![]);
    assert!(!ok(lib.delete_playlist(id)));
}

#[test]
#[ignore = "timing run: cargo test --release -p aria-library --test collections -- --ignored --nocapture"]
fn collections_build_time_for_a_large_library() {
    let env = Env::new();
    let (lib, folder, _) = with_tracks(&env, &[]);
    {
        let mut conn = ok(Connection::open(env.db()));
        let tx = ok(conn.transaction());
        for i in 0..50_000u32 {
            let name = format!("t{i}");
            ok(tx.execute(
                "INSERT INTO tracks (folder_id, path, size, mtime, title, added_at)
                 VALUES (?1, ?2, 1, 1, ?3, 1)",
                (folder.0, name.as_bytes(), &name),
            ));
        }
        for i in 0..5_000u32 {
            ok(tx.execute(
                "INSERT INTO favorites (path, added_at) VALUES (?1, ?2)",
                (format!("t{i}").as_bytes(), i64::from(i)),
            ));
        }
        for p in 0..50u32 {
            ok(tx.execute(
                "INSERT INTO playlists (name, created_at) VALUES (?1, 1)",
                [format!("playlist {p}")],
            ));
            let pid = tx.last_insert_rowid();
            for pos in 0..200u32 {
                ok(tx.execute(
                    "INSERT INTO playlist_items (playlist_id, position, path) VALUES (?1, ?2, ?3)",
                    (pid, pos, format!("t{}", p * 200 + pos).as_bytes()),
                ));
            }
        }
        for i in 0..10_000u32 {
            ok(tx.execute(
                "INSERT INTO plays (path, played_at) VALUES (?1, ?2)",
                (format!("t{}", i % 5_000).as_bytes(), i64::from(i)),
            ));
        }
        ok(tx.commit());
    }
    let start = std::time::Instant::now();
    let c = ok(lib.collections());
    let elapsed = start.elapsed();
    println!("collections() took {elapsed:?}");
    assert_eq!(c.favorites.len(), 5_000);
    assert_eq!(c.playlists.len(), 50);
    assert!(elapsed < std::time::Duration::from_millis(500));
}
