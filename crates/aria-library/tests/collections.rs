//! Tests for favorites, plays and the `Collections` view.

use std::path::{Path, PathBuf};

use aria_library::{
    Collections, FolderId, Library, PLAYS_KEPT, QueueRecord, RECENT_LIMIT, SCHEMA_VERSION, TrackId,
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
