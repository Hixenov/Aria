//! The background scanner: walks the library folders and keeps the database in step with disk.
//!
//! A scan never deletes a row unless it is sure the file is gone. A row is deleted only when
//! every directory on the way to it listed successfully and the entry was missing from its
//! parent's listing. An unreachable root, an unreadable subdirectory and an unreadable file all
//! keep their rows.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// What the scan knows about a stored row.
struct Existing {
    id: i64,
    size: i64,
    mtime: i64,
}

/// A row waiting to be written.
struct Pending {
    folder: FolderId,
    path: Vec<u8>,
    size: i64,
    mtime: i64,
    tags: TrackTags,
    artwork: Option<ArtworkId>,
}

/// The directories that listed successfully, with the names found in each.
///
/// Regular files without a supported extension are left out, so a file renamed to `.txt`
/// counts as gone. Everything else that was listed (directories, links, entries whose type
/// could not be read) is a name, so its rows are kept.
#[derive(Default)]
struct Listings(HashMap<PathBuf, HashSet<OsString>>);

impl Listings {
    /// Whether the row at `file`, in the folder rooted at `root`, may be deleted.
    ///
    /// Walking down from the root: a directory that did not list successfully keeps the row, and
    /// so does an unreachable root. A name missing from a directory that did list means the
    /// file, or a directory above it, is gone.
    fn is_gone(&self, root: &Path, file: &Path) -> bool {
        let Ok(rel) = file.strip_prefix(root) else {
            return false;
        };
        let mut dir = root.to_path_buf();
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

/// Lists a directory. The flag is false when an entry could not be read, so the listing is
/// incomplete and must not be used to delete anything.
fn list_dir(dir: &Path) -> io::Result<(Vec<Entry>, bool)> {
    let mut out = Vec::new();
    let mut complete = true;
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else {
            complete = false;
            continue;
        };
        let kind = match entry.file_type() {
            Ok(t) if t.is_dir() => Kind::Dir,
            Ok(t) if t.is_file() => Kind::File,
            Ok(_) => Kind::Other,
            Err(_) => Kind::Unknown,
        };
        out.push(Entry {
            name: entry.file_name(),
            path: entry.path(),
            kind,
        });
    }
    Ok((out, complete))
}

/// The thumbnail for the image file next to the tracks of a directory, if any.
fn folder_art(cache: &ArtworkCache, dir: &Path) -> Option<ArtworkId> {
    let image = find_folder_image(dir)?;
    let len = fs::metadata(&image).ok()?.len();
    if len > MAX_IMAGE_BYTES as u64 {
        tracing::debug!(len, "folder image refused: size");
        return None;
    }
    let bytes = fs::read(&image).ok()?;
    store_art(cache, &bytes)
}

/// The folder image for the directory of `file`, looked up once per directory per scan.
fn shared_dir_art(
    slot: &mut Option<Option<ArtworkId>>,
    cache: &ArtworkCache,
    file: &Path,
) -> Option<ArtworkId> {
    slot.get_or_insert_with(|| file.parent().and_then(|dir| folder_art(cache, dir)))
        .clone()
}

/// A disk failure in the cache costs the track its artwork, never the scan.
fn store_art(cache: &ArtworkCache, bytes: &[u8]) -> Option<ArtworkId> {
    cache.store(bytes).unwrap_or_else(|e| {
        tracing::debug!(error = %e, "artwork could not be stored");
        None
    })
}

/// State shared by the folders of one scan.
struct Run<'a> {
    cancel: &'a AtomicBool,
    cache: ArtworkCache,
    summary: ScanSummary,
    progress: ScanProgress,
    pending: Vec<Pending>,
}

impl Run<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

impl Library {
    /// Walks every folder and brings the database in step with the files on disk.
    ///
    /// Changed tracks are written in transactions of [`BATCH_SIZE`]; after each one `on_batch`
    /// runs with the library, so the caller can do its own writes without a second connection.
    /// A cancelled scan commits what it has and deletes nothing.
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
        };
        for folder in self.folders()? {
            if run.cancelled() {
                break;
            }
            self.scan_folder(&folder, &mut run, on_batch)?;
        }
        run.summary.cancelled = run.cancelled();
        tracing::info!(
            added = run.summary.added,
            updated = run.summary.updated,
            removed = run.summary.removed,
            skipped = run.summary.skipped,
            cancelled = run.summary.cancelled,
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
            Ok((
                r.get::<_, Vec<u8>>(1)?,
                Existing {
                    id: r.get(0)?,
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

            let mut names = HashSet::with_capacity(entries.len());
            let mut dir_art: Option<Option<ArtworkId>> = None;
            for entry in entries {
                let supported_file = matches!(entry.kind, Kind::File) && is_supported(&entry.name);
                if !matches!(entry.kind, Kind::File) || supported_file {
                    names.insert(entry.name.clone());
                }
                match entry.kind {
                    Kind::Dir => work.push(entry.path),
                    Kind::File if supported_file => {
                        if run.cancelled() {
                            break 'walk;
                        }
                        self.scan_file(folder.id, &entry.path, &existing, &mut dir_art, run)?;
                        if run.pending.len() >= BATCH_SIZE {
                            self.flush(run, on_batch)?;
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
            if complete {
                listings.0.insert(dir, names);
            }
        }
        self.flush(run, on_batch)?;
        if run.cancelled() {
            return Ok(());
        }

        let gone: Vec<i64> = existing
            .iter()
            .filter(|(path, _)| listings.is_gone(&folder.path, &decode_path(path)))
            .map(|(_, row)| row.id)
            .collect();
        if !gone.is_empty() {
            let tx = self.conn.transaction()?;
            {
                let mut del = tx.prepare("DELETE FROM tracks WHERE id = ?1")?;
                for id in &gone {
                    del.execute([id])?;
                }
            }
            tx.commit()?;
            run.summary.removed += gone.len() as u64;
            on_batch(self, run.progress);
        }
        Ok(())
    }

    /// Looks at one audio file and queues its row when it is new or changed.
    fn scan_file(
        &mut self,
        folder: FolderId,
        path: &Path,
        existing: &HashMap<Vec<u8>, Existing>,
        dir_art: &mut Option<Option<ArtworkId>>,
        run: &mut Run<'_>,
    ) -> Result<(), LibraryError> {
        run.progress.seen += 1;
        let key = encode_path(path);
        let Ok(meta) = fs::metadata(path) else {
            tracing::debug!(path = %path.display(), "file could not be inspected");
            run.summary.skipped += 1;
            return Ok(());
        };
        let size = i64::try_from(meta.len()).unwrap_or(i64::MAX);
        let mtime = meta.modified().ok().and_then(mtime_key);
        let stored = existing.get(&key);
        if let (Some(row), Some(m)) = (stored, mtime)
            && row.size == size
            && row.mtime == m
        {
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
        let artwork = match tags.picture.take() {
            Some(bytes) => match run.cache.store(&bytes) {
                Ok(Some(id)) => Some(id),
                Ok(None) => shared_dir_art(dir_art, &run.cache, path),
                Err(e) => {
                    tracing::debug!(error = %e, "artwork could not be stored");
                    None
                }
            },
            None => shared_dir_art(dir_art, &run.cache, path),
        };
        if stored.is_some() {
            run.summary.updated += 1;
        } else {
            run.summary.added += 1;
        }
        run.progress.changed += 1;
        run.pending.push(Pending {
            folder,
            path: key,
            size,
            mtime: mtime.unwrap_or(0),
            tags,
            artwork,
        });
        Ok(())
    }

    /// Writes the queued rows in one transaction, then calls `on_batch`.
    fn flush(
        &mut self,
        run: &mut Run<'_>,
        on_batch: &mut dyn FnMut(&mut Library, ScanProgress),
    ) -> Result<(), LibraryError> {
        if run.pending.is_empty() {
            return Ok(());
        }
        let now = now_secs();
        let tx = self.conn.transaction()?;
        {
            let mut up = tx.prepare(
                "INSERT INTO tracks (folder_id, path, size, mtime, title, artist, album,
                    album_artist, track_no, disc_no, year, genre, duration_ms, sample_rate,
                    channels, codec, artwork_hash, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                    ?17, ?18)
                 ON CONFLICT(path) DO UPDATE SET
                    folder_id = excluded.folder_id, size = excluded.size,
                    mtime = excluded.mtime, title = excluded.title, artist = excluded.artist,
                    album = excluded.album, album_artist = excluded.album_artist,
                    track_no = excluded.track_no, disc_no = excluded.disc_no,
                    year = excluded.year, genre = excluded.genre,
                    duration_ms = excluded.duration_ms, sample_rate = excluded.sample_rate,
                    channels = excluded.channels, codec = excluded.codec,
                    artwork_hash = excluded.artwork_hash",
            )?;
            for p in &run.pending {
                let t = &p.tags;
                up.execute(params![
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
                    i64::try_from(t.duration_ms).unwrap_or(i64::MAX),
                    t.sample_rate,
                    t.channels,
                    &*t.codec,
                    p.artwork.as_ref().map(|a| &*a.0),
                    now,
                ])?;
            }
        }
        tx.commit()?;
        run.pending.clear();
        on_batch(self, run.progress);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listings(dirs: &[(&str, &[&str])]) -> Listings {
        Listings(
            dirs.iter()
                .map(|(d, names)| {
                    (
                        PathBuf::from(d),
                        names.iter().map(OsString::from).collect::<HashSet<_>>(),
                    )
                })
                .collect(),
        )
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
}
