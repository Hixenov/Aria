# Aria — Phase 2a (Local Player) Design

- **Date:** 2026-11-06
- **Status:** Approved in conversation; pending written-spec review
- **Scope:** First part of Phase 2. Phase 2 is split into 2a (this document), 2b (favorites,
  local playlists and M3U, playback history, the Home page) and 2c (soundboard, specified
  separately in `2026-11-05-aria-phase2c-soundboard-design.md`).

## 1. Context and intent

Phase 1 delivered a window, navigation, themes, settings, localization, logging and CI, with
every playback control disabled. Phase 2a turns that shell into a working offline music player:
the user adds folders, Aria builds a library in the background, and tracks play through Aria's
own audio engine with a working player bar and queue.

The deliverable is a pre-release, 0.2.0, that plays a real local collection on Windows. Linux
and macOS are built and tested in CI only, and are described that way.

### Decisions made during design

| Decision | Choice |
|---|---|
| Phase split | 2a and 2b with separate PRs and releases; 2c follows |
| Keeping the library current | Fast check at startup plus a Rescan button; no filesystem watcher |
| Opus | Included through libopus (`symphonia-adapter-libopus`, bundled build) |
| Audio engine | Own pipeline: Symphonia, rubato, rtrb, CPAL. Not rodio |
| Local albums and artists | Grouped inside the Local Music page. The Albums and Artists routes stay Phase 3 (Spotify) |
| Local output device selection | In 2a. Spotify Connect devices stay Phase 3 |

## 2. Scope

### In Phase 2a

- Audio engine: decoding of MP3, FLAC, WAV, AAC, OGG Vorbis, Opus and ALAC; sample-accurate
  seeking where the format allows it; volume and mute; gapless transitions; output device
  selection; recovery when a device disappears.
- Library: add, remove and rescan folders; background incremental scanning; tags; artwork
  extraction with a thumbnail cache; SQLite storage.
- Local Music page: Tracks, Albums and Artists tabs, search, sorting, a virtualized table.
- Queue: context plus user queue, play next, add to queue, remove, clear, reorder, persistence.
- Working player bar, queue in the right panel, Playback and Library settings sections.
- Audio-layer capabilities that 2c depends on (section 3.2).

### Not in Phase 2a

- Favorites, local playlists, M3U files, playback history, recently played, the Home page: 2b.
- Soundboard, microphone capture, global hotkeys: 2c.
- Spotify, Connect devices: Phase 3. Tray, media keys, notifications, mini-player: Phase 4.
  Lyrics, equalizer, visualizer: Phase 5.
- Editing tags or writing to music files. Aria never modifies or deletes the user's audio files.
- A filesystem watcher.

## 3. Architecture

### 3.1 Crates

```
crates/
  aria-audio/      no GUI, no aria-* dependency. Devices, decoding, resampling, outputs, engine.
  aria-library/    no GUI, no aria-* dependency. SQLite, scanner, tags, artwork, snapshot.
  aria-core/       backend. Gains the pure `player` module; runtime wires library and engine.
  aria-app/        UI. Local Music page, player bar, queue panel, settings sections.
```

`aria-core` depends on `aria-audio` and `aria-library`. `aria-app` depends on `aria-core` and
reaches the other two only through types `aria-core` re-exports.

### 3.2 `aria-audio`

This crate is a shared sound layer, not only the music player's engine. Phase 2c builds on it
without reworking it, which sets four requirements:

- **Devices.** Enumerate input and output devices with stable identifiers. Where the host
  offers no stable identifier, the device name is the identifier.
- **Decoder and resampler.** Public modules usable outside the player. They support both
  streaming a track and decoding a short clip fully into memory.
- **Outputs.** A trait `AudioOutput` with a CPAL implementation and a fake one for tests. Each
  output is one stream on one device with its own mixer. A mixer sums several inputs, each with
  its own rtrb ring buffer and gain. Several outputs on different devices can be open at once.
  2a opens one output, "music", with one input.
- **No audio device required.** Everything except the CPAL implementation runs under test with
  the fake output.

The **player engine** is a dedicated decoder thread. It reads commands from a channel, decodes
packets, resamples to the output's rate when needed, and writes samples into the music input's
ring buffer. Near the end of a track it opens the next one and keeps writing into the same
buffer, which is what makes transitions gapless.

The real-time callback sums inputs, applies gain and copies to the device buffer. It takes no
locks, allocates nothing and touches no files.

Position is published through an atomic sample counter, tagged with the identity of the track
those samples belong to, so the reported track changes when its first sample is played and not
when it is decoded.

### 3.3 `aria-library`

Owns `library.db` in `AppPaths::data_dir` and the artwork cache in `cache_dir/artwork`. See
section 4. Its public surface is synchronous; the backend calls it from Tokio's blocking pool.

### 3.4 `aria-core`

- `player` module: pure logic with no audio or I/O. Holds the queue model, shuffle, repeat and
  the transport state machine. Given an event, it returns the actions to perform.
- `runtime`: gains library and playback commands, owns the engine handle, persists queue state.
- `Settings` gains the fields in section 7. The schema version stays 1 because every new field
  has a serde default.

### 3.5 Threads

1. **UI.** Never blocks.
2. **Backend (Tokio).** Settings, library commands, player coordination. Scanning and artwork
   decoding run on the blocking pool. A scan can be cancelled.
3. **Decoder thread.** One, owned by the engine.
4. **Real-time callback.** One per open output, owned by CPAL.

### 3.6 Data to the UI

- The backend sends `Event::LibraryUpdated(Arc<LibrarySnapshot>)` after a scan or folder change.
  A snapshot holds all tracks, albums and artists in memory, about 15 MB for 50,000 tracks.
- The UI filters and sorts the snapshot in memory and caches the result until the query, the
  sort or the snapshot changes.
- The UI reads position from the shared atomic. It repaints 4 times a second while playing and
  not at all while paused, so idle CPU stays near zero.

## 4. Library

### 4.1 Database

`library.db`, WAL mode, a single writer. Tables:

- `folders`: id, path, added_at.
- `tracks`: id, folder_id, path, size, mtime, title, artist, album, album_artist, track_no,
  disc_no, year, genre, duration_ms, sample_rate, channels, codec, artwork_hash, added_at.
- `queue_items`: position, list (context or user), track_id.
- `queue_state`: current track, position in ms, shuffle order seed.

Albums and artists are not stored. They are derived when the snapshot is built. The album key is
the album artist, or the track artist when there is none, plus the album title, compared without
regard to case.

Paths are stored as bytes: `as_bytes` on Unix, UTF-16 code units through `encode_wide` on
Windows. Both directions are safe APIs, and names that are not valid Unicode survive.

The schema version lives in `PRAGMA user_version`, and migrations run at open.

- **Corrupt database:** the file is renamed with a `.bak` suffix, a new database is created, and
  a notice tells the user that folders must be added again.
- **Database from a newer Aria:** the library is disabled with a notice and the file is left
  untouched.

### 4.2 Scanning

A scan runs at startup, when a folder is added, and from the Rescan button. Only one scan runs
at a time; a request during a scan queues one more.

1. Walk each folder, keeping files with a supported extension. Symbolic links are not followed,
   so there are no cycles.
2. Compare size and modification time with the stored row. Read tags only for new and changed
   files.
3. Write in transactions of 500 tracks. Send progress at most 4 times a second.
4. Delete rows for files that no longer exist, and send a new snapshot.

Rules:

- **Unreachable root.** If a folder's root cannot be read, its tracks are kept, and the folder is
  marked unavailable. A scan never deletes tracks because a drive is unplugged.
- **Removing a folder** deletes its rows only. Files on disk are not touched.
- **Nested folders.** Adding a folder inside an existing folder, or the same folder twice, is
  refused with a notice. Adding a parent of an existing folder absorbs that folder.
- **Unreadable subdirectory or file.** It is skipped and counted, and the scan continues. The
  final summary reports how many files were skipped. Tracks in an unreadable subdirectory are
  kept, for the same reason as an unreachable root.

### 4.3 Tags

Read with lofty. Broken or missing tags never reject a file:

- No title: the file name without its extension.
- No artist or album: stored as NULL and shown as a localized "Unknown artist" or "Unknown album".
- No duration from tags or properties: 0, shown as `--:--`, and filled in when the track plays.
- A file lofty cannot open at all is skipped and counted.

### 4.4 Artwork

- Source order: the embedded picture, then `cover`, `folder` or `front` (`.jpg`, `.jpeg`, `.png`)
  in the track's directory.
- A 256-pixel thumbnail is made during the scan and stored under the hash of the source image, so
  tracks that share a cover share one file.
- Images over 8,192 pixels on a side, or over 32 MB encoded, are rejected to guard against
  decompression bombs.
- The UI decodes thumbnails off the UI thread and keeps textures in a size-limited LRU cache.
- Clearing the artwork cache deletes the thumbnails and clears `artwork_hash`; the next rescan
  rebuilds them.

### 4.5 Search and sorting

In memory. Search is a case-insensitive substring match over title, artist and album, using
Unicode lowercasing so Russian and Armenian work. Sort keys: title, artist, album, duration, date
added. Sorting is stable, and missing values sort last.

## 5. Playback and queue

### 5.1 Queue model

Two lists:

- **Context:** the list a track was started from. Double-clicking a row makes the visible list,
  in its current sort and filter, the context and starts that track.
- **User queue:** "Play next" inserts at the front and "Add to queue" appends. The user queue
  always plays before the context. When it empties, the context continues where it left off.

The right panel shows the current track and everything upcoming. User-queue items can be dragged
and removed, and the queue can be cleared. Context items are view-only.

### 5.2 Transport

- Play/pause, next, seek, volume, mute.
- Previous: past 3 seconds restarts the track; otherwise it goes to the previous track.
- Shuffle affects only the context and does not repeat a track within one pass. Turning it off
  restores the original order, and the current track keeps playing.
- Repeat: off, all, one. With repeat one, Next still advances, and the track repeats only when it
  ends on its own.
- With repeat off, the end of the last track stops playback and keeps that track loaded at its
  start.
- Space toggles play/pause unless a text field has focus.

### 5.3 Transitions

- Gapless is on by default and can be turned off in settings. Encoder delay and padding are
  trimmed where the file carries that data.
- A next track with a different sample rate goes through the resampler; the output is not rebuilt.
  A different channel count is converted to the output's layout.
- Seek and manual track changes clear the ring buffer and apply a short fade to avoid clicks.

### 5.4 Output device

The setting is "System default" or a specific device.

- **Device lost while playing:** the engine rebuilds the output on the system default at the same
  position and shows a notice.
- **No devices at all:** playback pauses with a notice and the UI keeps working.
- **Chosen device missing at startup:** the system default is used, and the setting is kept. Aria
  returns to the device the next time the output is opened and the device is present.

### 5.5 Persistence

`settings.toml` holds volume, mute, shuffle, repeat, device and gapless. The database holds the
context, user queue, current track and position, saved every few seconds and at exit.

At startup everything is restored paused. Aria never starts sound on its own. Queue entries whose
tracks are gone from the library are dropped on restore.

### 5.6 Track errors

A missing file or a decode failure, at the start or mid-track, skips to the next track with a
notice. After 5 consecutive failures playback stops, so a dead drive does not walk the whole
queue. The count resets when a track plays for at least one second.

## 6. UI

- **Local Music.** Tabs for Tracks, Albums and Artists, a search field and a Rescan button. With no
  folders, the empty state offers Add folder. Scan progress shows on the page. Tracks use a
  virtualized table from `egui_extras`, with sortable headers. Albums and Artists open a filtered
  track list inside the page. A row's context menu offers Play, Play next and Add to queue.
- **Player bar.** Artwork, title and artist; transport, shuffle, repeat and the seek bar; volume,
  mute and the queue button. The Spotify device and lyrics buttons stay disabled with their phase
  tooltips.
- **Right panel.** The queue described in section 5.1.
- **Home** stays an empty state until 2b.
- All new text is in English, Russian and Armenian.

## 7. Settings

Page sections:

- **Playback:** output device, gapless.
- **Library:** folder list with status, Add, Remove, Rescan, Clear artwork cache.

New `Settings` fields, each with a serde default: `volume` (0.0–1.0, sanitized like
`sidebar_width`), `muted`, `shuffle`, `repeat`, `output_device` (optional identifier), `gapless`.
Existing `settings.toml` files keep loading. The folder dialog uses `rfd`, off the UI thread.

Folders live in the database, not in settings, so a folder and its tracks cannot disagree.

## 8. Error handling

Nothing in the audio or library paths may panic in production code. Failures become notices or
logged warnings, as sections 4 and 5 describe.

If the library cannot open at all, the app still runs with the library disabled and a notice,
following the same rule as Phase 1's read-only mode. If no audio output can be opened, the library
stays browsable and play controls report the failure.

Logs record file paths at debug level only, because paths can be personal. Notices name a track by
its title, not its path. Phase 2a sends nothing over the network.

## 9. Testing

Fixtures are short generated sine tones in each format, committed under
`crates/aria-audio/tests/fixtures` with the script that makes them. They are our own, so they
carry no third-party license.

- **`aria-audio`:** decoding every format; seek accuracy; no gap between two tracks, measured on
  the samples reaching the fake output; a different sample rate and channel count between tracks;
  the output failing and being rebuilt at the same position; several outputs open at once; a
  corrupt and a truncated file.
- **`aria-library`:** migrations; incremental scans touching only changed files; an unreachable
  root keeping its tracks; broken tags; Unicode file names; nested folders; a corrupt database; a
  newer database; a synthetic 50,000-track library with a recorded time.
- **`aria-core`:** queue operations, shuffle, repeat, save and restore, restore with missing tracks,
  the 5-failure limit.
- **`aria-app`:** kittest coverage for the table, player bar, queue panel and settings sections.

CI adds `libasound2-dev` on Ubuntu and builds libopus with CMake. Runners have no audio devices, so
CI never exercises real output. That is checked by hand on Windows.

## 10. Dependencies

Current versions at design time, to be confirmed in the plan, including their licenses and
`cargo deny` status:

| Crate | Version | License | Use |
|---|---|---|---|
| symphonia | 0.6.1 | MPL-2.0 | Decoding |
| symphonia-adapter-libopus | 0.3.0 | MIT OR Apache-2.0 | Opus, bundles libopus (BSD-3-Clause) |
| cpal | 0.18.2 | Apache-2.0 | Output and device enumeration |
| rubato | 5.0.1 | MIT | Resampling |
| rtrb | 0.4.1 | MIT OR Apache-2.0 | Ring buffers |
| rusqlite (bundled) | 0.41.1 | MIT | SQLite |
| lofty | 0.25.5 | MIT OR Apache-2.0 | Tags |
| image | 0.25.10 | MIT OR Apache-2.0 | Thumbnails |
| egui_extras | 0.37.0 | MIT OR Apache-2.0 | Table |

Building needs CMake for libopus. `rfd` gains its file dialog feature. Fixtures need ffmpeg once,
to generate them.

## 11. Definition of done (Phase 2a)

- By hand on Windows: a real folder scans; all listed formats play; seek, queue, gapless and
  unplugging headphones work; restarting restores the queue paused.
- Linux and macOS are stated as "built and tested in CI only".
- Measured and recorded in the README: startup time and memory with a large library, CPU while
  playing and while idle.
- `cargo fmt`, clippy with `-D warnings`, tests and `cargo deny` pass on all three systems.
- README, CHANGELOG, PRIVACY and third-party licenses are updated, including libopus.
  `docs/ROADMAP.md` is updated from the 2c branch's split, not edited separately.
- Delivered as a PR and, once the user confirms, a 0.2.0 Windows pre-release.

## 12. Known limitations (Phase 2a)

- Files changed while Aria runs appear after a rescan or the next start.
- Gapless trimming depends on data in the file. Formats without it may keep a small gap.
- Real audio output is verified on Windows only.
- Aria cannot edit tags.
