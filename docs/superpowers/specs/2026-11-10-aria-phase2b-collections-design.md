# Aria — Phase 2b (Favorites, Playlists, History, Home) Design

- **Date:** 2026-11-10
- **Status:** Design approved in conversation; pending written-spec review
- **Release:** 0.2.1 (pre-release)
- **Scope:** Second part of Phase 2. Phase 2a (0.2.0) delivered the local player. Phase 2c
  (soundboard) is specified in `2026-11-05-aria-phase2c-soundboard-design.md`.

## 1. Context and intent

Aria 0.2.0 plays local files from a scanned library, with a queue. The Home, Liked and Playlists
pages are empty states. Phase 2b makes them work, so that a listener can mark tracks they like,
keep their own lists of tracks, and get back to what they played or added recently.

Everything stays offline and local. Aria still never writes to or deletes audio files.

### Decisions made during design

| Decision | Choice |
|----------|--------|
| Scope | Favorites, playlists, playback history, Home page. No M3U import or export |
| Version | 0.2.1, released the same way as 0.2.0: branch, PR, `aria.exe` and the installer |
| Identity of a track in user data | Its file path, not its row id (section 3.1) |
| When a play counts | After 30 seconds of playing, or at its end if the track is shorter |
| History | On by default; a setting turns it off; a button clears it |

## 2. Scope

### In Phase 2b

- Favorites: mark and unmark a track; the Liked page.
- Playlists: create, rename, delete; add a track, remove an entry, reorder entries; play.
- Playback history: record plays; "Recently played".
- Home page: Recently played and Recently added.
- Settings: a history switch and a Clear history button.
- All new text in English, Russian and Armenian.

### Not in Phase 2b

- M3U import and export.
- A full history page, play counts, "most played".
- Adding several tracks or a whole album to a playlist in one action.
- Playlists listed in the sidebar; playlist artwork; playlist folders.
- Following a file that was renamed or moved (section 9).
- Soundboard (2c), Spotify (Phase 3), tray and media keys (Phase 4).

## 3. Data

### 3.1 Tracks are named by path

A track's row id does not survive removing a folder and adding it again, and a scan deletes the
rows of files it no longer finds. User data keyed by row id would vanish with them. So favorites,
playlist entries and plays store the track's **path**, in the same encoding as `tracks.path`.

- An entry whose path has no row in `tracks` is **unavailable**. It is kept in the database and
  hidden in the UI.
- When a scan finds the file again, the entry shows again with no further step.
- Removing a folder from the library hides its tracks' entries; it does not delete them.

### 3.2 Schema

One new migration (database `user_version` 1 to 2). It only adds tables, so a 0.2.0 database
keeps its folders, tracks and queue.

```sql
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
```

Times are Unix seconds, as `tracks.added_at` is.

### 3.3 Rules

- **Favorites.** A path is a favorite or it is not. Marking twice is the same as marking once.
- **Playlist names.** Trimmed of surrounding whitespace; 1 to 100 characters after trimming. Two
  playlists may share a name.
- **Playlist entries.** A playlist is an ordered list and may hold the same track more than once.
  Positions are dense, from 0, after every change. New entries go to the end.
- **Positions count every entry,** unavailable ones included. Commands name an entry by its
  stored position, which the UI gets with each visible entry (section 4.2). Unavailable entries
  keep their place when visible ones are moved around them.
- **Plays.** One row per play. After each insert the table is trimmed to its newest 10,000 rows.
- Every change is one transaction.

## 4. Architecture

### 4.1 `aria-library`

A new module `collections` adds methods to `Library` for the tables above and builds
`Collections` (section 4.2). The library thread stays the only owner of the database.

### 4.2 Data to the UI

Favorites, playlists and recent plays reach the UI as their own event, apart from
`LibrarySnapshot`. Marking a favorite must not rebuild the snapshot of a 50,000-track library.

```rust
pub struct Collections {
    /// Available favorites, newest first.
    pub favorites: Vec<TrackId>,
    /// Sorted by name without regard to case, then by id.
    pub playlists: Vec<Playlist>,
    /// Available tracks played most recently, newest first, each once, at most 20.
    pub recent: Vec<TrackId>,
}
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    /// Available entries in order.
    pub items: Vec<PlaylistItem>,
    /// How many entries are hidden because their file is not in the library.
    pub unavailable: usize,
}
pub struct PlaylistItem {
    /// The stored position, which commands use.
    pub position: u32,
    pub track: TrackId,
}
```

`Collections` also answers "is this track a favorite" without a scan of the list.

The backend sends `Event::Collections(Arc<Collections>)`:

- once at startup, after the first `LibraryUpdated`;
- after every `LibraryUpdated`, since availability may have changed;
- after every command in section 4.3 that changed something;
- after every recorded play.

"Recently added" is not part of `Collections`: the UI takes it from `LibrarySnapshot`, the 20
tracks with the newest `added_at`, newest first, ties by id.

### 4.3 Commands

| Command | Effect |
|---------|--------|
| `SetFavorite { track, favorite }` | Mark or unmark |
| `CreatePlaylist { name, first: Option<TrackId> }` | New playlist, with one track in it if given |
| `RenamePlaylist { id, name }` | |
| `DeletePlaylist(id)` | The playlist and its entries |
| `AddToPlaylist { playlist, track }` | Append |
| `RemoveFromPlaylist { playlist, position }` | Remove one entry |
| `MoveInPlaylist { playlist, from, to }` | Move the entry at `from` so that it ends up at `to` |
| `ClearHistory` | Delete every play |

A command that names a track id, playlist id or position that does not exist changes nothing and
is logged at debug level. A name that breaks the rules of section 3.3 changes nothing and gives
a notice.

Playing a list needs no new command: the UI sends the existing `PlayContext` with the list's
track ids.

### 4.4 Recording plays

The playback service counts the time a track spends actually playing: pauses add nothing, and a
seek adds nothing by itself. A play is recorded once, at the moment the track has played for 30
seconds, or when it ends if it ends before that. Skipping away earlier records nothing. A track
that starts again, by repeat or by being played again, can be recorded again.

With the history setting off, nothing is recorded. Plays already stored stay until cleared.

### 4.5 Settings

One new `Settings` field with a serde default: `history_enabled` (default `true`). Existing
`settings.toml` files keep loading and the settings schema version stays the same.

## 5. UI

- **Heart.** A narrow column in every track table, and a button beside the title in the player
  bar for the current track. A click sends `SetFavorite`. The row context menu gains "Add to
  favorites" or "Remove from favorites".
- **Add to playlist.** The row context menu gains a submenu listing the playlists, then "New
  playlist…", which asks for a name and creates the playlist with that track in it.
- **Liked page.** The favorites in a track table, newest first by default, with the same sortable
  headers, search field and row actions as Local Music. Empty state when there are none.
- **Playlists page.** The list of playlists and a "New playlist" button on one side; the selected
  playlist on the other: its name, Rename and Delete, a line "N tracks unavailable" when that is
  not zero, and its entries in playlist order. Entries are reordered by dragging, as in the queue,
  and removed from the row context menu. This table has no sortable headers. Delete asks first.
  Empty states for no playlists and for an empty playlist.
- **Home page.** "Recently played" and "Recently added", each a short list of rows with artwork,
  title and artist. A click plays that list from that row. With an empty library, the page points
  to Settings, Library. A section with nothing in it shows one line saying so.
- **Settings, Playback.** A "Keep playback history" switch with one line explaining that history
  stays on this computer, and a "Clear history" button that asks first.
- **Shared table.** The Local Music track table becomes a widget that Liked and Playlists reuse,
  so the three behave the same.
- The name dialog accepts only names that pass section 3.3: its confirm button is disabled
  otherwise.
- Nothing here blocks the UI thread; every change goes through a command.

## 6. Error handling

- A database error during a command: the change is rolled back, a notice is shown, and the UI
  keeps the last `Collections` it had.
- A database error while recording a play is logged and otherwise ignored. Playback goes on.
- A library that failed to open (the existing "library disabled" state) leaves the three pages
  in their empty states, and the new controls disabled.
- Paths in logs go through the existing redaction.

## 7. Privacy

Playback history is new personal data. It is stored only in the local database, is never sent
anywhere, and can be turned off and cleared in Settings. `docs/PRIVACY.md` says so, and lists
favorites and playlists as stored data too. No telemetry and no network requests are added.

## 8. Testing

- **Library:** the migration on a database made by 0.2.0 that holds folders, tracks and a queue;
  each rule in section 3.3; entries hidden when their folder is removed and back when it is added
  again; dense positions after every kind of change, with unavailable entries in the list; the
  10,000-row trim; `recent` listing each track once.
- **Core:** each command leads to the expected `Collections` event; invalid ids and names change
  nothing; the 30-second rule with the fake engine, including pause, seek, a track shorter than
  30 seconds, a skip before 30 seconds, and repeat; history off; Clear history.
- **App:** the pure parts (recently added, table rows for a playlist with unavailable entries,
  name validation) as unit tests.
- **Speed:** an ignored release test builds `Collections` for a 50,000-track library with 5,000
  favorites, 50 playlists of 200 entries and 10,000 plays, and reports the time. Target: under
  50 ms.
- **By hand:** `docs/testing/phase2b-manual-check.md` lists what a person must check in the
  window. The agent cannot click, so every such line starts as pending.

## 9. Known limitations

- A file that is renamed or moved is a new track: it leaves favorites and playlists, and its
  entries stay hidden until a file appears at the old path again.
- One track at a time goes into a playlist.
- No M3U files, no full history page, no play counts.
- The Russian and Armenian texts are not reviewed by a native speaker.

## 10. Definition of done

- Every item in section 2 works, and the tests in section 8 pass, with `cargo fmt`, `clippy -D
  warnings` and `cargo deny` clean on Windows, Linux and macOS in CI.
- A 0.2.0 database opens in 0.2.1 with nothing lost.
- README, CHANGELOG, `docs/PRIVACY.md` and the manual check list are up to date, and say plainly
  what nobody has verified.
- `aria.exe` and `Aria-0.2.1-setup.exe` are built and tested locally (launch; install, launch,
  uninstall) before any release. Merging, tagging and publishing wait for the user's explicit yes.
