# Aria 0.2.3 (Music to Chat) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Send a copy of the playing music and the user's microphone into the second output device (a virtual cable used as a voice-chat microphone), with their own volumes, a microphone toggle hotkey and a player-bar indicator, released as 0.2.3.

**Architecture:** The engine already owns a second, shared output for soundboard sounds (`sounds::SecondOutput`). Two more inputs join that output's mixer:
- **Music copy.** The engine thread writes, into a music-copy input, the same frames it gives the listener's output, resampled to the second output's format.
- **Microphone.** A CPAL input stream writes captured frames into a lock-free ring. The engine thread drains the ring, converts the format and writes into a microphone input.

Core adds settings, a toggle command and the microphone state. The app adds settings rows, an icon and the hotkey.

**Tech Stack:** Rust 1.99.1, CPAL 0.18 (input streams), rtrb, rubato, Tokio, egui/eframe 0.37 with `egui_kittest`, `global-hotkey` 0.10, Fluent (`locales/{en,ru,hy}/main.ftl`).

**Spec:** `docs/superpowers/specs/2026-11-17-aria-music-to-chat-design.md`

## Global Constraints

- **Code.** No `unwrap`/`expect` in production code, no `unsafe`, no suppressed warnings (`#[allow]`), no fake or stubbed features. Tests may panic with a message.
- **Microphone.** Audio from it goes only into the second output's mixer: never to disk, a log, the network, or the listener's own output. No input device is open unless "Send my microphone" is on and a second device is chosen.
- **No microphone in tests or checks.** No agent opens a real microphone in a test or check. Real-capture tests are `#[ignore]` and left for a person.
- **Threads.** Never block the UI thread or an audio callback. No allocation, lock or log on a real-time thread, the capture callback included. Every change from the UI is a `Command`.
- **Logs.** Paths in logs go through `aria_core::redact`; device names may be logged.
- **Text.** Every user-facing string exists in `locales/en/main.ftl`, `locales/ru/main.ftl` and `locales/hy/main.ftl`, and in `KEYS` in `crates/aria-core/src/i18n/mod.rs`.
- **Honesty.** Do not claim something works unless it was built and tested. Reports and docs say what was not verified.
- **Exact values:**
  - music-in-chat volume 0.0 to 1.0, default **0.5**;
  - microphone volume 0.0 to **2.0**, default **1.0**, clipped at full scale after gain;
  - the second output closes **15 s** after nothing needs it (`IDLE_GRACE`), and an open microphone always needs it;
  - sounds at once stay **6**;
  - microphone latency target is **under 60 ms**, measured by hand;
  - the microphone input's ring holds at most **40 ms**; older captured audio is dropped rather than letting latency grow.
- **Versions.** The version is **0.2.3**. No database change. The settings schema version does not change.
- **Checks.** Each task ends with `cargo fmt --all --check`, `cargo clippy -j 4 --workspace --all-targets --locked -- -D warnings` and `cargo test -j 4 --workspace --locked` passing. Run cargo commands one at a time in the foreground.
- **Shell environment** (Git Bash, every shell):
  ```bash
  cd /e/claude/aria
  export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0=E:/claude/aria
  export CARGO_TARGET_DIR=C:/Users/hixenpc/aria-target
  export CMAKE="C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe" CMAKE_GENERATOR="Visual Studio 17 2022"
  ```
- **Never start `aria.exe`** against `%APPDATA%\Aria`: that folder holds the user's data.
- **Git.** Work on branch `feat/music-to-chat`. Commits end with the attribution line the dispatch gives. Do not push, merge, tag or publish.

## Review Focus

1. **The microphone where it must not be.** It must never reach the listener's own output, never be open with the switch off or with no second device, and must close at once when the switch turns off. Tests:
   - Task 3: `the_microphone_never_reaches_the_local_output`, `turning_the_microphone_off_closes_the_capture`, `no_second_device_opens_no_microphone`.
   - Task 4: `the_microphone_stays_closed_without_a_second_device`.
2. **The copy that disturbs the music.** The listener's output must be bit-for-bit unchanged by sharing, a full copy ring must never stall the local music, and exclusive mode must stay bit-perfect with sharing on. Tests (Task 2): `sharing_leaves_the_local_output_unchanged`, `a_full_copy_ring_never_stalls_the_music`, `exclusive_stays_bit_perfect_while_sharing`.
3. **Latency that grows.** A microphone input whose ring fills must drop the oldest audio, not queue it. Test (Task 3): `the_microphone_never_queues_more_than_40_ms`.
4. **The second device is the local device.** No copy and no microphone in that case, and no echo. Test (Task 2): `no_copy_when_the_second_device_is_the_local_device`; test (Task 3): `no_microphone_when_the_second_device_is_the_local_device`.
5. **Hotkey collisions.** The microphone hotkey against sound hotkeys, "Stop all" and Aria's own shortcuts. Test (Task 4): `the_microphone_hotkey_is_unique`.

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/aria-audio/src/capture.rs` (new) | `CaptureFactory`/`Capture` traits, the CPAL input implementation, the capture ring |
| `crates/aria-audio/src/fake.rs` | A fake capture source that tests feed |
| `crates/aria-audio/src/engine.rs`, `sounds.rs`, `mixer.rs`, `lib.rs` | The music copy and the microphone on the second output; commands and events |
| `crates/aria-audio/tests/engine.rs` | Engine tests with fake outputs and fake capture |
| `crates/aria-core/src/settings/mod.rs`, `command.rs`, `event.rs`, `playback_service.rs`, `runtime.rs`, `soundboard.rs`, `notice.rs`, `i18n/mod.rs` | Settings, `ToggleMicrophone`, the input device list, `MicState`, hotkey uniqueness, notices |
| `crates/aria-app/src/ui/pages/settings/soundboard.rs`, `ui/layout/player_bar.rs`, `hotkeys.rs`, `app.rs`, `main.rs` | The settings rows, the microphone icon, the hotkey |
| `locales/*/main.ftl` | New text |
| `Cargo.toml`, `Cargo.lock`, `README.md`, `CHANGELOG.md`, `docs/PRIVACY.md`, `docs/ROADMAP.md`, `docs/testing/music-to-chat-manual-check.md` (new), `packaging/windows/aria.iss` | Version and docs |

---

### Task 1: Audio — the capture source

**Files:**
- Create: `crates/aria-audio/src/capture.rs`
- Modify: `crates/aria-audio/src/fake.rs`, `lib.rs`, `device.rs` (if the input list needs a helper)
- Test: in-file unit tests; `crates/aria-audio/tests/capture.rs` (new)

**Interfaces:**
- Produces:
  ```rust
  pub struct CaptureFormat { pub sample_rate: u32, pub channels: u16 }
  /// Interleaved f32 frames written by the capture callback, read by the engine thread.
  pub struct CaptureReader { /* rtrb::Consumer<f32> */ }
  impl CaptureReader {
      pub fn format(&self) -> CaptureFormat;
      /// Moves up to `out.len()` samples out; returns how many. Never blocks.
      pub fn read(&mut self, out: &mut [f32]) -> usize;
      /// Drops everything waiting but the newest `keep` frames.
      pub fn skip_to_newest(&mut self, keep: usize);
      /// Whether the stream has ended (device lost), so it must be reopened.
      pub fn failed(&self) -> bool;
  }
  pub trait CaptureFactory: Send {
      /// Open `device` (None: the system default) in shared mode. Dropping the reader closes the stream.
      fn open(&self, device: Option<&DeviceId>) -> Result<CaptureReader, AudioError>;
  }
  pub struct CpalCapture;   // implements CaptureFactory with cpal input streams
  // fake.rs: FakeCapture { pub fn new(format) -> (Self, FakeCaptureControl) }
  //          FakeCaptureControl { push(&[f32]), fail(), fail_open(bool), opened() -> usize, open_now() -> bool }
  ```
  `AudioError::PermissionDenied` is added for an input device the operating system refuses (mapped from the CPAL error kind if it has one; otherwise from the Windows HRESULT for access denied).

- [ ] **Step 1: Write the failing tests.**
  - Unit: `the_ring_holds_a_second_and_drops_on_overflow` — the callback side never blocks, and overflow drops the newest frames that do not fit and counts them.
  - Unit: `skip_to_newest_keeps_only_the_newest_frames`.
  - Unit: `a_closed_stream_reports_failed`.
  - `tests/capture.rs`:
    - `the_fake_capture_delivers_what_is_pushed`;
    - `a_refused_open_is_an_error`;
    - `#[ignore = "opens a real microphone; run by hand only"] the_default_microphone_opens` — opens, reads for 200 ms, checks only that `format()` is sane and that it did not fail, discards the samples, and prints nothing captured.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-audio --locked` — expected: fails to compile.
- [ ] **Step 3: Implement.**
  - The CPAL callback converts any input sample type to f32 and writes into the ring. Size the ring for 1 s at the device's rate.
  - The callback allocates nothing, takes no lock and logs nothing. Errors set a flag.
  - The stream lives on its own thread, as `cpal_output.rs` does, with the same open timeout.
- [ ] **Step 4: Run** the workspace checks. Do not run the ignored test.
- [ ] **Step 5: Commit** `feat(audio): a microphone capture source`.

### Task 2: Audio — the music copy on the second output

**Files:**
- Modify: `crates/aria-audio/src/engine.rs`, `sounds.rs`, `mixer.rs` (only if the input limit must rise), `lib.rs`
- Test: `crates/aria-audio/tests/engine.rs`

**Interfaces:**
- Consumes: `SecondOutput`, `Spares`, `Resampler`; the second device from `SetSoundDevice`.
- Produces: `EngineCommand::SetShareMusic { on: bool, gain: f32 }`.
  - While it is on and a second device is set and is not the listener's device, the engine writes a copy of every frame it writes to the listener's input into a music-copy input on the second output.
  - The copy is converted to the second output's format and plays at `gain`.
  - The copy follows pause, seek, stop and track changes, because it is fed from the same frames.
  - The local volume and mute do not apply to it.

- [ ] **Step 1: Write the failing tests** (fake outputs):
  - `sharing_sends_the_music_to_the_second_output_at_its_gain`;
  - `sharing_leaves_the_local_output_unchanged` (bit-identical to sharing off);
  - `no_copy_while_sharing_is_off`;
  - `no_copy_without_a_second_device`;
  - `no_copy_when_the_second_device_is_the_local_device`;
  - `local_mute_and_volume_do_not_touch_the_copy`;
  - `pause_and_seek_act_on_the_copy`;
  - `the_copy_is_resampled_to_the_second_output`;
  - `a_full_copy_ring_never_stalls_the_music`: the second fake output is never pulled, yet the local output keeps getting audio, and the copy drops what does not fit;
  - `a_reopen_of_the_local_output_pauses_the_copy_and_it_carries_on`;
  - `exclusive_stays_bit_perfect_while_sharing`: the local state is still `BitPerfect`, and the copy goes to the shared second output;
  - `sounds_and_the_copy_share_the_second_output`;
  - `the_second_output_stays_open_while_music_is_shared_and_playing`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-audio --locked --test engine` — expected: fails.
- [ ] **Step 3: Implement.**
  - The copy has its own resampler from the listener's output format to the second output's format.
  - The engine thread writes into the copy only what fits and drops the rest, counting the drops in a debug log no more than once a second. The local output keeps priority.
  - Make sure the second output can hold the music copy, the microphone (Task 3) and 6 sounds with their reuse spares. Raise `MAX_INPUTS` if needed (each mixer input costs nothing when empty), and update the compile-time assert.
- [ ] **Step 4: Run** the workspace checks.
- [ ] **Step 5: Commit** `feat(audio): share the music to the second output`.

### Task 3: Audio — the microphone on the second output

**Files:**
- Modify: `crates/aria-audio/src/engine.rs`, `sounds.rs`, `lib.rs`; `crates/aria-core/src/runtime.rs` (pass `CpalCapture` to the engine); `crates/aria-app/src/main.rs` (if the engine is built there)
- Test: `crates/aria-audio/tests/engine.rs`

**Interfaces:**
- Consumes: Task 1, Task 2.
- Produces:
  - `Engine::start` takes a `Box<dyn CaptureFactory>`. Every existing caller passes `CpalCapture`, or the fake in tests.
  - `EngineCommand::SetMicrophone { on: bool, device: Option<DeviceId>, gain: f32 }`.
  - `EngineEvent::{MicrophoneOpened, MicrophoneClosed, MicrophoneFailed { denied: bool }}`.
- **Rules:**
  - The capture is open exactly while `on` is set and a second device is set and is not the listener's device.
  - The captured audio goes only into a microphone input on the second output, at `gain`, clipped at full scale.
  - The microphone input's ring holds at most 40 ms. When it is full, the oldest captured audio is dropped with `skip_to_newest`.
  - While the microphone is open, the engine ticks every 10 ms even with nothing playing.

- [ ] **Step 1: Write the failing tests** (fake outputs and fake capture):
  - `the_microphone_goes_to_the_second_output_at_its_gain`;
  - `the_microphone_never_reaches_the_local_output`;
  - `a_mono_microphone_reaches_both_channels`;
  - `gain_above_one_amplifies_and_clips`;
  - `turning_the_microphone_off_closes_the_capture`;
  - `no_second_device_opens_no_microphone`;
  - `no_microphone_when_the_second_device_is_the_local_device`;
  - `the_microphone_never_queues_more_than_40_ms`: push 1 s at once, and no more than 40 ms is ever waiting in the input;
  - `a_failed_capture_reports_and_music_and_sounds_go_on`;
  - `a_denied_capture_reports_denied`;
  - `a_lost_capture_reports_failed_and_retries_on_the_next_change`;
  - `the_second_output_stays_open_while_the_microphone_is_on`;
  - `the_microphone_is_resampled_to_the_second_output`;
  - `a_second_device_change_moves_the_microphone`;
  - `the_input_limit_holds_copy_microphone_and_six_sounds`.
- [ ] **Step 2: Run** — expected: fails.
- [ ] **Step 3: Implement.**
  - Reuse the conversion code the sounds use (`convert_channels`, `Resampler`).
  - The engine never logs captured samples or their levels.
  - The idle wait in `wait_for_command` must not sleep past 10 ms while a capture is open.
- [ ] **Step 4: Run** the workspace checks. No real microphone is opened.
- [ ] **Step 5: Commit** `feat(audio): the microphone to the second output`.

### Task 4: Core — settings, commands and the microphone state

**Files:**
- Modify: `crates/aria-core/src/settings/mod.rs`, `command.rs`, `event.rs`, `playback_service.rs`, `runtime.rs`, `soundboard.rs`, `notice.rs`, `i18n/mod.rs`, `lib.rs`, `locales/*/main.ftl`
- Test: in-file tests

**Interfaces:**
- Consumes: Tasks 2 and 3; `Hotkey` and its rules (2c).
- Produces:
  - **Settings**, each lenient with a serde default:
    - `share_music: bool` (false)
    - `share_music_volume: f32` (0.5, clamped 0..=1)
    - `mic_enabled: bool` (false)
    - `mic_device: Option<DeviceId>` (None)
    - `mic_volume: f32` (1.0, clamped 0..=2)
    - `mic_toggle_hotkey: Option<Hotkey>` (None; one that is not allowed is treated as none)
  - **`Command::ToggleMicrophone`** flips `mic_enabled`, saves it, and shows a notice: `microphone-on` or `microphone-off`.
  - **`Command::ListInputDevices`** → `Event::InputDevices(Vec<DeviceInfo>)`.
  - **`PlayerView::microphone: MicState { Off, On, Failed }`**: On after `MicrophoneOpened`, Failed after `MicrophoneFailed`, and Off otherwise.
  - **Notices:**
    - `microphone-failed` ("The microphone could not be opened");
    - `microphone-denied` (allow microphone access for desktop apps in the system's privacy settings);
    - `microphone-on`, `microphone-off`.
  - **Engine commands sent at startup and on every relevant change:**
    - `SetShareMusic { on: share_music && second device set, gain }`;
    - `SetMicrophone { on: mic_enabled && second device set, device, gain }`.
  - **Uniqueness.** The microphone hotkey is checked against the sound hotkeys, "Stop all" and Aria's own shortcuts, and they are checked against it.

- [ ] **Step 1: Write the failing tests:**
  - `old_settings_load_with_sharing_and_microphone_off`;
  - `volumes_are_clamped` (including NaN → default);
  - `the_microphone_stays_closed_without_a_second_device` (the engine is told `on: false`);
  - `toggle_microphone_flips_saves_and_notices`;
  - `the_microphone_state_follows_the_engine`;
  - `a_denied_microphone_gives_the_denied_notice`;
  - `the_microphone_hotkey_is_unique` (both ways, against all three kinds);
  - `share_music_is_sent_at_startup_and_on_change`;
  - `input_devices_are_listed_on_request`;
  - `with_the_library_off_toggle_microphone_still_works` (it does not need the library).
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-core --locked` — expected: fails.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the workspace checks.
- [ ] **Step 5: Commit** `feat(core): settings and state for music and microphone in the chat`.

### Task 5: App — settings rows, the microphone icon and the hotkey

**Files:**
- Modify: `crates/aria-app/src/ui/pages/settings/soundboard.rs`, `ui/layout/player_bar.rs`, `hotkeys.rs`, `app.rs`, `ui/pages/mod.rs`, `locales/*/main.ftl`, `crates/aria-core/src/i18n/mod.rs`
- Test: in-file tests, `crates/aria-app/tests/ui_settings.rs`, `ui_player.rs`, `ui_soundboard.rs`

**Interfaces:**
- Consumes: Task 4; the hotkey dialog and `Hotkeys` from 2c.
- **Settings, "Sound panel"** gains these rows, after "Play sounds to":
  - "Music also to the second device" (switch);
  - "Music volume in the chat" (slider, sent on release);
  - "Microphone" (input list with "System default");
  - "Send my microphone to the second device" (switch);
  - "Microphone volume" (slider 0–200 %, sent on release);
  - "Hotkey to turn the microphone on/off" (button opening the hotkey dialog).
  - Without a second device, the two switches and their sliders are disabled, with a line saying so.
  - With the second device equal to the music device, a warning line shows instead.
- **Player bar:** while `microphone == On`, a microphone icon beside the volume. Its tooltip says the voice goes to the second device, and a click sends `ToggleMicrophone`. While `Failed`, a crossed-out icon with the failure tooltip. While `Off`, nothing.
- **Hotkeys:** `HotkeyAction::ToggleMicrophone` registers with the other hotkeys and sends `ToggleMicrophone`.

- [ ] **Step 1: Write the failing tests** (kittest and unit):
  - `the_new_rows_are_disabled_without_a_second_device`;
  - `the_same_device_warning_shows`;
  - `the_switches_send_update_settings`;
  - `the_volume_sliders_send_on_release_only`;
  - `the_microphone_list_requests_input_devices`;
  - `the_hotkey_row_opens_the_dialog`;
  - `the_icon_shows_only_while_on_and_a_click_toggles`;
  - `the_failed_icon_shows_the_failure`;
  - `the_microphone_hotkey_sends_toggle`;
  - `the_dialog_refuses_a_hotkey_taken_by_a_sound`.
- [ ] **Step 2: Run** `cargo test -j 4 -p aria-app --locked` — expected: fails.
- [ ] **Step 3: Implement.** Text keys:
  - `settings-share-music`, `settings-share-music-volume`, `settings-share-music-hint`;
  - `settings-mic-device`, `settings-mic-default`, `settings-mic-send`, `settings-mic-send-hint`, `settings-mic-volume`, `settings-mic-hotkey`;
  - `settings-chat-needs-device`, `settings-chat-same-device`;
  - `player-mic-on`, `player-mic-failed`.

  Write the Russian and Armenian text yourself.
- [ ] **Step 4: Run** the workspace checks. Do not start the app and do not send keys.
- [ ] **Step 5: Commit** `feat(app): music and microphone to the chat in settings, the bar and hotkeys`.

### Task 6: Version, docs and the manual check list

**Files:**
- Modify: `Cargo.toml` (workspace version `0.2.3`), `Cargo.lock`, `README.md`, `CHANGELOG.md`, `docs/PRIVACY.md`, `docs/ROADMAP.md`, the `packaging/windows/aria.iss` lines that name `0.2.2`
- Create: `docs/testing/music-to-chat-manual-check.md`

- [ ] **Step 1:** Set the version to 0.2.3 and refresh `Cargo.lock`.
- [ ] **Step 2: Write the docs.**
  - `CHANGELOG.md`: a `## [0.2.3] - unreleased (music and microphone to the chat)` entry with what was added and every limitation of spec section 9.
  - `README.md`:
    - the status for 0.2.3;
    - how to set it up: install a virtual cable, choose it as the second device, turn on sharing and the microphone, and choose the cable as the chat's microphone;
    - what was not verified.
  - `docs/PRIVACY.md`, per spec section 7:
    - replace the statement that Aria never reads the microphone;
    - say when the microphone is open, where its audio goes, and that nothing is stored or logged.
- [ ] **Step 3: Write `docs/testing/music-to-chat-manual-check.md`** in the format of `phase2c-manual-check.md`. List what the automated tests cover, and a table for a person with every line `pending`:
  - with a virtual cable as the second device and the cable chosen in a chat program, others hear the music and the voice together;
  - the chat music volume;
  - local mute does not silence the chat;
  - pause and seek;
  - the microphone hotkey with a game in the foreground and with Aria minimized;
  - microphone latency (target under 60 ms);
  - the Windows microphone-in-use mark shows only while the switch is on;
  - the player-bar icon;
  - unplugging the microphone;
  - a denied microphone permission;
  - choosing the local device as the second device;
  - exclusive mode with sharing on stays bit-perfect locally;
  - the RU and HY texts.
- [ ] **Step 4:** Run fmt, clippy and the workspace tests.
- [ ] **Step 5: Commit** `docs: 0.2.3, music and microphone to the chat`.
