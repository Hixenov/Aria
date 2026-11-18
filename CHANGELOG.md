# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.2.5] - 2026-11-18 (Aria's own icon)

Checked with automated tests on Windows and in CI on Linux and macOS. Windows reads the icon from the release `aria.exe` and from the installer. The icon was not looked at after an install, in the Start menu or the taskbar.

### Added

- Aria's own icon: a bold, rounded "A" in bright neon cyan, almost white at the top and deeper at the feet, whose crossbar is a sound wave, with a thin light rim inside its edge and a soft cyan glow around it, on a clear background. On the smallest sizes the glow and the rim are left out. It is built into `aria.exe` on Windows, so Explorer, the Start menu, the desktop shortcut, the taskbar and the list of installed programs show it; the window shows it in its title bar, the taskbar and Alt+Tab on every system; and the installer shows it too. Before, Aria had no icon of its own, and Windows showed its generic one.
- `crates/aria-app/examples/make_icon.rs`, which draws the icon from a few shapes and writes `assets/icon/aria.ico` (16 to 256 pixels) and `assets/icon/aria-256.png`. Each size is drawn on its own; on the smallest the wave becomes a plain crossbar.

### Fixed

- A microphone still opening (since 0.2.4 it opens on a thread of its own) did not count as needing the second device. If the second device had already been idle for most of its 15 seconds, for example after a sound, it could close while the microphone opened, and the microphone was then never opened, until the next change. It now keeps the second device, and if the second device fails while it opens, it is opened again with it.
- Some chat tests waited a fixed time for the engine and failed on a slow machine; they now wait for the sound itself.

### Changed

- Version 0.2.5.
- New build-time dependency on Windows: `embed-resource` 3.0.12 (MIT), which hands the icon to the Windows resource compiler. A Windows build now needs the Windows SDK's resource compiler, which the Visual Studio C++ build tools install.

## [0.2.4] - 2026-11-18 (one Aria at a time, and fixes to the chat features)

Checked with automated tests on Windows and in CI on Linux and macOS. The release build was run on one Windows PC: its log shows three later launches reaching the open copy and quitting. That the window came forward on screen, and the chat fixes, were not reported on; see `docs/testing/0.2.4-manual-check.md`.

### Added

- One Aria at a time per data folder. Starting Aria while it is already open brings the open window forward (shown, restored if minimized, and focused) instead of starting a second copy, which would write the same library database at once. If the open copy does not answer within two seconds, a message says that Aria is already open. The first copy holds a lock on `aria.lock` in the data folder; the second asks it to show itself over a local named pipe.
- The player-bar microphone icon names the microphone in its tooltip ("Microphone: <name>"), and assistive technology reads the same line. "System default" shows the default microphone's own name.

### Fixed

- The microphone hotkey turned the switch on and said "Microphone on" without a second device, though nothing could open. It now leaves it off and says to choose a second device in Settings, Soundboard, as the greyed-out switch there does.
- When the chosen music device could not be opened and the music fell back to the system default, and that default was the second device, the music copy went into the very device the music plays on, and the microphone stayed open there. Both now stop, as when the music is moved onto the second device.
- A slow or stuck microphone held the music thread for as long as it took to open (up to several seconds), so the music could run dry. The microphone now opens on a thread of its own.
- Screen readers read the microphone volume bar as a share of its 200 % range (50 % at 100 %). They now read the percentage shown beside it.

### Changed

- Version 0.2.4. No database or settings change.
- New dependency: `interprocess` 2.4.7, for the single-instance pipe. Two of its helpers are 0BSD-licensed, which `deny.toml` now allows.
- `docs/PRIVACY.md`: the lock file and the local pipe.

### Known limitations

- A copy of 0.2.3 or older does not take the lock, so 0.2.4 cannot tell that one is open, and the two can still run side by side. Close older copies before starting 0.2.4.
- Windows may refuse to bring a window forward when another program has just been used; then Aria's taskbar button flashes instead.
- Bringing the window forward is tested only as far as the request reaching the open copy; the window's own behaviour is not tested by hand yet.
- `ARIA_CONFIG_DIR` changes only the settings folder, so two copies with different settings folders still share one library and are still held to one.

## [0.2.3] - 2026-11-18 (music and microphone to the chat)

This release build was started on one Windows PC with a real library, and its log shows it starting cleanly and decoding tracks. Nobody has checked the new chat features by hand: no one has listened through a virtual cable, and no chat program was tried. See `docs/testing/music-to-chat-manual-check.md`, where every line for a person is pending. No test opens a real microphone; the microphone is tested with a fake capture only.

### Added

- "Music also to the second device" (Settings, Soundboard, off by default): the music that plays on your device is copied into the second device, for a chat that uses a virtual cable as its microphone. It has its own volume, "Music volume in the chat" (0 to 100 %, default 50 %); the player's volume and mute do not change it. Pause, seek, stop and track changes act on the copy too. The copy goes through a shared output, resampled if needed, so your own output can stay exclusive and bit-perfect.
- "Send my microphone to the second device" (off by default) and a "Microphone" choice (System default first): the microphone is captured in shared mode and mixed into the second device only, never into your own. "Microphone volume" goes from 0 to 200 %, clipped at full scale. At most 40 ms of microphone audio waits in the mix; older audio is dropped rather than letting the delay grow.
- A global hotkey that turns the microphone on and off, with a short "Microphone on" / "Microphone off" notice, checked against the sound hotkeys, "Stop all" and Aria's own shortcuts both ways.
- A microphone icon in the player bar while the microphone is open (a click turns it off), and a crossed-out one when it could not be opened.
- Finding the virtual cable: with no second device chosen, Settings, Soundboard offers an installed cable (a device whose name contains "CABLE Input", "VB-Audio", "Voicemeeter" or "Virtual Audio Cable") with a "Use it" button, or shows a three-step guide with a link to VB-CABLE's official page. With a cable chosen, it says which recording device to pick in the chat program ("CABLE Output" for VB-CABLE). Aria makes no network request; the link only opens your browser. Aria installs and changes no driver.
- Notices when the microphone cannot be opened, and when the system refuses it, with where to allow microphone access. On Windows a refusal is recognised by the access-denied code (E_ACCESSDENIED) that CPAL passes on; this was not tried on a real refusal.
- A virtual cable's recording side ("CABLE Output", or any device whose name marks it as a cable) is never opened as the microphone, chosen or as the system default, since the chat would hear itself; Settings warns, and a notice says so.
- If the second device fails while the microphone is on, the microphone shows as failed and opens again with the second device (the next sound or Play), or when it is toggled.
- English, Russian and Armenian texts for all of the above, written by the coding agent and not read by a native speaker.
- `docs/testing/music-to-chat-manual-check.md`.

### Changed

- Version 0.2.3. No database change; the settings schema version stays 1, and a 0.2.2 settings file loads with sharing and the microphone off.
- `docs/PRIVACY.md`: Aria now reads the microphone when, and only when, you turn it on with a second device chosen.

### Known limitations

- A virtual cable must be installed separately; Aria only finds it and guides. In the chat program the cable, not the real microphone, must be chosen. Playing into the real microphone without a cable is not supported. A cable whose name matches none of the known names is not found, but can be chosen by hand.
- You do not hear your own voice. There is no noise suppression or other voice processing in Aria; the chat program's own processing applies.
- The copy and your own music can drift by tens of milliseconds (the difference between the two devices' buffers); this is not corrected.
- When your output reopens, for example at a rate change in exclusive mode, the chat hears the same short silence.
- When sharing starts in the middle of a track, the chat hears up to about a second of silence first, so that the copy plays in step with your own music; your own music is not touched.
- If the second device does not keep up, the copy drops audio rather than hold up your music.
- Choosing your own music device as the second device disables the copy and the microphone.
- A microphone that fails or is refused is tried again only at the next change of the microphone, its switch, the second device or the music device (or, after a failure of the second device, when that opens again).
- When the microphone and the cable run at different rates, the resampler adds up to about 21 ms. The latency target of under 60 ms has not been measured.
- Assistive technology reads the microphone volume bar as a share of its 200 % range (50 % at 100 %); the shown label is right.
- The microphone icons (Phosphor "microphone" and "microphone-slash") have not been looked at.
- Checked by automated tests only, with fake devices, on Windows; built for Linux and macOS. On macOS microphone permission must be granted to Aria.
- The Russian and Armenian texts are not reviewed by a native speaker.
- The limitations listed for 0.2.2 and earlier still apply.

## [0.2.2] - 2026-11-16 (Phase 2c, lossless output and the soundboard)

Nobody has run this version by hand yet, and nobody has listened to it: see `docs/testing/phase2c-manual-check.md`, where every line for a person is still pending. The Windows installer and release `aria.exe` for 0.2.2 are not covered by this entry: they are built and tested separately before any release.

### Added

- Exclusive output (Windows only, off by default; Settings, Playback). The track's samples go to the device through WASAPI exclusive mode, at the track's own sample rate, with no resampling by the system mixer. It tries the track's own bit depth, then 24 bits in a 32-bit container, 32-bit integer and 16-bit, and uses the first the device accepts. If the device has no format at the track's rate, Aria opens the nearest rate it supports and resamples, and says so. If exclusive mode is refused, playback falls back to the system mixer with a notice. While it plays, other programs are silent on that device; Aria releases the device 15 seconds after pause. While the device is held exclusively the volume is locked at 100 % (mute still works); after a refused exclusive open the slider sets the level of the shared output as usual, and it locks again when an exclusive output opens. The saved volume returns when the mode is turned off. A saved volume of 0 (not muted) plays at full level while the volume is locked, as the slider shows, and is used again when the lock lifts.
- An output indicator in the player bar: the track's format and one state, "Bit-perfect", "Resampled", "Converted", "Mixed with a sound" or "System mixer", with a tooltip for each. "Bit-perfect" is shown only for an integer source of 24 bits or fewer, at gain 1.0, with no resampling, no other input and unchanged channels (mono played on both channels counts as unchanged). The 5 ms fades on start, pause, resume and seek are not bit-exact; gapless joins have no fade. ALAC is shown as "converted" because its decoder does not report a bit depth. Float sources and integer sources wider than 24 bits are never shown as bit-perfect.
- The library stores each track's bit depth. A Quality column in the track tables (Local Music and Liked) shows a "Lossless" badge (FLAC, ALAC, and WAV or AIFF whose data is integer or float PCM; A-law, µ-law and other compressed WAV or AIFF data count as lossy) or a "Hi-Res" badge (lossless and above 48 kHz or above 16 bits); a tooltip gives codec, depth and rate. A "Lossless only" toggle on the Tracks tab. ALAC is now told from AAC inside `.m4a`.
- Soundboard page: add sounds (several files at once), rename, remove, a volume per sound, a hotkey per sound, click to play, "Stop all". Up to 100 sounds; names of 1 to 50 characters; at most 6 play at once (a seventh stops the one that started first); firing a playing sound restarts it. A sound is a reference to a file: Aria never copies, changes or deletes it. Sounds are not tracks: they do not enter the queue or the history. A missing or unreadable file gives a notice when the sound is fired.
- Settings, Soundboard: a second output device and "Play sounds to" (My device, Second device, Both). The second device opens in shared mode when a sound needs it and closes 15 seconds after the last sound. With no second device, "Second device" and "Both" play on your own device.
- Global hotkeys (Settings, Soundboard, on by default): one per sound and one for "Stop all". A hotkey must include Ctrl, Alt or Win unless it is F1 to F24 or a numpad key. A hotkey used by another sound or by Aria's own shortcuts (Ctrl+0 to Ctrl+9, Ctrl+Alt+R; Space and Alt+Left/Right cannot be hotkeys at all), or held by another program, is refused with a notice and not saved. One switch turns them all off. Aria receives only the registered combinations from the system and installs no keyboard hook.
- English, Russian and Armenian texts for all of the above. The Russian and Armenian texts were written by the coding agent and have not been read by a native speaker.
- New dependencies: `wasapi` 0.27 (MIT, Windows only), `global-hotkey` 0.10 (Apache-2.0 OR MIT) and, through it, `keyboard-types` (MIT OR Apache-2.0). `cargo deny` was not run on the development PC; it runs in CI.
- `docs/testing/phase2c-manual-check.md`.

### Changed

- The library database goes from version 2 to 3: a bit-depth column and a `sounds` table are added, and every file is read once more on the first start (a big library may take a while). Favorites, playlists and history are kept. There is no way back: 0.2.1 refuses a version 3 database ("The library was saved by a newer version of Aria") and runs with the library turned off, leaving the file untouched, so 0.2.2 opens it again with nothing lost. Copy the data folder first if you may want to go back to 0.2.1.
- Version 0.2.2.

### Known limitations

- Exclusive output exists on Windows only. While it plays, other programs are silent on that device. If another program holds the device exclusively, Windows refuses Aria's shared stream too, so Aria cannot play on that device until it is released.
- Where the next track needs a different sample rate, a wider bit depth than the open output carries, or more channels, the output is closed and opened again: a short silence at that join. Tracks in the same format stay gapless.
- The first 5 ms after start, pause, resume and seek are faded and not bit-exact.
- Float sources, integer sources wider than 24 bits and ALAC are not shown as bit-perfect.
- What the device actually receives cannot be recorded through WASAPI loopback. That the samples arrive unchanged is a check by hand, best with a DAC that shows its input rate. Nobody has listened to exclusive output.
- The output thread runs at normal priority, so dropouts under heavy load are possible. Not checked.
- After a refused exclusive open Aria stays on the system mixer for that output: exclusive mode is tried again only when the output is opened again (after a pause of more than 15 seconds, at a device change, at a join that needs another format, or when you turn the mode off and on), not as soon as the device is free.
- The log level "Trace" (Settings, Log level) is for diagnostics only. When Aria starts with it, or with `ARIA_LOG=trace`, the WASAPI output thread makes a log call, which takes a lock, on every device period, even though the line is filtered out (found by reading the code; its cost was not measured). Use a lower level for listening.
- Sounds on your own device are mixed into the music stream: they break bit-perfect output while they play. They are cut when the music output is closed or reopened (a rate, depth or channel change at a join, a device change, switching exclusive mode, a device failure). A "Both" sound keeps playing on the second device if its part on your device is cut.
- A sound removed while it plays keeps playing until it ends ("Stop all" stops it).
- A sound fired while exclusive mode is on and no music plays opens your device exclusively, at the sound's own format: other programs (a video, a call) are silent on that device until 15 seconds after the last sound.
- The player's volume and mute do not apply to sounds; each sound has its own volume, and "Stop all" stops them.
- Opening and closing the second device run on the engine thread, which holds about one second of music. A device that is slow to open (Bluetooth, a sleeping USB interface; the wait is up to 7 s) or to close (up to 2 s) can briefly interrupt the music when a sound fires, or 15 seconds after the last one. A second device that fails to open is tried again, with a notice, on every sound.
- A library scan can delay the "file missing" notice for a sound by up to about 250 ms; the sound itself starts without waiting.
- Aria provides no virtual audio cable and does not read the microphone. For a voice chat, the second device must be a virtual cable that the chat program uses as its microphone; the chat hears the sounds and not your voice unless another program mixes them. No cable and no chat program were tested.
- Hotkeys were checked on Windows only, by one synthetic key press reaching a test window while unfocused and while minimized. The real Aria window has not been run with hotkeys by anyone; the final wiring is tested with a fake backend. While Aria is minimized a press may take about 100 ms to act. Games in the foreground are not tested.
- Capturing a hotkey in the dialog: the Win key cannot be captured (egui does not report it on Windows); numpad keys are set with a "Numpad" switch because egui cannot tell a numpad key from the main row; numpad `*` and `.` cannot be captured; Ctrl+C, Ctrl+X and Ctrl+V cannot be captured. Letters follow the keyboard layout.
- A global hotkey takes its combination from every other program while Aria runs, not only from Aria: for example, Ctrl+S as a sound's hotkey stops saving in editors. Turn the "Global hotkeys" switch off to give them back.
- Exclusive output and hotkeys are built for Linux and macOS where the crates support them, but verified on Windows only. Exclusive output does not exist there.
- CAF files are skipped by the library scan (as in 0.2.1), so no CAF track appears in the library; a CAF file works as a soundboard sound. IMA ADPCM WAV files (4 bits per sample) are skipped by the scan too, because lofty cannot read them; other ADPCM, A-law and µ-law WAV and AIFF files are listed as lossy. AIFC files with integer codings lofty does not name (for example in24, in32, twos) are listed as lossy too, although their data is PCM.
- Verified on this PC (Windows 11, Intel Core i3-1315U, Realtek headphones), with silence only and nobody listening: WASAPI exclusive opened the default device at 44.1 kHz and 16 bits; closing took about 42 to 49 ms; a second exclusive open was refused as busy; the device accepted 16-bit and 24-bit-in-32, mono and stereo, 32 to 192 kHz, and refused 32-bit integer and rates above 192 kHz. This is one device.
- The Russian and Armenian texts are not reviewed by a native speaker.
- The limitations listed for 0.2.1 and 0.2.0 still apply.

## [0.2.1] - 2026-11-11 (pre-release: Phase 2b, favorites, playlists, history)

Nobody has run this version by hand yet: see `docs/testing/phase2b-manual-check.md`, where every line for a person is still pending.

### Added

- Favorites: a heart column in every track table and a heart beside the title in the player bar, "Add to favorites" and "Remove from favorites" in the row menu, and a Liked page with the same table, search, sorting and row actions as Local Music.
- Playlists: a Playlists page to create, rename and delete playlists (Delete asks first); add a track from the row menu, either to a playlist in the list or to a new one ("New playlist..."); remove an entry from the row menu; reorder entries by dragging; play a playlist from any row. A line shows how many tracks of a playlist are unavailable.
- Playback history: a play counts after the track has played for 30 seconds. "Recently played" holds up to 20 tracks, each once; the newest 10,000 plays are kept.
- Home page: "Recently played" and "Recently added" (20 tracks); a click plays that list from that row.
- Settings, Playback: "Keep playback history" and "Clear playback history" (asks first).
- Favorites, playlist entries and plays are stored by file path, so they come back, in their old order, when a music folder is removed and added again. A 0.2.0 library database is upgraded in place (database version 1 to 2); the migration only adds tables. There is no way back: once 0.2.1 has opened the database, 0.2.0 refuses it ("The library was saved by a newer version of Aria") and runs with the library turned off, without folders, tracks or the saved queue. 0.2.0 leaves the file untouched, so 0.2.1 opens it again with nothing lost. To keep a library that 0.2.0 can open, copy the data folder before the first start of 0.2.1.
- English, Russian and Armenian texts for all of the above. The Russian and Armenian texts were written by the coding agent and have not been read by a native speaker.
- `docs/testing/phase2b-manual-check.md`.

### Changed

- The track table of Local Music is now shared with Liked. Playlist rows are drawn separately (a grip for dragging, no sortable headers), with the same heart and the same row-menu items, plus removing the entry.
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
