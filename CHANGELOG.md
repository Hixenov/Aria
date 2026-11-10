# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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
