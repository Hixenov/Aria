//! Favorites, playlists and playback history, and the `Collections` view of them.
//!
//! User data names a track by its path, not its row id, so it outlives a folder that is removed
//! and added again (see the design, section 3.1).

use std::collections::{HashMap, HashSet};

use rusqlite::OptionalExtension;

use crate::db::Library;
use crate::error::LibraryError;
use crate::model::TrackId;

/// How many plays the history keeps.
pub const PLAYS_KEPT: u64 = 10_000;
/// How many tracks `Collections::recent` holds at most.
pub const RECENT_LIMIT: usize = 20;

/// The longest playlist name, in characters.
pub const PLAYLIST_NAME_MAX: usize = 100;

/// The name as it would be stored, or `None` if it breaks the rules: trimmed, 1 to
/// [`PLAYLIST_NAME_MAX`] characters.
pub fn playlist_name(name: &str) -> Option<&str> {
    let name = name.trim();
    let chars = name.chars().count();
    (1..=PLAYLIST_NAME_MAX).contains(&chars).then_some(name)
}

/// The id of a playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlaylistId(pub i64);

/// One available entry of a playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaylistItem {
    /// The stored position, which commands use.
    pub position: u32,
    pub track: TrackId,
}

/// A playlist as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    /// Available entries in order.
    pub items: Vec<PlaylistItem>,
    /// How many entries are hidden because their file is not in the library.
    pub unavailable: usize,
}

/// Favorites, playlists and recent plays, read in one consistent state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Collections {
    /// Available favorites, newest first.
    pub favorites: Vec<TrackId>,
    /// Sorted by name without regard to case, then by id.
    pub playlists: Vec<Playlist>,
    /// Available tracks played most recently, newest first, each once, at most
    /// [`RECENT_LIMIT`].
    pub recent: Vec<TrackId>,
    favorite_set: HashSet<TrackId>,
}

impl Collections {
    /// Builds the view and its favorite index.
    pub fn new(favorites: Vec<TrackId>, playlists: Vec<Playlist>, recent: Vec<TrackId>) -> Self {
        let favorite_set = favorites.iter().copied().collect();
        Self {
            favorites,
            playlists,
            recent,
            favorite_set,
        }
    }

    /// Whether the track is a favorite, without a scan of the list.
    pub fn is_favorite(&self, id: TrackId) -> bool {
        self.favorite_set.contains(&id)
    }

    /// The playlist with this id.
    pub fn playlist(&self, id: PlaylistId) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }
}

impl Library {
    /// Marks or unmarks a track as a favorite. Returns whether anything changed; an unknown
    /// track changes nothing.
    pub fn set_favorite(
        &mut self,
        track: TrackId,
        favorite: bool,
        now: i64,
    ) -> Result<bool, LibraryError> {
        let tx = self.conn.transaction()?;
        let Some(path) = track_path(&tx, track)? else {
            return Ok(false);
        };
        let changed = if favorite {
            tx.execute(
                "INSERT OR IGNORE INTO favorites (path, added_at) VALUES (?1, ?2)",
                (&path, now),
            )?
        } else {
            tx.execute("DELETE FROM favorites WHERE path = ?1", [&path])?
        };
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Records one play and trims the history to the newest [`PLAYS_KEPT`] plays. Returns
    /// whether a play was recorded; an unknown track records nothing.
    pub fn record_play(&mut self, track: TrackId, now: i64) -> Result<bool, LibraryError> {
        let tx = self.conn.transaction()?;
        let Some(path) = track_path(&tx, track)? else {
            return Ok(false);
        };
        tx.execute(
            "INSERT INTO plays (path, played_at) VALUES (?1, ?2)",
            (&path, now),
        )?;
        tx.execute(
            "DELETE FROM plays WHERE id <= (SELECT id FROM plays ORDER BY id DESC LIMIT 1 OFFSET ?1)",
            [PLAYS_KEPT as i64],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Deletes every play. Favorites and playlists stay.
    pub fn clear_history(&mut self) -> Result<(), LibraryError> {
        self.conn.execute("DELETE FROM plays", [])?;
        Ok(())
    }

    /// Creates a playlist, with one track in it if `first` names a track that exists.
    pub fn create_playlist(
        &mut self,
        name: &str,
        first: Option<TrackId>,
        now: i64,
    ) -> Result<PlaylistId, LibraryError> {
        let name = playlist_name(name).ok_or(LibraryError::InvalidPlaylistName)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO playlists (name, created_at) VALUES (?1, ?2)",
            (name, now),
        )?;
        let id = PlaylistId(tx.last_insert_rowid());
        if let Some(track) = first
            && let Some(path) = track_path(&tx, track)?
        {
            tx.execute(
                "INSERT INTO playlist_items (playlist_id, position, path) VALUES (?1, 0, ?2)",
                (id.0, &path),
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Renames a playlist. Returns whether the name changed; an unknown playlist changes
    /// nothing.
    pub fn rename_playlist(&mut self, id: PlaylistId, name: &str) -> Result<bool, LibraryError> {
        let name = playlist_name(name).ok_or(LibraryError::InvalidPlaylistName)?;
        let changed = self.conn.execute(
            "UPDATE playlists SET name = ?1 WHERE id = ?2 AND name <> ?1",
            (name, id.0),
        )?;
        Ok(changed > 0)
    }

    /// Deletes a playlist and its entries. Returns whether it existed.
    pub fn delete_playlist(&mut self, id: PlaylistId) -> Result<bool, LibraryError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", [id.0])?;
        let changed = tx.execute("DELETE FROM playlists WHERE id = ?1", [id.0])?;
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Appends a track to a playlist; the same track may be added again. Returns whether an
    /// entry was added; an unknown playlist or track changes nothing.
    pub fn add_to_playlist(
        &mut self,
        playlist: PlaylistId,
        track: TrackId,
    ) -> Result<bool, LibraryError> {
        let tx = self.conn.transaction()?;
        let Some(path) = track_path(&tx, track)? else {
            return Ok(false);
        };
        if !playlist_exists(&tx, playlist)? {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO playlist_items (playlist_id, position, path)
             SELECT ?1, COUNT(*), ?2 FROM playlist_items WHERE playlist_id = ?1",
            (playlist.0, &path),
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Removes the entry at a stored position; later entries move up. Returns whether an entry
    /// was removed.
    pub fn remove_from_playlist(
        &mut self,
        playlist: PlaylistId,
        position: u32,
    ) -> Result<bool, LibraryError> {
        self.edit_playlist(playlist, |paths| {
            let at = position as usize;
            if at >= paths.len() {
                return false;
            }
            paths.remove(at);
            true
        })
    }

    /// Moves the entry at stored position `from` so that it ends up at `to`, as
    /// `Vec::remove(from)` then `Vec::insert(to, _)` would. Returns whether the order changed.
    pub fn move_in_playlist(
        &mut self,
        playlist: PlaylistId,
        from: u32,
        to: u32,
    ) -> Result<bool, LibraryError> {
        self.edit_playlist(playlist, |paths| {
            let (from, to) = (from as usize, to as usize);
            if from == to || from >= paths.len() || to >= paths.len() {
                return false;
            }
            let path = paths.remove(from);
            paths.insert(to, path);
            true
        })
    }

    /// Reads the playlist's paths in position order, lets `edit` change them, and when it
    /// reports a change rewrites the playlist's rows with dense positions.
    fn edit_playlist(
        &mut self,
        playlist: PlaylistId,
        edit: impl FnOnce(&mut Vec<Vec<u8>>) -> bool,
    ) -> Result<bool, LibraryError> {
        let tx = self.conn.transaction()?;
        if !playlist_exists(&tx, playlist)? {
            return Ok(false);
        }
        let mut paths = {
            let mut stmt = tx.prepare(
                "SELECT path FROM playlist_items WHERE playlist_id = ?1 ORDER BY position",
            )?;
            stmt.query_map([playlist.0], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        if !edit(&mut paths) {
            return Ok(false);
        }
        tx.execute(
            "DELETE FROM playlist_items WHERE playlist_id = ?1",
            [playlist.0],
        )?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO playlist_items (playlist_id, position, path) VALUES (?1, ?2, ?3)",
            )?;
            for (position, path) in paths.iter().enumerate() {
                insert.execute((playlist.0, position as i64, path))?;
            }
        }
        tx.commit()?;
        Ok(true)
    }

    /// Reads favorites, playlists and recent plays in one transaction.
    pub fn collections(&self) -> Result<Collections, LibraryError> {
        let tx = self.conn.unchecked_transaction()?;
        let favorites = {
            let mut stmt = tx.prepare(
                "SELECT tracks.id FROM favorites JOIN tracks ON tracks.path = favorites.path
                 ORDER BY favorites.added_at DESC, tracks.id DESC",
            )?;
            stmt.query_map([], |r| r.get(0).map(TrackId))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let recent = {
            let mut stmt = tx.prepare(
                "SELECT tracks.id FROM plays JOIN tracks ON tracks.path = plays.path
                 GROUP BY plays.path
                 ORDER BY MAX(plays.played_at) DESC, MAX(plays.id) DESC LIMIT ?1",
            )?;
            stmt.query_map([RECENT_LIMIT as i64], |r| r.get(0).map(TrackId))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let playlists = read_playlists(&tx)?;
        tx.commit()?;
        Ok(Collections::new(favorites, playlists, recent))
    }
}

/// The stored path of a track, if the track exists.
fn track_path(
    conn: &rusqlite::Connection,
    track: TrackId,
) -> Result<Option<Vec<u8>>, LibraryError> {
    Ok(conn
        .query_row("SELECT path FROM tracks WHERE id = ?1", [track.0], |r| {
            r.get(0)
        })
        .optional()?)
}

fn playlist_exists(conn: &rusqlite::Connection, id: PlaylistId) -> Result<bool, LibraryError> {
    Ok(conn
        .query_row("SELECT 1 FROM playlists WHERE id = ?1", [id.0], |_| Ok(()))
        .optional()?
        .is_some())
}

/// All playlists with their available entries, sorted by lowercased name, then id.
fn read_playlists(conn: &rusqlite::Connection) -> Result<Vec<Playlist>, LibraryError> {
    let mut playlists: Vec<Playlist> = {
        let mut stmt = conn.prepare("SELECT id, name FROM playlists ORDER BY id")?;
        stmt.query_map([], |r| {
            Ok(Playlist {
                id: PlaylistId(r.get(0)?),
                name: r.get(1)?,
                items: Vec::new(),
                unavailable: 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    let index: HashMap<PlaylistId, usize> = playlists
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id, i))
        .collect();
    let mut stmt = conn.prepare(
        "SELECT playlist_items.playlist_id, playlist_items.position, tracks.id
         FROM playlist_items LEFT JOIN tracks ON tracks.path = playlist_items.path
         ORDER BY playlist_items.playlist_id, playlist_items.position",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            PlaylistId(r.get(0)?),
            r.get::<_, u32>(1)?,
            r.get::<_, Option<i64>>(2)?,
        ))
    })?;
    for row in rows {
        let (id, position, track) = row?;
        let Some(&at) = index.get(&id) else { continue };
        let playlist = &mut playlists[at];
        match track {
            Some(track) => playlist.items.push(PlaylistItem {
                position,
                track: TrackId(track),
            }),
            None => playlist.unavailable += 1,
        }
    }
    playlists.sort_by_cached_key(|p| (p.name.to_lowercase(), p.id));
    Ok(playlists)
}
