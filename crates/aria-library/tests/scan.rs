//! Scanner and snapshot tests, against the shared audio fixtures.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use aria_library::{
    FolderId, Library, LibrarySnapshot, QueueRecord, ScanProgress, ScanSummary, TrackId,
};

/// Unwraps in helpers, which clippy's allow-unwrap-in-tests does not cover.
fn ok<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("test setup failed: {e:?}"),
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../aria-audio/tests/fixtures")
        .join(name)
}

struct Env {
    dir: tempfile::TempDir,
    lib: Library,
}

impl Env {
    fn new() -> Self {
        let dir = ok(tempfile::tempdir());
        let lib = ok(Library::open(
            &dir.path().join("library.db"),
            &dir.path().join("art"),
        ))
        .library;
        ok(fs::create_dir_all(dir.path().join("music")));
        Self { dir, lib }
    }

    fn music(&self) -> PathBuf {
        self.dir.path().join("music")
    }

    /// Copies a fixture to `rel` under the music folder, creating directories.
    fn put(&self, fixture_name: &str, rel: &str) -> PathBuf {
        let to = self.music().join(rel);
        if let Some(parent) = to.parent() {
            ok(fs::create_dir_all(parent));
        }
        ok(fs::copy(fixture(fixture_name), &to));
        to
    }

    fn add_music(&mut self) -> FolderId {
        let music = self.music();
        ok(self.lib.add_folder(&music))
    }

    fn scan(&mut self) -> ScanSummary {
        ok(self.lib.scan(&AtomicBool::new(false), &mut |_, _| {}))
    }

    fn snap(&self) -> LibrarySnapshot {
        ok(self.lib.snapshot())
    }
}

fn ids(s: &LibrarySnapshot) -> Vec<(PathBuf, TrackId)> {
    s.tracks.iter().map(|t| (t.path.clone(), t.id)).collect()
}

fn bump_mtime(path: &Path, secs: u64) {
    let f = ok(fs::OpenOptions::new().write(true).open(path));
    ok(f.set_modified(SystemTime::now() + Duration::from_secs(secs)));
}

#[test]
fn scans_a_tree() {
    let mut env = Env::new();
    env.put("tagged.mp3", "a/tagged.mp3");
    env.put("tone.flac", "a/b/c/tone.flac");
    env.put("tone.wav", "tone.wav");
    env.add_music();
    let s = env.scan();
    assert_eq!(s.added, 3);
    assert_eq!(s.updated, 0);
    assert_eq!(s.removed, 0);
    assert_eq!(s.skipped, 0);
    assert!(s.unavailable.is_empty());
    assert!(!s.cancelled);

    let snap = env.snap();
    assert_eq!(snap.tracks.len(), 3);
    assert_eq!(snap.folders.len(), 1);
    let album = snap
        .albums
        .iter()
        .find(|a| a.title.as_deref() == Some("Fixture Album"));
    let Some(album) = album else {
        panic!("no Fixture Album in {:?}", snap.albums);
    };
    assert_eq!(album.artist.as_deref(), Some("Fixture Artist"));
    assert_eq!(album.tracks.len(), 1);
    let Some(track) = snap.track(album.tracks[0]) else {
        panic!("album track missing");
    };
    assert_eq!(track.title, "Fixture Title");
    assert!(track.path.ends_with("tagged.mp3"));
    // The other two have no album and group under no artist.
    assert!(snap.albums.iter().any(|a| a.title.is_none()));
}

#[test]
fn rescan_reads_only_changed_files() {
    let mut env = Env::new();
    let changed = env.put("tone.mp3", "one.mp3");
    env.put("tone.flac", "two.flac");
    env.add_music();
    env.scan();
    let before = ids(&env.snap());

    let s = env.scan();
    assert_eq!((s.added, s.updated, s.removed), (0, 0, 0));

    ok(fs::copy(fixture("tone.mp3"), &changed));
    bump_mtime(&changed, 60);
    let s = env.scan();
    assert_eq!((s.added, s.updated, s.removed), (0, 1, 0));
    assert_eq!(ids(&env.snap()), before);
}

#[test]
fn size_change_alone_is_a_change() {
    let mut env = Env::new();
    let f = env.put("tone.wav", "one.wav");
    env.add_music();
    env.scan();
    let mtime = ok(ok(fs::metadata(&f)).modified());
    ok(fs::copy(fixture("tone-mono.wav"), &f));
    let h = ok(fs::OpenOptions::new().write(true).open(&f));
    ok(h.set_modified(mtime));
    drop(h);
    let s = env.scan();
    assert_eq!(s.updated, 1);
}

#[test]
fn deleted_file_is_removed() {
    let mut env = Env::new();
    let gone = env.put("tone.mp3", "a/one.mp3");
    env.put("tone.flac", "a/two.flac");
    env.put("tone.wav", "three.wav");
    env.add_music();
    env.scan();
    let before = ids(&env.snap());

    ok(fs::remove_file(&gone));
    let s = env.scan();
    assert_eq!(s.removed, 1);
    let after = ids(&env.snap());
    let expected: Vec<_> = before.into_iter().filter(|(p, _)| *p != gone).collect();
    assert_eq!(after, expected);
}

#[test]
fn deleted_subfolder_is_removed() {
    let mut env = Env::new();
    env.put("tone.mp3", "keep/one.mp3");
    env.put("tone.flac", "drop/deep/two.flac");
    env.put("tone.wav", "drop/three.wav");
    env.add_music();
    env.scan();
    assert_eq!(env.snap().tracks.len(), 3);

    ok(fs::remove_dir_all(env.music().join("drop")));
    let s = env.scan();
    assert_eq!(s.removed, 2);
    assert_eq!(env.snap().tracks.len(), 1);
}

#[cfg(unix)]
#[test]
fn unreadable_subfolder_keeps_tracks() {
    use std::os::unix::fs::PermissionsExt;
    let mut env = Env::new();
    env.put("tone.mp3", "keep/one.mp3");
    env.put("tone.flac", "locked/two.flac");
    env.add_music();
    env.scan();
    let locked = env.music().join("locked");
    ok(fs::set_permissions(
        &locked,
        fs::Permissions::from_mode(0o000),
    ));
    // A superuser can still list the directory, which makes the check meaningless.
    let forced = fs::read_dir(&locked).is_err();
    let s = env.scan();
    ok(fs::set_permissions(
        &locked,
        fs::Permissions::from_mode(0o755),
    ));
    if forced {
        assert_eq!(s.removed, 0);
        assert!(s.skipped >= 1);
        assert_eq!(env.snap().tracks.len(), 2);
    }
}

#[test]
fn unreachable_root_keeps_tracks() {
    let mut env = Env::new();
    env.put("tone.mp3", "one.mp3");
    env.put("tone.flac", "a/two.flac");
    let id = env.add_music();
    env.scan();
    let before = ids(&env.snap());

    let away = env.dir.path().join("away");
    ok(fs::rename(env.music(), &away));
    let s = env.scan();
    assert_eq!(s.unavailable, vec![id]);
    assert_eq!(s.removed, 0);
    let snap = env.snap();
    assert_eq!(ids(&snap), before);
    assert!(!snap.folders[0].available);

    ok(fs::rename(&away, env.music()));
    let s = env.scan();
    assert!(s.unavailable.is_empty());
    assert_eq!((s.added, s.updated, s.removed), (0, 0, 0));
    let snap = env.snap();
    assert!(snap.folders[0].available);
    assert_eq!(ids(&snap), before);
}

#[test]
fn lying_files_are_skipped() {
    let mut env = Env::new();
    ok(fs::write(env.music().join("empty.mp3"), b""));
    ok(fs::write(
        env.music().join("text.flac"),
        b"this is not audio",
    ));
    env.put("tone.wav", "good.wav");
    env.add_music();
    let s = env.scan();
    assert_eq!(s.added, 1);
    assert_eq!(s.skipped, 2);
    assert!(!s.cancelled);
    assert_eq!(env.snap().tracks.len(), 1);
}

#[test]
fn unreadable_file_keeps_its_row_and_is_retried() {
    let mut env = Env::new();
    let f = env.put("tone.mp3", "one.mp3");
    env.put("tone.flac", "two.flac");
    env.add_music();
    env.scan();
    let before = ids(&env.snap());

    // The file changed, but its new content cannot be read.
    ok(fs::write(&f, b"not audio any more"));
    let s = env.scan();
    assert_eq!(s.skipped, 1);
    assert_eq!(s.removed, 0);
    assert_eq!(ids(&env.snap()), before);

    // The stored size and mtime were left alone, so the next scan tries again.
    let s = env.scan();
    assert_eq!(s.skipped, 1);

    // Once the file reads again, it is updated in place.
    ok(fs::copy(fixture("tone.mp3"), &f));
    bump_mtime(&f, 120);
    let s = env.scan();
    assert_eq!((s.skipped, s.updated, s.removed), (0, 1, 0));
    assert_eq!(ids(&env.snap()), before);
}

#[cfg(windows)]
#[test]
fn locked_file_keeps_its_row() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut env = Env::new();
    let f = env.put("tone.mp3", "one.mp3");
    env.add_music();
    env.scan();
    let before = ids(&env.snap());

    bump_mtime(&f, 60);
    // Share mode 0 refuses every other open, including the tag reader's.
    let lock = ok(fs::OpenOptions::new().read(true).share_mode(0).open(&f));
    let s = env.scan();
    assert_eq!((s.skipped, s.removed, s.updated), (1, 0, 0));
    assert_eq!(ids(&env.snap()), before);
    drop(lock);

    let s = env.scan();
    assert_eq!((s.skipped, s.updated), (0, 1));
}

#[test]
fn unsupported_extensions_are_ignored() {
    let mut env = Env::new();
    ok(fs::write(env.music().join("notes.txt"), b"hello"));
    ok(fs::write(env.music().join("cover.jpg"), b"jpeg"));
    env.put("tone.wav", "good.wav");
    env.add_music();
    let s = env.scan();
    assert_eq!((s.added, s.skipped), (1, 0));
}

#[test]
fn extensions_match_case_insensitively() {
    let mut env = Env::new();
    env.put("tone.mp3", "LOUD.MP3");
    env.put("tone.flac", "Mixed.FlAc");
    env.add_music();
    assert_eq!(env.scan().added, 2);
}

#[test]
fn unicode_tree_scans() {
    let mut env = Env::new();
    env.put("tone.mp3", "Музыка/Альбом/песня.mp3");
    env.put("tone.flac", "Երաժշտություն/ձայն.flac");
    env.put("tone.wav", "音楽/🎵.wav");
    env.add_music();
    assert_eq!(env.scan().added, 3);
    let snap = env.snap();
    assert!(
        snap.tracks
            .iter()
            .any(|t| t.path.ends_with("Музыка/Альбом/песня.mp3"))
    );
    assert!(snap.tracks.iter().any(|t| t.title == "ձայն"));
    // And a rescan finds them unchanged.
    let s = env.scan();
    assert_eq!((s.added, s.updated, s.removed), (0, 0, 0));
}

#[test]
fn cancel_keeps_finished_work_and_deletes_nothing() {
    let mut env = Env::new();
    let gone = env.put("tone.mp3", "old/gone.mp3");
    env.put("tone.flac", "old/stay.flac");
    env.add_music();
    env.scan();
    assert_eq!(env.snap().tracks.len(), 2);

    ok(fs::remove_file(&gone));
    for i in 0..620 {
        env.put("tone.wav", &format!("new/{i:04}.wav"));
    }
    let cancel = AtomicBool::new(false);
    let mut first = None;
    let s = ok(env.lib.scan(&cancel, &mut |_, p| {
        first.get_or_insert(p.changed);
        cancel.store(true, Ordering::Relaxed);
    }));
    let Some(first) = first else {
        panic!("on_batch never ran");
    };
    assert!(first > 0);
    assert!(s.cancelled);
    assert_eq!(s.removed, 0);
    // Everything up to the first callback was committed; the vanished file was not deleted.
    let snap = env.snap();
    assert_eq!(s.added, first);
    assert_eq!(snap.tracks.len() as u64, 2 + first);
    assert!(snap.tracks.iter().any(|t| t.path == gone));

    // A later full scan finishes the job.
    let s = env.scan();
    assert_eq!((s.added, s.removed), (620 - first, 1));
}

#[test]
fn cancel_before_start_changes_nothing() {
    let mut env = Env::new();
    env.put("tone.mp3", "one.mp3");
    env.add_music();
    let cancel = AtomicBool::new(true);
    let s = ok(env.lib.scan(&cancel, &mut |_, _| {}));
    assert!(s.cancelled);
    assert_eq!(s.added, 0);
    assert!(env.snap().tracks.is_empty());
}

#[test]
fn on_batch_can_write() {
    let mut env = Env::new();
    env.put("tone.mp3", "one.mp3");
    env.add_music();
    let mut calls = 0;
    let mut saved = None;
    let s = ok(env
        .lib
        .scan(&AtomicBool::new(false), &mut |lib, _p: ScanProgress| {
            calls += 1;
            let q = QueueRecord {
                current: Some(TrackId(1)),
                ..QueueRecord::default()
            };
            saved = Some(lib.save_queue(&q).is_ok());
        }));
    assert_eq!(s.added, 1);
    assert_eq!(calls, 1);
    assert_eq!(saved, Some(true));
    assert_eq!(
        ok(env.lib.load_queue()).and_then(|q| q.current),
        Some(TrackId(1))
    );
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed() {
    let mut env = Env::new();
    env.put("tone.mp3", "a/one.mp3");
    ok(std::os::unix::fs::symlink(
        env.music(),
        env.music().join("a/loop"),
    ));
    ok(std::os::unix::fs::symlink(
        env.music().join("a/one.mp3"),
        env.music().join("a/link.mp3"),
    ));
    env.add_music();
    let s = env.scan();
    assert_eq!(s.added, 1);
}

#[test]
fn album_grouping_ignores_case() {
    let mut env = Env::new();
    for (name, album) in [("one.mp3", "ALBUM"), ("two.mp3", "album")] {
        let f = env.put("tone.mp3", name);
        tag(&f, "Some Artist", album);
    }
    env.add_music();
    env.scan();
    let snap = env.snap();
    assert_eq!(snap.albums.len(), 1);
    assert_eq!(snap.albums[0].tracks.len(), 2);
    assert_eq!(snap.artists.len(), 1);
    assert_eq!(snap.artists[0].tracks.len(), 2);
}

#[test]
fn strings_are_shared_in_a_snapshot() {
    let mut env = Env::new();
    for name in ["one.mp3", "two.mp3"] {
        let f = env.put("tone.mp3", name);
        tag(&f, "Some Artist", "Some Album");
    }
    env.add_music();
    env.scan();
    let snap = env.snap();
    let (Some(a), Some(b)) = (&snap.tracks[0].artist, &snap.tracks[1].artist) else {
        panic!("artists missing");
    };
    assert!(std::sync::Arc::ptr_eq(a, b));
}

#[test]
fn snapshots_are_deterministic() {
    let mut env = Env::new();
    env.put("tagged.mp3", "a.mp3");
    env.put("tone.flac", "b.flac");
    env.put("tone.wav", "c.wav");
    env.put("tone.ogg", "d.ogg");
    env.add_music();
    env.scan();
    assert_eq!(env.snap(), env.snap());
}

#[test]
fn folder_cover_becomes_artwork_and_clear_rebuilds_it() {
    let mut env = Env::new();
    env.put("tone.mp3", "a/one.mp3");
    env.put("tone.flac", "a/two.flac");
    let mut png = std::io::Cursor::new(Vec::new());
    ok(image::RgbImage::from_pixel(8, 8, image::Rgb([9, 99, 199]))
        .write_to(&mut png, image::ImageFormat::Png));
    ok(fs::write(env.music().join("a/cover.png"), png.into_inner()));
    env.add_music();
    env.scan();
    let snap = env.snap();
    assert!(snap.tracks.iter().all(|t| t.artwork.is_some()));
    assert_eq!(snap.tracks[0].artwork, snap.tracks[1].artwork);
    assert_eq!(snap.albums[0].artwork, snap.tracks[0].artwork);
    let art = env.dir.path().join("art");
    let thumbs = |dir: &Path| {
        ok(fs::read_dir(dir))
            .filter(|e| {
                ok(e.as_ref())
                    .path()
                    .extension()
                    .is_some_and(|x| x == "jpg")
            })
            .count()
    };
    assert_eq!(thumbs(&art), 1);

    ok(env.lib.clear_artwork());
    assert_eq!(thumbs(&art), 0);
    assert!(env.snap().tracks.iter().all(|t| t.artwork.is_none()));

    let s = env.scan();
    assert_eq!(s.updated, 2);
    assert_eq!(thumbs(&art), 1);
    assert!(env.snap().tracks.iter().all(|t| t.artwork.is_some()));
}

/// Writes artist and album tags into an mp3 copy.
fn tag(path: &Path, artist: &str, album: &str) {
    use lofty::config::WriteOptions;
    use lofty::tag::{Accessor, Tag, TagExt, TagType};
    let mut t = Tag::new(TagType::Id3v2);
    t.set_artist(artist.to_owned());
    t.set_album(album.to_owned());
    ok(t.save_to_path(path, WriteOptions::default()));
}

/// Inserts rows straight into the database file, the way a very large library would look.
fn insert_rows(db: &Path, folder: FolderId, rows: u32) {
    let mut conn = ok(rusqlite::Connection::open(db));
    let tx = ok(conn.transaction());
    {
        let mut ins = ok(tx.prepare(
            "INSERT INTO tracks (folder_id, path, size, mtime, title, artist, album, duration_ms,
                sample_rate, channels, codec, added_at)
             VALUES (?1, ?2, 1, 1, ?3, ?4, ?5, 1000, 44100, 2, 'flac', 0)",
        ));
        for i in 0..rows {
            let path = format!(
                "/library/artist {}/album {}/track {i}.flac",
                i % 500,
                i % 5000
            );
            let bytes: Vec<u8> = if cfg!(windows) {
                path.encode_utf16().flat_map(u16::to_le_bytes).collect()
            } else {
                path.into_bytes()
            };
            ok(ins.execute(rusqlite::params![
                folder.0,
                bytes,
                format!("Track {i}"),
                format!("Artist {}", i % 500),
                format!("Album {}", i % 5000),
            ]));
        }
    }
    ok(tx.commit());
}

#[test]
#[ignore = "performance check, run in release by Task 15"]
fn synthetic_50k_library() {
    let mut env = Env::new();
    let folder = env.add_music();
    insert_rows(&env.dir.path().join("library.db"), folder, 50_000);
    let start = std::time::Instant::now();
    let snap = env.snap();
    let took = start.elapsed();
    println!("snapshot of {} tracks took {took:?}", snap.tracks.len());
    assert_eq!(snap.tracks.len(), 50_000);
    assert_eq!(snap.albums.len(), 5000);
    assert_eq!(snap.artists.len(), 500);
    if !cfg!(debug_assertions) {
        assert!(took < Duration::from_secs(2), "snapshot took {took:?}");
    }
}

// ---- Fix round 1 ----

#[test]
fn root_replaced_by_empty_directory_keeps_rows() {
    let mut env = Env::new();
    env.put("tone.mp3", "a/one.mp3");
    env.put("tone.flac", "two.flac");
    let id = env.add_music();
    env.scan();
    let before = ids(&env.snap());

    let away = env.dir.path().join("away");
    ok(fs::rename(env.music(), &away));
    ok(fs::create_dir(env.music()));
    let s = env.scan();
    assert_eq!(s.removed, 0);
    assert_eq!(s.unavailable, vec![id]);
    let snap = env.snap();
    assert_eq!(ids(&snap), before);
    assert!(!snap.folders[0].available);

    // The real folder comes back: available again, nothing churned.
    ok(fs::remove_dir(env.music()));
    ok(fs::rename(&away, env.music()));
    let s = env.scan();
    assert!(s.unavailable.is_empty());
    assert_eq!((s.added, s.updated, s.removed), (0, 0, 0));
    let snap = env.snap();
    assert!(snap.folders[0].available);
    assert_eq!(ids(&snap), before);
}

#[test]
fn root_replaced_by_other_files_keeps_rows_and_adds_new() {
    let mut env = Env::new();
    env.put("tone.mp3", "one.mp3");
    env.put("tone.flac", "two.flac");
    let id = env.add_music();
    env.scan();
    let before = ids(&env.snap());

    let away = env.dir.path().join("away");
    ok(fs::rename(env.music(), &away));
    env.put("tone.wav", "other.wav");
    let s = env.scan();
    assert_eq!(s.removed, 0);
    assert_eq!(s.added, 1);
    assert_eq!(s.unavailable, vec![id]);
    let snap = env.snap();
    assert_eq!(snap.tracks.len(), 3);
    for (path, tid) in &before {
        assert!(snap.tracks.iter().any(|t| t.id == *tid && t.path == *path));
    }
    assert!(!snap.folders[0].available);
}

#[cfg(windows)]
fn with_last_upper(p: &Path) -> PathBuf {
    let Some(name) = p.file_name() else {
        panic!("no file name");
    };
    p.with_file_name(name.to_string_lossy().to_uppercase())
}

#[cfg(windows)]
fn icacls(p: &Path, args: &[&str]) {
    let o = ok(std::process::Command::new("icacls")
        .arg(p)
        .args(args)
        .output());
    assert!(o.status.success(), "{o:?}");
}

#[cfg(windows)]
#[test]
fn absorbing_a_parent_in_another_case_keeps_ids() {
    let mut env = Env::new();
    env.put("tone.mp3", "rock/a.mp3");
    env.put("tone.flac", "rock/b.flac");
    let music = env.music();
    ok(env.lib.add_folder(&music.join("rock")));
    env.scan();
    let before: Vec<_> = env.snap().tracks.iter().map(|t| t.id).collect();

    let parent = with_last_upper(&music);
    ok(env.lib.add_folder(&parent));
    let s = env.scan();
    assert_eq!((s.added, s.updated, s.removed), (0, 0, 0));
    let snap = env.snap();
    assert_eq!(snap.tracks.iter().map(|t| t.id).collect::<Vec<_>>(), before);
    // The stored path takes the spelling found on disk.
    assert!(snap.tracks.iter().all(|t| t.path.starts_with(&parent)));

    ok(fs::remove_file(music.join("rock/a.mp3")));
    let s = env.scan();
    assert_eq!(s.removed, 1);
    assert_eq!(env.snap().tracks.len(), 1);
}

#[cfg(windows)]
#[test]
fn absorbing_a_child_in_another_case_keeps_ids() {
    let mut env = Env::new();
    env.put("tone.mp3", "rock/a.mp3");
    env.put("tone.flac", "rock/b.flac");
    let music = env.music();
    ok(env.lib.add_folder(&music.join("ROCK")));
    env.scan();
    let before: Vec<_> = env.snap().tracks.iter().map(|t| t.id).collect();
    ok(env.lib.add_folder(&music));
    let s = env.scan();
    assert_eq!((s.added, s.updated, s.removed), (0, 0, 0));
    assert_eq!(
        env.snap().tracks.iter().map(|t| t.id).collect::<Vec<_>>(),
        before
    );
}

#[cfg(windows)]
#[test]
fn absorbed_child_in_another_case_that_cannot_be_listed_keeps_tracks() {
    let mut env = Env::new();
    env.put("tone.mp3", "rock/a.mp3");
    env.put("tone.flac", "rock/b.flac");
    let music = env.music();
    ok(env.lib.add_folder(&music.join("ROCK")));
    env.scan();
    ok(env.lib.add_folder(&music));
    let rock = music.join("rock");
    icacls(&rock, &["/deny", "*S-1-1-0:(RD)"]);
    let denied = fs::read_dir(&rock).is_err();
    let s = env.scan();
    icacls(&rock, &["/remove:d", "*S-1-1-0"]);
    assert!(denied, "could not make the directory unlistable");
    assert_eq!(s.removed, 0);
    assert_eq!(env.snap().tracks.len(), 2);
}

#[test]
fn a_rescan_without_changes_still_calls_on_batch() {
    let mut env = Env::new();
    for i in 0..520 {
        env.put("tone.wav", &format!("{i:04}.wav"));
    }
    env.add_music();
    env.scan();
    let mut calls = 0;
    let mut last = ScanProgress::default();
    let s = ok(env.lib.scan(&AtomicBool::new(false), &mut |lib, p| {
        calls += 1;
        last = p;
        // No transaction is open: a write succeeds.
        assert!(lib.save_queue(&QueueRecord::default()).is_ok());
    }));
    assert_eq!((s.added, s.updated, s.removed), (0, 0, 0));
    assert!(calls >= 1, "on_batch was never called");
    assert!(last.seen >= 500);
    assert_eq!(last.changed, 0);
}

#[test]
fn deleted_track_ids_are_not_reused() {
    let mut env = Env::new();
    env.put("tone.mp3", "a.mp3");
    let last = env.put("tone.flac", "z.flac");
    env.add_music();
    env.scan();
    let top = ok(env.lib.snapshot()).tracks.iter().map(|t| t.id).max();
    let Some(top) = top else {
        panic!("no tracks");
    };
    ok(fs::remove_file(&last));
    env.scan();
    env.put("tone.wav", "new.wav");
    env.scan();
    let snap = env.snap();
    let Some(new) = snap.tracks.iter().find(|t| t.path.ends_with("new.wav")) else {
        panic!("new track missing");
    };
    assert!(new.id > top, "{:?} reuses an id up to {top:?}", new.id);
}

fn break_artwork_dir(env: &Env) {
    let art = env.dir.path().join("art");
    ok(fs::remove_dir_all(&art));
    ok(fs::write(&art, b"in the way"));
}

fn repair_artwork_dir(env: &Env) {
    ok(fs::remove_file(env.dir.path().join("art")));
    ok(fs::create_dir(env.dir.path().join("art")));
}

fn put_cover(env: &Env) {
    let mut png = std::io::Cursor::new(Vec::new());
    ok(image::RgbImage::from_pixel(8, 8, image::Rgb([9, 99, 199]))
        .write_to(&mut png, image::ImageFormat::Png));
    ok(fs::write(env.music().join("cover.png"), png.into_inner()));
}

#[test]
fn artwork_disk_error_does_not_stop_the_scan_and_is_retried() {
    let mut env = Env::new();
    env.put("tone.mp3", "one.mp3");
    put_cover(&env);
    env.add_music();
    break_artwork_dir(&env);
    let s = env.scan();
    assert_eq!(s.added, 1);
    assert!(env.snap().tracks[0].artwork.is_none());

    repair_artwork_dir(&env);
    let s = env.scan();
    assert_eq!(s.updated, 1, "the track was not retried");
    assert!(env.snap().tracks[0].artwork.is_some());
    let s = env.scan();
    assert_eq!(s.updated, 0);
}

#[test]
fn artwork_disk_error_keeps_the_old_hash_on_update() {
    let mut env = Env::new();
    let f = env.put("tone.mp3", "one.mp3");
    put_cover(&env);
    env.add_music();
    env.scan();
    let old = env.snap().tracks[0].artwork.clone();
    assert!(old.is_some());

    bump_mtime(&f, 60);
    break_artwork_dir(&env);
    let s = env.scan();
    assert_eq!(s.updated, 1);
    assert_eq!(env.snap().tracks[0].artwork, old);
}

/// A WAV with a single sample: its duration rounds to 0 ms.
fn empty_wav(path: &Path) {
    let mut h = Vec::new();
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&38u32.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&44100u32.to_le_bytes());
    h.extend_from_slice(&88200u32.to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&2u32.to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes());
    ok(fs::write(path, h));
}

#[test]
fn a_zero_duration_from_tags_keeps_the_stored_duration() {
    let mut env = Env::new();
    let f = env.music().join("empty.wav");
    empty_wav(&f);
    env.add_music();
    let s = env.scan();
    assert_eq!(s.added, 1, "{s:?}");
    assert_eq!(env.snap().tracks[0].duration_ms, 0);
    let id = env.snap().tracks[0].id;
    ok(env.lib.set_duration(id, 4321));

    bump_mtime(&f, 60);
    let s = env.scan();
    assert_eq!(s.updated, 1);
    assert_eq!(env.snap().tracks[0].duration_ms, 4321);
}
