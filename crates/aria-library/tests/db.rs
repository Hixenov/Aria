//! Storage-layer tests for `Library`.

use std::path::{Path, PathBuf};

use aria_library::{FolderId, Library, LibraryError, QueueRecord, SCHEMA_VERSION, TrackId};
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
    fn open(&self) -> aria_library::Opened {
        ok(Library::open(&self.db(), &self.art()))
    }
    /// A path under the temp dir that is used only as a folder name.
    fn folder(&self, name: &str) -> PathBuf {
        self.dir.path().join("music").join(name)
    }
}

fn user_version(db: &Path) -> u32 {
    let c = ok(Connection::open(db));
    ok(c.pragma_query_value(None, "user_version", |r| r.get(0)))
}

fn insert_track(db: &Path, folder: FolderId, name: &str) {
    let c = ok(Connection::open(db));
    ok(c.execute(
        "INSERT INTO tracks (folder_id, path, size, mtime, title, added_at)
         VALUES (?1, ?2, 1, 1, ?3, 1)",
        (folder.0, name.as_bytes(), name),
    ));
}

fn track_folders(db: &Path) -> Vec<i64> {
    let c = ok(Connection::open(db));
    let mut s = ok(c.prepare("SELECT folder_id FROM tracks ORDER BY id"));
    ok(s.query_map([], |r| r.get(0))).map(ok).collect()
}

fn dir_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = ok(std::fs::read_dir(dir))
        .map(|e| ok(e).file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn fresh_database_migrates() {
    let env = Env::new();
    let opened = env.open();
    assert!(!opened.recovered);
    let mut lib = opened.library;
    let id = lib.add_folder(&env.folder("a")).expect("add");
    drop(lib);

    assert_eq!(user_version(&env.db()), SCHEMA_VERSION);

    let reopened = env.open();
    assert!(!reopened.recovered);
    let folders = reopened.library.folders().expect("folders");
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].id, id);
    assert!(folders[0].available);
}

#[test]
fn newer_database_is_refused_and_untouched() {
    let env = Env::new();
    {
        let c = Connection::open(env.db()).expect("raw open");
        c.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);")
            .expect("seed");
        c.pragma_update(None, "user_version", SCHEMA_VERSION + 3)
            .expect("set version");
    }
    let before = std::fs::read(env.db()).expect("read");
    let names_before = dir_names(env.dir.path());

    let err = Library::open(&env.db(), &env.art())
        .err()
        .expect("must refuse");
    match err {
        LibraryError::NewerSchema { found, supported } => {
            assert_eq!(found, SCHEMA_VERSION + 3);
            assert_eq!(supported, SCHEMA_VERSION);
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(std::fs::read(env.db()).expect("read"), before);
    assert_eq!(dir_names(env.dir.path()), names_before);
}

#[test]
fn corrupt_database_is_replaced() {
    let env = Env::new();
    let mut garbage = Vec::new();
    let mut x: u32 = 0x1234_5678;
    for _ in 0..8192 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        garbage.push((x & 0xff) as u8);
    }
    std::fs::write(env.db(), &garbage).expect("write");
    // Stale side files must not leak into the new database.
    std::fs::write(env.dir.path().join("library.db-wal"), b"stale").expect("wal");
    std::fs::write(env.dir.path().join("library.db-shm"), b"stale").expect("shm");

    let opened = env.open();
    assert!(opened.recovered);
    assert_eq!(
        std::fs::read(env.dir.path().join("library.db.bak")).expect("bak"),
        garbage
    );
    // The side files moved with it instead of being deleted or left for the new database.
    assert_eq!(
        std::fs::read(env.dir.path().join("library.db.bak-wal")).expect("bak-wal"),
        b"stale"
    );
    // SQLite rewrites a wal-index while probing, so only its presence is checked.
    assert!(env.dir.path().join("library.db.bak-shm").exists());
    assert!(
        !env.dir.path().join("library.db-wal").exists() || {
            // The new database may own a fresh WAL, but never the stale one.
            std::fs::read(env.dir.path().join("library.db-wal")).expect("wal") != b"stale"
        }
    );
    let mut lib = opened.library;
    lib.add_folder(&env.folder("a")).expect("new db works");
    assert_eq!(lib.folders().expect("folders").len(), 1);
    drop(lib);
    assert_eq!(user_version(&env.db()), SCHEMA_VERSION);
}

#[test]
fn corrupt_recovery_overwrites_older_backup() {
    let env = Env::new();
    std::fs::write(env.dir.path().join("library.db.bak"), b"old backup").expect("bak");
    std::fs::write(env.db(), vec![0xAB; 4096]).expect("write");
    let opened = env.open();
    assert!(opened.recovered);
    assert_eq!(
        std::fs::read(env.dir.path().join("library.db.bak")).expect("bak"),
        vec![0xAB; 4096]
    );
}

#[test]
fn paths_round_trip() {
    let env = Env::new();
    let mut lib = env.open().library;
    let mut names: Vec<std::ffi::OsString> =
        vec!["Музыка".into(), "Երաժշտություն".into(), "emoji 🎵🎶".into()];
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        names.push(std::ffi::OsString::from_vec(b"bad\xff\xfename".to_vec()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        names.push(std::ffi::OsString::from_wide(&[0x62, 0xD800, 0x61]));
    }
    let mut expected = Vec::new();
    for n in names {
        let p = env.dir.path().join("music").join(n);
        lib.add_folder(&p).expect("add");
        expected.push(p);
    }
    let got: Vec<PathBuf> = lib
        .folders()
        .expect("folders")
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(got.len(), expected.len());
    for p in &expected {
        assert!(got.contains(p), "missing a round-tripped path");
    }
}

#[test]
fn folder_rules() {
    let env = Env::new();
    let mut lib = env.open().library;

    let root = env.folder("root");
    let a = lib.add_folder(&root.join("a")).expect("add a");
    let b = lib.add_folder(&root.join("b")).expect("add b");
    let other = lib.add_folder(&env.folder("other")).expect("add other");

    // Same folder, also spelled differently.
    assert!(matches!(
        lib.add_folder(&root.join("a")),
        Err(LibraryError::FolderAlreadyAdded)
    ));
    assert!(matches!(
        lib.add_folder(&root.join("x").join("..").join("a")),
        Err(LibraryError::FolderAlreadyAdded)
    ));
    // Nested.
    assert!(matches!(
        lib.add_folder(&root.join("a").join("deep")),
        Err(LibraryError::FolderInsideExisting)
    ));
    // A sibling whose name only shares a string prefix is not nested.
    lib.add_folder(&root.join("a2")).expect("a2 is a sibling");

    insert_track(&env.db(), a, "t1");
    insert_track(&env.db(), b, "t2");
    insert_track(&env.db(), b, "t3");
    insert_track(&env.db(), other, "t4");

    // Parent absorbs a and b (and the a2 sibling) in one go.
    let parent = lib.add_folder(&root).expect("add parent");
    let ids: Vec<FolderId> = lib
        .folders()
        .expect("folders")
        .iter()
        .map(|f| f.id)
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&parent));
    assert!(ids.contains(&other));
    assert_eq!(lib.track_count().expect("count"), 4);
    assert_eq!(
        track_folders(&env.db()),
        vec![parent.0, parent.0, parent.0, other.0]
    );
}

#[cfg(windows)]
#[test]
fn folder_comparison_ignores_case_on_windows() {
    let env = Env::new();
    let mut lib = env.open().library;
    lib.add_folder(Path::new(r"C:\Music\Rock")).expect("add");
    assert!(matches!(
        lib.add_folder(Path::new(r"c:\music\ROCK")),
        Err(LibraryError::FolderAlreadyAdded)
    ));
    assert!(matches!(
        lib.add_folder(Path::new(r"c:\MUSIC\rock\Live")),
        Err(LibraryError::FolderInsideExisting)
    ));
}

#[test]
fn remove_folder_deletes_rows_only() {
    let env = Env::new();
    let real_dir = env.folder("real");
    std::fs::create_dir_all(&real_dir).expect("mkdir");
    let real_file = real_dir.join("song.flac");
    std::fs::write(&real_file, b"audio").expect("write");

    let mut lib = env.open().library;
    let id = lib.add_folder(&real_dir).expect("add");
    insert_track(&env.db(), id, "song");
    assert_eq!(lib.track_count().expect("count"), 1);

    lib.remove_folder(id).expect("remove");
    assert_eq!(lib.track_count().expect("count"), 0);
    assert!(lib.folders().expect("folders").is_empty());
    assert!(real_file.exists());
    assert_eq!(std::fs::read(&real_file).expect("read"), b"audio");
}

#[test]
fn queue_round_trips() {
    let env = Env::new();
    let mut lib = env.open().library;
    assert_eq!(lib.load_queue().expect("load"), None);

    let q = QueueRecord {
        context: vec![TrackId(5), TrackId(2), TrackId(9)],
        original: vec![TrackId(9), TrackId(5), TrackId(2)],
        user: vec![TrackId(7)],
        current: Some(TrackId(2)),
        position_ms: 61_500,
        shuffle_seed: u64::MAX - 3,
        context_index: Some(1),
    };
    lib.save_queue(&q).expect("save");
    assert_eq!(lib.load_queue().expect("load"), Some(q));

    let latest = QueueRecord {
        context: vec![TrackId(1)],
        original: vec![],
        user: vec![],
        current: None,
        position_ms: 0,
        shuffle_seed: 42,
        context_index: None,
    };
    lib.save_queue(&latest).expect("save");
    drop(lib);
    let lib = env.open().library;
    assert_eq!(lib.load_queue().expect("load"), Some(latest));
}

#[test]
fn a_position_save_keeps_the_lists() {
    let env = Env::new();
    let mut lib = env.open().library;
    lib.save_position(Some(TrackId(1)), 5_000).expect("save");
    assert_eq!(lib.load_queue().expect("load"), None, "no queue to update");

    let q = QueueRecord {
        context: vec![TrackId(5), TrackId(2), TrackId(9)],
        original: vec![],
        user: vec![TrackId(7)],
        current: Some(TrackId(5)),
        position_ms: 1_000,
        shuffle_seed: 11,
        context_index: Some(0),
    };
    lib.save_queue(&q).expect("save");
    lib.save_position(Some(TrackId(5)), 2_000).expect("save");
    lib.save_position(Some(TrackId(5)), 61_500).expect("save");
    drop(lib);

    let lib = env.open().library;
    let newest = QueueRecord {
        position_ms: 61_500,
        ..q
    };
    assert_eq!(lib.load_queue().expect("load"), Some(newest));
}

#[test]
fn set_duration_updates_track() {
    let env = Env::new();
    let mut lib = env.open().library;
    let f = lib.add_folder(&env.folder("a")).expect("add");
    insert_track(&env.db(), f, "t");
    let c = Connection::open(env.db()).expect("raw");
    let id: i64 = c
        .query_row("SELECT id FROM tracks", [], |r| r.get(0))
        .expect("id");
    lib.set_duration(TrackId(id), 123_456).expect("set");
    let ms: i64 = c
        .query_row("SELECT duration_ms FROM tracks WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .expect("ms");
    assert_eq!(ms, 123_456);
}

#[test]
fn error_text_has_no_paths() {
    let env = Env::new();
    let mut lib = env.open().library;
    lib.add_folder(&env.folder("a")).expect("add");
    let Err(err) = lib.add_folder(&env.folder("a")) else {
        panic!("second add must be refused");
    };
    let text = err.to_string();
    assert!(!text.contains(env.dir.path().to_string_lossy().as_ref()));
}

#[test]
fn locked_database_is_not_treated_as_corrupt() {
    let env = Env::new();
    // Rollback-journal mode, where an exclusive lock blocks readers too.
    {
        let c = ok(Connection::open(env.db()));
        ok(c.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);"));
    }
    let before = std::fs::read(env.db()).expect("read");
    let holder = ok(Connection::open(env.db()));
    ok(holder.execute_batch("BEGIN EXCLUSIVE"));

    let res = Library::open(&env.db(), &env.art());
    assert!(
        matches!(res, Err(LibraryError::Database(_))),
        "{:?}",
        res.err()
    );
    ok(holder.execute_batch("ROLLBACK"));
    drop(holder);

    assert!(!env.dir.path().join("library.db.bak").exists());
    assert_eq!(std::fs::read(env.db()).expect("read"), before);
}

#[test]
fn crash_left_wal_database_opens_with_its_data() {
    let env = Env::new();
    let mut lib = env.open().library;
    let id = lib.add_folder(&env.folder("kept")).expect("add");
    // Copy the files while the connection is still open: the data lives only in the WAL.
    let crash = ok(tempfile::tempdir());
    for name in ["library.db", "library.db-wal"] {
        let src = env.dir.path().join(name);
        assert!(src.exists(), "{name} should exist while open");
        ok(std::fs::copy(src, crash.path().join(name)));
    }
    let opened = ok(Library::open(
        &crash.path().join("library.db"),
        &crash.path().join("art"),
    ));
    assert!(!opened.recovered);
    let folders = opened.library.folders().expect("folders");
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].id, id);
    assert!(!crash.path().join("library.db.bak").exists());
}

#[test]
fn stray_wal_beside_missing_database_does_not_reach_new_one() {
    let source = Env::new();
    let mut old = source.open().library;
    old.add_folder(&source.folder("ghost")).expect("add");

    // A valid WAL (with committed frames) whose database file is gone.
    let env = Env::new();
    let wal = source.dir.path().join("library.db-wal");
    assert!(wal.exists());
    ok(std::fs::copy(&wal, env.dir.path().join("library.db-wal")));
    drop(old);

    let opened = env.open();
    assert!(!opened.recovered);
    assert!(opened.library.folders().expect("folders").is_empty());
}

#[test]
fn newer_wal_database_is_refused_and_main_file_untouched() {
    let env = Env::new();
    // Keep a connection open so the WAL and shm stay on disk.
    let c = ok(Connection::open(env.db()));
    ok(c.pragma_update(None, "journal_mode", "WAL"));
    ok(c.execute_batch("CREATE TABLE t (x INTEGER);"));
    ok(c.pragma_update(None, "user_version", SCHEMA_VERSION + 2));
    ok(c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);"));
    let before = std::fs::read(env.db()).expect("read");

    let res = Library::open(&env.db(), &env.art());
    assert!(matches!(
        res,
        Err(LibraryError::NewerSchema { found, .. }) if found == SCHEMA_VERSION + 2
    ));
    assert_eq!(std::fs::read(env.db()).expect("read"), before);
    drop(c);
    assert!(!env.dir.path().join("library.db.bak").exists());
}
