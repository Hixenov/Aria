# Aria 0.3.4 — Liking Spotify tracks and editing Spotify playlists

Approved by the user 2026-11-24 ("A: add, open, remove", then "Approve").

## Intent

The roadmap promised for 0.3.x that Spotify tracks can be liked and Spotify playlists edited from
Aria. Today Spotify tracks have no heart, no context menu, and a Spotify playlist only plays when
clicked. 0.3.4 gives Spotify tracks what local tracks already have: a heart, "Add to playlist",
"New playlist…", and the user's own playlists can be opened to see and remove their tracks.

## Goals

- A heart on every Spotify track row (Library, Search, an opened playlist) and in the player bar
  while a Spotify track plays. Clicking it saves the track to Liked Songs or removes it. The heart
  changes at once; when Spotify refuses, it changes back and a notice says why.
- Right-click on a Spotify track row: "Add to Liked Songs" / "Remove from Liked Songs",
  "Add to playlist" (a submenu of the playlists the user owns or collaborates on, then
  "New playlist…"). In an opened playlist also "Remove from this playlist", which says that
  Spotify removes every copy of the track in that playlist.
- "New playlist…" asks for a name, creates a **private** playlist, adds the track, and the new
  playlist appears at the top of the user's playlists.
- On Your Library, clicking a playlist the user owns or collaborates on opens it: a Back button,
  its name, a Play button, its tracks. A track click there plays the playlist from that track.
  Playlists the user only follows still play on click, and their tooltip says Spotify does not
  show what is in playlists you only follow.
- A like shows at the top of Liked Songs at once; an unlike removes it from the list at once.
- An open playlist reloads after every edit made to it.

## Non-goals

Renaming, reordering or deleting playlists; liking albums, artists or playlists; following
playlists; detecting duplicates before adding (Spotify allows them; a track always goes to the
end); playlist cover editing.

## Anti-goals

- Any private or undocumented Spotify endpoint. Only the documented Web API endpoints below.
- Logging or showing tokens. No new data leaves the computer other than these API calls.
- Claiming any of this was tried against the real Spotify: tests run only against the local fake.

## Constraints (Spotify, February 2026 rules for Development Mode apps)

- Save / remove from Liked Songs: `PUT /v1/me/library?uris=<uri>` and
  `DELETE /v1/me/library?uris=<uri>` (comma-separated URIs in the query, max 40; answer 200,
  empty). `PUT/DELETE /me/tracks` were removed. `GET /me/tracks` (reading Liked Songs) still works.
- Playlist items: `GET /v1/playlists/{id}/items?limit=50` (pages; each entry has `item`, the old
  `track` key is deprecated — read `item`, fall back to `track`). Only for playlists the user owns
  or collaborates on; others answer 403.
- Add: `POST /v1/playlists/{id}/items` with body `{"uris":[...]}` (max 100), answer 201
  `{"snapshot_id": ...}`.
- Remove: `DELETE /v1/playlists/{id}/items` with body `{"items":[{"uri":...}],"snapshot_id":...}`,
  answer 200 `{"snapshot_id": ...}`. Spotify removes every occurrence of the URI.
- Create: `POST /v1/me/playlists` with body `{"name":...,"public":false}`, answer 201 with the
  playlist object. `POST /users/{id}/playlists` was removed.
- `GET /me/playlists` items have `owner.id`, `collaborative`, `snapshot_id`; ownership is
  `owner.id == /me id`.
- New scopes: `user-library-modify playlist-modify-public playlist-modify-private`. An existing
  session lacks them; the user presses Reconnect once. Until then the hearts and the edit menu
  items are disabled with a tooltip saying to reconnect in Settings → Spotify.
- Existing project rules: no unwrap/expect outside tests, no unsafe, no `#[allow]`; never log
  tokens; never use the client secret.

## Approaches

1. **(Chosen) Extend the existing service and pages.** New `Client` methods in `aria-spotify`, new
   `SpotifyRequest`s handled on the Spotify thread, the liked set and the open playlist held in the
   UI state, the context menu and heart as a widget shared by every Spotify row. Fits how 0.3.0
   built everything; one thread keeps ordering simple.
2. A separate edit queue/thread with retries. Rejected: edits are rare, one at a time, and the
   existing thread already serialises calls and handles refresh/429.
3. Re-load the whole library after every edit instead of updating it in place. Rejected: slow for
   big libraries and the heart would not change at once.

## Decided details

- Whether a track is liked comes from the loaded Liked Songs (all pages are read, up to 50 000);
  the UI keeps a set of their URIs. No `/me/library/contains` calls.
- A track without a URI (a local file inside a Spotify playlist) has a disabled heart and no
  edit items at all, "Remove from this playlist" included (Spotify needs a URI for each).
- Episodes inside a playlist are skipped, as elsewhere.
- Account state carries `can_edit` (the session has all three new scopes).
- Notices: added ("Added to {playlist}"), created ("Created {playlist}"), removed nothing extra
  (the list reloads); failures use the existing Spotify failure notices, plus
  "Reconnect in Settings → Spotify to like and edit" for a 403 while the scopes are missing.

## Left to the builder

Layout of the heart inside a Spotify row, exact message wording in ru/hy, how the open playlist is
remembered on the Library page (egui temp state is fine), test structure.
