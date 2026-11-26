# Aria 0.4.0 Media Controls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make media keys and the system's media controls drive Aria, and show the system what is
playing. That means SMTC on Windows, Now Playing on macOS and MPRIS on Linux.

**Architecture:**
- A new `aria-app/src/media_controls.rs` holds the logic, which is easy to test:
  - it maps system events to Aria actions;
  - it decides what to send and when;
  - it does this behind a `MediaBackend` trait.
- The real backend is `souvlaki`. It is created in `logic` from eframe's window handle, through a
  factory in `AppDeps`. Tests use a fake.

**Tech Stack:** Rust 1.99.1, eframe/egui 0.37 with kittest, souvlaki 0.9.3, raw-window-handle 0.6,
crossbeam-channel.

**Spec:** docs/superpowers/specs/2026-11-26-aria-0-4-0-media-controls-design.md

## Global Constraints

**Crates and code rules**
- `souvlaki = "0.9.3"` with its default features (dbus).
- `raw-window-handle = "0.6.2"` is already in the lock.
- `cargo deny` must pass in CI.
- No `unsafe`, no `unwrap`/`expect` outside tests, no `#[allow]`.
- The HWND is passed as a pointer made with `std::ptr::with_exposed_provenance_mut`, from the
  `isize` in `Win32WindowHandle.hwnd`.

**Linux names**
- `PlatformConfig { display_name: "Aria", dbus_name: "aria", hwnd }`.

**Event mapping**

| Event | Action |
|---|---|
| Play | Only when not playing |
| Pause and Stop | Only when playing |
| Toggle | Always |
| Next, Previous | Always |
| SetPosition | Seek there |
| SeekBy with an amount | Seek by that amount |
| Seek with no amount | ±10 000 ms |
| SetVolume | Clamp to 0.0–1.0, then save it in settings |
| Raise | Visible(true), Minimized(false), Focus |
| Quit, OpenUri | Ignored |

- A seek target is clamped to `0..=duration_ms` when the duration is known.

**When things are sent**
- Metadata goes out when the track's identity changes, or when its cover URL appears later.
- Playback (playing or paused, and position) goes out:
  - when playing or paused changes;
  - when the position jumps by more than 2 000 ms against the expected position;
  - every 5 000 ms while playing.
- With nothing loaded, the controls show as stopped and the metadata is cleared.

**Covers**
- A local track: `file:///` + the absolute path of `<cache_dir>/artwork/<ArtworkId>.jpg`, only if
  that file exists.
- A Spotify track: `image_large` or `image`, https only.

**The setting**
- `media_controls: bool`, default `true`, in Settings, General.
- Off drops the backend and ignores any events still queued. On makes a new backend.

**Strings and errors**
- Strings in en, ru and hy:
  - `settings-media-controls = Show in the system's media controls`
  - `settings-media-controls-hint = Media keys, your headset's buttons and the system's media panel control Aria, and show what plays.`
- If the backend cannot be made, log it once at `warn` and carry on. Show no notice.

**Tests and version**
- Tests never touch the real OS media controls. They use a fake backend.
- Version 0.4.0.

## Review Focus

1. **A track change is not a seek.** A new identity sends metadata and its own position. The "jump"
   rule never reports a seek across tracks.
   Test: `a_new_track_sends_metadata_not_a_seek`, in Task 2.
2. **Play while already playing, or Pause while paused, does nothing.** A Spotify state can lag.
   Test: `play_and_pause_follow_the_state`, in Task 2.
3. **Seeking past either end** clamps to 0 or the duration.
   Test: `seeks_are_clamped`, in Task 2.
4. **Events queued just before the setting turns off** do nothing.
   Test: `turning_the_setting_off_drops_the_controls`, in Task 3.
5. **Nothing loaded:** the controls show as stopped, and Play starts what would play next
   (TogglePlay).
   Test: `nothing_loaded_shows_stopped`, in Task 2.

---

### Task 1: The setting and its switch

**Files:** `crates/aria-core/src/settings/mod.rs`,
`crates/aria-app/src/ui/pages/settings/mod.rs` (`general`), `crates/aria-core/src/i18n/mod.rs`
(`KEYS`), `locales/{en,ru,hy}/main.ftl`.

**Interfaces:** produces `Settings.media_controls: bool`. It is deserialized with a lenient helper
that defaults to `true`, like `lenient_play_in_aria`.

- [ ] **Step 1: Failing tests**
  - `media_controls_default_on`: `Settings::default().media_controls` is `true`.
  - `media_controls_reads_off_and_falls_back`:
    - `"media_controls = false"` reads as `false`.
    - `"media_controls = \"x\""` reads as `true`.
  - In the settings page tests, `the_media_controls_switch_updates_settings`: clicking the
    CheckBox "Show in the system's media controls" gives one `UpdateSettings` with
    `media_controls == false`.
- [ ] **Step 2:** Run `cargo test -p aria-core --lib settings::`. Expected: a compile error.
- [ ] **Step 3: Implement**
  - Add the field, its default and the lenient helper.
  - Add the row in `general`, after "Minimize to tray": a checkbox with its hint as caption and
    accessible description, as that row does.
  - Add the keys and strings.
- [ ] **Step 4:** Run the aria-core settings tests and the aria-app settings page tests. Expected:
  PASS.
- [ ] **Step 5: Commit** `feat(settings): media controls switch`

### Task 2: `media_controls.rs`: event mapping and what to send

**Files:** create `crates/aria-app/src/media_controls.rs`, and add `pub mod media_controls;` to
`crates/aria-app/src/lib.rs`.

**Interfaces** (all `pub`):

- **`NowPlaying`** (`#[derive(Clone, Debug, PartialEq)]`):
  - `identity: String`, `title: String`, `artist: String`
  - `album: Option<String>`, `duration_ms: Option<u64>`, `cover_url: Option<String>`
  - `position_ms: u64`, `playing: bool`
- **`MediaEvent`**:
  - `Play`, `Pause`, `Toggle`, `Stop`, `Next`, `Previous`
  - `SetPosition(u64)`
  - `Seek { forward: bool, by_ms: Option<u64> }`
  - `SetVolume(f64)`
  - `Raise`
  - `Ignored`
- **`MediaAction`**:
  - `TogglePlay`, `Next`, `Previous`
  - `SeekTo(u64)`
  - `SetVolume(f32)`
  - `Raise`
- **`fn action_for(event: MediaEvent, now: Option<&NowPlaying>) -> Option<MediaAction>`**
  - With `now == None`, Play and Toggle give `TogglePlay`. Pause, Stop and seeks give `None`.
- **`trait MediaBackend`**:
  - `fn metadata(&mut self, meta: Option<&NowPlaying>);`
  - `fn playback(&mut self, playing: Option<bool>, position_ms: u64);`
    - `None` means stopped.
- **`MediaSync`**:
  - `pub fn new(backend: Box<dyn MediaBackend>) -> MediaSync`
  - `pub fn sync(&mut self, now: Option<&NowPlaying>, at: Instant)`
- **`PERIOD: Duration = 5 s`**, **`JUMP_MS: u64 = 2_000`**, **`SEEK_STEP_MS: u64 = 10_000`**.

- [ ] **Step 1: Failing tests**, with a fake that records each call as an enum in a
  `Rc<RefCell<Vec<_>>>`:
  - **`a_new_track_sends_metadata_not_a_seek`**:
    - Sync track A at 0 ms, playing, at t0. Then track B at 0 ms, at t0 + 1 s.
    - Calls: `metadata(A)`, `playback(Some(true), 0)`, `metadata(B)`, `playback(Some(true), 0)`.
    - Nothing else.
  - **`only_changes_are_sent`**:
    - Sync A at 1 000 ms, playing, at t0. Then A at 1 500 ms at t0 + 500 ms.
    - The second sync sends nothing.
  - **`a_jump_is_sent_at_once`**:
    - A at 1 000 ms at t0. Then A at 30 000 ms at t0 + 500 ms.
    - Gives one `playback(Some(true), 30_000)`.
  - **`the_position_is_resent_every_five_seconds_while_playing`**:
    - Steady position at t0 + 4.9 s sends nothing.
    - At t0 + 5.1 s, one `playback` call.
    - While paused, nothing even after 10 s.
  - **`pausing_is_sent`**: `playing` flips to `false`. Gives `playback(Some(false), pos)`.
  - **`a_late_cover_resends_metadata`**: the same identity, with `cover_url` going from `None` to
    `Some`. Gives one `metadata` call.
  - **`nothing_loaded_shows_stopped`**:
    - After A, `sync(None)` gives `metadata(None)` and `playback(None, 0)`.
    - `action_for(Play, None) == Some(TogglePlay)`.
  - **`play_and_pause_follow_the_state`**:
    - Play while playing gives `None`. Pause while paused gives `None`.
    - Play while paused gives `TogglePlay`.
    - Stop while playing gives `TogglePlay`.
  - **`seeks_are_clamped`**:
    - With duration 60 000 and position 55 000: `Seek{forward: true, by_ms: None}` gives
      `SeekTo(60_000)`.
    - At position 3 000: `Seek{forward: false, by_ms: Some(5_000)}` gives `SeekTo(0)`.
    - `SetPosition(70_000)` gives `SeekTo(60_000)`.
  - **`volume_is_clamped`**: `SetVolume(1.7)` gives `SetVolume(1.0)`, and `SetVolume(-1.0)` gives
    `SetVolume(0.0)`.
- [ ] **Step 2:** Run `cargo test -p aria-app --lib media_controls` in the background, with the log
  in `.local/build-logs`. Expected: FAIL.
- [ ] **Step 3: Implement**
  - `MediaSync` keeps `last: Option<NowPlaying>` and `sent_at: Instant`.
  - The expected position is `last.position_ms + elapsed` while playing.
- [ ] **Step 4:** Same command. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): media controls sync and event mapping`

### Task 3: The souvlaki backend and the app

**Files:** `crates/aria-app/src/media_controls.rs` (the `SouvlakiBackend`), `crates/aria-app/src/app.rs`,
`crates/aria-app/src/main.rs`, `crates/aria-app/Cargo.toml`, `Cargo.toml`.

**Interfaces:**
- **`pub type MediaFactory = Box<dyn Fn(Option<isize>, crossbeam_channel::Sender<MediaEvent>, egui::Context) -> Result<Box<dyn MediaBackend>, String>>`**
  - The `isize` is the Win32 HWND, when there is one.
  - The backend must wake the UI with `ctx.request_repaint()` after each event.
- **`pub fn system_factory() -> MediaFactory`** builds a `SouvlakiBackend`:
  - Make `MediaControls::new(PlatformConfig{..})`.
  - Then `attach(move |event| { tx.send(map(event)); ctx.request_repaint() })`.
  - Then send `set_metadata`, and `set_playback` with `MediaPlayback::{Stopped, Paused{progress},
    Playing{progress}}`.
- **`AppDeps.media: Option<MediaFactory>`**: `main` sets `Some(system_factory())`, and tests set a
  fake.
- **In the app:**
  - **New fields:** `media: Option<MediaSync>`, `media_events: Option<Receiver<MediaEvent>>`, and
    `media_failed: bool`.
  - **Making it:** in `logic(ctx, frame)`, when `settings.media_controls`, `media` is `None` and
    `!media_failed`:
    - Read the HWND from `frame.window_handle()` (`RawWindowHandle::Win32`). Use `None` on other
      platforms, or when there is no handle yet; on Windows, skip this frame and retry.
    - Call the factory.
    - On `Err`, log at `warn` once and set `media_failed`.
  - **Each `logic` frame:**
    - Drain the events, and map each through `action_for` with the current `NowPlaying`.
    - `TogglePlay`, `Next` and `Previous` become the same `Command`s the player bar sends.
    - `SeekTo(ms)` becomes `Command::Seek(ms)`.
    - `SetVolume(v)` becomes `update_settings` with that volume.
    - `Raise` sends the three viewport commands to `ViewportId::ROOT`.
    - Then `media.sync(now, Instant::now())`.
  - **`NowPlaying` comes from what the player bar shows:**
    - **Spotify, when the bar's active player is Spotify:**
      - name, artists, album, `duration_ms` and `uri` from `NowItem::Track`;
      - position from `progress_ms`, plus the time since `at` while playing;
      - playing from `state.playing`.
    - **Local:**
      - title, artists and album from the library's track for `player.current`;
      - the duration from `player.duration_ms` (when greater than 0);
      - position and playing from `clock.read()` when its token equals `player.token`;
      - the cover from the track's `artwork`;
      - the identity from the file path.
    - **Nothing**, when neither applies.
  - **The setting turning off:** set `media` and `media_events` to `None`, and `media_failed` to
    `false`, so turning it on again tries again. Events still queued are never read.
  - **`on_exit`:** drop `media` before the backend shuts down.

- [ ] **Step 1: Failing tests** in `app.rs` `mod media`, with a fake factory and a fake backend
  that records calls and keeps the event `Sender` for the test:
  - `media_keys_become_player_commands`:
    - Toggle gives `Command::TogglePlay`. Next and Previous give their commands.
    - `SetPosition(5_000)` with a loaded local track gives `Command::Seek(5_000)`.
  - `raise_shows_the_window`: Raise gives a Focus viewport command for ROOT.
  - `the_playing_track_reaches_the_system`:
    - With a library track "Song" by "Ann" on album "LP" playing, the fake's last `metadata` has
      those fields.
    - Its `cover_url` is `None` when the thumbnail file is missing.
  - `turning_the_setting_off_drops_the_controls`:
    - Queue Toggle, then apply settings with `media_controls: false`, then run `logic`.
    - No `TogglePlay` is sent, and the factory is not called again until it is turned back on.
  - `a_failing_factory_is_tried_once`: a factory returning `Err` is called once over 3 frames.
- [ ] **Step 2:** Run the aria-app tests in the background. Expected: FAIL.
- [ ] **Step 3: Implement**
  - Add `souvlaki` and `raw-window-handle` to the workspace dependencies and to aria-app.
  - `SouvlakiBackend` is compiled on all platforms (souvlaki has an empty platform). Its code is
    not covered by tests.
- [ ] **Step 4:** Run the aria-app tests and `cargo clippy -p aria-app --all-targets -D warnings`.
  Expected: PASS, and clippy clean.
- [ ] **Step 5: Commit** `feat(app): media keys and the system media controls via souvlaki`

### Task 4: Docs, version, full checks

**Files:**
- `Cargo.toml` (version 0.4.0)
- `CHANGELOG.md`
- `README.md` (the status list)
- `docs/PRIVACY.md` (a section "System media controls (since 0.4.0)")
- `docs/ROADMAP.md` (Phase 4: media keys in 0.4.0)
- create `docs/testing/0.4.0-manual-check.md`

- [ ] **Step 1:** The docs.
  - PRIVACY.md says:
    - Title, artist, album, cover and position are handed to the OS's media session.
    - For a Spotify track, the OS fetches the cover from Spotify's image server.
    - Nothing else leaves the computer.
    - The setting turns it off.
  - The manual check has every visible step marked pending. It includes the Spotify desktop app
    conflict, and the macOS and Linux lines.
- [ ] **Step 2:** Run `cargo update --workspace --offline`, then the full workspace tests and
  clippy, in the background. Expected: all pass.
- [ ] **Step 3: Commit** `chore: 0.4.0 media controls docs and version`
