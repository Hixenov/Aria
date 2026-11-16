# Phase 2c manual check

Checklist for the 0.2.2 pre-release (spec section 10): exclusive output, the output indicator,
the Lossless / Hi-Res badge and filter, the soundboard and global hotkeys. Each line says who
verified it:

- **agent**: run by the coding agent on the Windows PC described below, with no sound judged.
- **user**: has to be done by a person with the app in front of them. Audio cannot be judged by an agent.
- **pending**: not verified by anyone yet. Replace `pending` with `user` (and the date) when you have checked it.

Run the release build (`cargo build --release --locked`, then `target\release\aria.exe`). Back up
your Aria data folder first: 0.2.2 upgrades the library database (version 2 to 3), and 0.2.1
cannot open it afterwards. Use a library with some lossless files at different rates (for example
44.1 kHz and 96 kHz), some short sound files, and, for the second-device lines, a second output
device (headphones, speakers, or a virtual audio cable).

**Nobody has run the 0.2.2 app by hand and nobody has listened to it.** The agent that wrote it
could not hear or see the window. The real Aria app was never started with hotkeys, and never
played through exclusive mode.

## What automated tests cover

These run in `cargo test --workspace --locked`. They are real tests, but none of them replaces
listening or looking at the window.

| Area | What is tested |
|------|----------------|
| Bit-exact path | Integer sources of 16 and 24 bits go through the float mixer at gain 1.0 with no resampling and come out equal to Symphonia's own conversion (divisors 2^15, 2^23, 2^31). Mono played on both channels counts as unchanged. Not bit-perfect, and reported so: a gain other than 1.0, a resample, a reduced depth, changed channels, a float or wider-than-24-bit source, a sound mixed in. The 5 ms fades are not bit-exact and are not claimed to be |
| Format choice | The order own depth, 24-in-32, 32-bit integer, 16-bit against fake devices; the nearest-rate rule; a refused exclusive open falls back to shared; a failed exclusive output is replaced and playback carries on |
| Engine | A rate change reopens the output between tracks and each track is heard whole and in order; same-format neighbours stay gapless; the output state event for each state; volume locked in exclusive mode and restored when it is turned off |
| Library | The migration of a 0.2.1 database (tracks, favorites, playlists, plays) to version 3 with nothing lost; the rescan fills the depth; ALAC told from AAC; the rules for sounds (count 100, names 1 to 50 characters, volume range, order) |
| Soundboard | At most 6 sounds at once and the seventh stops the oldest; firing a playing sound restarts it; a sound that cannot be opened changes nothing; stop all; every routing case (My device, Second device, Both, no second device, the same device) with fake outputs; sounds cut by a reopen |
| Hotkeys | The allowed-key rule (a bare typing key is refused), a duplicate inside Aria is refused, a combination the system refuses is not saved; the final wiring (event handler and `App::logic`) with a fake backend |
| App | The Soundboard page, the indicator, the Quality column and filter, and the new settings drawn in a test harness (egui_kittest); the Russian and Armenian locale keys match the English ones |

Not covered by any test: that the device receives unchanged samples; anything audible; the real
hotkey backend inside the Aria window; the file dialog; a second real device.

## What the agent ran on real hardware

PC: Windows 11 Pro for Workstations, Intel Core i3-1315U, Realtek headphones as the default
output. Only silence was ever written to the device, and nobody listened.

| Check | Who | Result |
|-------|-----|--------|
| Task 1: a global hotkey (one synthetic Ctrl+Alt+F15) reaches an eframe test window | agent | Yes, while the window was unfocused and while it was minimized. This was a separate test window, not Aria |
| Task 6: WASAPI exclusive output opens the default device at 44.1 kHz and 16 bits and plays 500 ms of silence | agent | Yes |
| Task 6: closing the exclusive output | agent | About 42 to 49 ms (it lets the last device buffer play out) |
| Task 6: a second exclusive open while the first holds the device | agent | Refused (device busy) |
| Task 6: formats the device accepted in exclusive mode | agent | 16-bit and 24-bit in a 32-bit container, mono and stereo, 32 to 192 kHz. It refused 32-bit integer and anything above 192 kHz. One device only |
| Hotkeys in the real Aria window | pending | Never run |
| `aria.exe` and `Aria-0.2.2-setup.exe` built, installed, launched, uninstalled | pending | Done separately by the controller before any release; not recorded here |

## What you need to check by hand

### Exclusive output

| Check | Who |
|-------|-----|
| Settings, Playback: switch "Exclusive mode" on. A 44.1 kHz track plays, then a 96 kHz track plays | pending |
| While a track plays in exclusive mode, other programs (a browser video, for example) are silent; about 15 seconds after you pause, they sound again | pending |
| The DAC, or the Windows sound panel (Advanced, default format), shows the track's own rate (44.1 kHz, then 96 kHz) | pending |
| The player bar indicator in each state: "Bit-perfect" (a 16 or 24-bit FLAC), "Resampled" (a rate the device lacks, if you have one), "Converted" (an ALAC file, or a 32-bit float WAV), "Mixed with a sound" (while a sound plays), "System mixer" (exclusive mode off). The tooltip of each reads correctly | pending |
| The gap at a rate change: a 44.1 kHz track followed by a 96 kHz one has a short silence between them, and the end of the first track is not cut; two tracks of the same rate join without a gap | pending |
| The volume slider is locked at 100 % with its tooltip; mute still works; turning the mode off brings back the saved volume | pending |
| With another program holding the device exclusively, Aria falls back with a notice (or, if Windows refuses the shared stream as well, says so) and playback never stops because of the setting | pending |
| Unplug the output device (or switch it off) while a track plays: playback goes on in the best mode left and a notice appears | pending |
| A mono track is heard in both ears in exclusive mode | pending |
| Dropouts under load (for example a heavy compile running) in exclusive mode; the output thread runs at normal priority | pending |
| Bit-perfect delivery: a DAC that shows its input format shows the track's rate and depth. Samples reaching the device unchanged cannot be recorded through WASAPI loopback and cannot be confirmed any other way here | pending |

### Library: quality

| Check | Who |
|-------|-----|
| After starting 0.2.2 on a copy of a 0.2.1 data folder, the library is read again (a big library may take a while) and the Quality column fills in; folders, tracks, favorites, playlists, history and the saved queue are all still there | pending |
| The Quality column shows "Lossless" for FLAC, ALAC, WAV and AIFF, "Hi-Res" above 48 kHz or above 16 bits, and nothing for MP3, AAC, Ogg and Opus; the tooltip gives codec, depth and rate | pending |
| "Lossless only" on the Tracks tab shows only lossless tracks | pending |
| Russian "Без потерь" and Armenian "Առանց կորստի" fit in the 64 px column; the "Качество" header fits; the "Lossless only" checkbox in Russian and Armenian fits on a narrow window | pending |
| The player-bar format line with a long text (for example a long codec and rate) does not overlap or get cut off | pending |
| Hover tooltips on the indicator, on the badge and on the locked volume appear and read correctly | pending |

### Soundboard

| Check | Who |
|-------|-----|
| Add sounds with the real file dialog, several files at once; they appear with the file name as the name | pending |
| Rename a sound (including 50 characters; an empty name or 51 characters cannot be confirmed); remove a sound (it asks first and the file stays on disk) | pending |
| Tile look with long names and with a hotkey set; a missing file shows a greyed tile and a notice when fired | pending |
| The volume slider of a sound changes how loud it is | pending |
| Click a sound while music plays: both are heard. Click it again while it plays: it restarts | pending |
| Six sounds at once, then a seventh: the oldest stops. "Stop all" stops every sound and leaves the music playing | pending |
| Play sounds to "Second device" with a real second device: heard there only. "Both": heard on both. "My device": heard on your own device only. No second device chosen: they play on your own device and the setting says so | pending |
| A virtual audio cable as the second device, used as the microphone of a voice chat program: the other side hears the sounds | pending |
| A sound plays while exclusive mode is on: the indicator shows "Mixed with a sound" for that time and returns to "Bit-perfect" afterwards | pending |
| A sound playing on your device is cut at a rate change in exclusive mode (stated limitation) | pending |

### Hotkeys

| Check | Who |
|-------|-----|
| A hotkey fires its sound with a game (or another full-screen program) in the foreground | pending |
| A hotkey fires its sound with Aria minimized (a press may take about 100 ms to act) | pending |
| A combination another program holds is refused with a notice and not saved | pending |
| A bare typing key is refused; F1 to F24 and a combination with Ctrl, Alt or Win are accepted; a hotkey used by another sound is refused | pending |
| Setting a numpad key with the "Numpad" switch in the dialog | pending |
| Escape cancels the dialog; Delete or Backspace clears the hotkey | pending |
| The "Global hotkeys" switch off: no hotkey fires; on again: they fire again | pending |
| The "Stop all" hotkey stops the sounds | pending |
| Idle CPU with hotkeys registered (nothing polls, but it was not measured) | pending |

### Translations

The Russian and Armenian texts of the indicator, the Quality column, the Soundboard page, the
hotkey dialog and the new settings were written by the coding agent and have not been read by a
native speaker. Please read them in both languages (Settings, Language) and report anything
wrong, awkward or cut off. The strings are in `locales/ru/main.ftl` and `locales/hy/main.ftl`.

| Check | Who |
|-------|-----|
| Russian texts of all new screens read by a native speaker | pending |
| Armenian texts of all new screens read by a native speaker | pending |
| Aria does not change your music or sound files (compare a file's modified time before and after) | pending |
