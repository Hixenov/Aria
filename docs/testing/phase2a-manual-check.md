# Phase 2a manual check

Checklist for the 0.2.0 pre-release (spec section 11). Each line says who verified it:

- **agent**: run by the coding agent on the Windows PC described in the README, with no sound played and no keyboard or mouse input sent to the app.
- **user**: has to be done by a person with the app in front of them and speakers or headphones on. Audio cannot be judged by an agent.
- **pending**: not verified by anyone yet. Replace `pending` with `user` (and the date) when you have checked it.

Run the release build (`cargo build --release --locked`, then `target\release\aria.exe`).

## What the agent ran

| Check | Who | Result |
|-------|-----|--------|
| Release build starts, window opens, first frame is logged | agent | Yes. First frame 179-337 ms across 8 launches (see README). |
| A library folder is scanned in the background | agent | Yes, for a generated folder of 4,000 files (MP3, FLAC, Ogg, Opus, M4A, WAV). The folder was registered through the library API in a throwaway program, because the folder dialog needs input. The scan log line reported 4,000 added, 0 skipped. A second launch reported 0 added, 0 updated, 0 removed. |
| A real music folder scans | pending | Only generated 3-second tones were scanned. Try your own folder through Settings, Library. |
| Logs contain no errors or warnings after those runs | agent | Yes, only INFO lines (startup stages and scan results) in the scratch log of the runs. |
| Idle CPU and memory with a large library | agent | See the README: about 0.1-0.5 % of one core, 127-129 MB working set. |
| Screenshot of the window, off-screen (`scripts/screenshot-window.ps1 -Background`) | agent | Taken; the app shows the Home page (a Phase 2b page with an "available later" message). The Local Music page was not reached, since that needs a click. |
| 50,000-track library snapshot (ignored test, release) | agent | Passed; the snapshot took 93 ms. |
| `cargo fmt`, `clippy -D warnings`, `cargo test --workspace --locked` | agent | Pass (see the task report). |
| `cargo deny` | pending | Not installed on the agent's PC. It runs in CI. |
| Linux and macOS | pending | Built and tested in CI only. Nobody has run the app on them. |

## What you need to check by hand

Use a folder of your own music that has a few formats, at least one full album, and tracks at different sample rates.

| Check | Who |
|-------|-----|
| Add your music folder in Settings, Library. It scans in the background, the progress shows, and the Local Music page lists Tracks, Albums and Artists | pending |
| Search and sorting on the Local Music page work | pending |
| MP3 plays | pending |
| FLAC plays | pending |
| WAV plays | pending |
| AAC / M4A plays | pending |
| ALAC (M4A) plays | pending |
| Ogg Vorbis plays | pending |
| Opus plays | pending |
| Seek by dragging the progress bar: playback continues from the new position, with no click or repeat of audio | pending |
| Queue: add a track, play next, remove a track | pending |
| Queue: drag a track to a new position | pending |
| Queue: clear | pending |
| Gapless: play an album that flows from one track into the next (a live or classical album is a good test) and listen at the joins for a gap | pending |
| Sample-rate join: play a 44.1 kHz track followed by a 48 kHz track, and listen at the join for a click or pop | pending |
| Space plays and pauses (while no text box has focus) | pending |
| Volume and mute work | pending |
| Settings, Playback: choose another output device; the sound moves to it | pending |
| Unplug headphones in the middle of a track: playback falls back to another device and a notice is shown | pending |
| Close Aria while a track is playing, start it again: the queue is restored and paused, at the saved position | pending |
| Glitches or dropouts under load: play while the PC is busy (a video, a build or a browser with many tabs). The audio thread runs at normal priority (the CPAL `realtime` feature is off), so say what you hear | pending |
| A stuck or slow audio driver does not freeze the window when you press Play (for example right after waking the PC, or with a Bluetooth device that is switching on) | pending |
| CPU while playing: with a track playing, read Aria's CPU in Task Manager over a minute and add the number to the README | pending |
| A file changed or added while Aria runs appears after Rescan or the next start | pending |
| Aria does not change your music files (compare a file's modified time before and after a scan) | pending |

## Translations

The Russian and Armenian texts for the Phase 2a screens and notices have not been read by a native
speaker. Please read through the Library, Local Music, queue, player bar, Settings and notice
texts in both languages (Settings, Language) and report anything that is wrong, awkward or cut off.
The strings are in `locales/ru/main.ftl` and `locales/hy/main.ftl`.

| Check | Who |
|-------|-----|
| Russian texts read by a native speaker | pending |
| Armenian texts read by a native speaker | pending |
