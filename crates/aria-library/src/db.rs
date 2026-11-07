//! The SQLite-backed library: opening, recovery, folders and the saved queue.
//!
//! A `Library` owns one connection and is meant to live on a single dedicated thread, so it uses
//! no locking of its own.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension};

use crate::error::LibraryError;
use crate::migrations::{SCHEMA_VERSION, migrate};
use crate::model::{Folder, FolderId, QueueRecord, TrackId};
use crate::paths::{decode_path, encode_path};

const LIST_CONTEXT: i64 = 0;
const LIST_USER: i64 = 1;

/// The result of opening the library.
pub struct Opened {
    pub library: Library,
    /// True when the old database was corrupt and was moved to `library.db.bak`.
    pub recovered: bool,
}

/// The library database. Not shared between threads; the backend's library thread owns it.
pub struct Library {
    conn: Connection,
    artwork_dir: PathBuf,
}

/// What the read-only probe found.
enum Probe {
    Healthy,
    Corrupt,
}

fn is_corrupt(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if matches!(f.code, ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt)
    )
}

/// Reads `user_version` and runs `quick_check` on a read-only connection, so a database from a
/// newer Aria or a damaged one is judged before anything writes to the file.
fn probe(db: &Path) -> Result<Probe, LibraryError> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let version: Result<u32, _> = conn.pragma_query_value(None, "user_version", |r| r.get(0));
    let version = match version {
        Ok(v) => v,
        Err(e) if is_corrupt(&e) => return Ok(Probe::Corrupt),
        Err(e) => return Err(e.into()),
    };
    if version > SCHEMA_VERSION {
        return Err(LibraryError::NewerSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    let check: Result<String, _> = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0));
    match check {
        Ok(s) if s == "ok" => Ok(Probe::Healthy),
        Ok(_) => Ok(Probe::Corrupt),
        Err(e) if is_corrupt(&e) => Ok(Probe::Corrupt),
        Err(e) => Err(e.into()),
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Renames `from` to `to`, replacing `to`. A missing `from` is not an error.
fn rename_if_present(from: &Path, to: &Path) -> Result<(), LibraryError> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Moves a corrupt database aside, with its `-wal` and `-shm` files, to `.bak`, `.bak-wal` and
/// `.bak-shm`. The user's data is moved, never deleted. The database goes first, so if a side
/// file cannot be moved the next open finds no database and clears the strays itself.
fn move_aside(db: &Path) -> Result<(), LibraryError> {
    std::fs::rename(db, with_suffix(db, ".bak"))?;
    for suffix in ["-wal", "-shm"] {
        rename_if_present(
            &with_suffix(db, suffix),
            &with_suffix(db, &format!(".bak{suffix}")),
        )?;
    }
    Ok(())
}

/// Removes `-wal` and `-shm` files left beside a database that does not exist, so a stale WAL
/// can never meet a new database.
fn remove_stray_side_files(db: &Path) -> Result<(), LibraryError> {
    for suffix in ["-wal", "-shm"] {
        match std::fs::remove_file(with_suffix(db, suffix)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// A path reduced to comparable components: made absolute, `.` and `..` resolved lexically.
struct Normalized {
    path: PathBuf,
    key: Vec<Vec<u8>>,
}

fn key_component(c: &OsStr) -> Vec<u8> {
    if cfg!(windows) {
        c.to_string_lossy().to_lowercase().into_bytes()
    } else {
        encode_path(Path::new(c))
    }
}

fn normalize(path: &Path) -> Result<Normalized, LibraryError> {
    let abs = std::path::absolute(path)?;
    let mut parts: Vec<&OsStr> = Vec::new();
    for c in abs.components() {
        match c {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                parts.push(c.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                // Never pop the prefix or the root.
                if parts
                    .last()
                    .is_some_and(|l| Path::new(l).components().next().is_some_and(is_normal))
                {
                    parts.pop();
                }
            }
        }
    }
    let rebuilt: PathBuf = parts.iter().collect();
    let key = parts.iter().map(|p| key_component(p)).collect();
    Ok(Normalized { path: rebuilt, key })
}

fn is_normal(c: Component<'_>) -> bool {
    matches!(c, Component::Normal(_))
}

impl Library {
    /// Opens or creates the library database, migrating it and recovering from corruption.
    ///
    /// A database from a newer Aria gives `NewerSchema` and is left untouched. A corrupt one is
    /// moved to `library.db.bak` (replacing an older backup) and `recovered` is set.
    pub fn open(db: &Path, artwork_dir: &Path) -> Result<Opened, LibraryError> {
        if let Some(parent) = db.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut recovered = false;
        if db.exists() {
            match probe(db)? {
                Probe::Healthy => {}
                Probe::Corrupt => {
                    move_aside(db)?;
                    recovered = true;
                    tracing::warn!("library database was corrupt and has been moved aside");
                }
            }
        } else {
            remove_stray_side_files(db)?;
        }

        std::fs::create_dir_all(artwork_dir)?;
        let mut conn = Connection::open(db)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut conn)?;
        tracing::debug!(db = %db.display(), "library database opened");
        Ok(Opened {
            library: Self {
                conn,
                artwork_dir: artwork_dir.to_path_buf(),
            },
            recovered,
        })
    }

    /// Directory holding artwork thumbnails.
    pub fn artwork_dir(&self) -> &Path {
        &self.artwork_dir
    }

    /// All folders, oldest first.
    pub fn folders(&self) -> Result<Vec<Folder>, LibraryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, available FROM folders ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            let path: Vec<u8> = r.get(1)?;
            Ok(Folder {
                id: FolderId(r.get(0)?),
                path: decode_path(&path),
                available: r.get::<_, i64>(2)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Adds a folder. A folder inside an existing one, or the same folder, is refused. A parent
    /// of existing folders absorbs them: their tracks move to the new folder and their rows are
    /// deleted, in one transaction.
    pub fn add_folder(&mut self, path: &Path) -> Result<FolderId, LibraryError> {
        let new = normalize(path)?;
        let tx = self.conn.transaction()?;

        let existing: Vec<(i64, Vec<u8>)> = {
            let mut stmt = tx.prepare("SELECT id, path FROM folders")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut absorbed = Vec::new();
        for (id, bytes) in existing {
            let old = normalize(&decode_path(&bytes))?;
            if old.key == new.key {
                return Err(LibraryError::FolderAlreadyAdded);
            }
            if new.key.starts_with(&old.key) {
                return Err(LibraryError::FolderInsideExisting);
            }
            if old.key.starts_with(&new.key) {
                absorbed.push(id);
            }
        }

        tx.execute(
            "INSERT INTO folders (path, added_at, available) VALUES (?1, ?2, 1)",
            (encode_path(&new.path), now_secs()),
        )?;
        let new_id = tx.last_insert_rowid();
        for id in absorbed {
            tx.execute(
                "UPDATE tracks SET folder_id = ?1 WHERE folder_id = ?2",
                (new_id, id),
            )?;
            tx.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(FolderId(new_id))
    }

    /// Removes a folder and its tracks from the database. Files on disk are not touched.
    pub fn remove_folder(&mut self, id: FolderId) -> Result<(), LibraryError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM tracks WHERE folder_id = ?1", [id.0])?;
        tx.execute("DELETE FROM folders WHERE id = ?1", [id.0])?;
        tx.commit()?;
        Ok(())
    }

    /// Replaces the saved queue.
    pub fn save_queue(&mut self, q: &QueueRecord) -> Result<(), LibraryError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM queue_items", [])?;
        {
            let mut ins = tx.prepare(
                "INSERT INTO queue_items (list, position, track_id) VALUES (?1, ?2, ?3)",
            )?;
            for (list, items) in [(LIST_CONTEXT, &q.context), (LIST_USER, &q.user)] {
                for (pos, t) in items.iter().enumerate() {
                    ins.execute((list, i64::try_from(pos).unwrap_or(i64::MAX), t.0))?;
                }
            }
        }
        tx.execute(
            "INSERT INTO queue_state (id, current, position_ms, shuffle_seed, context_index)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                current = excluded.current,
                position_ms = excluded.position_ms,
                shuffle_seed = excluded.shuffle_seed,
                context_index = excluded.context_index",
            (
                q.current.map(|t| t.0),
                i64::try_from(q.position_ms).unwrap_or(i64::MAX),
                // The seed is stored as the same 64 bits, read back the same way.
                q.shuffle_seed as i64,
                q.context_index
                    .map(|i| i64::try_from(i).unwrap_or(i64::MAX)),
            ),
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The saved queue, or `None` when nothing was saved.
    pub fn load_queue(&self) -> Result<Option<QueueRecord>, LibraryError> {
        let state = self
            .conn
            .query_row(
                "SELECT current, position_ms, shuffle_seed, context_index
                 FROM queue_state WHERE id = 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((current, position_ms, seed, context_index)) = state else {
            return Ok(None);
        };

        let mut stmt = self
            .conn
            .prepare("SELECT track_id FROM queue_items WHERE list = ?1 ORDER BY position")?;
        let mut read = |list: i64| -> Result<Vec<TrackId>, rusqlite::Error> {
            stmt.query_map([list], |r| Ok(TrackId(r.get(0)?)))?
                .collect()
        };
        let context = read(LIST_CONTEXT)?;
        let user = read(LIST_USER)?;

        Ok(Some(QueueRecord {
            context,
            user,
            current: current.map(TrackId),
            position_ms: u64::try_from(position_ms).unwrap_or(0),
            shuffle_seed: seed as u64,
            context_index: context_index.and_then(|i| usize::try_from(i).ok()),
        }))
    }

    /// Stores a duration learned while the track played.
    pub fn set_duration(&mut self, id: TrackId, ms: u64) -> Result<(), LibraryError> {
        self.conn.execute(
            "UPDATE tracks SET duration_ms = ?1 WHERE id = ?2",
            (i64::try_from(ms).unwrap_or(i64::MAX), id.0),
        )?;
        Ok(())
    }

    /// Number of tracks in the library.
    pub fn track_count(&self) -> Result<u64, LibraryError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}
