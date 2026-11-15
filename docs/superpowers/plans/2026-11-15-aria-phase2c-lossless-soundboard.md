# Aria Phase 2c (Lossless Output and Soundboard) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an exclusive, bit-perfect output on Windows with an honest indicator, a Lossless / Hi-Res badge and filter, and a soundboard with a second output device and global hotkeys, released as 0.2.2.

**Architecture:** The float mixer stays; a second output implementation (WASAPI exclusive) converts its float output to the device's integer format, which is exact for sources of 24 bits or fewer at gain 1.0. The engine asks an output for the track's format, reopens it when the rate changes, and reports how the sound reaches the device. Sounds are extra mixer inputs on the music output and on a second, shared output. Hotkeys are registered by the app on the window's thread and become ordinary commands.

**Tech Stack:** Rust 1.99.1, Symphonia, CPAL 0.18, rubato, `wasapi` 0.27 (Windows only), `global-hotkey` 0.10, rusqlite, lofty, Tokio, egui/eframe 0.37 with `egui_extras` and `egui_kittest`, Fluent (`locales/{en,ru,hy}/main.ftl`).

**Spec:** `docs/superpowers/specs/2026-11-15-aria-phase2c-lossless-soundboard-design.md`

## Global Constraints

- No `unwrap`/`expect` in production code, no `unsafe`, no suppressed warnings (`#[allow]`), no fake or stubbed features. Tests may panic with a message.
- Aria never writes to, renames or deletes audio files. No network requests, no telemetry. Aria never reads the microphone and never reads keystrokes other than registered hotkeys.
- Never block the UI thread or an audio callback. No allocation, lock or log call on a real-time audio thread. Every change from the UI is a `Command`.
- Paths in logs go through the existing redaction (`aria_core::redact`). Error text shown to the user holds no paths.
- Every user-facing string exists in `locales/en/main.ftl`, `locales/ru/main.ftl` and `locales/hy/main.ftl` under the same key, and in the `KEYS` list of `crates/aria-core/src/i18n/mod.rs`.
- Do not claim something works unless it was built and tested. Docs and reports say what was not verified.
- Exact values:
  - at most **6** sounds play at once and at most **100** sounds are stored;
  - sound names are **1 to 50 characters** after trimming, counted in `char`s;
  - sound volume is **0.0 to 1.0**, default **1.0**;
  - an idle second output closes after **15 s** (`IDLE_GRACE`);
  - the mixer fade stays **5 ms** (`FADE_MS`);
  - Hi-Res means lossless with a rate above **48 000 Hz** or a depth above **16** bits;
  - bit-perfect needs an integer source of **24 bits or fewer**.
- Version is **0.2.2**. The database `user_version` goes from 2 to **3**. The settings schema version does not change.
- Each task ends with `cargo fmt --all --check`, `cargo clippy -j 4 --workspace --all-targets --locked -- -D warnings` and `cargo test -j 4 --workspace --locked` passing (the whole workspace: a test in another crate checks the locale keys). Run cargo commands one at a time in the foreground. Use `-j 4`. Warnings about "hard linking files in the incremental compilation cache" come from the exFAT drive and are not code warnings.
- `cargo deny` is not installed on this PC and must not be installed; it runs in CI.
- Never start `aria.exe` against the PC's real data: `%APPDATA%\Aria` holds the user's 0.2.1 library, and a 0.2.2 build would upgrade it. Tests use temporary folders.
- Environment for every shell (Git Bash):
  ```bash
  cd /e/claude/aria
  export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0=E:/claude/aria
  export CMAKE="C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe" CMAKE_GENERATOR="Visual Studio 17 2022"
  ```
- Commits end with the attribution line the dispatch gives. Work on branch `feat/phase2c-lossless-soundboard`. Do not push, merge, tag or publish.

## Review Focus

1. **A state that claims more than is true.** "Bit-perfect" must never show with a gain other than 1.0, a resample, a reduced depth, changed channels, a float or wider-than-24-bit source, or a sound in the stream. Tests: Task 4 `the_state_is_bit_perfect_only_when_every_condition_holds`, Task 5 `each_condition_that_breaks_bit_perfect_is_reported`.
2. **A device that refuses.** Exclusive mode busy, a rate the device lacks, a device unplugged mid-track: playback goes on in the best mode left and never stops because of the setting. Tests: Task 5 `a_refused_exclusive_open_falls_back_to_shared`, `an_unsupported_rate_opens_the_nearest_and_resamples`.
3. **Tracks of different rates in a row.** 44.1 kHz then 96 kHz then 44.1 kHz in exclusive mode: each is heard whole, in order, with the right position on the clock. Same-rate neighbours stay gapless. Test: Task 5 `a_rate_change_reopens_the_output_between_tracks`.
4. **A seventh sound, the same sound twice, a missing file.** The oldest stops, a repeat restarts, a missing file plays nothing and leaves the music alone. Tests: Task 7 `a_seventh_sound_stops_the_oldest`, `firing_a_playing_sound_restarts_it`, `a_sound_that_cannot_be_opened_changes_nothing`.
5. **Hotkeys that take typing keys or collide.** A bare letter, a duplicate inside Aria, a combination the system refuses. Tests: Task 9 `a_plain_typing_key_is_refused`, `a_hotkey_used_by_another_sound_is_refused`; Task 12 `a_hotkey_the_system_refuses_is_not_saved`.

## File Structure

| File | Responsibility |
|------|----------------|
| `Cargo.toml`, `crates/aria-audio/Cargo.toml`, `crates/aria-app/Cargo.toml`, `deny.toml` | The two new dependencies |
| `crates/aria-app/examples/hotkey_probe.rs` (new) | Proof that a global hotkey reaches an eframe window |
| `crates/aria-library/src/migrations.rs`, `tags.rs`, `scan.rs`, `model.rs`, `db.rs` | Migration 3, bit depth, ALAC told from AAC, `Quality` |
| `crates/aria-library/src/sounds.rs` (new), `tests/sounds.rs` (new) | `Sound`, `SoundId`, the `Library` methods for the `sounds` table |
| `crates/aria-audio/src/sample_format.rs` (new) | Float to integer conversion, format choice, nearest rate, `OutputState` |
| `crates/aria-audio/src/output.rs`, `fake.rs`, `cpal_output.rs` | Output requests and what an output reports about itself |
| `crates/aria-audio/src/wasapi_output.rs` (new, Windows only), `system_output.rs` (new) | The exclusive output; a factory that picks shared or exclusive |
| `crates/aria-audio/src/engine.rs` | Exclusive mode, reopening on a rate change, the output state event |
| `crates/aria-audio/src/sounds.rs` (new) | Sound voices: decode, resample, write to inputs on one or two outputs |
| `crates/aria-core/src/hotkey.rs` (new) | `Hotkey`: parse, format, the allowed-key rule |
| `crates/aria-core/src/soundboard.rs` (new) | `SoundboardState`; maps commands to library requests and engine commands |
| `crates/aria-core/src/command.rs`, `event.rs`, `library_service.rs`, `playback_service.rs`, `runtime.rs`, `settings/mod.rs`, `notice.rs` | Commands, events, requests, settings, notices |
| `crates/aria-app/src/hotkeys.rs` (new) | Registers hotkeys with the system and turns presses into commands |
| `crates/aria-app/src/ui/layout/player_bar.rs` | The output indicator; the locked volume |
| `crates/aria-app/src/ui/pages/local_music/` | The Quality column and the "Lossless only" toggle |
| `crates/aria-app/src/ui/pages/soundboard.rs` (new), `ui/widgets/hotkey_dialog.rs` (new) | The Soundboard page and the dialog that captures a hotkey |
| `crates/aria-app/src/ui/pages/settings/playback.rs`, `settings/soundboard.rs` (new) | The exclusive switch; the Soundboard settings |
| `crates/aria-app/src/nav.rs`, `shortcuts.rs`, `ui/layout/sidebar.rs` | `Route::Soundboard` |
| `locales/*/main.ftl`, `crates/aria-core/src/i18n/mod.rs` | New text |
| `README.md`, `CHANGELOG.md`, `docs/PRIVACY.md`, `docs/ROADMAP.md`, `docs/testing/phase2c-manual-check.md` (new), `packaging/windows/aria.iss` | Version and docs |

---

### Task 1: Dependencies and the hotkey probe

**Files:**
- Modify: `Cargo.toml`, `crates/aria-audio/Cargo.toml`, `crates/aria-app/Cargo.toml`, `Cargo.lock`, `deny.toml` (only if a new license needs listing; stop and report instead of adding a license outside MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, MPL-2.0)
- Create: `crates/aria-app/examples/hotkey_probe.rs`

**Interfaces:**
- Produces: workspace dependencies `wasapi = "0.27"` (used by `aria-audio` under `[target.'cfg(windows)'.dependencies]`) and `global-hotkey = "0.10"` (used by `aria-app`).

- [ ] **Step 1:** Add the two dependencies. Run `cargo tree -e normal -p aria-app --format "{p} {l}" --locked` after refreshing the lock and list every license the two crates bring in. Any license outside the allowed set: stop and report.
- [ ] **Step 2:** Write `hotkey_probe.rs`: an eframe window that creates a `GlobalHotKeyManager` on the main thread before `eframe::run_native`, registers Ctrl+Alt+F15, reads `GlobalHotKeyEvent::receiver()` in `update`, prints `hotkey pressed` and `hotkey released` to stdout with the frame time, requests a repaint every 50 ms, and closes itself after 20 s or after the first press.
- [ ] **Step 3:** Run `cargo run -j 4 --locked -p aria-app --example hotkey_probe` in the background, and while it runs send the combination from PowerShell (`Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('^%{F15}')`) with another window focused. Expected: `hotkey pressed` in the probe's output. Repeat with the probe's window minimized.
- [ ] **Step 4:** Record in the report what happened in each case. If the event does not arrive, stop and report BLOCKED with the evidence; later tasks depend on it. If the press cannot be sent from the shell, report that the probe is unverified and what was tried.
- [ ] **Step 5: Commit** `build: add wasapi and global-hotkey, and a hotkey probe`.

### Task 2: Library — bit depth, codec and quality

**Files:**
- Modify: `crates/aria-library/src/migrations.rs`, `tags.rs`, `scan.rs`, `model.rs`, `db.rs`, `snapshot.rs`, `lib.rs`
- Test: `crates/aria-library/tests/db.rs`, `tests/tags.rs`, `tests/scan.rs`

**Interfaces:**
- Produces:
  - `Track::bits_per_sample: u16` (0 when the format has none).
  - `pub enum Quality { Lossy, Lossless, HiRes }` and `impl Track { pub fn quality(&self) -> Quality }`.
  - Codec names for MP4 files become `"alac"` or `"aac"` (other MP4 codecs stay `"mp4"`). Lossless codecs are exactly `flac`, `alac`, `wav`, `aiff` and `caf` when its data is PCM.
  - `SCHEMA_VERSION == 3`; the migration of spec section 5 without the `sounds` table (Task 3 adds it to the same migration).

- [ ] **Step 1: Write the failing tests.**
  - `tests/db.rs`: `a_0_2_1_database_upgrades_with_everything_kept` (build a version-2 database with a folder, tracks, a queue, a favorite, a playlist and a play; open it; all are still there, `user_version` is 3, every track has `bits_per_sample == 0`).
  - `tests/scan.rs`: `the_first_scan_after_the_upgrade_reads_every_file_again` (after the upgrade, a scan of unchanged files reports them updated and fills `bits_per_sample`); `a_second_scan_changes_nothing`.
  - `tests/tags.rs`: `flac_and_wav_report_their_depth` (16 and 24 bit fixtures); `mp3_has_no_depth`; `alac_and_aac_in_m4a_are_told_apart`.
  - Unit, `model.rs`: `quality_follows_codec_rate_and_depth` with the table: flac 16/44100 → Lossless; flac 24/44100 → HiRes; flac 16/96000 → HiRes; flac 16/48000 → Lossless; alac 24/48000 → HiRes; mp3 0/44100 → Lossy; aac → Lossy; wav 16/44100 → Lossless; unknown → Lossy.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-library --locked` — expected: the new tests fail.
- [ ] **Step 3: Implement.** First read how `scan.rs` decides a file is unchanged. If it compares `mtime`, the migration's `UPDATE tracks SET mtime = 0` is enough; if not, add what is needed so every file is read once and say so in the report. Depth comes from lofty's `bit_depth()`; the MP4 codec from `lofty::mp4::Mp4File`'s properties. Generate any missing fixtures with the tools the existing fixtures were made with (see `crates/aria-audio/tests/fixtures`); keep each under 50 KB. `ffmpeg` is on this PC. Make the 16-bit and 24-bit FLAC and WAV fixtures from the same samples, since Task 4 compares them.
- [ ] **Step 4: Run** `cargo test -j 4 --workspace --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(library): store bit depth and tell lossless tracks apart`.

### Task 3: Library — sounds

**Files:**
- Create: `crates/aria-library/src/sounds.rs`, `crates/aria-library/tests/sounds.rs`
- Modify: `crates/aria-library/src/migrations.rs` (the `sounds` table joins migration 3), `error.rs`, `lib.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct SoundId(pub i64);
  pub struct Sound { pub id: SoundId, pub path: PathBuf, pub name: String, pub volume: f32, pub hotkey: Option<String> }
  pub const SOUNDS_MAX: usize = 100;
  pub const SOUND_NAME_MAX: usize = 50;
  pub fn sound_name(raw: &str) -> Option<String>;          // trimmed, 1..=50 chars
  impl Library {
      pub fn sounds(&self) -> Result<Vec<Sound>, LibraryError>;                    // in the order added
      pub fn add_sounds(&mut self, paths: &[PathBuf]) -> Result<usize, LibraryError>; // how many were added
      pub fn rename_sound(&mut self, id: SoundId, name: &str) -> Result<bool, LibraryError>;
      pub fn set_sound_volume(&mut self, id: SoundId, volume: f32) -> Result<bool, LibraryError>;
      pub fn set_sound_hotkey(&mut self, id: SoundId, hotkey: Option<&str>) -> Result<bool, LibraryError>;
      pub fn remove_sound(&mut self, id: SoundId) -> Result<bool, LibraryError>;
  }
  ```
  `LibraryError::InvalidSoundName`. A `bool` result is `false` when the id does not exist. `hotkey` is an opaque string here; uniqueness is checked in Task 9.

- [ ] **Step 1: Write the failing tests** in `tests/sounds.rs`: `sounds_keep_the_order_they_were_added_in`; `the_default_name_is_the_file_stem`; `names_are_trimmed_and_measured_in_characters` (empty, whitespace, 50 and 51 characters, Armenian, emoji); `volume_is_clamped_to_zero_and_one` (-1.0 → 0.0, 2.0 → 1.0, NaN → 1.0); `the_same_file_can_be_added_twice`; `adding_stops_at_one_hundred` (98 stored, add 5 → 2 added, 100 stored); `an_unknown_id_changes_nothing`; `removing_a_sound_keeps_the_others_in_order`; `sounds_survive_closing_and_opening`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-library --locked --test sounds` — expected: fails to compile.
- [ ] **Step 3: Implement.** Paths are stored with `paths::encode_path`. Every change is one transaction. `add_sounds` does not check that the files exist.
- [ ] **Step 4: Run** `cargo test -j 4 --workspace --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(library): store soundboard sounds`.

### Task 4: Audio — integer formats and the output state

**Files:**
- Create: `crates/aria-audio/src/sample_format.rs`
- Modify: `crates/aria-audio/src/lib.rs`, `decode.rs` (`StreamInfo` gains `bits_per_sample: Option<u16>` and `float: bool`)
- Test: in-file unit tests; `crates/aria-audio/tests/decode.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum SampleKind { I16, I24In32, I32 }                 // what the device takes; I24In32 is left-justified in 32 bits
  pub fn write_samples(kind: SampleKind, from: &[f32], to: &mut [u8]);   // little-endian, no dither, no allocation
  pub struct SourceFormat { pub sample_rate: u32, pub channels: u16, pub bits: Option<u16>, pub float: bool }
  pub struct DeviceFormat { pub sample_rate: u32, pub channels: u16, pub kind: SampleKind }
  /// The order of spec 3.2: the source's own depth, 24 in 32, 32-bit, 16-bit, at the source's rate;
  /// then the same at the nearest supported rate.
  pub fn choose_format(source: SourceFormat, rates: &[u32], accepts: &mut dyn FnMut(DeviceFormat) -> bool) -> Option<DeviceFormat>;
  /// A whole multiple of `wanted`, else the next higher, else the highest.
  pub fn nearest_rate(wanted: u32, supported: &[u32]) -> Option<u32>;
  pub enum OutputPath { SystemMixer, BitPerfect, Resampled, Converted, MixedWithSound }
  pub struct OutputState { pub path: OutputPath, pub device_rate: u32, pub device_bits: u16 }
  pub struct PathFacts { pub exclusive: bool, pub source: SourceFormat, pub device: DeviceFormat, pub gain_is_unity: bool, pub channels_unchanged: bool, pub sound_playing: bool }
  pub fn output_path(facts: PathFacts) -> OutputPath;
  ```
  Order of precedence in `output_path`: not exclusive → `SystemMixer`; a sound playing → `MixedWithSound`; rates differ → `Resampled`; depth reduced, channels changed, float source, source wider than 24 bits, unknown depth for a lossless codec, or gain not unity → `Converted`; else `BitPerfect`.

- [ ] **Step 1: Write the failing tests.**
  - `write_samples`: `sixteen_bit_values_round_trip_exactly` (every i16 value → `f32` as `v / 32768.0` → I16 equals `v`); `twenty_four_bit_values_round_trip_exactly` (a stride through the i24 range plus both ends, into I24In32 and I32, compared after the shift); `full_scale_and_beyond_saturates` (1.0 and 2.0 → max, -1.0 → min); `nan_is_silence`.
  - `choose_format`: `the_source_depth_is_tried_first`; `twenty_four_bit_falls_back_in_order` (device takes only I32 → I32; only I16 → I16); `an_unsupported_rate_takes_the_nearest` (44 100 wanted, device has 48 000 and 88 200 → 88 200; 96 000 wanted, device has 44 100 and 48 000 → 48 000); `nothing_accepted_is_none`.
  - `output_path`: `the_state_is_bit_perfect_only_when_every_condition_holds` — one row per condition, each flipping exactly one fact from an all-true base.
  - `tests/decode.rs`: `decoded_flac_and_wav_match_their_source_samples` — decode the 16-bit and 24-bit FLAC and WAV fixtures with `TrackDecoder`, convert with `write_samples`, and compare with the WAV fixture's raw PCM bytes, sample for sample. `stream_info_reports_depth_and_float`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-audio --locked` — expected: fails to compile.
- [ ] **Step 3: Implement.** Scale factors are `2^15`, `2^23` and `2^31`; round to nearest; clamp before the cast. For I32, compute in `f64`.
- [ ] **Step 4: Run** `cargo test -j 4 --workspace --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(audio): exact integer conversion and the output state`.

### Task 5: Audio — exclusive mode in the engine

**Files:**
- Modify: `crates/aria-audio/src/output.rs`, `fake.rs`, `cpal_output.rs`, `engine.rs`, `lib.rs`
- Test: `crates/aria-audio/tests/engine.rs`

**Interfaces:**
- Consumes: Task 4.
- Produces:
  ```rust
  pub struct OutputRequest { pub exclusive: bool, pub source: Option<SourceFormat> }   // None: the device's own format
  pub struct Opened { pub output: Box<dyn AudioOutput>, pub exclusive: bool, pub device: DeviceFormat }
  pub trait OutputFactory: Send {
      fn open(&self, device: Option<&DeviceId>, request: OutputRequest) -> Result<Opened, AudioError>;
      fn device_lister(&self) -> DeviceLister;
  }
  // EngineCommand gains:  SetExclusive(bool)
  // EngineEvent gains:    Output(OutputState)  — sent when the state changes
  //                       ExclusiveUnavailable — the shared output was used although exclusive was asked
  ```
  A factory that cannot open exclusively returns a shared output with `exclusive: false`; it never fails only because exclusive was asked. `CpalFactory` always answers shared, with `kind: I32` standing for its float stream and `device` describing the stream. `FakeFactory` gains `set_exclusive_rates(Vec<u32>)`, `set_exclusive_kinds(Vec<SampleKind>)` and `refuse_exclusive(bool)`, and `FakeControl::opened_as() -> (bool, DeviceFormat)`.

- [ ] **Step 1: Write the failing tests** in `tests/engine.rs`, with the fake factory:
  - `exclusive_mode_opens_the_output_at_the_track_rate` (44.1 kHz file, fake default 48 kHz → opened at 44 100, state `BitPerfect`).
  - `a_rate_change_reopens_the_output_between_tracks` (44.1 → 96 → 44.1 kHz: three outputs opened in turn, each track's `TrackStarted` and `TrackEnded` in order, every frame of each track pulled from its own output, the clock position right after each reopen).
  - `tracks_of_one_rate_stay_on_one_output` (two 44.1 kHz files: one output, no silence at the join).
  - `an_unsupported_rate_opens_the_nearest_and_resamples` (state `Resampled`).
  - `a_refused_exclusive_open_falls_back_to_shared` (`ExclusiveUnavailable` once, state `SystemMixer`, the track plays).
  - `each_condition_that_breaks_bit_perfect_is_reported` (24-bit file on a device taking only I16 → `Converted`; a mono file on a stereo device stays `BitPerfect`).
  - `volume_is_unity_in_exclusive_mode` (`SetVolume(0.5)` while exclusive: pulled samples equal the source; after `SetExclusive(false)` they are halved).
  - `switching_the_mode_while_playing_carries_on_from_the_same_place`.
  - `a_seek_in_exclusive_mode_keeps_the_output`.
  - `a_failed_exclusive_output_is_replaced_and_playback_carries_on` (the fake output fails mid-track: `DeviceFellBack`, a new output, the position kept).
  - `shared_mode_behaves_as_before` (the existing engine tests still pass unchanged apart from the factory signature).
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-audio --locked --test engine` — expected: fails to compile.
- [ ] **Step 3: Implement.** In exclusive mode the engine opens the output when it knows the track's `StreamInfo`. At a join where the next track's rate differs from the open output's, it does not queue the next track behind the current one: it lets the current track drain, drops the sink, opens again and starts the next track there. The music input's gain is 1.0 while the output is exclusive (mute still pauses or zeroes as today). `Output` is emitted when the computed `OutputState` differs from the last one sent.
- [ ] **Step 4: Run** `cargo test -j 4 --workspace --locked` — expected: all pass (aria-core compiles with the new factory signature).
- [ ] **Step 5: Commit** `feat(audio): exclusive mode in the engine`.

### Task 6: Audio — the WASAPI exclusive output

**Files:**
- Create: `crates/aria-audio/src/wasapi_output.rs` (`#[cfg(windows)]`), `crates/aria-audio/src/system_output.rs`
- Modify: `crates/aria-audio/src/lib.rs`, `device.rs`; `crates/aria-core/src/runtime.rs` and `lib.rs` (use `SystemFactory` where `CpalFactory` is used)
- Test: `crates/aria-audio/tests/output.rs`

**Interfaces:**
- Consumes: Tasks 4 and 5.
- Produces: `pub struct SystemFactory` implementing `OutputFactory`: a shared request, or any request off Windows, goes to `CpalFactory`; an exclusive request on Windows tries `wasapi_output` and falls back to `CpalFactory` with `exclusive: false`.

- [ ] **Step 1: Write the tests.**
  - Unit, with a fake endpoint list: `a_stored_device_id_finds_its_endpoint` and `an_unknown_id_falls_back_to_the_name` — first print what `list_devices` returns as ids on this PC and compare with the WASAPI endpoint ids; write the mapping the evidence shows, and put the evidence in the report.
  - `tests/output.rs`, `#[ignore = "needs a real output device"]`: `the_default_device_opens_exclusively_and_plays_silence` (open with a 44 100 Hz 16-bit source, write 500 ms of zeros through an input, the output reports `exclusive: true` and has not failed, then closes); `a_second_exclusive_open_on_the_same_device_falls_back` (while the first is open, a second request answers `exclusive: false`).
- [ ] **Step 2: Implement** with the `wasapi` crate: a render thread per output that initializes COM, opens the endpoint in exclusive, event-driven mode with the format `choose_format` picked (asking the device with `IsFormatSupported`), and on each event renders the mixer into a float buffer allocated up front and converts with `write_samples`. The thread reports the opened format, or the error, over a channel before the factory returns, with the same wait as `cpal_output.rs`. A device error sets the `failed` flag and ends the thread. No allocation, lock or log on the render loop after it starts.
- [ ] **Step 3: Run** `cargo test -j 4 --workspace --locked`, then the ignored tests: `cargo test -j 4 -p aria-audio --locked --test output -- --ignored --nocapture`. They write silence only. Report each result, and the formats the device accepted, word for word.
- [ ] **Step 4:** If the device on this PC refuses exclusive mode altogether, report it: the code path is then verified only by the fakes, and the docs must say so.
- [ ] **Step 5: Commit** `feat(audio): WASAPI exclusive output`.

### Task 7: Audio — sounds

**Files:**
- Create: `crates/aria-audio/src/sounds.rs`
- Modify: `crates/aria-audio/src/engine.rs`, `lib.rs`, `fake.rs` (outputs are told apart by device)
- Test: `crates/aria-audio/tests/engine.rs`

**Interfaces:**
- Consumes: Task 5.
- Produces:
  ```rust
  pub enum SoundRoute { Mine, Second, Both }
  pub const SOUNDS_AT_ONCE: usize = 6;
  // EngineCommand gains:
  //   PlaySound { token: u64, path: PathBuf, gain: f32, route: SoundRoute }
  //   StopSounds
  //   SetSoundDevice(Option<DeviceId>)
  // EngineEvent gains:
  //   SoundStarted { token }, SoundEnded { token }, SoundFailed { token }, SoundDeviceFailed
  ```
  `token` is chosen by the caller; a `PlaySound` with the token of a playing sound restarts it. `SoundEnded` is sent when a sound finishes, is stopped, is pushed out as the oldest, or is cut by a reopen.

- [ ] **Step 1: Write the failing tests** in `tests/engine.rs`:
  - `a_sound_plays_on_the_music_output_over_the_music` (pulled samples equal music plus sound times gain).
  - `a_sound_alone_opens_the_output_and_it_closes_when_idle` (no track loaded; with a short idle grace, the output closes after the sound).
  - `a_sound_for_the_second_device_leaves_the_music_stream_alone` (music samples unchanged; the sound's samples on the second fake output).
  - `both_plays_on_both_outputs`; `the_second_device_being_the_music_device_plays_once`.
  - `no_second_device_plays_on_mine` (`Second` and `Both` with none set).
  - `a_second_device_that_cannot_open_reports_and_both_still_plays_on_mine` (`SoundDeviceFailed`; with `Second`, nothing plays and `SoundEnded` follows).
  - `a_seventh_sound_stops_the_oldest`; `firing_a_playing_sound_restarts_it`; `stop_sounds_ends_them_all_and_not_the_music`.
  - `a_sound_that_cannot_be_opened_changes_nothing` (`SoundFailed`, music samples unchanged).
  - `a_sound_of_another_rate_is_resampled_to_the_output`.
  - `a_sound_in_an_exclusive_stream_changes_the_state_and_back` (`MixedWithSound` while it plays, then `BitPerfect`).
  - `a_reopen_for_a_new_rate_cuts_sounds_on_that_output` (`SoundEnded` for each).
  - `the_second_output_closes_after_its_grace`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-audio --locked --test engine` — expected: fails to compile.
- [ ] **Step 3: Implement.** A voice is a decoder, one resampler per target and one mixer input per target; the engine's existing pass feeds voices as it feeds the track. The music output keeps at most 2 inputs for music, so with `MAX_INPUTS == 8` six sounds fit; confirm this in the code and raise `MAX_INPUTS` only if the engine really needs more. The second output is opened shared with `OutputRequest { exclusive: false, source: None }`.
- [ ] **Step 4: Run** `cargo test -j 4 --workspace --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(audio): soundboard voices`.

### Task 8: Core — exclusive setting and the output state

**Files:**
- Modify: `crates/aria-core/src/settings/mod.rs`, `playback_service.rs`, `event.rs`, `runtime.rs`, `notice.rs`, `i18n/mod.rs`, `locales/*/main.ftl`
- Test: in-file tests of `settings/mod.rs` and `playback_service.rs`

**Interfaces:**
- Consumes: Tasks 2, 5.
- Produces:
  - `Settings::exclusive_output: bool` (serde default `false`, lenient like `history_enabled`).
  - `PlayerView` gains `output: Option<OutputState>` and `format: Option<TrackFormat>` where `pub struct TrackFormat { pub codec: Arc<str>, pub sample_rate: u32, pub bits_per_sample: u16, pub quality: Quality }`, taken from the library's `Track`.
  - `PlayerView::volume_locked: bool` (true while `exclusive_output` is on and the platform has the exclusive output).
  - `pub const EXCLUSIVE_SUPPORTED: bool = cfg!(windows)`.
  - Notice key `exclusive-unavailable` (warning; en: "Exclusive mode isn't available on this device right now, so Aria is using the system mixer."), shown at most once per track.

- [ ] **Step 1: Write the failing tests.** Settings: `an_old_settings_file_loads_with_exclusive_off`; `a_bad_exclusive_value_falls_back_to_off`. Playback service, with the fake engine rig: `turning_exclusive_on_tells_the_engine`; `the_setting_is_sent_at_startup`; `the_output_state_reaches_the_player_view`; `the_format_comes_from_the_library_track`; `exclusive_unavailable_gives_one_notice_per_track`; `volume_is_locked_only_while_exclusive_is_on`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-core --locked` — expected: fails.
- [ ] **Step 3: Implement.** With exclusive on, the service still stores the user's volume in settings and does not change it; it only stops sending it as a gain other than 1.0 (the engine enforces unity anyway).
- [ ] **Step 4: Run** `cargo test -j 4 --workspace --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(core): the exclusive output setting and the output state`.

### Task 9: Core — soundboard commands and hotkeys

**Files:**
- Create: `crates/aria-core/src/hotkey.rs`, `crates/aria-core/src/soundboard.rs`
- Modify: `crates/aria-core/src/command.rs`, `event.rs`, `library_service.rs`, `playback_service.rs` or `runtime.rs` (where engine commands are sent), `settings/mod.rs`, `notice.rs`, `lib.rs`, `i18n/mod.rs`, `locales/*/main.ftl`

**Interfaces:**
- Consumes: Tasks 3, 7.
- Produces:
  ```rust
  // hotkey.rs
  pub struct Hotkey { pub ctrl: bool, pub alt: bool, pub shift: bool, pub win: bool, pub key: HotkeyKey }
  pub enum HotkeyKey { Letter(char), Digit(u8), F(u8), Numpad(u8), NumpadAdd, NumpadSubtract, NumpadMultiply, NumpadDivide, NumpadDecimal }
  impl FromStr for Hotkey;      // "Ctrl+Alt+F1", "Ctrl+Shift+A", "Numpad3"; order Ctrl, Alt, Shift, Win
  impl Display for Hotkey;      // the same text; round-trips
  impl Hotkey { pub fn is_allowed(&self) -> bool; }   // Ctrl, Alt or Win held, or an F1..=F24 or numpad key
  // soundboard.rs
  pub struct SoundView { pub id: SoundId, pub name: String, pub volume: f32, pub hotkey: Option<Hotkey>, pub missing: bool, pub playing: bool }
  pub struct SoundboardState { pub sounds: Vec<SoundView> }
  pub enum SoundTarget { Mine, Second, Both }          // serde: "mine", "second", "both"
  ```
  - Commands: `AddSounds(Vec<PathBuf>)`, `RenameSound { id, name }`, `SetSoundVolume { id, volume }`, `SetSoundHotkey { id, hotkey: Option<Hotkey> }`, `RemoveSound(SoundId)`, `PlaySound(SoundId)`, `StopSounds`.
  - Event: `Event::Soundboard(Arc<SoundboardState>)`, sent once after the library opens and after every change, including a sound starting or ending.
  - Settings: `soundboard_device: Option<DeviceId>`, `soundboard_target: SoundTarget` (default `Mine`), `global_hotkeys: bool` (default `true`), `stop_sounds_hotkey: Option<Hotkey>`; all lenient.
  - Notices: `sound-name-invalid`, `sound-limit` (with `{ $max }`), `sound-missing`, `sound-failed`, `sound-device-failed`, `hotkey-not-allowed`, `hotkey-taken` (used inside Aria).
  - `missing` is checked when the state is built and when a sound is fired, on the library thread, never on the UI thread.

- [ ] **Step 1: Write the failing tests.**
  - `hotkey.rs`: `text_round_trips` (a table of 12 hotkeys); `a_plain_typing_key_is_refused` (`A`, `Shift+A`, `5`, `Shift+5` not allowed; `F5`, `Numpad0`, `Ctrl+A`, `Alt+5`, `Win+Shift+S` allowed); `unknown_text_is_an_error`; `f_keys_stop_at_twenty_four`.
  - Service tests in the style of the existing collections tests: each command leads to the expected `Soundboard` event; `an_invalid_name_changes_nothing_and_gives_a_notice`; `adding_past_the_limit_gives_a_notice`; `a_hotkey_used_by_another_sound_is_refused`; `a_hotkey_equal_to_stop_all_is_refused` (and the other way round, when settings change); `a_hotkey_that_is_not_allowed_is_refused`; `playing_a_sound_sends_its_path_gain_and_route` (route from `soundboard_target`); `playing_a_missing_sound_gives_a_notice_and_sends_nothing`; `playing_marks_the_sound_and_ending_unmarks_it`; `stop_sounds_reaches_the_engine`; `the_sound_device_is_sent_at_startup_and_on_change`; `commands_with_the_library_off_give_the_library_notice`; `an_unknown_id_changes_nothing`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-core --locked` — expected: fails to compile.
- [ ] **Step 3: Implement.** The engine token of a sound is its `SoundId`, so firing it again restarts it. A stored hotkey string that no longer parses is dropped with a debug log, not an error.
- [ ] **Step 4: Run** `cargo test -j 4 --workspace --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(core): soundboard commands and hotkey rules`.

### Task 10: App — indicator, exclusive setting, quality badge and filter

**Files:**
- Modify: `crates/aria-app/src/ui/layout/player_bar.rs`, `ui/pages/settings/playback.rs`, `ui/pages/local_music/tracks.rs`, `view.rs`, `mod.rs`, `ui/pages/liked.rs` (only if the table call changes), `locales/*/main.ftl`, `crates/aria-core/src/i18n/mod.rs`
- Test: in-file tests, `crates/aria-app/tests/ui_player.rs`, `ui_settings.rs`, `ui_library.rs`

**Interfaces:**
- Consumes: Tasks 2, 8.
- Produces: `pub fn format_label(format: &TrackFormat) -> String` ("FLAC 24/96", "FLAC 16/44.1", "MP3 44.1"; codec upper-cased, rate in kHz without a trailing ".0"); `TrackView::set_lossless_only(bool)`.

- [ ] **Step 1: Write the failing tests.**
  - Unit: `format_labels` (flac 24/96000 → "FLAC 24/96"; flac 16/44100 → "FLAC 16/44.1"; mp3 0/44100 → "MP3 44.1"; alac 24/192000 → "ALAC 24/192"); `lossless_only_keeps_lossless_and_hi_res_rows`; `lossless_only_and_search_combine`.
  - `ui_player.rs`: `the_indicator_names_each_output_path` (one case per `OutputPath`, text from keys `output-bit-perfect`, `output-resampled`, `output-converted`, `output-mixed`, `output-system-mixer`); `no_track_shows_no_indicator`; `the_volume_slider_is_disabled_and_full_when_locked`.
  - `ui_settings.rs`: `the_exclusive_switch_updates_settings`; `the_exclusive_switch_is_disabled_where_unsupported` (driven by a flag in the page context, so the test runs on every OS).
  - `ui_library.rs`: `the_quality_column_shows_lossless_and_hi_res_badges`; `the_lossless_toggle_filters_the_table`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-app --locked` — expected: fails.
- [ ] **Step 3: Implement.** The indicator sits under the artist in the player bar, as small secondary text, with a tooltip from `output-<state>-hint`; the bit-perfect hint mentions the 5 ms fades. The Quality column is 64 px, not sortable, after Duration. The toggle sits beside the search field on the Tracks tab only and its state lives with the view, like the search text. Other keys: `settings-exclusive`, `settings-exclusive-hint` (other programs are silent on the device while Aria plays; set the level on the device), `settings-exclusive-windows-only`, `quality-lossless`, `quality-hi-res`, `local-lossless-only`.
- [ ] **Step 4: Run** `cargo test -j 4 --workspace --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(app): the output indicator and the lossless badge and filter`.

### Task 11: App — Soundboard page and settings

**Files:**
- Create: `crates/aria-app/src/ui/pages/soundboard.rs`, `crates/aria-app/src/ui/pages/settings/soundboard.rs`, `crates/aria-app/tests/ui_soundboard.rs`
- Modify: `crates/aria-app/src/nav.rs`, `shortcuts.rs`, `ui/layout/sidebar.rs`, `ui/pages/mod.rs`, `ui/pages/settings/mod.rs`, `app.rs`, `folders.rs` (a file dialog for sounds, beside the folder dialog), `locales/*/main.ftl`, `crates/aria-core/src/i18n/mod.rs`

**Interfaces:**
- Consumes: Task 9; `name_dialog`, `confirm_dialog`.
- Produces: `Route::Soundboard`, placed after `LocalMusic` in `Route::ALL` (now 10 routes) with `phase() == None`; Ctrl+1…8 keep their routes, Ctrl+9 stays Settings and Ctrl+0 opens Soundboard. `PageAction::{AddSounds, AskSoundName(SoundId), SetSoundVolume, RemoveSound, PlaySound, StopSounds, AskSoundHotkey(SoundId), AskStopHotkey}`; `PageCtx::soundboard: &SoundboardState`.

- [ ] **Step 1: Write the failing tests** (kittest): `no_sounds_shows_the_empty_state_and_the_add_button`; `tiles_show_name_hotkey_and_playing_state`; `a_click_on_a_tile_plays_it`; `a_missing_sound_is_greyed_and_still_clickable` (the click sends `PlaySound`, so the notice explains); `stop_all_sends_stop_sounds`; `the_tile_menu_offers_rename_hotkey_volume_and_remove`; `remove_asks_first`; `rename_opens_the_name_dialog_with_the_current_name` (limit 50); `the_volume_slider_sends_set_sound_volume_on_release`; settings: `the_second_device_list_includes_none`; `choosing_a_target_updates_settings`; `the_target_note_shows_without_a_second_device`; `the_global_hotkeys_switch_updates_settings`; `ctrl_9_is_settings_and_ctrl_0_is_soundboard`; sidebar order test updated.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-app --locked` — expected: fails.
- [ ] **Step 3: Implement.** The file dialog runs off the UI thread as the folder dialog does, allows several files, and filters by the extensions the scan accepts. Tiles are 140 × 72 px in a wrapping grid inside a scroll area. `AskSoundHotkey` and `AskStopHotkey` are only emitted here; Task 12 handles them (until then the app logs them at debug level). Keys: `nav-soundboard`, `soundboard-empty`, `soundboard-empty-hint`, `soundboard-add`, `soundboard-stop-all`, `sound-rename`, `sound-hotkey`, `sound-hotkey-none`, `sound-volume`, `sound-remove`, `sound-remove-confirm` (with `{ $name }`), `settings-soundboard`, `settings-sound-device`, `settings-sound-device-none`, `settings-sound-target`, `settings-sound-target-mine`, `settings-sound-target-second`, `settings-sound-target-both`, `settings-sound-target-note`, `settings-global-hotkeys`, `settings-global-hotkeys-hint`, `settings-stop-hotkey`, `settings-voice-chat-hint` (a virtual cable is needed and Aria does not install one; the chat hears the sounds, not your microphone).
- [ ] **Step 4: Run** `cargo test -j 4 --workspace --locked` — expected: all pass.
- [ ] **Step 5: Commit** `feat(app): the Soundboard page and its settings`.

### Task 12: App — global hotkeys

**Files:**
- Create: `crates/aria-app/src/hotkeys.rs`, `crates/aria-app/src/ui/widgets/hotkey_dialog.rs`
- Modify: `crates/aria-app/src/app.rs`, `main.rs`, `ui/pages/soundboard.rs` (inactive mark), `locales/*/main.ftl`, `crates/aria-core/src/i18n/mod.rs`
- Remove: `crates/aria-app/examples/hotkey_probe.rs` once the real manager is in

**Interfaces:**
- Consumes: Tasks 1, 9, 11.
- Produces:
  ```rust
  pub(crate) trait HotkeyBackend { fn register(&mut self, hotkey: Hotkey) -> Result<(), String>; fn unregister(&mut self, hotkey: Hotkey); fn pressed(&mut self) -> Vec<Hotkey>; }
  pub(crate) enum HotkeyAction { Play(SoundId), StopAll }
  pub(crate) struct Hotkeys<B: HotkeyBackend> { /* wanted and registered sets */ }
  impl<B: HotkeyBackend> Hotkeys<B> {
      /// Make the registered set match `wanted`; returns the hotkeys the system refused.
      pub(crate) fn sync(&mut self, enabled: bool, wanted: &[(Hotkey, HotkeyAction)]) -> Vec<Hotkey>;
      pub(crate) fn poll(&mut self) -> Vec<HotkeyAction>;
      /// Whether the system accepts `hotkey` right now; leaves nothing registered that was not before.
      pub(crate) fn try_register(&mut self, hotkey: Hotkey) -> bool;
  }
  ```
  The real backend wraps `GlobalHotKeyManager`, created on the main thread before the window, and reacts to key-down only. `fn to_system(Hotkey) -> global_hotkey::hotkey::HotKey` is a pure, tested mapping.

- [ ] **Step 1: Write the failing tests** with a fake backend: `sync_registers_new_and_unregisters_removed`; `turning_the_switch_off_unregisters_everything_and_keeps_the_list`; `a_refused_hotkey_is_reported_and_retried_on_the_next_sync`; `a_press_becomes_its_action`; `a_press_of_a_hotkey_no_longer_wanted_is_dropped`; `every_hotkey_key_maps_to_a_system_key` (all `HotkeyKey` values); dialog (kittest): `pressing_a_combination_shows_it_and_confirm_sends_it`; `escape_cancels`; `delete_clears_the_hotkey`; `a_combination_that_is_not_allowed_cannot_be_confirmed`; app-level: `a_hotkey_the_system_refuses_is_not_saved` (the fake refuses → a `hotkey-unavailable` notice and no `SetSoundHotkey` command); `inactive_hotkeys_are_marked_on_their_tiles_and_counted_in_one_notice`; `hotkeys_do_not_fire_while_the_hotkey_dialog_is_open`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-app --locked` — expected: fails.
- [ ] **Step 3: Implement.** The app calls `sync` when a `Soundboard` event or the settings change, and `poll` every frame; while any hotkey is registered and the window is not focused, it asks for a repaint every 50 ms so presses are seen (measure idle CPU with hotkeys registered and report it). While the capture dialog is open all hotkeys are unregistered, so the combination being typed reaches the dialog. Keys: `hotkey-dialog-title`, `hotkey-dialog-hint`, `hotkey-dialog-clear`, `hotkey-unavailable`, `hotkeys-inactive` (with `{ $count }`, plural forms for ru), `hotkey-inactive-hint`.
- [ ] **Step 4:** Run the whole workspace tests. Then start the app with `ARIA_CONFIG_DIR` pointing at a temporary folder — and only if a way to keep the data folder away from `%APPDATA%\Aria` exists; if it does not, do not start the app, and report that the by-hand hotkey check is left to the user.
- [ ] **Step 5: Commit** `feat(app): global hotkeys for the soundboard`.

### Task 13: Version, docs and the manual check list

**Files:**
- Modify: `Cargo.toml` (workspace version `0.2.2`), `Cargo.lock`, `README.md`, `CHANGELOG.md`, `docs/PRIVACY.md`, `docs/ROADMAP.md` (2c is lossless output and the soundboard), `THIRD_PARTY_NOTICES.md` (the two crates), `packaging/windows/aria.iss` lines that name `0.2.1`
- Create: `docs/testing/phase2c-manual-check.md`

- [ ] **Step 1:** Set the version to 0.2.2 and refresh `Cargo.lock` (`cargo metadata` without `--locked`, then everything with `--locked`).
- [ ] **Step 2:** `CHANGELOG.md`: a `## [0.2.2] - unreleased (pre-release: Phase 2c, lossless output and the soundboard)` entry with what was added and every limitation of spec section 11. `README.md`: status for 0.2.2, the exclusive mode and what "bit-perfect" does and does not mean here, the soundboard and voice chat (spec 4.5, word for word in substance), upgrading from 0.2.1 (the database is upgraded and every file is read once more; 0.2.1 then runs with the library off). `docs/PRIVACY.md`: sounds, hotkeys (only registered combinations reach Aria), no microphone.
- [ ] **Step 3:** `docs/testing/phase2c-manual-check.md` in the format of `phase2b-manual-check.md`: what automated tests cover; what the agent ran on real hardware in Tasks 1 and 6, with the results from those reports; and a table for a person, every line `pending`: exclusive switch on, a 44.1 kHz and a 96 kHz track play; other programs are silent meanwhile and sound again 15 s after pause; the DAC or the Windows sound panel shows the track's rate; the indicator in each state; the gap at a rate change; volume locked; a busy device falls back with a notice; unplugging the device; the Quality column and filter after the upgrade rescan; add, rename, remove a sound; six sounds at once and a seventh; each target with a real second device; a virtual cable heard in a voice chat; a hotkey with a game in the foreground and with Aria minimized; a hotkey another program holds; the switch off; RU and HY texts.
- [ ] **Step 4:** Run fmt, clippy and the workspace tests. Run the ignored timing tests in release and add any new numbers to the README with the machine named.
- [ ] **Step 5: Commit** `docs: 0.2.2, Phase 2c`.
