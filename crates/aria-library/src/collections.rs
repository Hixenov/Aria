//! Favorites, playlists and playback history, and the `Collections` view of them.
//!
//! User data names a track by its path, not its row id, so it outlives a folder that is removed
//! and added again (see the design, section 3.1).

use std::collections::HashSet;

use rusqlite::OptionalExtension;

use crate::db::Library;
use crate::error::LibraryError;
use crate::model::TrackId;

/// How many plays the history keeps.
pub const PLAYS_KEPT: u64 = 10_000;
/// How many tracks `Collections::recent` holds at most.
pub const RECENT_LIMIT: usize = 20;

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
        tx.commit()?;
        Ok(Collections::new(favorites, Vec::new(), recent))
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
