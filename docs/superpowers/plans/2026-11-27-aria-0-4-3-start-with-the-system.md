# Aria 0.4.3: start with the system, implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a "Start with the system" switch that registers Aria for login with `--autostart`. A login start waits hidden in the tray and is never stranded.

**Architecture:**
- `aria-core` knows the argument (`opened.rs`), the setting, and a quiet instance claim.
- `crates/aria-app/src/autostart.rs` builds each system's entry and applies it: a file on
  macOS and Linux, a registry value on Windows.
- `AriaApp` applies the setting through a closure in `AppDeps` (a fake in tests). It starts
  hidden when told to, and reveals the window minimized when it can't hide.

**Tech Stack:** Rust, `winreg` 0.56 (Windows only), egui/eframe 0.37 viewport commands.

**Spec:** docs/superpowers/specs/2026-11-27-aria-0-4-3-start-with-the-system-design.md

## Global Constraints

- No `unwrap` or `expect` outside tests, no `unsafe`, no `#[allow]`.
- Tests never touch the real Run key, LaunchAgents or autostart folder. The registry is tested
  only under `HKCU\Software\AriaTest-<unique>`, deleted afterwards.
- `start_with_system` defaults to false and reads leniently.
- An entry is removed only when it is Aria's: its name is right and it contains `--autostart`.
- A hidden start never stays hidden without `can_hide()`.

## Review Focus

- An autostart where the tray factory fails: the window must appear (minimized). Test:
  `a_hidden_start_without_a_tray_shows_minimized`.
- An autostart on Wayland (`cannot_hide`): the same. Test:
  `a_hidden_start_where_hiding_fails_shows_minimized`.
- A foreign value named `Aria`, or a foreign autostart file, without `--autostart`: it is left
  alone when the switch is off. Tests: `a_foreign_file_is_left_alone` and
  `a_foreign_value_is_left_alone`.
- A path with spaces, quotes or `$` in the Linux `Exec` line or the macOS plist is escaped.
  Tests: `the_desktop_entry_quotes_the_path` and `the_launch_agent_escapes_xml`.
- A write that fails is not retried every frame. Test: `a_failed_change_is_tried_once_and_told`.

---

### Task 1: core (argument, setting, quiet claim)

- `aria_core::opened::AUTOSTART_ARG = "--autostart"`.
- `pub fn is_autostart(args) -> bool`.
- `launch_paths` skips the argument.
- `Settings.start_with_system` (lenient, default false).
- `instance::claim_quietly(data_dir) -> io::Result<Claim>`: on `Running` it returns
  `reached: false` and never reaches the running copy.
- Tests:
  - `the_autostart_argument_is_not_a_file`
  - `start_with_system_defaults_off`
  - `a_quiet_claim_does_not_wake_the_running_copy`

### Task 2: `autostart.rs`

- `pub fn run_value(exe) -> String` (`"<exe>" --autostart`).
- `pub fn desktop_entry(exe) -> String`.
- `pub fn launch_agent(exe) -> String`.
- `pub enum Entry { File { path, content }, Registry { subkey, name, data } }` with
  `pub fn apply(&self, on: bool) -> io::Result<()>`:
  - On: write it if different.
  - Off: remove it only if it is ours.
- `pub fn system_entry(exe) -> Option<Entry>`.
- `pub type Autostart = Box<dyn FnMut(bool) -> Result<(), String>>`.
- `pub fn system() -> Option<Autostart>`, built from `std::env::current_exe`.
- Tests:
  - `the_desktop_entry_quotes_the_path`
  - `the_launch_agent_escapes_xml`
  - `the_run_value_quotes_the_path`
  - `a_file_entry_is_written_rewritten_and_removed`
  - `a_foreign_file_is_left_alone`
  - On Windows: `a_registry_entry_is_written_and_removed` and `a_foreign_value_is_left_alone`
    (throwaway key).

### Task 3: the app

- `AppDeps.autostart: Option<Autostart>` and `AppDeps.start_hidden: bool`.
- Fields `autostart_applied: Option<bool>`; `hidden` starts as `start_hidden`.
- `run_autostart(ctx)` in `logic_frame` applies the setting when it differs from what was last
  applied. It records the outcome even when it fails, and tells the user with the notice
  `notice-autostart-failed`.
- `run_tray`: when `hidden && !can_hide()` (or there is no tray at all), it sends `Visible(true)`
  and `Minimized(true)` and clears `hidden`.
- Settings row "Start with the system" with its hint, in en, ru and hy.
- `main.rs`:
  - `is_autostart`.
  - `native_options(hidden)` adds `with_visible(false)`.
  - The claim is quiet when autostarted.
  - The deps carry `autostart: aria_app::autostart::system()` and `start_hidden`.
- Tests:
  - `the_setting_is_applied_once_at_start_and_on_change`
  - `a_failed_change_is_tried_once_and_told`
  - `a_hidden_start_waits_in_the_tray`
  - `a_hidden_start_without_a_tray_shows_minimized`
  - `a_hidden_start_where_hiding_fails_shows_minimized`
  - `the_start_with_system_switch_updates_settings`

### Task 4: installer, docs, version

- aria.iss gets `[Registry] Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Aria"; Flags: uninsdeletevalue dontcreatekey`. ISCC compiles it.
- CHANGELOG [0.4.3], README, ROADMAP, PRIVACY ("Start with the system"),
  docs/testing/0.4.3-manual-check.md.
- Version 0.4.3, then `cargo update --workspace --offline`.
- The whole suite, clippy and fmt pass. Final review by a fresh reviewer, fixes, push, and CI
  passes.
