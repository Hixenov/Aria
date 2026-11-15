//! Schema migrations, indexed by `PRAGMA user_version`.

use rusqlite::Connection;

use crate::error::LibraryError;

/// Migration `i` upgrades a database from `user_version == i` to `i + 1`.
const MIGRATIONS: &[&str] = &[
    "
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
",
    "
CREATE TABLE favorites (
    path     BLOB PRIMARY KEY,
    added_at INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE playlists (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE playlist_items (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    path        BLOB NOT NULL,
    PRIMARY KEY (playlist_id, position)
);
CREATE TABLE plays (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    path      BLOB NOT NULL,
    played_at INTEGER NOT NULL
);
CREATE INDEX plays_played_at ON plays(played_at);
",
    "
ALTER TABLE tracks ADD COLUMN bits_per_sample INTEGER NOT NULL DEFAULT 0;
-- The scan keeps a file whose size and mtime match its row. No file has an mtime of 0, so this
-- makes the next scan read every file again: it fills the depth and corrects the codec of
-- .m4a files. Favorites, playlists and plays are keyed by path and are not touched.
UPDATE tracks SET mtime = 0;
",
];

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
