[![CI](https://github.com/Hixenov/Aria/actions/workflows/ci.yml/badge.svg)](https://github.com/Hixenov/Aria/actions/workflows/ci.yml)

# Aria

Aria is a native, lightweight, cross-platform desktop music player written in Rust with
egui/eframe. Over six phases it grows into a local music player, a Spotify library and
remote-control client, a lyrics viewer, an equalizer and visualizer, and a tray/mini-player
app. See the [roadmap](docs/ROADMAP.md).

Spotify audio is not streamed or decoded inside Aria; Spotify support means library access and remote control of your existing Spotify devices, as explained in the [roadmap](docs/ROADMAP.md#the-spotify-playback-limitation).

## Status: Phase 2c (lossless output and the soundboard), version 0.2.2 (pre-release)

Version 0.2.2 adds an optional exclusive output on Windows, a Lossless / Hi-Res badge and filter in the library, and a soundboard with global hotkeys, on top of 0.2.1 (below). **Nobody has run 0.2.2 by hand and nobody has listened to it.** The agent that wrote it tested it with automated tests, and checked on one PC that WASAPI exclusive mode opens the default device and plays silence; no person has heard exclusive output, a sound, or a hotkey with the real app. Every line for a person in [docs/testing/phase2c-manual-check.md](docs/testing/phase2c-manual-check.md) is still pending. The Windows installer and `aria.exe` for 0.2.2 are built and tested separately before any release; this README does not claim they have been.

### Exclusive mode and what "bit-perfect" means here

Settings, Playback has an "Exclusive mode" switch (Windows only, off by default). When it is on, Aria sends the track's samples to the output device through WASAPI exclusive mode, at the track's own sample rate, instead of through the system mixer.

- While it plays, other programs are silent on that device. Aria releases the device 15 seconds after you pause.
- While Aria holds the device exclusively, its volume slider is locked at 100 % (mute still works); set the level on the device or amplifier. If exclusive mode is refused and the music goes through the system mixer, the slider is free and sets the level as usual; it locks again when an exclusive output opens. The saved volume comes back when you turn the mode off. A saved volume of 0 (not muted) plays at full level too, as the locked slider shows; the 0 is kept and used again when the lock lifts.
- If the device refuses exclusive mode, playback goes on through the system mixer and a notice says so. If another program holds the device exclusively, Windows refuses Aria's shared stream as well, and Aria cannot play on that device until it is released.
- After a refused exclusive open Aria stays on the system mixer for that output: exclusive mode is tried again only when the output is opened again (after a pause of more than 15 seconds, at a device change, at a join that needs another format, or when you turn the mode off and on), not as soon as the device is free.
- If the device has no format at the track's rate, Aria opens the nearest rate it supports and resamples, and the indicator says "Resampled".
- The player bar shows the track's format (for example "FLAC 24/96") and one state: "Bit-perfect", "Resampled", "Converted", "Mixed with a sound" or "System mixer". A tooltip explains each.

"Bit-perfect" is shown only when all of these hold: exclusive output, an integer source of 24 bits or fewer, the music at full gain, no resampling, nothing else mixed in, and the channels unchanged (mono played on both channels counts as unchanged). Automated tests check that the mixer's path is exact in that case for 16- and 24-bit integer sources, against the decoder's own conversion. What it does **not** mean:

- The 5 ms fades when playback starts, pauses, resumes or seeks are not bit-exact. Gapless joins between tracks have no fade.
- ALAC is never shown as bit-perfect (its decoder reports no bit depth, so it shows "Converted"). Float sources and integer sources wider than 24 bits are never bit-perfect either.
- It is a claim about what Aria hands to the output. Whether the device receives the samples unchanged cannot be recorded through WASAPI loopback; it is a check by hand, best with a DAC that shows its input rate, and nobody has done it.
- Where the next track needs a different sample rate, a wider bit depth than the open output carries, or more channels, the output is closed and opened again, with a short silence at that join. Tracks in the same format stay gapless.
- The output thread runs at normal priority, so under heavy load there may be dropouts. Not checked.
- The log level "Trace" (Settings, Log level) is for diagnostics only. When Aria starts with it, or with `ARIA_LOG=trace`, the WASAPI output thread makes a log call, which takes a lock, on every device period, even though the line is filtered out (found by reading the code; its cost was not measured). Use a lower level for listening.

Library: every track now stores its bit depth. The track tables (Local Music and Liked) have a Quality column with a "Lossless" badge (FLAC, ALAC, and WAV or AIFF whose data is integer or float PCM) or a "Hi-Res" badge (lossless, and above 48 kHz or above 16 bits), and the Tracks tab has a "Lossless only" toggle. CAF files are skipped by the library scan (as in 0.2.1), so no CAF track appears in the library. WAV and AIFF files coded as A-law, µ-law or another compression get no badge (the codec shows as "ALAW", "ULAW", "WAV-COMPRESSED" or "AIFF-COMPRESSED"); ADPCM WAV files are skipped by the scan, because lofty cannot read data of 4 bits per sample.

### Soundboard and voice chat

The Soundboard page holds up to 100 sounds. A sound is a reference to an audio file (any format Aria plays); Aria never copies, changes or deletes the file. Names are 1 to 50 characters. Each sound has a volume and an optional global hotkey, and there is one hotkey for "Stop all". A click or the hotkey plays the sound from its start; firing a playing sound restarts it; at most 6 sounds play at once, and a seventh stops the one that started first. Sounds are not tracks: they do not enter the queue or the history. A missing file gives a notice when fired.

Settings, Soundboard chooses where sounds go: "My device" (the music's device, the default), "Second device" or "Both", and the second device itself. The second device is opened in shared mode when a sound needs it and closed 15 seconds after the last sound; with no second device chosen, "Second device" and "Both" play on your own device.

- Sounds on your own device are mixed into the music stream, so they break bit-perfect output while they play (the indicator shows "Mixed with a sound"), and they are cut when the music output is closed or reopened (a rate, depth or channel change at a join, a device change, switching exclusive mode, a device failure). A sound removed while it plays keeps playing until it ends.
- A sound fired while exclusive mode is on and no music plays opens your device exclusively, at the sound's own format: other programs (a video, a call) are silent on that device until 15 seconds after the last sound.
- The player's volume and mute do not apply to sounds; each sound has its own volume, and "Stop all" stops them.
- Opening and closing the second device run on the engine thread, which holds about one second of music. A device that is slow to open (Bluetooth, a sleeping USB interface; the wait is up to 7 s) or to close (up to 2 s) can briefly interrupt the music when a sound fires, or 15 seconds after the last one. A second device that fails to open is tried again, with a notice, on every sound.
- **Voice chat.** To be heard in a voice chat, the second device must be a virtual audio cable that the chat program uses as its microphone. Aria does not provide or install one, and never reads the microphone. With only a cable, the chat hears the sounds and not your voice; mixing the two needs another program. No cable and no chat program were tested.
- **Hotkeys** work while Aria runs, including minimized (a press may take about 100 ms to act then), with a switch in Settings that turns them all off. A hotkey must include Ctrl, Alt or Win unless it is F1 to F24 or a numpad key. One used by another sound or by Aria itself (Ctrl+0 to Ctrl+9 and Ctrl+Alt+R; Command on a Mac), or held by another program, is refused and not saved. Aria receives only the registered combinations from the system; it installs no keyboard hook and reads no other keystrokes. In the capture dialog the Win key, numpad `*` and `.`, and Ctrl+C, Ctrl+X and Ctrl+V cannot be captured, and numpad keys are chosen with a "Numpad" switch. Letters follow the keyboard layout. A global hotkey takes its combination from every other program while Aria runs, not only from Aria: for example, Ctrl+S as a sound's hotkey stops saving in editors. Turn the "Global hotkeys" switch off to give them back. On one PC a synthetic key press reached a test window while it was unfocused and minimized; the real app has not been run with hotkeys, and games in the foreground are not tested.
- Exclusive output and hotkeys are built for Linux and macOS where the crates support them, but verified on Windows only. Exclusive output does not exist there.

### Upgrading from 0.2.1

The library database is upgraded in place (version 2 to 3), and every file is read once more on the first start, so a big library may take a while. Favorites, playlists and history are kept. The automated migration test passes; nobody has tried it on a real 0.2.1 library. After the upgrade 0.2.1 no longer opens it: it says the library was saved by a newer version and runs with the library turned off, leaving the file untouched. Copy the data folder first if you may want to go back to 0.2.1.

English, Russian and Armenian; the new Russian and Armenian texts were written by the coding agent and not read by a native speaker.

### Phase 2b (favorites, playlists, history), version 0.2.1 (pre-release)

Version 0.2.1 adds favorites, playlists, playback history and a Home page on top of the 0.2.0 local player (below). **Nobody has run 0.2.1 by hand**: the hearts, the Liked and Playlists pages (including drag to reorder, which an automated pointer-event test covers but no person has tried), Home and the history settings have not been looked at in a running window. Every line for a person in [docs/testing/phase2b-manual-check.md](docs/testing/phase2b-manual-check.md) is still pending. The Windows installer and `aria.exe` for 0.2.1 are built and tested separately before any release; this README does not claim they have been.

- Favorites: a heart in every track table and in the player bar, and a Liked page
- Playlists: create, rename and delete; add a track from the row menu (to a playlist or to a new one); remove an entry; reorder by dragging; play from any row
- Playback history: a play counts after 30 seconds; Home shows "Recently played" (20 tracks) and "Recently added" (20 tracks)
- Settings, Playback: "Keep playback history" and "Clear playback history". Favorites, playlists and history stay on this computer (see [docs/PRIVACY.md](docs/PRIVACY.md))
- Favorites and playlist entries are kept by file path, so they come back when a folder is removed and added again. A 0.2.0 library database is upgraded in place; the automated migration test passes, and nobody has yet tried it on a real 0.2.0 library. After the upgrade 0.2.0 no longer opens it: it says the library was saved by a newer version and runs with the library turned off, leaving the file untouched. Copy the data folder first if you may want to go back to 0.2.0
- English, Russian and Armenian; the new Russian and Armenian texts were written by the coding agent and not read by a native speaker

### Phase 2a (local player), version 0.2.0

Phase 1 built the shell: window and layout, themes, settings, localization, logging and CI.
Phase 2a turns it into an offline music player for local files:

- Add, remove and rescan music folders. A background scan builds the library (SQLite) and reads tags and embedded artwork. Aria never writes to or deletes your audio files.
- Local Music page with Tracks, Albums and Artists tabs, search, sorting and a virtualized table
- Aria's own audio engine (Symphonia decoding, rubato resampling, CPAL output): play, pause, seek, volume and mute, gapless transitions, output device choice, recovery when a device disappears
- Play queue (play next, add to queue, remove, drag to reorder, clear), kept between runs and restored paused
- Playback and Library sections in Settings; English, Russian and Armenian

Formats with automated test coverage: MP3, FLAC, WAV, AIFF, CAF, AAC and ALAC in `.m4a`, Ogg Vorbis, and Opus (through the bundled libopus). The scanner also picks up `.aac`, `.mp4`, `.oga`, `.mka` and `.aif` files, but those were not tested with real audio, so they may not play. Playing each format by ear is on the manual check list and has not been done yet.

Not yet: Spotify (Phase 3), tray and media keys (Phase 4), lyrics and equalizer (Phase 5). See the [roadmap](docs/ROADMAP.md).

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

## Windows installer

[Inno Setup](https://jrsoftware.org/isinfo.php) 6 builds the installer from `packaging/windows/aria.iss`:

```
cargo build --release --locked -p aria-app
ISCC.exe /DAppVersion=0.2.2 packaging\windows\aria.iss
```

It writes `target\installer\Aria-0.2.2-setup.exe`. That command is how the installer is built; the 0.2.2 installer is built and tested separately before any release, and this README does not claim it has been. The installer is per-user (no administrator rights), installs into `%LOCALAPPDATA%\Programs\Aria`, and adds a Start-menu shortcut and an uninstaller. Uninstalling removes the program only; settings, the library database and logs stay (see [docs/PRIVACY.md](docs/PRIVACY.md)).

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
| CPU when idle (window open, nothing playing; measured before any audio output had been opened) | 0.08-0.47 % of one core, in four 20-second windows |
| `PRAGMA quick_check` on the 4,000-track database | 82 ms on the first run in a fresh process, 3-4 ms afterwards |
| Library snapshot of a synthetic 50,000-track database | 93 ms |

- The first-frame times are Aria's own `startup stage=first_frame` log lines. The scan and `quick_check` run on the library thread, not before the first frame, so the first frame does not wait for them.
- Memory is the process working set read from Windows, a few seconds after launch and again after about 90 seconds.
- Idle CPU is the change in the process's total processor time over each window. A screenshot taken with `PrintWindow` just before one measurement coincided with a much higher reading (14.5 % of a core over 60 s); it was not reproduced without the screenshot, and its cause was not investigated.
- The scan is quick here because the files are 3 seconds long. Real albums with large embedded artwork will take longer.
- CPU while playing has not been measured: playing needs input that an agent may not send. It is left to the person checking by hand (see the manual check).
- These numbers come from one machine and are not a benchmark. Earlier Phase 1 figures (an empty shell, no audio engine) were 94 MB idle.

## Performance (Phase 2b, measured)

The release build of the Collections test in `aria-library` (`cargo test --release -p aria-library --test collections -- --ignored --nocapture`) builds the favorites, playlists and recent plays that the app sends to the window for a synthetic library of 50,000 tracks with 5,000 favorites, 50 playlists of 200 entries each and 10,000 plays. It took **14.0 ms** in one run, against the target of under 50 ms. The machine is the one described above for Phase 2a (Intel Core i3-1315U, Windows 11 Pro for Workstations 10.0.26300). This is one run on one machine, the time is that of the `collections()` call alone; the database was a fresh one in a scratch folder; drawing the pages and the 30-second play counting were not timed.

## Performance (Phase 2c, measured)

Re-run on the 0.2.2 code, release build, same machine as above (Intel Core i3-1315U, Windows 11 Pro for Workstations 10.0.26300), one run each: the 2b `Collections` test (50,000 tracks) took **12.2 ms**, and the library snapshot of the synthetic 50,000-track database (`cargo test --release -p aria-library --test scan -- --ignored --nocapture`) took **93.7 ms**; the snapshot now carries each track's bit depth. **Not measured:** the time from a soundboard click to the first samples written (the spec asks for it; no test measures it), CPU while playing or in exclusive mode, and idle CPU with hotkeys registered.

## Known limitations

Phase 2c: the limits of exclusive output, sounds and hotkeys are listed under Status above and in the 0.2.2 entry of [CHANGELOG.md](CHANGELOG.md). In short: nobody has listened to any of it; bit-perfect delivery to the device cannot be recorded and is a hand check; the Russian and Armenian texts are not reviewed by a native speaker.

Phase 2b:

- A favorite is shown as the outline heart in the accent colour, not a filled heart, because only the regular icon weight is bundled. The difference is colour only.
- A file that is renamed or moved is a new track: it leaves favorites and playlists, and its playlist entries stay hidden until a file appears at the old path again.
- One track at a time goes into a playlist.
- A new playlist does not become the selected one automatically.
- No M3U import or export, no full history page, no play counts.
- Nobody has looked at the new screens in a running window, and long playlist names and row layout at narrow widths are unchecked (see the manual check).
- The Russian and Armenian texts of the new screens have not been read by a native speaker.

Phase 2a:

- Files changed while Aria runs appear after a rescan or the next start.
- Gapless trimming depends on data in the file. Formats without it may keep a small gap.
- Real audio output is to be verified on Windows only (by hand, pending).
- Aria cannot edit tags.
- The audio thread runs at normal priority, so heavy system load can cause glitches.
- The Russian and Armenian texts for the new features have not been read by a native speaker.

From Phase 1, still true:

- Spotify audio is not streamed in-app. This is a compliance choice; see the [roadmap](docs/ROADMAP.md#the-spotify-playback-limitation).
- The Armenian translation needs native-speaker review, and Armenian headings render at regular weight because there is no Armenian semibold font.
- egui's colour-picker popup and the panel splitter's accessibility label are English only.
- Alt+Left/Right and Ctrl+1...9 and Ctrl+0 are app-wide shortcuts.
- `ARIA_CONFIG_DIR` overrides only the config directory. Logs, the library database, the artwork cache and the window state still go to the OS defaults.
- Only Windows has been run by hand (on one machine). Linux and macOS are built and tested by CI only; nobody has opened the app on them yet.
- In the narrowest layout (a 900 px window with the widest sidebar and the queue open), the Russian theme switch does not fit on one line, so its "System" button wraps under the other two inside one outline.
- Release builds on Windows open no console window, so nothing printed to stderr is visible. Diagnostics go to the log files and `panic.log` (see [docs/PRIVACY.md](docs/PRIVACY.md) for where), and a failure to start is also shown in a message box. To see stderr, run a debug build (`cargo run -p aria-app`) from a terminal.
- If any one of Aria's folders (config, data, cache or logs) cannot be created at startup, settings are read-only for the session and file logging (including `panic.log`) is off, even if the other folders are fine.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Bundled fonts and third-party components: see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
