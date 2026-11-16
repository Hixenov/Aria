# Aria — Phase 2c (Lossless Output and Soundboard) Design

- **Date:** 2026-11-15
- **Status:** Design approved in conversation; pending written-spec review
- **Release:** 0.2.2 (pre-release)
- **Scope:** Third part of Phase 2. Phase 2a (0.2.0) delivered the local player and 2b (0.2.1)
  favorites, playlists and history.

## 1. Context and intent

Aria 0.2.1 plays lossless files, but through the system mixer: the audio is resampled to the
device's rate and Aria's volume is applied in software, so what reaches the device is not the
file's samples. Phase 2c adds two things:

1. **Lossless.** An optional exclusive output on Windows that sends a track's samples to the
   device unchanged, an indicator that says honestly what is happening to the sound, and a
   Lossless / Hi-Res badge and filter in the library.
2. **Soundboard.** A page of sounds the listener can fire with a click or a global hotkey, heard
   on their own device, on a second device (for example a virtual cable into a voice chat), or
   both.

Everything stays offline and local. Aria still never writes to or deletes audio files.

### Decisions made during design

| Decision | Choice |
|----------|--------|
| What "lossless" means | Both bit-perfect output and a badge with a filter |
| What 2c holds | Lossless and the soundboard, in one release (0.2.2) |
| Soundboard use | Own device, a second device, or both |
| Where sounds go | One setting for all sounds, not one per sound |
| Global hotkeys | In 2c: one per sound and one "stop all" |
| Microphone | Aria does not touch it. Mixing a microphone into the second device is not in 2c |
| Platforms | Exclusive output is Windows only. Hotkeys are verified on Windows only |

## 2. Scope

### In Phase 2c

- Exclusive output mode (Windows, WASAPI), off by default.
- An output indicator in the player bar.
- Bit depth stored for every track; Lossless and Hi-Res badges; a "Lossless only" filter.
- Soundboard page: add, rename, remove sounds; per-sound volume and hotkey; play; stop all.
- A second output device for sounds and a setting for where sounds go.
- Global hotkeys with a switch that turns them all off.
- All new text in English, Russian and Armenian.

### Not in Phase 2c

- Exclusive output on Linux or macOS.
- Microphone capture, microphone pass-through, or installing any audio driver.
- A route per sound; reordering sound tiles; folders of sounds; sound trimming or effects.
- DSD, dithering, replay gain, an equalizer.
- Media keys and shortcuts for the player (Phase 4). Tray (Phase 4).

## 3. Lossless

### 3.1 Why the existing mixer can stay

The engine decodes to 32-bit float and mixes in float. An integer sample of 24 bits or fewer
converts to `f32` and back without loss. So the float path is bit-exact when all of these hold:

- the output runs at the track's sample rate, so nothing is resampled;
- the music gain is exactly 1.0;
- nothing else is mixed into the stream;
- the track's channels reach the device unchanged (mono to both channels counts as unchanged);
- the source is integer PCM of 24 bits or fewer after decoding.

Only the output is new. The decoder, resampler, mixer and engine thread stay.

One exception is stated to the user: the mixer fades an input in and out over 5 ms when
playback starts, pauses, resumes or seeks, to avoid clicks. Those 5 ms are not bit-exact. Gapless
joins between tracks have no fade.

### 3.2 Exclusive output

A second implementation of the output traits, beside the CPAL one, built on the `wasapi` crate
(section 9). It exists only on Windows. `unsafe` stays forbidden in Aria's own code.

- **Opening.** The engine asks for the track's sample rate and bit depth. The output tries, in
  order: the track's own depth, 24 bits in a 32-bit container, 32-bit integer, 16-bit. The first
  format the device accepts in exclusive mode is used. Converting to a wider integer format is
  exact; converting 24-bit audio to a 16-bit device is not, and the indicator says so.
- **Rate not supported.** If the device accepts no format at the track's rate, the engine opens
  the nearest rate the device supports (preferring a whole multiple, then the next higher, then
  the highest) and resamples as it does today. The indicator says "resampled".
- **Exclusive mode not available** (another program holds the device, or the driver refuses):
  the engine falls back to the normal shared output, a notice is shown once per attempt, and the
  indicator says the sound goes through the system mixer. Playback is never refused because of
  this setting.
- **Rate change between tracks.** When the next track needs a different rate, the engine lets
  the current track finish, closes the output and opens it again. There is a short silence at
  that join, and anything else playing on that output (section 4.4) is cut. Tracks with the same
  rate join as before, gapless.
- **Releasing the device.** The existing rule holds: 15 seconds after pausing, the output is
  closed, so other programs can use the device again.
- **Device loss** is handled as today: fall back, notice.
- **Device identity.** The chosen output device in settings must open the same physical device
  in both implementations. The plan verifies how the stored identifier maps to a WASAPI endpoint
  and falls back to matching by name.

### 3.3 Volume

While exclusive mode is on, Aria's volume slider is disabled and shows 100 %. The music gain is
1.0. Mute still works. A line under the setting tells the listener to set the level on the
device or amplifier. Turning the mode off restores the saved volume.

### 3.4 Output indicator

The engine reports how the current track reaches the device. The player bar shows the track's
format (for example "FLAC 24/96") and one state:

| State | Shown when |
|-------|-----------|
| Bit-perfect | Exclusive output, every condition of section 3.1 holds |
| Exclusive, resampled | Exclusive output at a rate other than the track's |
| Exclusive, converted | Exclusive output, but the depth was reduced, the channels were changed, or the source is float or wider than 24 bits |
| Exclusive, mixed with a sound | A soundboard sound is playing into the same stream |
| System mixer | Shared output |

A lossy track in exclusive mode shows the same states: they describe the path, and the badge
(section 3.5) describes the file. A tooltip explains each state in one sentence and mentions the
5 ms fades.

### 3.5 Badge and filter

- The scan stores each track's bit depth (`0` when the format has none, as with MP3).
- A track is **lossless** if its codec is FLAC, ALAC, or integer or float PCM (WAV, AIFF, CAF).
  The scan must tell ALAC from AAC inside `.m4a`.
- A lossless track is **Hi-Res** if its rate is above 48 kHz or its depth above 16 bits.
- The track table (Local Music and Liked) gains a narrow Quality column with a "Lossless" or
  "Hi-Res" badge and nothing for lossy tracks. Its tooltip gives codec, depth and rate.
- The Local Music Tracks tab gains a "Lossless only" toggle beside the search field.
- Playlists and Home rows do not show the badge.

## 4. Soundboard

### 4.1 Sounds

A sound is a reference to an audio file in any format Aria plays. Aria stores its path, a name,
a volume and an optional hotkey. The file is never copied, changed or deleted.

- **Name.** Trimmed; 1 to 50 characters. Defaults to the file name without its extension.
- **Volume.** 0 to 100 %, default 100 %.
- **Count.** At most 100 sounds. The same file may be added more than once.
- **Order.** The order they were added in.
- **Missing file.** The sound stays, its tile is greyed, and playing it gives a notice.

### 4.2 Playing

- A click on a tile, or its hotkey, plays the sound from its start. Firing a sound that is
  already playing starts it again.
- At most 6 sounds play at once. A seventh stops the one that started first.
- "Stop all" stops every sound. It does not touch the music.
- Sounds are not tracks: they do not enter the queue, the history or "Recently played", and
  they do not move the player bar.
- A sound is decoded on a worker thread and streamed, as tracks are, so its length is not
  limited. The time from a click to the first samples written is measured and reported in the
  README.

### 4.3 Where sounds go

Settings gain a Soundboard section:

- **Second device:** an output device, or none.
- **Play sounds to:** "My device" (default), "Second device", or "Both".

"My device" is the device the music uses. Rules:

- With no second device chosen, "Second device" and "Both" behave as "My device", and the
  setting shows a line saying so.
- If the second device is the same device as the music's, the sound plays once.
- The second device is opened in shared mode when a sound needs it and closed 15 seconds after
  the last sound on it ends. If it cannot be opened, a notice is shown. With "Both", the sound
  still plays on the listener's device; with "Second device", it does not play.

### 4.4 Sounds and the music output

A sound for "My device" is one more input of the mixer that the music uses.

- If the music output is not open, the engine opens it for the sound, in the mode the settings
  ask for and at the device's own rate. With no music playing, it is closed 15 seconds after the
  last sound ends.
- In exclusive mode, the stream is not bit-perfect while a sound plays into it. The indicator
  changes for that time (section 3.4).
- When the engine reopens the output for a track with a different rate (section 3.2), sounds
  playing into it are cut. This is a stated limitation.
- A sound that goes only to the second device does not affect the music stream.

### 4.5 Voice chat

To be heard in a voice chat, the second device must be a virtual audio cable that the chat
program uses as its microphone. Aria does not provide or install one. With only a cable, the
chat hears the sounds and not the listener's voice; mixing the two needs another program. The
README says this plainly, with no claim that any particular cable or chat program was tested
unless it was.

### 4.6 Hotkeys

Built on the `global-hotkey` crate (section 9).

- Each sound may have one hotkey, and there is one hotkey for "Stop all".
- **Allowed keys.** A hotkey must include Ctrl, Alt or Win, unless its key is F1 to F24 or a
  numpad key. This keeps plain typing keys from being taken.
- **Setting one.** A dialog waits for the keys to be pressed. Escape cancels; Delete or
  Backspace clears the hotkey.
- **Conflicts.** A hotkey already used inside Aria is refused. A hotkey the system refuses to
  register (another program has it) is not saved. Both give a notice.
- **Switch.** "Global hotkeys" in Settings, Soundboard, on by default. Off unregisters all of
  them; the hotkeys stay stored.
- Hotkeys work while Aria is running, including when minimized. Aria receives only the
  registered combinations from the system. It does not read other keystrokes.
- On Linux and macOS the feature is built where the crate supports it and is labelled as not
  verified.

## 5. Data

One new migration (database `user_version` 2 to 3):

```sql
ALTER TABLE tracks ADD COLUMN bits_per_sample INTEGER NOT NULL DEFAULT 0;
UPDATE tracks SET mtime = 0;
CREATE TABLE sounds (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    path     BLOB NOT NULL,
    name     TEXT NOT NULL,
    volume   REAL NOT NULL DEFAULT 1.0,
    hotkey   TEXT,
    position INTEGER NOT NULL
);
```

Resetting `mtime` makes the next scan read every file again, which fills the depth and corrects
the codec of `.m4a` files. The plan confirms that the scan decides by `mtime` and uses another
marker if it does not. Favorites, playlists and history are keyed by path and are not
affected. A 0.2.1 database keeps everything; after the upgrade, 0.2.1 refuses the database and
runs with the library off, as 0.2.0 does with a 0.2.1 database.

New `Settings` fields, each with a serde default so existing files load:

| Field | Default |
|-------|---------|
| `exclusive_output` | `false` |
| `soundboard_device` | none |
| `soundboard_target` | `mine` |
| `global_hotkeys` | `true` |
| `stop_sounds_hotkey` | none |

## 6. Architecture

- **`aria-audio`.** The exclusive output (Windows only) behind the existing output traits; a
  way for the engine to ask an output for a format; sound playback as extra mixer inputs on the
  music output and on a second output; engine events for the output state and for sounds
  starting and ending. Conversion from float to each integer format is a pure, tested function.
- **`aria-library`.** The migration, the depth and codec in the scan, and methods for the
  `sounds` table. The library thread stays the only owner of the database.
- **`aria-core`.** Commands and events below; the settings; hotkey validation as a pure
  function. A soundboard service maps sound ids to engine requests.
- **`aria-app`.** The indicator, the Quality column and filter, the Soundboard page, the
  settings, and the hotkey manager. The hotkey manager lives where the platform needs it (on
  Windows, the thread with the window's message loop) and turns key presses into commands. The
  first task of the plan proves this works inside the eframe window before anything is built on
  it.

Commands: `AddSounds(Vec<PathBuf>)`, `RenameSound { id, name }`, `SetSoundVolume { id, volume }`,
`SetSoundHotkey { id, hotkey }`, `RemoveSound(id)`, `PlaySound(id)`, `StopSounds`. Settings
changes go through the existing settings command.

Events: `Soundboard(Arc<SoundboardState>)` with the sounds, which are playing and which files
are missing, sent at startup and after every change; `Output(OutputState)` with the track
format and the state of section 3.4.

A command that names an id that does not exist changes nothing and is logged at debug level.
Nothing here blocks the UI thread.

## 7. Error handling

- Exclusive mode not available, a rate not supported, a device lost: section 3.2. Playback
  continues in the best mode available and the indicator tells the truth.
- A sound whose file is missing or cannot be decoded: a notice, nothing plays, music unaffected.
- A database error during a soundboard command: rolled back, a notice, the UI keeps its last
  state.
- A hotkey that cannot be registered at startup (another program took it since): the hotkey
  stays stored, its tile shows it as inactive, and one notice lists how many are inactive.
- A library that failed to open leaves the Soundboard page empty and answers commands with the
  existing "library unavailable" notice.
- Paths in logs go through the existing redaction.

## 8. Privacy

The soundboard stores file paths, names and hotkeys in the local database and settings. Nothing
is sent anywhere. Global hotkeys deliver only the registered combinations to Aria.
`docs/PRIVACY.md` says both. No telemetry and no network requests are added.

## 9. Dependencies

| Crate | Version | License | Use |
|-------|---------|---------|-----|
| `wasapi` | 0.27 | MIT | Exclusive output, Windows only |
| `global-hotkey` | 0.10 | Apache-2.0 OR MIT | Global hotkeys |

Both must pass `cargo deny` with their dependency trees. If either does not, the plan stops and
asks before choosing another crate.

## 10. Testing

- **Bit-exact path (automated):** FLAC and WAV files of 16 and 24 bits go through the real
  decoder, the mixer at gain 1.0 and the integer conversion, and the result must equal the
  source samples exactly, outside the 5 ms fade at the start. The same test with a gain other
  than 1.0, with a resample, and with a sound mixed in must not be reported as bit-perfect.
- **Format choice:** the order of section 3.2 against a fake device that accepts chosen
  formats; the nearest-rate rule; fallback to shared.
- **Engine:** rate change reopens the output; same rate stays gapless; the output state event
  for every row of section 3.4; volume locked in exclusive mode.
- **Library:** the migration on a database made by 0.2.1 that holds tracks, favorites,
  playlists and plays; the rescan filling depth; ALAC told from AAC; every rule of section 4.1.
- **Soundboard:** the limit of 6 with the oldest stopped; restart of a playing sound; stop all;
  every row of the routing rules in section 4.3 with fake outputs; sounds cut by a reopen.
- **Hotkeys:** the allowed-key rule and conflicts as unit tests. Registration with the system is
  checked by the plan's first task and then by hand.
- **App:** the pages and settings in the test harness; the pure parts as unit tests.
- **By hand:** `docs/testing/phase2c-manual-check.md`. What the device actually receives in
  exclusive mode cannot be recorded through WASAPI loopback, so bit-perfect delivery to the
  device is a hand check, with a DAC that shows its input rate if one is at hand. Hotkeys in a
  full-screen game and sounds heard in a voice chat are hand checks too. Every such line starts
  as pending.

## 11. Known limitations

- Exclusive output is Windows only. While it plays, other programs are silent on that device.
- A short silence where consecutive tracks differ in sample rate, in exclusive mode.
- The first 5 ms after start, resume and seek are faded and not bit-exact.
- Float sources and integer sources wider than 24 bits are not reported as bit-perfect.
- Sounds on the listener's device break bit-perfect output while they play, and are cut when the
  output is reopened for a different rate.
- Aria provides no virtual cable and no microphone mixing.
- Hotkeys are verified on Windows only, and only while Aria runs.
- The Russian and Armenian texts are not reviewed by a native speaker.

## 12. Definition of done

- Every item in section 2 works, and the tests in section 10 pass, with `cargo fmt`, `clippy -D
  warnings` and `cargo deny` clean on Windows, Linux and macOS in CI.
- A 0.2.1 database opens in 0.2.2 with nothing lost.
- README, CHANGELOG, ROADMAP, `docs/PRIVACY.md` and the manual check list are up to date, and
  say plainly what nobody has verified.
- `aria.exe` and `Aria-0.2.2-setup.exe` are built and tested locally (launch; install, launch,
  uninstall) before any release, with the PC's real Aria data copied aside first and restored
  afterwards. Merging, tagging and publishing wait for the user's explicit yes.
