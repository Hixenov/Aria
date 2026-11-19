<p align="center">
  <img src="assets/icon/aria-256.png" width="128" height="128" alt="Aria's icon: a neon cyan letter A whose crossbar is a sound wave">
</p>

<h1 align="center">Aria</h1>

<p align="center">
  <a href="https://github.com/Hixenov/Aria/actions/workflows/ci.yml"><img src="https://github.com/Hixenov/Aria/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
</p>

Aria is a native, lightweight, cross-platform desktop music player written in Rust with
egui/eframe. Over six phases it grows into a local music player, a Spotify library and
remote-control client, a lyrics viewer, an equalizer and visualizer, and a tray/mini-player
app. See the [roadmap](docs/ROADMAP.md).

Spotify audio is not streamed or decoded inside Aria; Spotify support means library access and remote control of your existing Spotify devices, as explained in the [roadmap](docs/ROADMAP.md#the-spotify-playback-limitation).

## Status: version 0.2.8

What Aria does now:

- **Local library.** Add, remove and rescan music folders. A background scan builds the library (SQLite) and reads tags and embedded artwork; Aria never writes to or deletes your audio files. Folders are watched, so music copied into them shows up by itself (Settings, Library). Local Music has Tracks, Albums and Artists tabs, search, sorting, a Quality column with Lossless and Hi-Res badges, and a "Lossless only" filter.
- **Playback.** Aria's own audio engine (Symphonia decoding, rubato resampling, CPAL output): gapless transitions, seek, volume and mute, output device choice, recovery when a device disappears, and optional exclusive output on Windows (below). Volume levelling by ReplayGain tags, by track or by album (Settings, Playback). A sleep timer in the player bar: 15 minutes to an hour and a half, or after this track; the music fades out and pauses.
- **Queue.** Play next, add to queue, remove, drag to reorder, clear; kept between runs and restored paused.
- **Favorites, playlists, history.** A heart in every track table and in the player bar, a Liked page, playlists (create, rename, delete, reorder by dragging), playback history and a Home page with "Recently played" and "Recently added".
- **Opening files.** A double-click, "Open with", `aria.exe <files>` or a drop on the window plays the files; a file outside the library plays without being added, for the session only. With Aria already open, the open window plays them. Folders dropped or opened are added to the library.
- **Soundboard.** Up to 100 sounds with their own volume and global hotkeys, played on your device, on a second device, or both (below).
- **Music and microphone to a voice chat**, through a virtual audio cable (below).
- **One copy at a time**: starting Aria again brings the open window forward.
- English, Russian and Armenian.

Formats: MP3, FLAC, WAV, AIFF, AAC and ALAC in `.m4a`, Ogg Vorbis and Opus. What changed in each version is in the [changelog](CHANGELOG.md), and the checks for a person are in [docs/testing](docs/testing).

## Download

From the [latest release](https://github.com/Hixenov/Aria/releases/latest):

| System | File | |
|--------|------|-|
| Windows 10/11 x64 | `Aria-<version>-setup.exe` | Installer, per user, no administrator rights. Adds Aria to "Open with" for audio files. |
| Windows 10/11 x64 | `aria.exe` | Portable: run it from any folder. |
| macOS 11 or later, Apple silicon | `Aria-<version>-macos-arm64.zip` | `Aria.app`. Unpack it and move it to Applications. |
| Linux x86_64 | `Aria-<version>-linux-x86_64.tar.gz` | The `aria` program, with a desktop entry and the icon. |

None of the files is signed. On Windows, SmartScreen may warn on the first launch. On macOS, Gatekeeper refuses an app that is not signed: open it once with Control-click, Open, or run `xattr -dr com.apple.quarantine /Applications/Aria.app`. On Linux, Aria needs the usual desktop libraries (X11 or Wayland, OpenGL, ALSA); unpack the archive and run `./aria`, or copy `aria` to `~/.local/bin`, `aria.desktop` to `~/.local/share/applications` and `aria.png` to `~/.local/share/icons/hicolor/256x256/apps`.

`SHA256SUMS.txt` in each release has the checksums of all the files.

## Music and your microphone in a voice chat

A chat program (Discord, a game, a call) takes sound only from a recording device. Aria does not install anything into Windows and never plays into your real microphone directly; that would take a system audio component installed with administrator rights. It uses a **virtual audio cable** instead, a free program you install once:

1. Install a virtual cable, for example VB-CABLE from its [official page](https://vb-audio.com/Cable/). Settings, Soundboard shows this guide and the link when no cable is found.
2. Restart Aria, or open Settings again. Aria finds an installed cable by its name (VB-CABLE, VoiceMeeter, Virtual Audio Cable) and offers it with a "Use it" button; a cable it does not recognise can be chosen by hand as the second device.
3. In Settings, Soundboard, turn on "Music also to the second device" and, if you want to be heard, "Send my microphone to the second device", and choose your microphone.
4. In the chat program, choose the cable's recording side as the microphone (for VB-CABLE, "CABLE Output"). Settings says which.

- The music copy has its own volume, "Music volume in the chat" (default 50 %). Your player volume and mute do not change it, so you can turn your own speakers down and keep the music going for the others. Pause, seek and track changes act on both. Some chat programs level their microphone themselves (Discord's "Automatic gain control"); turn that off, or the music volume in the chat seems not to change.
- The microphone has its own volume, 0 to 200 %; above 100 % amplifies and clips at full scale. A global hotkey can turn it on and off (Settings, Soundboard), and the player bar shows a microphone icon while it is open; a click on it turns it off. There is no noise suppression in Aria and you do not hear yourself.
- The microphone is open only while "Send my microphone" is on and a second device is chosen. What it captures goes only into the second device; Aria never records, stores, logs or sends it (see [docs/PRIVACY.md](docs/PRIVACY.md)). On macOS, Aria asks for microphone permission.
- Exclusive mode stays possible on your own device: the copy goes to the cable through the system mixer and your own path can stay bit-perfect.
- Without a second device the switches are disabled. If the second device is the one the music plays on, nothing is copied and the microphone stays closed, and Settings warns about it.
- The copy and your own music can drift apart by tens of milliseconds. When your output reopens (a rate change in exclusive mode, a device change), the chat hears the same short silence. Turning sharing on in the middle of a track starts the copy with up to about a second of silence, so that it plays in step; your own music is not touched.
- Aria never opens a virtual cable's recording side ("CABLE Output") as the microphone, whether you choose it or it is the Windows default recording device: the chat would hear itself. Settings warns, and a notice says so.
- If the cable fails while the microphone is on, the microphone shows as failed and comes back when the cable opens again (with the next sound or Play), or when you toggle it.
- Aria's own path from the microphone to the cable takes about 40 ms; the devices' own buffers come on top of that.

## Exclusive mode and what "bit-perfect" means here

Settings, Playback has an "Exclusive mode" switch (Windows only, off by default). When it is on, Aria sends the track's samples to the output device through WASAPI exclusive mode, at the track's own sample rate, instead of through the system mixer.

- While it plays, other programs are silent on that device. Aria releases the device 15 seconds after you pause.
- While Aria holds the device exclusively, its volume slider is locked at 100 % (mute still works); set the level on the device or amplifier. If exclusive mode is refused and the music goes through the system mixer, the slider is free and sets the level as usual; it locks again when an exclusive output opens. The saved volume comes back when you turn the mode off.
- If the device refuses exclusive mode, playback goes on through the system mixer and a notice says so. If another program holds the device exclusively, Windows refuses Aria's shared stream as well, and Aria cannot play on that device until it is released.
- After a refused exclusive open Aria stays on the system mixer for that output: exclusive mode is tried again only when the output is opened again (after a pause of more than 15 seconds, at a device change, at a join that needs another format, or when you turn the mode off and on).
- If the device has no format at the track's rate, Aria opens the nearest rate it supports and resamples, and the indicator says "Resampled".
- The player bar shows the track's format (for example "FLAC 24/96") and one state: "Bit-perfect", "Resampled", "Converted", "Mixed with a sound" or "System mixer". A tooltip explains each.

"Bit-perfect" is shown only when all of these hold: exclusive output, an integer source of 24 bits or fewer, the music at full gain with no volume levelling, no resampling, nothing else mixed in, and the channels unchanged (mono played on both channels counts as unchanged). What it does **not** mean:

- The 5 ms fades when playback starts, pauses, resumes or seeks are not bit-exact. Gapless joins between tracks have no fade.
- ALAC is never shown as bit-perfect (its decoder reports no bit depth, so it shows "Converted"). Float sources and integer sources wider than 24 bits are never bit-perfect either.
- It is a claim about what Aria hands to the output, not about what the device does with it.
- Where the next track needs a different sample rate, a wider bit depth than the open output carries, or more channels, the output is closed and opened again, with a short silence at that join. Tracks in the same format stay gapless.

## Soundboard

The Soundboard page holds up to 100 sounds. A sound is a reference to an audio file (any format Aria plays); Aria never copies, changes or deletes the file. Names are 1 to 50 characters. Each sound has a volume and an optional global hotkey, and there is one hotkey for "Stop all". A click or the hotkey plays the sound from its start; firing a playing sound restarts it; at most 6 sounds play at once, and a seventh stops the one that started first. Sounds are not tracks: they do not enter the queue or the history.

Settings, Soundboard chooses where sounds go: "My device" (the music's device, the default), "Second device" or "Both", and the second device itself. The second device is opened in shared mode when a sound needs it and closed 15 seconds after the last sound; with no second device chosen, "Second device" and "Both" play on your own device.

- Sounds on your own device are mixed into the music stream, so they break bit-perfect output while they play (the indicator shows "Mixed with a sound"), and they are cut when the music output is closed or reopened.
- A sound fired while exclusive mode is on and no music plays opens your device exclusively: other programs are silent on that device until 15 seconds after the last sound.
- The player's volume and mute do not apply to sounds; each sound has its own volume, and "Stop all" stops them.
- A second device that is slow to open (Bluetooth, a sleeping USB interface) does not interrupt the music: a sound for it waits until it is open, and a sound for both devices starts on both together.
- **Hotkeys** work while Aria runs, including minimized, with a switch in Settings that turns them all off. A hotkey must include Ctrl, Alt or Win unless it is F1 to F24 or a numpad key. One used by another sound or by Aria itself (Ctrl+0 to Ctrl+9, on the main row or the numpad, and Ctrl+Alt+R; Command on a Mac), or held by another program, is refused. Aria receives only the registered combinations from the system; it installs no keyboard hook and reads no other keystrokes. A global hotkey takes its combination from every other program while Aria runs: for example, Ctrl+S as a sound's hotkey stops saving in editors. Turn the "Global hotkeys" switch off to give them back.

## Upgrading

Install a new version over the old one; the library, playlists and settings are kept. A library database that a newer version has upgraded (0.2.1 and 0.2.2 did) no longer opens in an older one: copy the data folder first if you may want to go back. Close older copies of Aria before starting a new one.

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
  libxcb-shape0-dev libxcb-xfixes0-dev libgl1-mesa-dev libasound2-dev
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
ISCC.exe /DAppVersion=0.2.8 packaging\windows\aria.iss
```

It writes `target\installer\Aria-0.2.8-setup.exe`. The installer is per-user (no administrator rights), installs into `%LOCALAPPDATA%\Programs\Aria`, and adds a Start-menu shortcut, "Open with" entries and an uninstaller. Uninstalling removes the program only; settings, the library database and logs stay (see [docs/PRIVACY.md](docs/PRIVACY.md)).

The macOS and Linux files of a release are built by `.github/workflows/release.yml` when the release is published.

## Configuration and data

`ARIA_CONFIG_DIR` overrides the config directory only. See [docs/PRIVACY.md](docs/PRIVACY.md) for the exact paths of everything Aria stores.

## Performance

Release build of 0.2.9 in development (`cb2e647`), portable mode, on one machine (Intel Core i3-1315U, Windows 11), with a generated library of 4,000 short WAV files (40 artists, 200 albums), measured 2026-11-20:

| What | Result |
|------|--------|
| First frame, first launch (database created, folder added) | 196 ms |
| First frame, later launches (3 launches) | 83-107 ms |
| Scan of the 4,000 files, first launch | done 0.5 s after the first frame |
| Working set after the scan / when idle | 103-115 MB (private 55 MB) |
| CPU when idle (window open, nothing playing), 20-second windows | 0-0.31 % of one core |
| Library snapshot of a synthetic 50,000-track database | 93 ms |
| Favorites, playlists and recent plays for 50,000 tracks | 12-14 ms |

These numbers come from one machine and are not a benchmark.

## Known limitations

- A favorite is shown as the outline heart in the accent colour, not a filled heart, because only the regular icon weight is bundled.
- A file that is renamed or moved is a new track: it leaves favorites and playlists, and its playlist entries stay hidden until a file appears at the old path again.
- One track at a time goes into a playlist; a new playlist does not become the selected one automatically. No M3U import or export, no full history page, no play counts.
- Gapless trimming depends on data in the file. Formats without it may keep a small gap.
- Aria cannot edit tags.
- The audio thread runs at normal priority, so heavy system load can cause glitches.
- CAF files and ADPCM WAV files are skipped by the library scan.
- Crossfading between tracks is not there yet (Phase 5).
- `ARIA_CONFIG_DIR` overrides only the config directory. Logs, the library database, the artwork cache and the window state still go to the OS defaults.
- Armenian headings render at regular weight, because there is no Armenian semibold font. egui's colour-picker popup and the panel splitter's accessibility label are English only.
- In the narrowest layout (a 900 px window with the widest sidebar and the queue open), the Russian theme switch does not fit on one line.
- Release builds on Windows open no console window. Diagnostics go to the log files and `panic.log` (see [docs/PRIVACY.md](docs/PRIVACY.md)), and a failure to start is also shown in a message box.
- If any one of Aria's folders (config, data, cache or logs) cannot be created at startup, settings are read-only for the session and file logging is off.
- Exclusive output exists on Windows only.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Bundled fonts and third-party components: see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
