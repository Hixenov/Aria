# Aria Phase 2b (Favorites, Playlists, History, Home) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Home, Liked and Playlists pages work: favorites, local playlists, playback history and a Home page, released as 0.2.1.

**Architecture:** User data lives in new SQLite tables keyed by the track's file path, so it survives a folder being removed and added again. The library thread builds a small `Collections` value and the backend sends it to the UI as its own event, apart from `LibrarySnapshot`. The playback service decides when a play counts and asks the library thread to store it.

**Tech Stack:** Rust 1.99.1, rusqlite, Tokio, egui/eframe 0.37 with `egui_extras` and `egui_kittest`, Fluent (`locales/{en,ru,hy}/main.ftl`).

**Spec:** `docs/superpowers/specs/2026-11-10-aria-phase2b-collections-design.md`

## Global Constraints

- No `unwrap`/`expect` in production code, no `unsafe`, no suppressed warnings (`#[allow]`), no fake or stubbed features. Tests may panic with a message.
- Aria never writes to, renames or deletes audio files. No network requests, no telemetry.
- Never block the UI thread or the audio callback. Every change from the UI is a `Command`.
- Paths in logs go through the existing redaction (`aria_core::redact`). Error text shown to the user holds no paths.
- Every user-facing string exists in `locales/en/main.ftl`, `locales/ru/main.ftl` and `locales/hy/main.ftl` under the same key.
- Database times are Unix seconds (`db::now_secs()`). Paths are stored with `paths::encode_path`, as `tracks.path` is.
- Exact values: play counts after **30 000 ms** of playing; history keeps the newest **10 000** plays; `recent` holds at most **20** tracks; "Recently added" shows **20**; playlist names are **1 to 100 characters** after trimming, counted in `char`s.
- Version is **0.2.1**. The database `user_version` goes from 1 to **2**. The settings schema version does not change.
- Each task ends with `cargo fmt --all --check`, `cargo clippy -j 4 --workspace --all-targets --locked -- -D warnings` and `cargo test -j 4 -p <crates touched> --locked` passing. Use `-j 4`: linking can fail with os error 1450 otherwise. Warnings about "hard linking files in the incremental compilation cache" come from the exFAT drive and are not code warnings.
- Environment for every shell (Git Bash):
  ```bash
  cd /e/claude/aria
  export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0=E:/claude/aria
  export CMAKE="C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe" CMAKE_GENERATOR="Visual Studio 17 2022"
  ```
- Commits end with `Co-Authored-By: Claude Fable 5.2 <noreply@anthropic.com>`. Work on branch `feat/phase2b-collections`. Do not push, merge, tag or publish.

## Review Focus

1. **A folder removed and added again.** Favorites, playlist entries and recent plays of its tracks must come back, in their old order, under the new track ids. Tests: Task 1 `favorites_survive_a_folder_removed_and_added_again`, Task 2 `entries_hide_and_return_with_their_folder`.
2. **Playlist names at the edges.** Only whitespace, 100 and 101 characters, and names in Armenian or with emoji, where bytes and characters differ. Test: Task 2 `names_are_trimmed_and_measured_in_characters`.
3. **Moving and removing around hidden entries.** A playlist with unavailable entries between visible ones must keep the hidden ones in place and positions dense. Test: Task 2 `moves_keep_unavailable_entries_in_place`.
4. **`Collections` and `LibrarySnapshot` out of step.** The two events arrive separately, so the UI can hold ids that its snapshot does not know. Every lookup must skip them, never index or panic. Tests: Task 6 `ids_missing_from_the_snapshot_are_skipped`, Task 8 `home_rows_skip_unknown_ids`.
5. **A play that must not count, or must count once.** Pause for a minute, seek forward past 30 s, skip at 29 s, repeat-one, a track of 10 s, history switched off mid-track. Tests: Task 4.

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/aria-library/src/migrations.rs` | Second migration (section 3.2 of the spec) |
| `crates/aria-library/src/collections.rs` (new) | `Collections`, `Playlist`, `PlaylistItem`, `PlaylistId`; the `Library` methods for favorites, playlists and plays |
| `crates/aria-library/tests/collections.rs` (new) | Tests of the above through the public API |
| `crates/aria-core/src/command.rs`, `event.rs`, `library_service.rs`, `runtime.rs` | New commands, requests and the `Collections` event |
| `crates/aria-core/src/playback_service.rs`, `settings/mod.rs` | The 30-second rule; `history_enabled` |
| `crates/aria-app/src/ui/widgets/track_menu.rs` (new) | The row context-menu items shared by every track list, and the heart button |
| `crates/aria-app/src/ui/widgets/name_dialog.rs` (new) | The dialog that asks for a playlist name |
| `crates/aria-app/src/ui/pages/liked.rs`, `playlists.rs`, `home.rs` | The three pages |
| `crates/aria-app/src/ui/pages/local_music/tracks.rs` | Gains the heart column and the shared menu; reused by Liked |
| `docs/testing/phase2b-manual-check.md` (new), README, CHANGELOG, `docs/PRIVACY.md` | Docs |

---

### Task 1: Library — migration, favorites, plays, `Collections`

**Files:**
- Modify: `crates/aria-library/src/migrations.rs`, `crates/aria-library/src/lib.rs`, `crates/aria-library/src/error.rs`
- Create: `crates/aria-library/src/collections.rs`, `crates/aria-library/tests/collections.rs`

**Interfaces:**
- Consumes: `Library` (field `conn`), `paths::encode_path`, `TrackId`, `LibraryError`.
- Produces (all re-exported from `lib.rs`):
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
  pub struct PlaylistId(pub i64);
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub struct PlaylistItem { pub position: u32, pub track: TrackId }
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct Playlist { pub id: PlaylistId, pub name: String, pub items: Vec<PlaylistItem>, pub unavailable: usize }
  #[derive(Debug, Clone, PartialEq, Eq, Default)]
  pub struct Collections { pub favorites: Vec<TrackId>, pub playlists: Vec<Playlist>, pub recent: Vec<TrackId>, /* private: favorite_set */ }
  impl Collections {
      pub fn new(favorites: Vec<TrackId>, playlists: Vec<Playlist>, recent: Vec<TrackId>) -> Self;
      pub fn is_favorite(&self, id: TrackId) -> bool;          // O(1)
      pub fn playlist(&self, id: PlaylistId) -> Option<&Playlist>;
  }
  pub const PLAYS_KEPT: u64 = 10_000;
  pub const RECENT_LIMIT: usize = 20;
  impl Library {
      /// Returns whether anything changed. An unknown track changes nothing.
      pub fn set_favorite(&mut self, track: TrackId, favorite: bool, now: i64) -> Result<bool, LibraryError>;
      pub fn record_play(&mut self, track: TrackId, now: i64) -> Result<bool, LibraryError>;
      pub fn clear_history(&mut self) -> Result<(), LibraryError>;
      pub fn collections(&self) -> Result<Collections, LibraryError>;
  }
  ```
  In this task `collections()` returns `playlists: vec![]`.

- [ ] **Step 1: Write the failing tests** in `tests/collections.rs`. Reuse the helpers of `tests/db.rs` (`ok`, opening a library in a temp dir, `insert_track` by raw SQL); copy the few lines needed rather than sharing a module.
  - `a_0_2_0_database_upgrades_and_keeps_its_data`: build a database with only the first migration applied (`PRAGMA user_version = 1`, a folder, two tracks, a saved queue), open it with `Library::open`; assert `user_version == 2`, `SCHEMA_VERSION == 2`, the tracks and `load_queue()` are unchanged, `collections() == Collections::default()`.
  - `favorites_are_listed_newest_first_and_marking_twice_is_once`: mark tracks A (now=10), B (now=20), A again (now=30) → `favorites == [B, A]`, the second mark of A returns `false`; `is_favorite(A)`; unmark B → `[A]`.
  - `an_unknown_track_changes_nothing`: `set_favorite(TrackId(999), true, 1) == Ok(false)`, `record_play(TrackId(999), 1) == Ok(false)`.
  - `favorites_survive_a_folder_removed_and_added_again`: mark A and B, `remove_folder` → `favorites == []`; add the folder and insert tracks at the same paths (new ids A2, B2) → `favorites == [B2, A2]`.
  - `recent_lists_each_track_once_newest_first`: plays A@1, B@2, A@3 → `recent == [A, B]`.
  - `recent_holds_at_most_twenty`: 25 tracks played once each → `recent.len() == 20`, newest first.
  - `plays_are_trimmed_to_the_newest_ten_thousand`: insert 10 005 plays (one transaction of raw SQL for 10 004, then one `record_play`) → `SELECT COUNT(*) FROM plays == 10000` and the oldest five are gone.
  - `clear_history_empties_recent_and_keeps_favorites`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-library --test collections` — expected: does not compile (`collections` missing).
- [ ] **Step 3: Implement.** Add the migration exactly as in spec section 3.2 as the second entry of `MIGRATIONS`. In `collections.rs`, resolve a `TrackId` to its stored path with `SELECT path FROM tracks WHERE id = ?1`; list favorites with `JOIN tracks ON tracks.path = favorites.path ORDER BY favorites.added_at DESC, tracks.id DESC`; `recent` with `GROUP BY plays.path ORDER BY MAX(played_at) DESC, MAX(plays.id) DESC LIMIT 20` joined to `tracks`. `record_play` inserts and trims in one transaction (`DELETE FROM plays WHERE id <= (SELECT id FROM plays ORDER BY id DESC LIMIT 1 OFFSET 10000)`). `collections()` reads inside one `unchecked_transaction`, as `snapshot()` does.
- [ ] **Step 4: Run** the tests of step 2 and `cargo test -j 4 -p aria-library --locked` — expected: all pass, including the existing `fresh_database_migrates` (update its expected version to 2 if it names 1).
- [ ] **Step 5: Commit** `feat(library): favorites, plays and the collections view`.

### Task 2: Library — playlists

**Files:**
- Modify: `crates/aria-library/src/collections.rs`, `crates/aria-library/src/error.rs`, `crates/aria-library/src/lib.rs`, `crates/aria-library/tests/collections.rs`

**Interfaces:**
- Produces:
  ```rust
  pub const PLAYLIST_NAME_MAX: usize = 100;
  /// The name as it would be stored, or `None` if it breaks the rules.
  pub fn playlist_name(name: &str) -> Option<&str>;   // trimmed; 1..=100 chars
  // LibraryError gains: InvalidPlaylistName
  impl Library {
      pub fn create_playlist(&mut self, name: &str, first: Option<TrackId>, now: i64) -> Result<PlaylistId, LibraryError>;
      // Each returns whether anything changed; unknown ids and positions change nothing.
      pub fn rename_playlist(&mut self, id: PlaylistId, name: &str) -> Result<bool, LibraryError>;
      pub fn delete_playlist(&mut self, id: PlaylistId) -> Result<bool, LibraryError>;
      pub fn add_to_playlist(&mut self, playlist: PlaylistId, track: TrackId) -> Result<bool, LibraryError>;
      pub fn remove_from_playlist(&mut self, playlist: PlaylistId, position: u32) -> Result<bool, LibraryError>;
      pub fn move_in_playlist(&mut self, playlist: PlaylistId, from: u32, to: u32) -> Result<bool, LibraryError>;
  }
  ```
  `collections().playlists` is now filled, sorted by `name` lowercased (Unicode), then id.

- [ ] **Step 1: Write the failing tests.**
  - `names_are_trimmed_and_measured_in_characters`: `playlist_name("  Road  ") == Some("Road")`; `""`, `"   "` → `None`; a name of 100 `'ե'` → `Some`; 101 → `None`; `"🎵 mix"` → `Some`. `create_playlist("   ", None, 1)` is `Err(InvalidPlaylistName)` and creates nothing; `rename_playlist(id, "")` likewise.
  - `two_playlists_may_share_a_name_and_sort_by_name_then_id`: create "b", "A", "a" → order `["A", "a", "b"]` with the earlier id first among equals.
  - `entries_append_and_may_repeat`: add A, B, A → items `[(0,A),(1,B),(2,A)]`.
  - `create_with_a_first_track_holds_it`; `create_with_an_unknown_first_track_is_empty`.
  - `removing_an_entry_closes_the_gap`: `[A,B,C]`, remove position 1 → `[(0,A),(1,C)]`; remove position 9 → `Ok(false)`.
  - `moving_an_entry_lands_it_at_the_target`: `[A,B,C,D]`: move 0→2 → `[B,C,A,D]`; move 3→0 → `[D,B,C,A]`; move 1→1 → `Ok(false)`; move 0→9 → `Ok(false)`.
  - `entries_hide_and_return_with_their_folder`: playlist `[A,B]`; remove the folder → `items == []`, `unavailable == 2`; add it again with the same paths → items `[(0,A2),(1,B2)]`, `unavailable == 0`.
  - `moves_keep_unavailable_entries_in_place`: stored `[A, X, B, C]` where X's track row is deleted by raw SQL → visible `[(0,A),(2,B),(3,C)]`, `unavailable == 1`. Move 3→0 → stored `[C, A, X, B]`: visible `[(0,C),(1,A),(3,B)]`. Remove position 1 → visible `[(0,C),(2,B)]`.
  - `deleting_a_playlist_deletes_its_entries`: `SELECT COUNT(*) FROM playlist_items == 0` afterwards.
  - `#[ignore] collections_build_time_for_a_large_library`: 50 000 tracks, 5 000 favorites, 50 playlists of 200 entries, 10 000 plays, all by raw SQL in one transaction; print the time of `collections()`; assert under 500 ms (the 50 ms target is for a release build and is reported, not asserted).
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-library --test collections` — expected: does not compile.
- [ ] **Step 3: Implement.** A change of order reads the playlist's paths in position order, edits the `Vec`, then deletes and reinserts the playlist's rows in one transaction: simple, and it never trips the primary key. `move` follows the semantics of `Vec::remove(from)` then `Vec::insert(to, _)`.
- [ ] **Step 4: Run** `cargo test -j 4 -p aria-library --locked`, then `cargo test -j 4 --release -p aria-library --test collections -- --ignored --nocapture` and note the printed time in the task report.
- [ ] **Step 5: Commit** `feat(library): local playlists`.

### Task 3: Core — commands, requests and the `Collections` event

**Files:**
- Modify: `crates/aria-core/src/command.rs`, `event.rs`, `library_service.rs`, `runtime.rs`, `lib.rs` (re-exports), `locales/*/main.ftl` (one notice)

**Interfaces:**
- Consumes: Task 1 and 2.
- Produces:
  ```rust
  // Command and LibraryRequest both gain:
  SetFavorite { track: TrackId, favorite: bool },
  CreatePlaylist { name: String, first: Option<TrackId> },
  RenamePlaylist { id: PlaylistId, name: String },
  DeletePlaylist(PlaylistId),
  AddToPlaylist { playlist: PlaylistId, track: TrackId },
  RemoveFromPlaylist { playlist: PlaylistId, position: u32 },
  MoveInPlaylist { playlist: PlaylistId, from: u32, to: u32 },
  ClearHistory,
  // LibraryRequest only:
  RecordPlay(TrackId),
  // LibraryEvent and Event gain:
  Collections(Arc<Collections>),
  ```
  Notice key `playlist-name-invalid` (level Warning). `aria_core` re-exports what the app needs from `aria_library` the way it already does for `TrackId`.

- [ ] **Step 1: Write the failing tests** in the existing test modules.
  - `library_service`: `collections_follow_every_snapshot` (after start: a `Snapshot` then a `Collections`); `a_favorite_is_reported_without_a_new_snapshot` (send `SetFavorite`; the next events hold one `Collections` with the track and **no** `Snapshot`); `an_invalid_playlist_name_gives_a_notice_and_no_collections`; `a_command_that_changes_nothing_sends_nothing` (`DeletePlaylist(PlaylistId(99))`); `collection_changes_are_served_during_a_scan` (follow the pattern of the existing test that removes a folder mid-scan); `record_play_then_clear_history`.
  - `runtime`: `a_collection_command_without_a_library_gives_the_unavailable_notice` (a backend started with no library paths: `Command::SetFavorite` → `Event::Notice(library_service::UNAVAILABLE)`).
  - `runtime`: `collection_commands_reach_the_library_and_come_back_as_events` (send `Command::CreatePlaylist { name: "Road".into(), first: None }`, expect `Event::Collections` whose `playlists[0].name == "Road"`).
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-core --locked collections` — expected: does not compile.
- [ ] **Step 3: Implement.** In `LibraryInner::serve_now`, handle the nine requests at once (they never wait for a scan). After one that returned `Ok(true)` or created a playlist, call a new `send_collections(library)`. `send_snapshot` calls `send_collections` after emitting the snapshot. `Err(InvalidPlaylistName)` emits the notice; other errors go to `report_failure`. `Ok(false)` logs at debug and sends nothing. `RecordPlay` and `SetFavorite` take the time from `SystemTime` in the service (add a small `now_secs()` there; `aria-library`'s is crate-private). In `runtime.rs`, route the eight commands to the library and forward `LibraryEvent::Collections` as `Event::Collections`. With no library, those eight commands emit `Event::Notice(library_service::UNAVAILABLE)` instead of going nowhere (spec section 6); the older commands keep their behaviour.
- [ ] **Step 4: Run** `cargo test -j 4 -p aria-core --locked` — expected: all pass. Every-language tests for notices must pass, so add `playlist-name-invalid` to the three `.ftl` files: en "A playlist name needs 1 to 100 characters."; ru «Название плейлиста должно быть от 1 до 100 символов.»; hy «Երգացանկի անունը պետք է լինի 1-ից 100 նիշ։».
- [ ] **Step 5: Commit** `feat(core): collection commands and the Collections event`.

### Task 4: Core — recording plays, and the history setting

**Files:**
- Modify: `crates/aria-core/src/playback_service.rs`, `crates/aria-core/src/settings/mod.rs`

**Interfaces:**
- Consumes: `LibraryRequest::RecordPlay(TrackId)` from Task 3.
- Produces: `Settings::history_enabled: bool`, default `true`, with a lenient deserializer like `lenient_gapless`; `pub const PLAY_COUNTS_AFTER_MS: u64 = 30_000;` in `playback_service`.

- [ ] **Step 1: Write the failing tests** with the existing `Rig` (it collects `requests`). A helper `plays(&rig) -> Vec<TrackId>` filters `RecordPlay`. Use tone files long enough, made the way the existing tests make them.
  - `a_play_counts_after_thirty_seconds`: nothing recorded when the clock reads 29 900 ms; exactly one `RecordPlay` after a tick at 30 000 ms; no second one at 60 000 ms.
  - `a_short_track_counts_when_it_ends`: a 10 s track played to its end → one record, even with no tick in between.
  - `a_skip_before_thirty_seconds_does_not_count`: `Next` at 29 s → none for that track.
  - `a_pause_adds_nothing`: play 10 s, pause, tick many times → none.
  - `a_seek_adds_nothing_by_itself`: play 5 s, `Seek(200_000)`, tick at once → none; after 25 more seconds of playing → one.
  - `time_before_a_seek_still_counts`: play 20 s, seek back to 0, play 10 s → one.
  - `repeat_one_counts_each_time_round`.
  - `nothing_is_recorded_with_history_off`, and `switching_history_off_mid_track_stops_the_record`.
  - Settings: `history_enabled_defaults_to_true_and_survives_a_bad_value` (a `settings.toml` with `history_enabled = "x"` loads as `true`); an existing 0.2.0 `settings.toml` without the field loads.
  - `the_threshold_is_thirty_seconds`: `PLAY_COUNTS_AFTER_MS == 30_000`, and a new service uses it.
  - The fake device runs at four times real time, so 30 seconds of audio would cost each test about 8 seconds. Keep the threshold in a private field that starts at `PLAY_COUNTS_AFTER_MS`, and let the tests above lower it (for example to 400 ms, with every time in the list scaled the same way). Only `the_threshold_is_thirty_seconds` reads the real value.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-core --locked play` — expected: fails to compile or fails.
- [ ] **Step 3: Implement.** Add fields `heard_ms: u64` (playing time before the last seek), `play_recorded: bool`, `history: bool`. Time played now is `heard_ms + position.saturating_sub(played_from)` while the clock's token is the current track's. Where `seek` sets `played_from`, first add the time since the old `played_from` to `heard_ms`. Reset both fields wherever `current` becomes a new track (`load`, `started`). Check the threshold in `tick_at`; on `TrackEnded` of the current track, record if not yet recorded. `on_settings` reads `history_enabled`. Recording pushes `LibraryRequest::RecordPlay(track)` onto `requests`.
- [ ] **Step 4: Run** `cargo test -j 4 -p aria-core --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(core): record a play after 30 seconds`.

### Task 5: App — collections state, heart, shared row menu, name dialog

**Files:**
- Create: `crates/aria-app/src/ui/widgets/track_menu.rs`, `crates/aria-app/src/ui/widgets/name_dialog.rs`
- Modify: `crates/aria-app/src/app.rs`, `ui/pages/mod.rs`, `ui/pages/local_music/tracks.rs`, `ui/layout/player_bar.rs`, `ui/widgets/mod.rs`, `ui/theme/icons` (a heart, filled and outline, from the icon font already bundled), `locales/*/main.ftl`

**Interfaces:**
- Consumes: `Event::Collections`, the commands of Task 3, `aria_library::playlist_name`.
- Produces:
  ```rust
  // PageCtx gains:
  pub collections: &'a Collections,
  // PageAction gains (handle_page_action maps each to the Command of the same name):
  SetFavorite { track: TrackId, favorite: bool },
  AddToPlaylist { playlist: PlaylistId, track: TrackId },
  RemoveFromPlaylist { playlist: PlaylistId, position: u32 },
  MoveInPlaylist { playlist: PlaylistId, from: u32, to: u32 },
  DeletePlaylist(PlaylistId),
  ClearHistory,
  /// Open the name dialog. Confirming it sends CreatePlaylist or RenamePlaylist.
  AskPlaylistName(NameFor),
  pub enum NameFor { New { first: Option<TrackId> }, Rename(PlaylistId) }
  // PlayerAction gains: SetFavorite { track: TrackId, favorite: bool }
  // widgets::track_menu:
  /// The favorite item and the "Add to playlist" submenu, for a row's context menu.
  pub fn items(ui: &mut Ui, tr: &Localizer, collections: &Collections, track: TrackId, out: &mut Vec<PageAction>);
  /// The heart button. Returns the action when clicked.
  pub fn heart(ui: &mut Ui, tr: &Localizer, p: &Palette, collections: &Collections, track: TrackId) -> Option<PageAction>;
  ```
  The app owns the name dialog's state (`Option<(NameFor, String)>`), draws it as a modal that sets `modal_open`, and keeps its confirm button disabled while `playlist_name(text)` is `None`. Enter confirms, Escape cancels.

- [ ] **Step 1: Write the failing tests** (kittest `Harness`, following `space_harness` and `toggles_sent` in `app.rs`).
  - `a_collections_event_is_kept`: `handle_event(Event::Collections(c))` then the app's collections equal `c`.
  - `page_actions_become_collection_commands`: each new `PageAction` except `AskPlaylistName` sends the matching `Command` (count them as `toggles_sent` does).
  - `the_name_dialog_refuses_an_empty_name_and_creates_on_enter`: open with `AskPlaylistName(New { first: Some(t) })`; with empty text the confirm button is disabled; type "Road", press Enter → `Command::CreatePlaylist { name: "Road", first: Some(t) }` sent and the dialog closed.
  - `the_name_dialog_renames`: opened for `Rename(id)`, it starts with the playlist's current name and sends `RenamePlaylist`.
  - `track_menu` unit test: with the track a favorite the item reads `track-unfavorite`, else `track-favorite`; the submenu lists playlists in the order of `collections.playlists`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-app --locked` — expected: does not compile.
- [ ] **Step 3: Implement.** Add a heart column (`Column::exact`, about 32 px) as the first column of the track table in `local_music/tracks.rs`, and call `track_menu::items` in its context menu after the three existing items. Add the heart beside the title in the player bar's `now_playing`, disabled with nothing loaded. New Fluent keys, in all three languages: `track-favorite` (Add to favorites / Добавить в избранное / Ավելացնել ընտրանի), `track-unfavorite` (Remove from favorites / Убрать из избранного / Հեռացնել ընտրանուց), `track-add-to-playlist` (Add to playlist / Добавить в плейлист / Ավելացնել երգացանկին), `playlist-new` (New playlist… / Новый плейлист… / Նոր երգացանկ…), `playlist-name-label` (Name / Название / Անուն), `playlist-create` (Create / Создать / Ստեղծել), `playlist-rename` (Rename / Переименовать / Վերանվանել), `dialog-cancel` only if no cancel key exists yet.
- [ ] **Step 4: Run** `cargo test -j 4 -p aria-app --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(app): hearts, the shared track menu and the playlist name dialog`.

### Task 6: App — Liked page

**Files:**
- Modify: `crates/aria-app/src/ui/pages/liked.rs`, `ui/pages/local_music/view.rs` (+ `mod.rs` re-exports), `crates/aria-app/src/nav.rs`, `locales/*/main.ftl`

**Interfaces:**
- Consumes: `TrackView`, `tracks::show`, `PageCtx::collections`.
- Produces: `Scope::Ids(Arc<[TrackId]>)` in `view.rs` — the rows are these tracks in this order until the user sorts; ids the snapshot does not hold are skipped. (`Scope` loses `Copy`; adjust its users.) `Route::Liked.phase()` returns `None`.

- [ ] **Step 1: Write the failing tests.**
  - `view.rs`: `an_id_scope_keeps_its_order_until_sorted` (ids `[3,1,2]` → rows in that order; after `set_sort(Title, true)` by title); `ids_missing_from_the_snapshot_are_skipped` (ids `[3, 99, 1]` → rows for 3 and 1 only); `search_filters_an_id_scope`.
  - `liked.rs` (kittest): `no_favorites_shows_the_empty_state` (text of `liked-empty`); `favorites_are_listed_newest_first` (two favorites → both titles drawn, the newer above); `a_new_collections_value_replaces_the_rows` (unfavorite one → it is gone next frame).
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-app --locked liked` — expected: fails.
- [ ] **Step 3: Implement.** The page keeps its own `TrackView` and query in egui temp memory, as Local Music does, and resets the scope when `collections.favorites` differs from the ids it last used. Header, search field, table via `tracks::show`. Keys: `liked-empty` (No favorites yet / Пока нет избранного / Դեռ ընտրանի չկա), `liked-empty-hint` (Click the heart next to a track. / Нажмите на сердечко рядом с треком. / Սեղմեք սրտիկը երգի կողքին։).
- [ ] **Step 4: Run** `cargo test -j 4 -p aria-app --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(app): the Liked page`.

### Task 7: App — Playlists page

**Files:**
- Modify: `crates/aria-app/src/ui/pages/playlists.rs`, `crates/aria-app/src/nav.rs`, `locales/*/main.ftl`

**Interfaces:**
- Consumes: `Collections::playlists`, `PageAction::{AskPlaylistName, DeletePlaylist, RemoveFromPlaylist, MoveInPlaylist, PlayContext}`, `track_menu`, the drag-and-drop row pattern of `ui/layout/right_panel.rs` (`dnd_drag_source`, the grip, the drop line, `drop_action`).
- Produces: `Route::Playlists.phase()` returns `None`. A pure helper `fn drop_move(from: u32, to: u32) -> Option<(u32, u32)>` that returns `None` when the row is dropped where it is.

- [ ] **Step 1: Write the failing tests.**
  - Unit: `dropping_a_row_where_it_is_asks_for_nothing`; `rows_use_stored_positions` (a playlist with items `[(0,A),(2,B)]` and `unavailable == 1`: the second row's remove action is `RemoveFromPlaylist { position: 2 }`).
  - kittest: `no_playlists_shows_the_empty_state_and_the_new_button`; `clicking_new_asks_for_a_name` (`AskPlaylistName(New { first: None })`); `the_selected_playlist_shows_its_tracks_and_the_unavailable_line` (line text from `playlist-unavailable` with count 1); `delete_asks_first` (clicking Delete opens the confirm dialog and sends nothing; confirming sends `DeletePlaylist`); `double_click_plays_the_playlist_from_that_row` (`PlayContext { ids: [A,B], start: 1 }`); `a_deleted_selection_falls_back_to_the_first_playlist`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-app --locked playlists` — expected: fails.
- [ ] **Step 3: Implement.** Two columns: the playlist list (name and track count; "New playlist" button on top) and the selected playlist. The selection is a `PlaylistId` in egui temp memory. Entries are drawn with `ScrollArea::show_rows` so a long playlist stays cheap; each row: grip, heart, title, artist, duration; context menu: Play, Play next, Add to queue, `track_menu::items`, Remove from playlist. Delete confirmation follows the confirm dialog in `ui/pages/settings/mod.rs`. Keys: `playlists-empty`, `playlists-empty-hint`, `playlist-empty` (This playlist is empty), `playlist-delete`, `playlist-delete-confirm` (with `{ $name }`), `playlist-remove-entry`, `playlist-unavailable` (with `{ $count }`, plural forms for ru), `playlist-track-count` (with `{ $count }`).
- [ ] **Step 4: Run** `cargo test -j 4 -p aria-app --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(app): the Playlists page`.

### Task 8: App — Home page and the history setting

**Files:**
- Modify: `crates/aria-app/src/ui/pages/home.rs`, `ui/pages/settings/playback.rs`, `ui/pages/settings/mod.rs` (the confirm dialog gains "clear history"), `crates/aria-app/src/nav.rs`, `locales/*/main.ftl`

**Interfaces:**
- Consumes: `Collections::recent`, `LibrarySnapshot`, `Settings::history_enabled`, `PageAction::{ClearHistory, UpdateSettings, PlayContext}`, `ArtworkTextures` as the player bar uses it.
- Produces: `pub fn recently_added(snapshot: &LibrarySnapshot, limit: usize) -> Vec<TrackId>` (newest `added_at` first, ties by higher id first); `Route::Home.phase()` returns `None`.

- [ ] **Step 1: Write the failing tests.**
  - Unit: `recently_added_takes_the_newest_twenty` (25 tracks → 20, newest first, ties by id); `home_rows_skip_unknown_ids` (`recent == [5, 99]` with a snapshot holding only 5 → one row).
  - kittest: `an_empty_library_points_to_settings` (`home-empty` text); `a_click_plays_the_section_from_that_row` (click the second row of Recently played → `PlayContext { ids: recent, start: 1 }`); `an_empty_section_says_so` (library with tracks, no plays → `home-recent-empty`); settings: `the_history_switch_updates_settings` (`UpdateSettings` with `history_enabled == false`); `clear_history_asks_first` (confirming sends `ClearHistory`).
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-app --locked home` — expected: fails.
- [ ] **Step 3: Implement.** Two sections with a heading each; rows show artwork (or the placeholder), title and artist, with `track_menu::items` in the context menu. Compute `recently_added` once per snapshot (cache by `Arc::ptr_eq`). Keys: `home-recent` (Recently played / Недавно играло / Վերջերս նվագարկված), `home-added` (Recently added / Недавно добавлено / Վերջերս ավելացված), `home-empty`, `home-empty-hint`, `home-recent-empty`, `settings-history` (Keep playback history / Вести историю прослушиваний / Պահել նվագարկման պատմությունը), `settings-history-hint` (History stays on this computer. / История хранится только на этом компьютере. / Պատմությունը մնում է միայն այս համակարգչում։), `settings-history-clear`, `settings-history-clear-confirm`.
- [ ] **Step 4: Run** `cargo test -j 4 -p aria-app --locked`, then the whole workspace: `cargo test -j 4 --workspace --locked`.
- [ ] **Step 5: Commit** `feat(app): the Home page and the history setting`.

### Task 9: Version, docs and the manual check list

**Files:**
- Modify: `Cargo.toml` (workspace version `0.2.1`), `Cargo.lock`, `README.md`, `CHANGELOG.md`, `docs/PRIVACY.md`, `docs/ROADMAP.md` (mark 2b), `packaging/windows/aria.iss` comment lines that name `0.2.0`
- Create: `docs/testing/phase2b-manual-check.md`

- [ ] **Step 1:** Set the version to 0.2.1 and run `cargo build -j 4 --locked` once without `--locked` to refresh `Cargo.lock`, then with it.
- [ ] **Step 2:** `CHANGELOG.md`: a `## [0.2.1] - unreleased (pre-release: Phase 2b, favorites, playlists, history)` entry listing what was added and the limitations of spec section 9. `README.md`: status section for 0.2.1, the installer command with `0.2.1`, limitations. `docs/PRIVACY.md`: favorites, playlists and playback history are stored in the local database; history can be turned off and cleared in Settings, Playback; nothing leaves the computer.
- [ ] **Step 3:** `docs/testing/phase2b-manual-check.md`, in the format of `phase2a-manual-check.md`: what automated tests cover, and a table of checks for a person, every line `pending`: heart in the table and the player bar; Liked page lists, searches, sorts, plays; create, rename, delete a playlist; add from the context menu, including "New playlist…"; drag to reorder, remove an entry; a playlist survives removing and re-adding its folder; Home lists after playing for 30 seconds; history switch off stops new entries; Clear history; a 0.2.0 database upgrades with library and queue intact; RU and HY texts read by a native speaker.
- [ ] **Step 4:** Run `cargo fmt --all --check`, clippy and `cargo test -j 4 --workspace --locked`. Run the release build and the ignored speed test, and put the measured `Collections` time in the README's performance notes, saying which machine.
- [ ] **Step 5: Commit** `docs: 0.2.1, Phase 2b`.
