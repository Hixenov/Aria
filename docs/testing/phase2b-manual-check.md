# Phase 2b manual check

Checklist for the 0.2.1 pre-release (spec section 10): favorites, playlists, playback history and
the Home page. Each line says who verified it:

- **agent**: run by the coding agent on the Windows PC described in the README, with no sound played and no keyboard or mouse input sent to the app.
- **user**: has to be done by a person with the app in front of them. Audio cannot be judged by an agent.
- **pending**: not verified by anyone yet. Replace `pending` with `user` (and the date) when you have checked it.

Run the release build (`cargo build --release --locked`, then `target\release\aria.exe`). Use a
library of your own with at least a few dozen tracks, and some albums.

**Nobody has run the 0.2.1 app by hand.** The agent that wrote it could not click or see the
window, so nothing in the window has been looked at by a person: the hearts, Liked, Playlists,
Home and the history settings.

## What automated tests cover

These run in `cargo test --workspace --locked`. They are real tests, but none of them replaces
looking at the window.

| Area | What is tested |
|------|----------------|
| Database | A 0.2.0 database (folders, tracks, queue) is migrated to version 2 with nothing lost. Favorites, playlist entries and plays are kept by path: they hide with their folder and come back in their old order under the new track ids. |
| Rules | Playlist names are trimmed and counted in characters (empty, 100 and 101 characters, Armenian, emoji). Positions stay dense after every move and removal, including with unavailable entries between visible ones. History keeps the newest 10,000 plays; `recent` lists each track once. |
| Core | Each command leads to the expected `Collections` event; invalid ids and names change nothing. A play counts after 30 seconds of playing, and not for time spent paused, for a seek past 30 s, for a skip at 29 s, or with history off. A track shorter than 30 s counts when it ends. With repeat-one a track counts once each time round. A seek back (such as Previous restarting a track) adds no time, even if a tick comes before the engine shows the new position. Clear history. |
| App | Pure parts: Recently added, table rows for a playlist with unavailable entries, name validation, ids the library snapshot does not know are skipped. Pages are drawn in a test harness (egui_kittest): Liked, Playlists, Home and the Settings history controls. Dragging an entry to reorder is covered by a simulated pointer-event test, not by a person. |
| Speed | The ignored release test for `Collections` on a 50,000-track library (see the README, Performance). |

## What you need to check by hand

| Check | Who |
|-------|-----|
| Heart column in the track tables (Local Music, Liked, Playlists): click a heart to favorite a track, click it again to remove it. A favorite is the outline heart in the accent colour, not a filled one, so check that you can tell it apart from the others in both themes | pending |
| Heart beside the title in the player bar: shows the current track's state and toggles it; it matches the table's heart | pending |
| Row context menu: "Add to favorites" / "Remove from favorites" | pending |
| Liked page lists your favorites, newest first; search, sorting and the row actions work; playing starts from the clicked row | pending |
| Liked page with no favorites shows its empty state | pending |
| Create a playlist: it appears in the list (it does not become selected by itself; that is known) | pending |
| Rename a playlist, including a name of 100 characters; an empty name or one of 101 characters cannot be confirmed | pending |
| Delete a playlist: it asks first; cancel keeps it; confirm removes it | pending |
| Add a track to a playlist from the row context menu | pending |
| Add a track to a new playlist from the row context menu ("New playlist...") | pending |
| Drag an entry to a new position in a playlist; it stays there after leaving the page and coming back, and after restarting Aria | pending |
| Remove an entry from the row context menu | pending |
| Play a playlist by clicking a row: playback starts from that row and continues through the playlist | pending |
| A playlist survives removing its music folder in Settings, Library and adding it again: the entries come back in their old order, and "N tracks unavailable" shows while the folder is gone | pending |
| Favorites survive the same remove and add | pending |
| Long playlist names next to Rename and Delete: the name and both buttons stay usable and nothing is cut off, in a wide window and at 900 px wide, in English, Russian and Armenian | pending |
| Home and Playlists row layout: cover, title and artist (Home), and grip, heart, title, artist and duration (Playlists), do not overlap or get cut off with long titles and artists, in a wide window and at 900 px wide | pending |
| Home lists a track under "Recently played" after it has played for 30 seconds, and not before; a click on a row plays that list from there | pending |
| Home "Recently added" shows the newest 20 tracks; with an empty library the page points to Settings, Library | pending |
| Settings, Playback: switch "Keep playback history" off; tracks played afterwards do not appear in "Recently played" | pending |
| Settings, Playback: "Clear playback history" asks first and empties "Recently played"; favorites and playlists stay | pending |
| Upgrade: start 0.2.1 on a copy of a 0.2.0 data folder; the folders, tracks and the saved queue are all still there | pending |
| Aria does not change your music files (compare a file's modified time before and after using these features) | pending |

## Translations

The Russian and Armenian texts for the Home, Liked and Playlists pages, the heart, the row menu,
the name dialog, the confirmations, the notices and the history settings were written by the
coding agent and have not been read by a native speaker. Please read them in both languages
(Settings, Language) and report anything wrong, awkward or cut off. The strings are in
`locales/ru/main.ftl` and `locales/hy/main.ftl`.

| Check | Who |
|-------|-----|
| Russian texts of the new screens read by a native speaker | pending |
| Armenian texts of the new screens read by a native speaker | pending |
