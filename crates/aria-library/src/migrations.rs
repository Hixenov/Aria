//! Schema migrations, indexed by `PRAGMA user_version`.

use rusqlite::Connection;

use crate::error::LibraryError;

/// Migration `i` upgrades a database from `user_version == i` to `i + 1`.
const MIGRATIONS: &[&str] = &["
CREATE TABLE folders (
    id        INTEGER PRIMARY KEY,
    path      BLOB NOT NULL UNIQUE,
    added_at  INTEGER NOT NULL,
    available INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE tracks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    folder_id    INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    path         BLOB NOT NULL UNIQUE,
    size         INTEGER NOT NULL,
    mtime        INTEGER NOT NULL,
    title        TEXT NOT NULL,
    artist       TEXT,
    album        TEXT,
    album_artist TEXT,
    track_no     INTEGER,
    disc_no      INTEGER,
    year         INTEGER,
    genre        TEXT,
    duration_ms  INTEGER NOT NULL DEFAULT 0,
    sample_rate  INTEGER NOT NULL DEFAULT 0,
    channels     INTEGER NOT NULL DEFAULT 0,
    codec        TEXT NOT NULL DEFAULT '',
    artwork_hash TEXT,
    added_at     INTEGER NOT NULL
);
CREATE INDEX tracks_folder ON tracks(folder_id);
-- list: 0 = context (playback order), 1 = user queue, 2 = context before shuffle.
CREATE TABLE queue_items (
    list     INTEGER NOT NULL,
    position INTEGER NOT NULL,
    track_id INTEGER NOT NULL,
    PRIMARY KEY (list, position)
);
CREATE TABLE queue_state (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    current       INTEGER,
    position_ms   INTEGER NOT NULL,
    shuffle_seed  INTEGER NOT NULL,
    context_index INTEGER
);
"];

/// Highest schema version this build understands.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// Applies every migration newer than the database's `user_version`.
pub(crate) fn migrate(conn: &mut Connection) -> Result<(), LibraryError> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for version in current..SCHEMA_VERSION {
        let Some(sql) = MIGRATIONS.get(version as usize) else {
            break;
        };
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
    }
    Ok(())
}
