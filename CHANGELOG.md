# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.2.1] - unreleased (pre-release: Phase 2b, favorites, playlists, history)

Nobody has run this version by hand yet: see `docs/testing/phase2b-manual-check.md`, where every line for a person is still pending.

### Added

- Favorites: a heart column in every track table and a heart beside the title in the player bar, "Add to favorites" and "Remove from favorites" in the row menu, and a Liked page with the same table, search, sorting and row actions as Local Music.
- Playlists: a Playlists page to create, rename and delete playlists (Delete asks first); add a track from the row menu, either to a playlist in the list or to a new one ("New playlist..."); remove an entry from the row menu; reorder entries by dragging; play a playlist from any row. A line shows how many tracks of a playlist are unavailable.
- Playback history: a play counts after the track has played for 30 seconds. "Recently played" holds up to 20 tracks, each once; the newest 10,000 plays are kept.
- Home page: "Recently played" and "Recently added" (20 tracks); a click plays that list from that row.
- Settings, Playback: "Keep playback history" and "Clear playback history" (asks first).
- Favorites, playlist entries and plays are stored by file path, so they come back, in their old order, when a music folder is removed and added again. A 0.2.0 library database is upgraded in place (database version 1 to 2); the migration only adds tables.
- English, Russian and Armenian texts for all of the above. The Russian and Armenian texts were written by the coding agent and have not been read by a native speaker.
- `docs/testing/phase2b-manual-check.md`.

### Changed

- The track table of Local Music is now one widget shared by Local Music, Liked and Playlists.
- Version 0.2.1.

### Known limitations

- A favorite is shown as the outline heart in the accent colour, not a filled heart, because only the regular icon weight is bundled. The difference between a favorite and any other track is colour only.
- A file that is renamed or moved is a new track: it leaves favorites and playlists, and its playlist entries stay hidden until a file appears at the old path again.
- One track at a time goes into a playlist.
- A new playlist does not become the selected one automatically.
- No M3U import or export, no full history page, no play counts.
- Nobody has looked at the hearts, Liked, Playlists (including drag to reorder, which an automated pointer test covers but no person has tried), Home or the history settings in a running window. Layout with long playlist names or long titles is unchecked.
- The Russian and Armenian texts are not reviewed by a native speaker.
- The Windows installer and release `aria.exe` for 0.2.1 are not covered by this entry: they are built and tested separately before any release.
- The limitations listed for 0.2.0 still apply.

## [0.2.0] - 2026-11-10 (pre-release: Phase 2a, local player)

### Added

- Local library: add, remove and rescan music folders; a background scan reads tags and embedded artwork into a SQLite database (`library.db`) and a thumbnail cache. Aria never writes to or deletes your audio files.
- Local Music page with Tracks, Albums and Artists tabs, search, sorting and a virtualized table.
- Audio engine: Symphonia decoding (MP3, FLAC, WAV, AAC, ALAC, Ogg Vorbis, and Opus through the bundled libopus), rubato resampling, CPAL output, seeking, volume and mute, gapless transitions, output device selection and recovery when a device disappears.
- Working player bar and play queue (play next, add to queue, remove, reorder by dragging, clear); the queue is saved and restored paused at the next start.
- Playback and Library sections in Settings.
- English, Russian and Armenian texts for all of the above. The Russian and Armenian texts have not been read by a native speaker.
- Space toggles play and pause.
- Documentation: measurements on one Windows PC in the README, and a manual check list in `docs/testing/phase2a-manual-check.md`.
- Windows installer (`Aria-<version>-setup.exe`, Inno Setup script in `packaging/windows/aria.iss`): per-user, no administrator rights, Start-menu shortcut and uninstaller. Uninstalling keeps settings and the library. The portable `aria.exe` is still offered.

### Changed

- Version 0.2.0.

### Known limitations

- Files changed while Aria runs appear after a rescan or the next start.
- Gapless trimming depends on data in the file; formats without it may keep a small gap.
- Real audio output is to be verified on Windows only (by hand, pending); Linux and macOS are built and tested in CI only.
- Aria cannot edit tags.
- The audio thread runs at normal priority, so heavy system load can cause glitches.
- Hand verification by a person (listening to each format, gapless, unplugging headphones) is pending; see the manual check list.

## [0.1.0] - 2026-11-05 (pre-release: Phase 1, foundation)

### Added

- Native egui/eframe window with the final layout shell: sidebar, content area, bottom player bar and collapsible right panel.
- Nine routes with localized "Available in Phase N" empty states; navigation by mouse, `Ctrl+1...9`, `Alt+Left/Right` and mouse back/forward buttons.
- Theme system: dark, light and system themes with separate accent colours.
- Settings page and persistent `settings.toml`, with backup and defaults on corruption.
- Localization in English, Russian and Armenian, switchable live.
- File logging with daily rotation, panic logging and startup stage timings.
- Release builds on Windows open no console window; a failure to start is shown in a message box.
- GitHub Actions CI on Windows, Linux and macOS (build and tests; only Windows run by hand), and `cargo-deny` checks.
- Repository documentation and MIT / Apache-2.0 licenses.
