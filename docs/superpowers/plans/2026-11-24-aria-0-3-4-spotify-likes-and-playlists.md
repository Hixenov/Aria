# Aria 0.3.4 — Spotify likes and playlist editing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hearts on Spotify tracks, "Add to playlist"/"New playlist…", and opening/removing from the user's own Spotify playlists.

**Architecture:** New documented Web API calls in `aria-spotify::Client`; new `SpotifyRequest`s handled on the existing Spotify thread in `aria-core::spotify_service`, which updates Liked Songs in place and holds one open playlist; the app keeps a liked-URI set and the open playlist in `SpotifyUi`, draws the heart and context menu from one shared widget, and the Library page opens editable playlists.

**Tech Stack:** Rust, ureq 3.5, serde_json, egui/eframe 0.37, egui_kittest, Fluent.

**Spec:** `docs/superpowers/specs/2026-11-24-aria-0-3-4-spotify-likes-and-playlists-design.md`

## Global Constraints

- Endpoints exactly: `PUT /v1/me/library?uris=`, `DELETE /v1/me/library?uris=`, `GET /v1/playlists/{id}/items?limit=50`, `POST /v1/playlists/{id}/items` body `{"uris":[..]}` (201), `DELETE /v1/playlists/{id}/items` body `{"items":[{"uri":..}],"snapshot_id":..}` (200), `POST /v1/me/playlists` body `{"name":..,"public":false}` (201).
- Scopes added: `user-library-modify playlist-modify-public playlist-modify-private`.
- No unwrap/expect outside tests, no unsafe, no `#[allow]`; never log tokens; no client secret.
- Every new user-facing string is a Fluent key in `locales/{en,ru,hy}` and in the i18n `KEYS` list.
- Tests never touch the real Spotify, the real microphone, `%APPDATA%\Aria`, or run the built exe.
- Build: source the scratchpad `env.sh`, `CARGO_INCREMENTAL=0`, `-j 4`; long output to `.local/build-logs`.

## Review Focus

1. A playlist id from Spotify containing `/`, `?` or `..` must never change the request path — ids that are not ASCII alphanumeric are refused without a request (Task 1 test).
2. A like/unlike that fails must leave Liked Songs exactly as before, even when the user clicked twice quickly (Task 2 test: two likes, the second fails, the first stays).
3. Signing out, or a new sign-in as another account, must drop the open playlist and its snapshot (Task 2 test).
4. An open playlist whose items answer 403 (followed, not owned, or a stale editable flag) shows a failed state, not an endless loading one (Task 2 test).
5. A track with no URI (a local file in a Spotify playlist) offers no heart and no edit items anywhere (Task 3 test).

---

### Task 1: Web API calls in aria-spotify

**Files:**
- Modify: `crates/aria-spotify/src/lib.rs` (SCOPES), `src/token.rs` (`can_edit`), `src/model.rs` (SpPlaylist fields, playlist item entry, created playlist), `src/client.rs` (Method::Delete, new methods)
- Create fixtures: `crates/aria-spotify/tests/fixtures/playlist-items.json`, `playlist-items-2.json`, `snapshot.json`, `created-playlist.json`; extend `playlists.json` with `collaborative` and `snapshot_id`

**Interfaces — Produces:**
- `SpPlaylist { uri, name, owner, image, id: String, owner_id: String, collaborative: bool, snapshot: Option<String>, editable: bool }` (`editable` is false here; the service sets it).
- `Tokens::can_edit(&self) -> bool` — scopes contain all three new scopes.
- `Client::save_track(&mut self, uri: &str) -> Result<(), SpotifyError>`; `Client::unsave_track(&mut self, uri: &str) -> Result<(), SpotifyError>`
- `Client::playlist_tracks(&mut self, id: &str) -> Result<Vec<SpTrack>, SpotifyError>` — all pages; entry field `item`, else `track`; entries whose URI is not `spotify:track:` and not local are skipped.
- `Client::add_to_playlist(&mut self, id: &str, uri: &str) -> Result<Option<String>, SpotifyError>` (new snapshot)
- `Client::remove_from_playlist(&mut self, id: &str, uri: &str, snapshot: Option<&str>) -> Result<Option<String>, SpotifyError>`
- `Client::create_playlist(&mut self, name: &str) -> Result<SpPlaylist, SpotifyError>`
- A playlist id that is empty or not all ASCII alphanumeric → `Err(SpotifyError::BadAnswer("an odd playlist id"))` with no request.

- [ ] **Step 1: Write failing tests** in `client.rs` tests / `lib.rs` / `token.rs`:
  - `the_scopes_include_the_edit_permissions` — SCOPES contains the three scopes.
  - `editing_needs_all_three_scopes` — `can_edit` false for `None`, for `"streaming user-library-modify"`, true for all three.
  - `playlists_carry_their_id_owner_and_snapshot` — `playlists()[0]`: id `"plA"`, owner_id `"aria-tester"`, collaborative false, snapshot `Some("snapA")`; `[1]` collaborative true.
  - `liking_and_unliking_go_to_me_library` — seen[0] `PUT /v1/me/library` param `uris` = `spotify:track:t1`; seen[1] `DELETE` same.
  - `playlist_tracks_read_item_then_track_and_skip_the_rest` — two pages; result names in order; an episode, a `null` item and an unreadable entry skipped; a local file kept with `uri: None`; request has `limit=50`.
  - `adding_posts_the_uri_and_returns_the_snapshot` — 201 reply `snapshot.json`; body JSON equals `{"uris":["spotify:track:t1"]}`; returns `Some("snap2")`.
  - `removing_sends_the_items_and_the_snapshot` — DELETE body equals `{"items":[{"uri":"spotify:track:t1"}],"snapshot_id":"snapA"}`; with `None` the body has no `snapshot_id`.
  - `creating_posts_a_private_playlist` — body `{"name":"Mix","public":false}`; 201 `created-playlist.json` → id/name/owner_id read.
  - `an_odd_playlist_id_is_refused_without_a_request` — ids `"../me"`, `"a/b"`, `""` → `BadAnswer`, fake saw nothing.
- [ ] **Step 2: Run** `cargo test -p aria-spotify -j 4` — Expected: FAIL (missing methods/fields).
- [ ] **Step 3: Implement.** `Method::Delete` sends a body with ureq's `agent.delete(url).force_send_body().send_json(body)` and without one `.call()`; `send` accepts any 2xx (201 included). `SpPlaylist::from(RawPlaylist)` reads `id` (else the URI's last part), `owner.id`, `collaborative`, `snapshot_id`.
- [ ] **Step 4: Run** `cargo test -p aria-spotify -j 4` and `cargo clippy -p aria-spotify --all-targets -j 4 -- -D warnings` — Expected: PASS, no warnings.
- [ ] **Step 5: Commit** `feat(spotify): Web API calls to like tracks and edit playlists`

### Task 2: Likes and playlist edits in the Spotify service

**Files:**
- Modify: `crates/aria-core/src/spotify_service.rs` (+ tests), `crates/aria-core/src/runtime.rs` (route new requests if it matches requests exhaustively), `crates/aria-core/src/lib.rs` (re-exports)

**Interfaces:**
- Consumes: Task 1 methods and fields.
- Produces:
  - `SpotifyAccount::SignedIn { name: String, can_edit: bool }`
  - `SpotifyRequest::Like { track: SpTrack, liked: bool }`, `OpenPlaylist(String /*id*/)`, `AddToPlaylist { playlist: String, name: String, uri: String }`, `RemoveFromPlaylist { playlist: String, uri: String }`, `CreatePlaylist { name: String, first: Option<String> }`
  - `pub struct SpotifyPlaylist { pub id: String, pub tracks: Vec<SpTrack>, pub snapshot: Option<String>, pub loading: bool, pub failed: bool }`; `SpotifyEvent::Playlist(Option<Arc<SpotifyPlaylist>>)` (`None` = nothing open)
  - Notice keys: `spotify-added` (`playlist`), `spotify-created` (`playlist`), `spotify-reconnect-to-edit`.

Behaviour:
- After `me()`, the service keeps the account id; each loaded/created playlist gets `editable = owner_id == me.id || collaborative`.
- Without `can_edit`, every edit request sends nothing and gives `spotify-reconnect-to-edit` (Warning) once per request.
- `Like`: change `library.tracks` at once (liked: insert at index 0 unless present; unliked: remove every entry with that URI), emit Library, then call. On failure restore the entry as it was before *this* request (re-insert at its old index / remove the inserted one), emit Library, report through `fail`.
- `OpenPlaylist(id)`: emit `Playlist(Some{loading:true, tracks: [], snapshot from the library's playlist})`, load, emit with tracks; on failure `failed: true, loading: false`.
- `AddToPlaylist`: call; on success notice `spotify-added` (Info) and, if that playlist is open, update its snapshot and reload it.
- `RemoveFromPlaylist`: call with the open playlist's snapshot when it is that one; then reload it.
- `CreatePlaylist`: create, add `first` if given, insert the playlist at index 0 of `library.playlists` (editable), emit Library, notice `spotify-created` (Info).
- `SignOut` and a sign-in emit `Playlist(None)` and drop the open playlist.

- [ ] **Step 1: Write failing tests** (service Harness with the fake):
  - `signed_in_says_whether_the_session_can_edit` (stored token with/without scopes)
  - `own_and_collaborative_playlists_are_editable`
  - `a_like_shows_at_once_then_is_sent`
  - `a_refused_like_is_undone_and_reported`
  - `a_second_refused_like_keeps_the_first` (Review Focus 2)
  - `an_unlike_takes_the_track_out_at_once`
  - `opening_a_playlist_loads_its_tracks`
  - `a_playlist_spotify_refuses_shows_failed` (403; Review Focus 4)
  - `adding_says_added_and_reloads_the_open_playlist`
  - `removing_sends_the_snapshot_and_reloads`
  - `creating_a_playlist_adds_the_first_track_and_lists_it_first`
  - `edits_without_the_permissions_send_nothing_and_say_reconnect`
  - `signing_out_closes_the_open_playlist` (Review Focus 3)
- [ ] **Step 2: Run** `cargo test -p aria-core spotify_service -j 4` — Expected: FAIL.
- [ ] **Step 3: Implement** as above; update every `SignedIn { name }` match/constructor in core and app (app compile fixes only, behaviour in Task 3).
- [ ] **Step 4: Run** `cargo test -p aria-core -j 4`, `cargo check -p aria-app -j 4` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): like Spotify tracks and edit Spotify playlists`

### Task 3: Heart and context menu on Spotify tracks (app)

**Files:**
- Create: `crates/aria-app/src/ui/widgets/spotify_track_menu.rs`
- Modify: `crates/aria-app/src/spotify.rs` (SpotifyUi), `ui/widgets/mod.rs`, `ui/widgets/spotify_row.rs` (room on the right for the heart), `ui/pages/spotify_lists.rs` (track rows: heart + context menu), `ui/pages/mod.rs` (`NameFor::SpotifyPlaylist { first: Option<String> }`), `app.rs` (name dialog → `CreatePlaylist`; `Playlist` event), `ui/layout/player_bar.rs` (Spotify heart), i18n `KEYS`, `locales/{en,ru,hy}/*.ftl`

**Interfaces:**
- Consumes: Task 2 requests/events.
- Produces:
  - `SpotifyUi.liked: HashSet<String>` (rebuilt from each Library event), `SpotifyUi.playlist: Option<Arc<SpotifyPlaylist>>`, `fn can_edit(&self) -> bool`, `fn is_liked(&self, uri: &str) -> bool`.
  - `spotify_track_menu::heart(ui, tr, p, spotify: &SpotifyUi, track: &SpTrack) -> Option<PageAction>` — disabled with tooltip `spotify-reconnect-to-edit` when `!can_edit`, disabled without tooltip text change when `uri` is None.
  - `spotify_track_menu::items(ui, tr, spotify, track, open: Option<&SpPlaylist>, out: &mut Vec<PageAction>)` — like item (`spotify-like`/`spotify-unlike`), submenu `track-add-to-playlist` with editable playlists then `playlist-new`, and with `open` `spotify-remove-from-playlist` (hover text `spotify-remove-every-copy`).
- New keys: `spotify-like`, `spotify-unlike`, `spotify-remove-from-playlist`, `spotify-remove-every-copy`, `spotify-reconnect-to-edit`, `spotify-added`, `spotify-created`, `spotify-follow-only`, `spotify-playlist-back`, `spotify-playlist-play`, `spotify-playlist-empty`, `spotify-playlist-failed`.

- [ ] **Step 1: Write failing kittest tests:**
  - `a_liked_track_shows_a_selected_heart_that_unlikes` → `SpotifyRequest::Like{liked:false}`
  - `the_heart_says_to_reconnect_without_the_permissions`
  - `the_menu_lists_only_editable_playlists_then_new`
  - `new_playlist_asks_for_a_name_for_spotify` → `AskPlaylistName(NameFor::SpotifyPlaylist{first:Some(uri)})`
  - `remove_from_this_playlist_only_inside_an_open_playlist`
  - `a_track_without_a_uri_offers_no_heart_and_no_edits` (Review Focus 5)
  - `the_player_bar_heart_likes_the_spotify_track`
  - `the_spotify_name_dialog_creates_on_spotify` (app: confirming sends `CreatePlaylist{name, first}`)
  - `liked_set_follows_the_library` (SpotifyUi unit test)
- [ ] **Step 2: Run** `cargo test -p aria-app -j 4 spotify` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p aria-app -j 4` and `cargo test -p aria-core i18n -j 4` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): hearts and playlist menu on Spotify tracks`

### Task 4: Opening your own playlists on Your Library

**Files:**
- Modify: `crates/aria-app/src/ui/pages/library.rs` (+ tests), `ui/pages/spotify_lists.rs` (playlist row click: open if editable, else play with tooltip `spotify-follow-only`)

**Interfaces:**
- Consumes: Task 3 widget and `SpotifyUi.playlist`.
- Produces: Library page state `open: Option<String>` (egui temp data). Opening sends `SpotifyRequest::OpenPlaylist(id)`. The detail view: Back (`spotify-playlist-back`), the name as heading, Play (`spotify-playlist-play`, `PlayRequest::Context{uri, offset_uri: None}`), track rows whose click plays `PlayRequest::Context{uri, offset_uri: track uri}` and whose menu has `open` set; loading / failed / empty states. If the open id is no longer in the library playlists, the page goes back to the list.

- [ ] **Step 1: Write failing tests:**
  - `clicking_an_own_playlist_opens_it_and_asks_for_its_tracks`
  - `a_followed_playlist_still_plays_on_click`
  - `the_open_playlist_shows_back_play_and_its_tracks`
  - `back_returns_to_the_library`
  - `a_track_in_an_open_playlist_plays_the_playlist_from_it`
  - `an_open_playlist_that_failed_says_so`
- [ ] **Step 2: Run** `cargo test -p aria-app -j 4 library` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace -j 4` and `cargo clippy --workspace --all-targets -j 4 -- -D warnings`, `cargo fmt --check` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): open and edit your Spotify playlists`

### Task 5: Docs and version

**Files:** `Cargo.toml` workspace version 0.3.4 (and `Cargo.lock`), `CHANGELOG.md` `[0.3.4] - 2026-11-24 (Spotify likes and playlists)`, `README.md`, `docs/PRIVACY.md` (the new permissions and what they allow), `docs/ROADMAP.md` (0.3.4 done), `docs/testing/0.3.4-manual-check.md` (all pending; first line: press Reconnect).

- [ ] **Step 1:** Write the docs; state plainly nothing was tried against the real Spotify.
- [ ] **Step 2: Run** `cargo test --workspace -j 4` — Expected: PASS with the new count.
- [ ] **Step 3: Commit** `docs: 0.3.4 — Spotify likes and playlists`
