[![CI](https://github.com/Hixenov/Aria/actions/workflows/ci.yml/badge.svg)](https://github.com/Hixenov/Aria/actions/workflows/ci.yml)

# Aria

Aria is a native, lightweight, cross-platform desktop music player written in Rust with
egui/eframe. Over six phases it grows into a local music player, a Spotify library and
remote-control client, a lyrics viewer, an equalizer and visualizer, and a tray/mini-player
app. See the [roadmap](docs/ROADMAP.md).

Spotify audio is not streamed or decoded inside Aria; Spotify support means library access and remote control of your existing Spotify devices, as explained in the [roadmap](docs/ROADMAP.md#the-spotify-playback-limitation).

## Status: Phase 1 (Foundation)

Phase 1 delivers the shell every later phase builds on:

- Window with the final layout: sidebar, content area, bottom player bar, collapsible right panel
- Nine routes (Home, Search, Your Library, Liked Songs, Playlists, Albums, Artists, Local Music, Settings); pages owned by later phases show a localized empty state
- Dark, light and system themes with separate accent colours
- Persistent settings (`settings.toml`), with backup and defaults when the file is corrupt
- Localization: English, Russian, Armenian
- File and panic logging
- CI on Windows, Linux and macOS: format, lint, tests (including headless UI tests) and release build

There is no audio playback yet.

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

`ARIA_CONFIG_DIR` overrides the config directory. See [docs/PRIVACY.md](docs/PRIVACY.md) for the exact paths of everything Aria stores.

## Performance (Phase 1, measured)

Release build, measured on one machine (the developer's Windows 11 PC):

- First frame: 213 ms cold, 93-127 ms warm
- Idle working set: 94.1 MB
- Idle CPU: 0 %

These numbers come from a single machine and are not a benchmark.

## Known limitations (Phase 1)

- Spotify audio is not streamed in-app. This is a compliance choice; see the [roadmap](docs/ROADMAP.md#the-spotify-playback-limitation).
- The Armenian translation needs native-speaker review, and Armenian headings render at regular weight because there is no Armenian semibold font.
- egui's colour-picker popup and the panel splitter's accessibility label are English only.
- Alt+Left/Right and Ctrl+1...9 are app-wide shortcuts.
- `ARIA_CONFIG_DIR` overrides only the config directory. Logs, cache and the window state still go to the OS defaults.
- Only Windows has been run by hand (on one machine). Linux and macOS are built and tested by CI only; nobody has opened the app on them yet.
- In the narrowest layout (a 900 px window with the widest sidebar and the queue open), the Russian theme switch does not fit on one line, so its "System" button wraps under the other two inside one outline.
- Release builds on Windows open no console window, so nothing printed to stderr is visible. Diagnostics go to the log files and `panic.log` (see [docs/PRIVACY.md](docs/PRIVACY.md) for where), and a failure to start is also shown in a message box. To see stderr, run a debug build (`cargo run -p aria-app`) from a terminal.
- If any one of Aria's folders (config, data, cache or logs) cannot be created at startup, settings are read-only for the session and file logging (including `panic.log`) is off, even if the other folders are fine.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Bundled fonts: see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
