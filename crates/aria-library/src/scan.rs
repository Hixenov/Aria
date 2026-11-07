//! The background scanner: walks the library folders and keeps the database in step with disk.
//!
//! A scan never deletes a row unless it is sure the file is gone. A row is deleted only when
//! every directory on the way to it listed successfully and the entry was missing from its
//! parent's listing. An unreachable root, an unreadable subdirectory and an unreadable file all
//! keep their rows.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::params;

use crate::artwork::{ArtworkCache, MAX_IMAGE_BYTES, find_folder_image};
use crate::db::{Library, now_secs};
use crate::error::LibraryError;
use crate::model::{ArtworkId, Folder, FolderId};
use crate::paths::{decode_path, encode_path};
use crate::tags::{TrackTags, read_tags};

/// File extensions the scanner picks up, compared without regard to case.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "m4a", "aac", "mp4", "ogg", "oga", "opus", "mka", "aif", "aiff", "caf",
];

/// Changed tracks are written, and `on_batch` is called, after this many.
pub const BATCH_SIZE: usize = 500;

/// Running counts handed to `on_batch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanProgress {
    /// Audio files looked at so far.
    pub seen: u64,
    /// Files added or updated so far.
    pub changed: u64,
}

/// What a scan did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanSummary {
    pub added: u64,
    pub updated: u64,
    pub removed: u64,
    /// Files and directories that could not be read. Their rows are kept.
    pub skipped: u64,
    /// Folders whose root could not be listed. Their tracks are kept.
    pub unavailable: Vec<FolderId>,
    pub cancelled: bool,
}

/// True when the file name has a supported extension.
fn is_supported(name: &std::ffi::OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            let e = e.to_lowercase();
            SUPPORTED_EXTENSIONS.contains(&e.as_str())
        })
}

/// Modification time as signed nanoseconds since the epoch, or `None` when out of range.
fn mtime_key(t: SystemTime) -> Option<i64> {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).ok(),
        Err(e) => i64::try_from(e.duration().as_nanos()).ok().map(|n| -n),
    }
}

/// How a path is compared: the way `add_folder` compares folders, so case-insensitive on Windows
/// and exact elsewhere.
fn fold_name(name: &OsStr) -> OsString {
    if cfg!(windows) {
        OsString::from(name.to_string_lossy().to_lowercase())
    } else {
        name.to_owned()
    }
}

fn fold_path(path: &Path) -> PathBuf {
    if cfg!(windows) {
        path.components()
            .map(|c| fold_name(c.as_os_str()))
            .collect()
    } else {
        path.to_path_buf()
    }
}

/// The key two spellings of one path share.
fn path_key(path: &Path) -> Vec<u8> {
    encode_path(&fold_path(path))
}

/// What the scan knows about a stored row.
struct Existing {
    id: i64,
    /// The path as stored, which may differ from the disk spelling in case.
    path: Vec<u8>,
    size: i64,
    mtime: i64,
}

/// A row waiting to be written.
struct Pending {
    folder: FolderId,
    /// The id of the stored row to update in place, if there is one.
    id: Option<i64>,
    path: Vec<u8>,
    /// -1 marks a row to read again on the next scan; its artwork hash is left alone.
    size: i64,
    mtime: i64,
    tags: TrackTags,
    artwork: Option<ArtworkId>,
}

/// The directories that listed successfully, with the names found in each. Directories and names
/// are folded, see [`fold_path`].
///
/// Regular files without a supported extension are left out, so a file renamed to `.txt`
/// counts as gone. Everything else that was listed (directories, links, entries whose type
/// could not be read) is a name, so its rows are kept.
#[derive(Default)]
struct Listings(HashMap<PathBuf, HashSet<OsString>>);

impl Listings {
    /// Records a directory's listing, but only a complete one: an incomplete listing must never
    /// allow a delete.
    fn record(&mut self, dir: &Path, names: impl Iterator<Item = OsString>, complete: bool) {
        if complete {
            self.0
                .insert(fold_path(dir), names.map(|n| fold_name(&n)).collect());
        }
    }

    /// Whether the row at `file`, in the folder rooted at `root`, may be deleted.
    ///
    /// Walking down from the root: a directory that did not list successfully keeps the row, and
    /// so does an unreachable root. A name missing from a directory that did list means the
    /// file, or a directory above it, is gone.
    fn is_gone(&self, root: &Path, file: &Path) -> bool {
        let root = fold_path(root);
        let file = fold_path(file);
        let Ok(rel) = file.strip_prefix(&root) else {
            return false;
        };
        let mut dir = root;
        for part in rel.components() {
            let name = part.as_os_str();
            match self.0.get(&dir) {
                None => return false,
                Some(names) if !names.contains(name) => return true,
                Some(_) => {}
            }
            dir.push(name);
        }
        false
    }
}

enum Kind {
    Dir,
    File,
    /// Symbolic links, junctions and other reparse points, devices and so on.
    Other,
    Unknown,
}

struct Entry {
    name: OsString,
    path: PathBuf,
    kind: Kind,
}

/// Collects the entries of a listing. The flag is false when any entry could not be read, so the
/// listing is incomplete and must not be used to delete anything.
fn collect_listing<T>(entries: impl Iterator<Item = io::Result<T>>) -> (Vec<T>, bool) {
    let mut out = Vec::new();
    let mut complete = true;
    for entry in entries {
        match entry {
            Ok(e) => out.push(e),
            Err(_) => complete = false,
        }
    }
    (out, complete)
}

fn read_entry(entry: fs::DirEntry) -> io::Result<Entry> {
    let kind = match entry.file_type() {
        Ok(t) if t.is_dir() => Kind::Dir,
        Ok(t) if t.is_file() => Kind::File,
        Ok(_) => Kind::Other,
        Err(_) => Kind::Unknown,
    };
    Ok(Entry {
        name: entry.file_name(),
        path: entry.path(),
        kind,
    })
}

/// Lists a directory; see [`collect_listing`] for the flag.
fn list_dir(dir: &Path) -> io::Result<(Vec<Entry>, bool)> {
    Ok(collect_listing(
        fs::read_dir(dir)?.map(|e| e.and_then(read_entry)),
    ))
}

/// What looking for artwork found.
#[derive(Clone)]
enum Art {
    Found(ArtworkId),
    Absent,
    /// The cache could not be written; the track is read again on the next scan.
    Failed,
}

/// A disk failure in the cache costs the track its artwork for now, never the scan.
fn store_art(cache: &ArtworkCache, bytes: &[u8]) -> Art {
    match cache.store(bytes) {
        Ok(Some(id)) => Art::Found(id),
        Ok(None) => Art::Absent,
        Err(e) => {
            tracing::debug!(error = %e, "artwork could not be stored");
            Art::Failed
        }
    }
}

/// The thumbnail for the image file next to the tracks of a directory, if any.
fn folder_art(cache: &ArtworkCache, dir: &Path) -> Art {
    let Some(image) = find_folder_image(dir) else {
        return Art::Absent;
    };
    let Ok(meta) = fs::metadata(&image) else {
        return Art::Absent;
    };
    if meta.len() > MAX_IMAGE_BYTES as u64 {
        tracing::debug!(len = meta.len(), "folder image refused: size");
        return Art::Absent;
    }
    match fs::read(&image) {
        Ok(bytes) => store_art(cache, &bytes),
        Err(_) => Art::Absent,
    }
}

/// The folder image for the directory of `file`, looked up once per directory per scan.
fn shared_dir_art(slot: &mut Option<Art>, cache: &ArtworkCache, file: &Path) -> Art {
    slot.get_or_insert_with(|| file.parent().map_or(Art::Absent, |d| folder_art(cache, d)))
        .clone()
}

/// How often `on_batch` is called when nothing is being written.
const TICK: Duration = Duration::from_millis(250);

/// Whether `on_batch` is due: after [`BATCH_SIZE`] files, or [`TICK`] since the last call.
fn tick_due(files_since: usize, elapsed: Duration) -> bool {
    files_since >= BATCH_SIZE || elapsed >= TICK
}

/// State shared by the folders of one scan.
struct Run<'a> {
    cancel: &'a AtomicBool,
    cache: ArtworkCache,
    summary: ScanSummary,
    progress: ScanProgress,
    pending: Vec<Pending>,
    /// Stored rows whose path to rewrite to the disk spelling.
    respelled: Vec<(i64, Vec<u8>)>,
    /// Rows to delete once the whole walk has finished.
    gone: Vec<i64>,
    files_since_call: usize,
    last_call: Instant,
}

impl Run<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn queued(&self) -> usize {
        self.pending.len() + self.respelled.len()
    }
}

const COLUMNS: &str = "folder_id, path, size, mtime, title, artist, album, album_artist, \
     track_no, disc_no, year, genre, duration_ms, sample_rate, channels, codec, artwork_hash";

impl Library {
    /// Walks every folder and brings the database in step with the files on disk.
    ///
    /// Changed tracks are written in transactions of [`BATCH_SIZE`]. `on_batch` runs with the
    /// library after each of them, and also after every [`BATCH_SIZE`] files looked at or every
    /// 250 ms, whichever comes first, even when nothing changed, so the caller can serve its own
    /// requests during a long walk. It always runs with no transaction open, and may write
    /// (save the queue, store a duration).
    ///
    /// `on_batch` must not add or remove folders or call [`Library::clear_artwork`]: the scan
    /// holds a folder's rows across calls. A scan runs to the end of every folder before it
    /// deletes anything, and a cancelled scan commits what it has and deletes nothing.
    pub fn scan(
        &mut self,
        cancel: &AtomicBool,
        on_batch: &mut dyn FnMut(&mut Library, ScanProgress),
    ) -> Result<ScanSummary, LibraryError> {
        let mut run = Run {
            cancel,
            cache: ArtworkCache::new(self.artwork_dir.clone()),
            summary: ScanSummary::default(),
            progress: ScanProgress::default(),
            pending: Vec::new(),
            respelled: Vec::new(),
            gone: Vec::new(),
            files_since_call: 0,
            last_call: Instant::now(),
        };
        for folder in self.folders()? {
            if run.cancelled() {
                break;
            }
            self.scan_folder(&folder, &mut run, on_batch)?;
        }
        let cancelled = run.cancelled();
        if !cancelled {
            self.delete_gone(&mut run, on_batch)?;
        }
        run.summary.cancelled = cancelled;
        tracing::info!(
            added = run.summary.added,
            updated = run.summary.updated,
            removed = run.summary.removed,
            skipped = run.summary.skipped,
            cancelled,
            "library scan finished"
        );
        Ok(run.summary)
    }

    /// Deletes every thumbnail and clears the stored artwork hashes. The files are read again
    /// by the next scan, which rebuilds the thumbnails.
    pub fn clear_artwork(&mut self) -> Result<(), LibraryError> {
        let tx = self.conn.transaction()?;
        // A size of -1 matches no file, so the next scan reads every track again.
        tx.execute("UPDATE tracks SET artwork_hash = NULL, size = -1", [])?;
        tx.commit()?;
        ArtworkCache::new(self.artwork_dir.clone()).clear()
    }

    fn set_available(&mut self, id: FolderId, available: bool) -> Result<(), LibraryError> {
        self.conn.execute(
            "UPDATE folders SET available = ?1 WHERE id = ?2",
            (i64::from(available), id.0),
        )?;
        Ok(())
    }

    fn existing_rows(&self, folder: FolderId) -> Result<HashMap<Vec<u8>, Existing>, LibraryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, size, mtime FROM tracks WHERE folder_id = ?1")?;
        let rows = stmt.query_map([folder.0], |r| {
            let path: Vec<u8> = r.get(1)?;
            Ok((
                path_key(&decode_path(&path)),
                Existing {
                    id: r.get(0)?,
                    path,
                    size: r.get(2)?,
                    mtime: r.get(3)?,
                },
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn scan_folder(
        &mut self,
        folder: &Folder,
        run: &mut Run<'_>,
        on_batch: &mut dyn FnMut(&mut Library, ScanProgress),
    ) -> Result<(), LibraryError> {
        let existing = self.existing_rows(folder.id)?;
        let mut listings = Listings::default();
        // A work list, so a very deep tree cannot overflow the stack.
        let mut work = vec![folder.path.clone()];
        'walk: while let Some(dir) = work.pop() {
            if run.cancelled() {
                break;
            }
            let is_root = dir == folder.path;
            let (entries, complete) = match list_dir(&dir) {
                Ok(listed) => listed,
                Err(e) => {
                    tracing::debug!(dir = %dir.display(), error = %e, "directory could not be listed");
                    if is_root {
                        run.summary.unavailable.push(folder.id);
                        self.set_available(folder.id, false)?;
                    } else {
                        run.summary.skipped += 1;
                    }
                    continue;
                }
            };
            if is_root {
                self.set_available(folder.id, true)?;
            }
            if !complete {
                run.summary.skipped += 1;
            }

            let mut names = Vec::with_capacity(entries.len());
            let mut dir_art: Option<Art> = None;
            for entry in entries {
                let supported_file = matches!(entry.kind, Kind::File) && is_supported(&entry.name);
                if !matches!(entry.kind, Kind::File) || supported_file {
                    names.push(entry.name.clone());
                }
                match entry.kind {
                    Kind::Dir => work.push(entry.path),
                    Kind::File if supported_file => {
                        if run.cancelled() {
                            break 'walk;
                        }
                        self.scan_file(folder.id, &entry.path, &existing, &mut dir_art, run)?;
                        run.files_since_call += 1;
                        if run.queued() >= BATCH_SIZE
                            || tick_due(run.files_since_call, run.last_call.elapsed())
                        {
                            self.flush(run, on_batch, true)?;
                        }
                    }
                    Kind::File | Kind::Other => {}
                    Kind::Unknown => {
                        if is_supported(&entry.name) {
                            run.summary.skipped += 1;
                        }
                    }
                }
            }
            listings.record(&dir, names.into_iter(), complete);
        }
        self.flush(run, on_batch, false)?;
        if run.cancelled() {
            return Ok(());
        }

        let gone: Vec<i64> = existing
            .values()
            .filter(|row| listings.is_gone(&folder.path, &decode_path(&row.path)))
            .map(|row| row.id)
            .collect();
        if !existing.is_empty() && gone.len() == existing.len() {
            // Every stored track of the folder looks gone: an unmounted drive, a swapped
            // stick or a mount point that lists empty looks just like this. Keep them all.
            tracing::debug!("a scan would remove every track of a folder; keeping them");
            if !run.summary.unavailable.contains(&folder.id) {
                run.summary.unavailable.push(folder.id);
            }
            self.set_available(folder.id, false)?;
        } else {
            run.gone.extend(gone);
        }
        Ok(())
    }

    /// Deletes the rows found gone by a scan that ran to the end.
    fn delete_gone(
        &mut self,
        run: &mut Run<'_>,
        on_batch: &mut dyn FnMut(&mut Library, ScanProgress),
    ) -> Result<(), LibraryError> {
        if run.gone.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        {
            let mut del = tx.prepare("DELETE FROM tracks WHERE id = ?1")?;
            for id in &run.gone {
                del.execute([id])?;
            }
        }
        tx.commit()?;
        run.summary.removed += run.gone.len() as u64;
        run.gone.clear();
        on_batch(self, run.progress);
        Ok(())
    }

    /// Looks at one audio file and queues its row when it is new or changed.
    fn scan_file(
        &mut self,
        folder: FolderId,
        path: &Path,
        existing: &HashMap<Vec<u8>, Existing>,
        dir_art: &mut Option<Art>,
        run: &mut Run<'_>,
    ) -> Result<(), LibraryError> {
        run.progress.seen += 1;
        let Ok(meta) = fs::metadata(path) else {
            tracing::debug!(path = %path.display(), "file could not be inspected");
            run.summary.skipped += 1;
            return Ok(());
        };
        let size = i64::try_from(meta.len()).unwrap_or(i64::MAX);
        let mtime = meta.modified().ok().and_then(mtime_key);
        let disk_path = encode_path(path);
        let stored = existing.get(&path_key(path));
        if let (Some(row), Some(m)) = (stored, mtime)
            && row.size == size
            && row.mtime == m
        {
            if row.path != disk_path {
                run.respelled.push((row.id, disk_path));
            }
            return Ok(());
        }
        let mut tags = match read_tags(path) {
            Ok(tags) => tags,
            Err(e) => {
                // The stored row, if any, keeps its old size and mtime, so the next scan retries.
                tracing::debug!(path = %path.display(), error = %e, "file could not be read");
                run.summary.skipped += 1;
                return Ok(());
            }
        };
        let art = match tags.picture.take() {
            Some(bytes) => match store_art(&run.cache, &bytes) {
                Art::Absent => shared_dir_art(dir_art, &run.cache, path),
                other => other,
            },
            None => shared_dir_art(dir_art, &run.cache, path),
        };
        let (artwork, size) = match art {
            Art::Found(id) => (Some(id), size),
            Art::Absent => (None, size),
            Art::Failed => (None, -1),
        };
        if stored.is_some() {
            run.summary.updated += 1;
        } else {
            run.summary.added += 1;
        }
        run.progress.changed += 1;
        run.pending.push(Pending {
            folder,
            id: stored.map(|r| r.id),
            path: disk_path,
            size,
            mtime: mtime.unwrap_or(0),
            tags,
            artwork,
        });
        Ok(())
    }

    /// Writes the queued rows in one transaction, then calls `on_batch` if anything was written
    /// or `always` is set.
    fn flush(
        &mut self,
        run: &mut Run<'_>,
        on_batch: &mut dyn FnMut(&mut Library, ScanProgress),
        always: bool,
    ) -> Result<(), LibraryError> {
        let wrote = run.queued() > 0;
        if wrote {
            self.write_pending(run)?;
        }
        if wrote || always {
            run.files_since_call = 0;
            run.last_call = Instant::now();
            on_batch(self, run.progress);
        }
        Ok(())
    }

    fn write_pending(&mut self, run: &mut Run<'_>) -> Result<(), LibraryError> {
        let now = now_secs();
        let tx = self.conn.transaction()?;
        {
            // A size of -1 means the artwork could not be stored: keep the old hash, and a
            // duration of 0 means the tags had none: keep the stored one.
            let mut insert = tx.prepare(&format!(
                "INSERT INTO tracks ({COLUMNS}, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                    ?17, ?18)
                 ON CONFLICT(path) DO UPDATE SET
                    folder_id = excluded.folder_id, size = excluded.size,
                    mtime = excluded.mtime, title = excluded.title, artist = excluded.artist,
                    album = excluded.album, album_artist = excluded.album_artist,
                    track_no = excluded.track_no, disc_no = excluded.disc_no,
                    year = excluded.year, genre = excluded.genre,
                    duration_ms = CASE WHEN excluded.duration_ms = 0
                        THEN tracks.duration_ms ELSE excluded.duration_ms END,
                    sample_rate = excluded.sample_rate,
                    channels = excluded.channels, codec = excluded.codec,
                    artwork_hash = CASE WHEN excluded.size = -1
                        THEN tracks.artwork_hash ELSE excluded.artwork_hash END"
            ))?;
            let mut update = tx.prepare(
                "UPDATE OR IGNORE tracks SET
                    folder_id = ?1, path = ?2, size = ?3, mtime = ?4, title = ?5, artist = ?6,
                    album = ?7, album_artist = ?8, track_no = ?9, disc_no = ?10, year = ?11,
                    genre = ?12,
                    duration_ms = CASE WHEN ?13 = 0 THEN duration_ms ELSE ?13 END,
                    sample_rate = ?14, channels = ?15, codec = ?16,
                    artwork_hash = CASE WHEN ?3 = -1 THEN artwork_hash ELSE ?17 END
                 WHERE id = ?18",
            )?;
            for p in &run.pending {
                let t = &p.tags;
                let duration = i64::try_from(t.duration_ms).unwrap_or(i64::MAX);
                let artwork = p.artwork.as_ref().map(|a| &*a.0);
                match p.id {
                    Some(id) => update.execute(params![
                        p.folder.0,
                        p.path,
                        p.size,
                        p.mtime,
                        t.title,
                        t.artist.as_deref(),
                        t.album.as_deref(),
                        t.album_artist.as_deref(),
                        t.track_no,
                        t.disc_no,
                        t.year,
                        t.genre.as_deref(),
                        duration,
                        t.sample_rate,
                        t.channels,
                        &*t.codec,
                        artwork,
                        id,
                    ])?,
                    None => insert.execute(params![
                        p.folder.0,
                        p.path,
                        p.size,
                        p.mtime,
                        t.title,
                        t.artist.as_deref(),
                        t.album.as_deref(),
                        t.album_artist.as_deref(),
                        t.track_no,
                        t.disc_no,
                        t.year,
                        t.genre.as_deref(),
                        duration,
                        t.sample_rate,
                        t.channels,
                        &*t.codec,
                        artwork,
                        now,
                    ])?,
                };
            }
            let mut respell = tx.prepare("UPDATE OR IGNORE tracks SET path = ?1 WHERE id = ?2")?;
            for (id, path) in &run.respelled {
                respell.execute(params![path, id])?;
            }
        }
        tx.commit()?;
        run.pending.clear();
        run.respelled.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listings(dirs: &[(&str, &[&str])]) -> Listings {
        let mut l = Listings::default();
        for (d, names) in dirs {
            l.record(Path::new(d), names.iter().map(OsString::from), true);
        }
        l
    }

    #[test]
    fn file_missing_from_a_listed_directory_is_gone() {
        let l = listings(&[("/r", &["a"]), ("/r/a", &["x.mp3"])]);
        assert!(l.is_gone(Path::new("/r"), Path::new("/r/a/y.mp3")));
        assert!(!l.is_gone(Path::new("/r"), Path::new("/r/a/x.mp3")));
    }

    #[test]
    fn subdirectory_missing_from_a_listed_parent_takes_its_tracks() {
        let l = listings(&[("/r", &["a"]), ("/r/a", &[])]);
        assert!(l.is_gone(Path::new("/r"), Path::new("/r/b/deep/x.mp3")));
    }

    #[test]
    fn directory_that_did_not_list_keeps_everything_under_it() {
        // `/r/a` exists in the root listing but has no listing of its own: it failed.
        let l = listings(&[("/r", &["a"])]);
        assert!(!l.is_gone(Path::new("/r"), Path::new("/r/a/x.mp3")));
        assert!(!l.is_gone(Path::new("/r"), Path::new("/r/a/deep/er/x.mp3")));
    }

    #[test]
    fn unreachable_root_keeps_everything() {
        let l = listings(&[]);
        assert!(!l.is_gone(Path::new("/r"), Path::new("/r/x.mp3")));
    }

    #[test]
    fn supported_extensions_ignore_case() {
        assert!(is_supported(std::ffi::OsStr::new("A.MP3")));
        assert!(is_supported(std::ffi::OsStr::new("b.Flac")));
        assert!(!is_supported(std::ffi::OsStr::new("c.txt")));
        assert!(!is_supported(std::ffi::OsStr::new("mp3")));
    }

    #[test]
    fn an_unreadable_entry_makes_the_listing_incomplete() {
        let fake = vec![
            Ok("a.mp3"),
            Ok("b.mp3"),
            Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            Ok("c.mp3"),
        ];
        let (entries, complete) = collect_listing(fake.into_iter());
        assert!(!complete);
        assert_eq!(entries, vec!["a.mp3", "b.mp3", "c.mp3"]);

        let (entries, complete) = collect_listing(vec![Ok(1), Ok(2)].into_iter());
        assert!(complete);
        assert_eq!(entries, vec![1, 2]);
    }

    #[test]
    fn an_incomplete_listing_is_not_recorded_so_nothing_under_it_is_deleted() {
        let mut l = Listings::default();
        l.record(Path::new("/r"), ["a"].into_iter().map(OsString::from), true);
        // `/r/a` listed part-way: `x.mp3` is missing from what was read.
        l.record(
            Path::new("/r/a"),
            ["y.mp3"].into_iter().map(OsString::from),
            false,
        );
        assert!(!l.is_gone(Path::new("/r"), Path::new("/r/a/x.mp3")));
        // A complete one would have deleted it.
        l.record(
            Path::new("/r/a"),
            ["y.mp3"].into_iter().map(OsString::from),
            true,
        );
        assert!(l.is_gone(Path::new("/r"), Path::new("/r/a/x.mp3")));
    }

    #[cfg(windows)]
    #[test]
    fn paths_compare_without_regard_to_case_on_windows() {
        let l = listings(&[(r"c:\music", &["rock"]), (r"c:\music\rock", &["a.mp3"])]);
        let root = Path::new(r"C:\Music");
        assert!(!l.is_gone(root, Path::new(r"C:\MUSIC\ROCK\A.mp3")));
        assert!(l.is_gone(root, Path::new(r"C:\MUSIC\ROCK\B.mp3")));
        assert_eq!(
            path_key(Path::new(r"C:\Music\Rock")),
            path_key(Path::new(r"c:\music\ROCK"))
        );
    }

    #[test]
    fn on_batch_is_due_after_enough_files_or_enough_time() {
        assert!(!tick_due(1, Duration::from_millis(10)));
        assert!(tick_due(BATCH_SIZE, Duration::ZERO));
        assert!(tick_due(1, TICK));
    }

    #[test]
    #[ignore = "performance check"]
    fn deletion_check_over_50k_rows_is_fast() {
        let root = Path::new("/lib");
        let mut l = Listings::default();
        let artists: Vec<String> = (0..500).map(|i| format!("Artist {i}")).collect();
        l.record(root, artists.iter().map(OsString::from), true);
        let mut rows = Vec::new();
        for (i, a) in artists.iter().enumerate() {
            let albums: Vec<String> = (0..10).map(|j| format!("Album {j}")).collect();
            l.record(&root.join(a), albums.iter().map(OsString::from), true);
            for al in &albums {
                let dir = root.join(a).join(al);
                let names: Vec<String> = (0..10).map(|k| format!("{i}-{k}.flac")).collect();
                l.record(&dir, names.iter().map(OsString::from), true);
                rows.extend(names.iter().map(|n| dir.join(n)));
            }
        }
        assert_eq!(rows.len(), 50_000);
        let start = Instant::now();
        let gone = rows.iter().filter(|r| l.is_gone(root, r)).count();
        let took = start.elapsed();
        println!("is_gone over 50000 rows took {took:?}");
        assert_eq!(gone, 0);
        assert!(took < Duration::from_secs(1), "{took:?}");
    }
}
