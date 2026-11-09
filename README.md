[![CI](https://github.com/Hixenov/Aria/actions/workflows/ci.yml/badge.svg)](https://github.com/Hixenov/Aria/actions/workflows/ci.yml)

# Aria

Aria is a native, lightweight, cross-platform desktop music player written in Rust with
egui/eframe. Over six phases it grows into a local music player, a Spotify library and
remote-control client, a lyrics viewer, an equalizer and visualizer, and a tray/mini-player
app. See the [roadmap](docs/ROADMAP.md).

Spotify audio is not streamed or decoded inside Aria; Spotify support means library access and remote control of your existing Spotify devices, as explained in the [roadmap](docs/ROADMAP.md#the-spotify-playback-limitation).

## Status: Phase 2a (local player), version 0.2.0

Phase 1 built the shell: window and layout, themes, settings, localization, logging and CI.
Phase 2a turns it into an offline music player for local files:

- Add, remove and rescan music folders. A background scan builds the library (SQLite) and reads tags and embedded artwork. Aria never writes to or deletes your audio files.
- Local Music page with Tracks, Albums and Artists tabs, search, sorting and a virtualized table
- Aria's own audio engine (Symphonia decoding, rubato resampling, CPAL output): play, pause, seek, volume and mute, gapless transitions, output device choice, recovery when a device disappears
- Play queue (play next, add to queue, remove, drag to reorder, clear), kept between runs and restored paused
- Playback and Library sections in Settings; English, Russian and Armenian

Formats with automated test coverage: MP3, FLAC, WAV, AAC and ALAC in `.m4a`, Ogg Vorbis, and Opus (through the bundled libopus). The scanner also picks up `.aac`, `.mp4`, `.oga`, `.mka`, `.aif`, `.aiff` and `.caf` files, but those were not tested with real audio, so they may not play. Playing each format by ear is on the manual check list and has not been done yet.

Not yet: favorites, playlists and playback history (Phase 2b), the soundboard (2c), Spotify (Phase 3), tray and media keys (Phase 4), lyrics and equalizer (Phase 5). See the [roadmap](docs/ROADMAP.md).

What has been checked by hand is recorded in [docs/testing/phase2a-manual-check.md](docs/testing/phase2a-manual-check.md). Anything marked pending there has not been verified by a person yet, and that includes listening to audio.

Linux and macOS are built and tested in CI only. Nobody has run the app on them.

## Build, run, test

The toolchain is pinned to Rust 1.99.1 by `rust-toolchain.toml`; rustup installs it automatically.

```sh
cargo run -p aria-app                 # run the app
cargo build --release --locked        # release build
cargo test --workspace --locked       # tests
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

### Linux development packages

```sh
sudo apt-get install -y libxkbcommon-dev libwayland-dev libxcb-render0-dev \
  libxcb-shape0-dev libxcb-xfixes0-dev libgl1-mesa-dev
```

### Git on exFAT or other non-NTFS drives

On drives without ownership information (for example exFAT), git may refuse to work with
"dubious ownership". Mark the repository as safe for your user:

```sh
git config --global --add safe.directory /path/to/aria
```

## Configuration and data

`ARIA_CONFIG_DIR` overrides the config directory only. See [docs/PRIVACY.md](docs/PRIVACY.md) for the exact paths of everything Aria stores.

## Performance (Phase 2a, measured)

Release build (`cargo build --release --locked`) on one machine: Intel Core i3-1315U (4 performance + 4 efficiency cores), 7.7 GB RAM (about 83 % in use at the time), Windows 11 Pro for Workstations 10.0.26300, integrated graphics.

Library used: 4,000 generated files (about 485 MB), 40 artists, 200 albums, 3-second tones in MP3, FLAC, Ogg Vorbis, Opus, M4A (AAC) and WAV. Tags differ per album; every track in an album shares one title.

| What | Result |
|------|--------|
| First frame, first launch (database created, library scanned) | 337 ms |
| First frame, later launches (7 launches) | 179-243 ms |
| First frame, same build with no folders added (3 launches) | 167-202 ms |
| Scan of the 4,000 files, first launch | finished 0.6 s after the first frame |
| Rescan of the unchanged 4,000 files | finished 0.3 s after the first frame |
| Working set after the scan, first launch | 143.5 MB (private bytes 71 MB) |
| Working set when idle, later launches | 127-129 MB (private bytes 64-66 MB) |
| Working set when idle, no folders added | 122.5-123.2 MB |
| CPU when idle (window open, nothing playing) | 0.08-0.47 % of one core, in four 20-second windows |
| `PRAGMA quick_check` on the 4,000-track database | 82 ms on the first run in a fresh process, 3-4 ms afterwards |
| Library snapshot of a synthetic 50,000-track database | 93 ms |

- The first-frame times are Aria's own `startup stage=first_frame` log lines. The scan and `quick_check` run on the library thread, not before the first frame, so the first frame does not wait for them.
- Memory is the process working set read from Windows, a few seconds after launch and again after about 90 seconds.
- Idle CPU is the change in the process's total processor time over each window. A screenshot taken with `PrintWindow` just before one measurement coincided with a much higher reading (14.5 % of a core over 60 s); it was not reproduced without the screenshot, and its cause was not investigated.
- The scan is quick here because the files are 3 seconds long. Real albums with large embedded artwork will take longer.
- CPU while playing has not been measured: playing needs input that an agent may not send. It is left to the person checking by hand (see the manual check).
- These numbers come from one machine and are not a benchmark. Earlier Phase 1 figures (an empty shell, no audio engine) were 94 MB idle.

## Known limitations

Phase 2a:

- Files changed while Aria runs appear after a rescan or the next start.
- Gapless trimming depends on data in the file. Formats without it may keep a small gap.
- Real audio output is verified on Windows only.
- Aria cannot edit tags.
- The audio thread runs at normal priority, so heavy system load can cause glitches.
- The Russian and Armenian texts for the new features have not been read by a native speaker.

From Phase 1, still true:

- Spotify audio is not streamed in-app. This is a compliance choice; see the [roadmap](docs/ROADMAP.md#the-spotify-playback-limitation).
- The Armenian translation needs native-speaker review, and Armenian headings render at regular weight because there is no Armenian semibold font.
- egui's colour-picker popup and the panel splitter's accessibility label are English only.
- Alt+Left/Right and Ctrl+1...9 are app-wide shortcuts.
- `ARIA_CONFIG_DIR` overrides only the config directory. Logs, the library database, the artwork cache and the window state still go to the OS defaults.
- Only Windows has been run by hand (on one machine). Linux and macOS are built and tested by CI only; nobody has opened the app on them yet.
- In the narrowest layout (a 900 px window with the widest sidebar and the queue open), the Russian theme switch does not fit on one line, so its "System" button wraps under the other two inside one outline.
- Release builds on Windows open no console window, so nothing printed to stderr is visible. Diagnostics go to the log files and `panic.log` (see [docs/PRIVACY.md](docs/PRIVACY.md) for where), and a failure to start is also shown in a message box. To see stderr, run a debug build (`cargo run -p aria-app`) from a terminal.
- If any one of Aria's folders (config, data, cache or logs) cannot be created at startup, settings are read-only for the session and file logging (including `panic.log`) is off, even if the other folders are fine.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Bundled fonts and third-party components: see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
