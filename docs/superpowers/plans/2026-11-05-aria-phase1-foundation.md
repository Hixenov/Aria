# Aria Phase 1 (Foundation) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A launchable native Aria window with the final layout shell, nine-route navigation, dark/light/system theming with custom accents, persistent settings, EN/RU/HY localization, file logging and green cross-platform CI.

**Architecture:** Cargo workspace with two crates. `aria-core` holds settings, i18n, paths, logging, `Command`/`Event`, and a `Backend` thread owning a Tokio runtime; it has no GUI dependencies. `aria-app` is the egui/eframe (glow) binary. The UI drains backend events once per frame without blocking, and the backend wakes the UI via `request_repaint`.

**Tech Stack:** Rust 1.99.1 (edition 2024), eframe/egui 0.37 (glow), egui_kittest 0.37, tokio 1.54, crossbeam-channel 0.5.17, serde 1 + toml 1.2, directories 6, fluent-bundle 0.16 + unic-langid 0.9.6, tracing 0.1.44 + tracing-subscriber 0.3.23 + tracing-appender 0.2.5, thiserror 2, anyhow 1, sys-locale, open, tempfile 3.27 (dev), ttf-parser (dev).

**Spec:** `docs/superpowers/specs/2026-11-05-aria-phase1-foundation-design.md`. Read it before starting any task; section numbers below (§N) refer to it.

## Global Constraints

- Toolchain pinned by `rust-toolchain.toml`: `channel = "1.99.1"`, `components = ["clippy", "rustfmt"]`. Edition `2024` for both crates.
- Every crate has `[lints] workspace = true`. Workspace lints: `rust.unsafe_code = "forbid"`, `clippy.all = { level = "warn", priority = -1 }`, `clippy.unwrap_used = "deny"`, `clippy.expect_used = "deny"`. `clippy.toml` sets `allow-unwrap-in-tests = true` and `allow-expect-in-tests = true`.
- Never add `#[allow(...)]` to silence a warning without a comment giving the technical reason.
- Every cargo command uses `--locked` once `Cargo.lock` exists. CI uses `-D warnings`.
- eframe dependency: `default-features = false`, `features = ["glow", "persistence", "accesskit", "clipboard", "links", "default_fonts", "wayland", "x11"]`.
- License `MIT OR Apache-2.0` in every `Cargo.toml`.
- No color literals outside `crates/aria-app/src/ui/theme/palette.rs`.
- No user-facing string literals in UI code. Every visible string comes from `Localizer::tr` with a key present in `locales/en/main.ftl`, `locales/ru/main.ftl` and `locales/hy/main.ftl`.
- Phase 1 makes no network requests at runtime.
- No blocking I/O on the UI thread except startup (before `run_native`) and opening a folder via `open::that`.
- Git on this machine: the E: drive does not record ownership, so prefix git commands with `git -c safe.directory=E:/claude/aria` unless the user has added it globally. Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Downloading font files (Task 7) requires asking the user first, stating each file's name, source URL and size.

## Review Focus

1. **Non-ASCII config path** (e.g. a Windows user profile `C:\Users\Пользователь`): settings must load and save normally. Test `store_roundtrip_non_ascii_path` in Task 3.
2. **Fluent bidi isolation marks**: Fluent wraps placeables in U+2068/U+2069 by default, and egui renders them as boxes. `tr_args` output must contain neither. Test `tr_args_has_no_isolation_marks` in Task 4.
3. **Unknown keys in settings.toml** (from a typo or a future version): the file must load without triggering corrupt-file recovery. Test `unknown_keys_are_ignored` in Task 3.
4. **Malformed accent hex** (`"2ec4b6"`, `"#2EC4B6"`, `"#zzz"`): parsing is case-insensitive and `#` is optional. An invalid value makes just that field `None` (logged at warn), and the rest of the file still loads. Tests `rgb_parse_variants` and `invalid_accent_falls_back_to_none` in Task 3.
5. **Save failure** (disk full, read-only file): the backend emits `Event::Notice` with `NoticeLevel::Error` and key `notice-settings-save-failed`, the app keeps running, and a later save is retried. Test `save_failure_emits_error_notice` in Task 6.

---

### Task 1: Workspace scaffold and first window

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml`, `.editorconfig`, `.gitignore`
- Create: `crates/aria-core/Cargo.toml`, `crates/aria-core/src/lib.rs`
- Create: `crates/aria-app/Cargo.toml`, `crates/aria-app/src/main.rs`

**Interfaces:**
- Produces: package names `aria-core` (lib `aria_core`) and `aria-app` (bin `aria`). All later dependencies are declared once in `[workspace.dependencies]` and used as `dep = { workspace = true }`.

- [ ] **Step 1: Write workspace `Cargo.toml`**

`[workspace] members = ["crates/*"]`, `resolver = "3"`. `[workspace.package]` with `version = "0.1.0"`, `edition = "2024"`, `rust-version = "1.99"`, `license = "MIT OR Apache-2.0"`, `repository` left out until Task 13. Lints exactly as in Global Constraints. Release profile from spec §3.3: `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`, `panic = "unwind"`. In `[workspace.dependencies]`, add every crate from Tech Stack at the stated version. Add `sys-locale`, `open` and `ttf-parser` with `cargo add --package <crate>` at the task that first needs them, then move the line into `[workspace.dependencies]`.

- [ ] **Step 2: Toolchain and formatting files**

`rust-toolchain.toml` as in Global Constraints. `rustfmt.toml`: `edition = "2024"`. `.gitignore`: `/target`, `*.tmp`, `.DS_Store`. `.editorconfig`: utf-8, lf, 4-space indent for `*.rs`, 2-space for `*.toml`/`*.yml`/`*.ftl`.

- [ ] **Step 3: Minimal `aria-app` main**

`main() -> eframe::Result` that opens a 1200×780 window titled `Aria` (min 900×600, `persist_window: true`, renderer glow) and shows an empty `CentralPanel`. This is replaced in Task 11.

- [ ] **Step 4: Build and lint**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: finishes with no warnings and creates `Cargo.lock`.

- [ ] **Step 5: Launch check**

Run: `cargo run -p aria-app` in the background, wait about 5 s, confirm with `Get-Process aria` that the process is alive and its `MainWindowTitle` is `Aria`, then stop it.
Expected: the process ran and the window title matched.

- [ ] **Step 6: Commit**

`git add -A && git commit -m "chore: scaffold Aria workspace with first window"`

---

### Task 2: Paths and `Redacted<T>`

**Files:**
- Create: `crates/aria-core/src/paths.rs`, `crates/aria-core/src/redact.rs`, `crates/aria-core/src/error.rs`
- Modify: `crates/aria-core/src/lib.rs` (add `pub mod` lines)

**Interfaces:**
- Produces:
  - `pub struct AppPaths { pub config_dir: PathBuf, pub data_dir: PathBuf, pub cache_dir: PathBuf, pub log_dir: PathBuf }`
  - `AppPaths::resolve() -> Result<AppPaths, PathError>`: reads `ARIA_CONFIG_DIR` and calls `resolve_with`.
  - `AppPaths::resolve_with(config_override: Option<PathBuf>) -> Result<AppPaths, PathError>`: uses `ProjectDirs::from("", "", "Aria")`. The override replaces only `config_dir`. `log_dir = data_dir.join("logs")`.
  - `AppPaths::settings_file(&self) -> PathBuf` returns `config_dir/settings.toml`.
  - `AppPaths::ensure_dirs(&self) -> Result<(), PathError>` creates all four dirs.
  - `pub struct Redacted<T>(T)` with `new(T) -> Self`, `expose(&self) -> &T`, `Debug` and `Display` writing `***`, and `Clone` when `T: Clone`.
  - `error.rs`: `#[derive(thiserror::Error, Debug)] pub enum PathError { NoHome, Io { path: PathBuf, source: io::Error } }`.

- [ ] **Step 1: Write failing tests**

```rust
// paths.rs #[cfg(test)]
#[test] fn override_replaces_config_dir_only() {
    let p = AppPaths::resolve_with(Some("/tmp/x".into())).unwrap();
    assert_eq!(p.config_dir, PathBuf::from("/tmp/x"));
    assert_eq!(p.settings_file(), PathBuf::from("/tmp/x/settings.toml"));
    assert!(p.log_dir.ends_with("logs"));
}
#[test] fn ensure_dirs_creates_all() { /* tempdir-based AppPaths via struct literal; assert all exist */ }
// redact.rs
#[test] fn redacted_never_prints_inner() {
    let r = Redacted::new("secret-token".to_string());
    assert_eq!(format!("{r:?}"), "***");
    assert_eq!(format!("{r}"), "***");
    assert_eq!(r.expose(), "secret-token");
}
```

- [ ] **Step 2: Run tests, expect compile failure**

Run: `cargo test -p aria-core --locked`
Expected: FAIL (types not defined).

- [ ] **Step 3: Implement `paths.rs`, `redact.rs`, `error.rs`**

- [ ] **Step 4: Run tests**

Run: `cargo test -p aria-core --locked`
Expected: 3 passed.

- [ ] **Step 5: Commit**

`git commit -m "feat(core): app paths and Redacted wrapper"`

---

### Task 3: Settings model and store

**Files:**
- Create: `crates/aria-core/src/settings/mod.rs` (model), `crates/aria-core/src/settings/store.rs` (I/O), `crates/aria-core/src/notice.rs`
- Modify: `crates/aria-core/src/error.rs` (add `SettingsError`), `lib.rs`

**Interfaces:**
- Produces in `notice.rs`:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum NoticeLevel { Info, Warning, Error }`
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub struct Notice { pub level: NoticeLevel, pub key: &'static str }`
- Produces in `settings/mod.rs`:
  - `pub const CURRENT_SCHEMA_VERSION: u32 = 1;`
  - `Settings` with exactly the fields, types and defaults in spec §5.2. `#[serde(default)]` is on the struct, and there is no `deny_unknown_fields`.
  - `Language { En, Ru, Hy }` with serde rename to `"en"`, `"ru"`, `"hy"`, `Language::ALL: [Language; 3]` and `fn code(self) -> &'static str`.
  - `Theme { Dark, Light, System }` with serde lowercase.
  - `LogLevel { Error, Warn, Info, Debug, Trace }` with serde lowercase and `fn as_filter(self) -> &'static str`.
  - `Rgb { r: u8, g: u8, b: u8 }` with `FromStr` (case-insensitive, optional `#`, exactly 6 hex digits), `Display` as `#RRGGBB` uppercase, and serde via those.
  - `accent_dark` and `accent_light` use `#[serde(default, deserialize_with = "lenient_rgb")]`. An invalid string gives `None` plus `tracing::warn!`.
  - `Settings::sanitized(self) -> Settings` clamps `sidebar_width` to `180.0..=320.0`, and NaN becomes `220.0`.
  - `pub const SIDEBAR_MIN: f32 = 180.0; pub const SIDEBAR_MAX: f32 = 320.0;`
- Produces in `settings/store.rs`:
  - `pub struct SettingsStore { path: PathBuf }`, `SettingsStore::new(path: PathBuf) -> Self`, `fn path(&self) -> &Path`
  - `pub struct LoadResult { pub settings: Settings, pub notice: Option<Notice> }`
  - `SettingsStore::load(&self) -> LoadResult`. This never fails; it follows spec §5.3 steps 1–3. The recovery notice is `Notice { level: Warning, key: "notice-settings-recovered" }`, and the backup name is `settings.toml.bak-<unix_secs>`.
  - `SettingsStore::save(&self, s: &Settings) -> Result<(), SettingsError>`: atomic per spec §5.4 (`settings.toml.tmp`, `sync_all`, rename).
  - `fn migrate(value: toml::Table, from: u32) -> toml::Table` is a no-op for v1 but exists with a match arm per version so v2 has an obvious place to go.
  - `SettingsError { Io { path, source }, Parse(toml::de::Error), Serialize(toml::ser::Error), UnsupportedVersion(u32) }`
- Load algorithm: read the string → parse to `toml::Table` → read `schema_version` (default 1) → if greater than `CURRENT_SCHEMA_VERSION`, treat as corrupt → `migrate` → deserialize `Settings` → `sanitized()`.

- [ ] **Step 1: Write failing model tests** (in `settings/mod.rs`)

```rust
#[test] fn defaults_roundtrip() {
    let s = Settings::default();
    let back: Settings = toml::from_str(&toml::to_string(&s).unwrap()).unwrap();
    assert_eq!(s, back);
    assert_eq!(s.schema_version, 1);
    assert_eq!(s.theme, Theme::System);
    assert_eq!(s.sidebar_width, 220.0);
    assert!(s.language.is_none() && !s.minimize_to_tray && !s.right_panel_open);
}
#[test] fn partial_file_fills_defaults() {
    let s: Settings = toml::from_str("theme = \"dark\"").unwrap();
    assert_eq!(s.theme, Theme::Dark);
    assert_eq!(s.log_level, LogLevel::Info);
}
#[test] fn rgb_parse_variants() {
    for ok in ["#2EC4B6", "#2ec4b6", "2ec4b6"] {
        assert_eq!(ok.parse::<Rgb>().unwrap(), Rgb { r: 0x2e, g: 0xc4, b: 0xb6 });
    }
    for bad in ["#zzz", "", "#2ec4b", "#2ec4b6ff"] { assert!(bad.parse::<Rgb>().is_err()); }
    assert_eq!(Rgb { r: 0x2e, g: 0xc4, b: 0xb6 }.to_string(), "#2EC4B6");
}
#[test] fn invalid_accent_falls_back_to_none() {
    let s: Settings = toml::from_str("accent_dark = \"#zzz\"\ntheme = \"light\"").unwrap();
    assert_eq!(s.accent_dark, None);
    assert_eq!(s.theme, Theme::Light);
}
#[test] fn unknown_keys_are_ignored() {
    let s: Settings = toml::from_str("future_option = 1\ntheme = \"dark\"").unwrap();
    assert_eq!(s.theme, Theme::Dark);
}
#[test] fn sidebar_width_clamps() {
    let mut s = Settings::default();
    s.sidebar_width = 9000.0; assert_eq!(s.clone().sanitized().sidebar_width, 320.0);
    s.sidebar_width = 10.0;   assert_eq!(s.clone().sanitized().sidebar_width, 180.0);
    s.sidebar_width = f32::NAN; assert_eq!(s.sanitized().sidebar_width, 220.0);
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test -p aria-core --locked settings`
Expected: FAIL (compile errors).

- [ ] **Step 3: Implement `notice.rs` and `settings/mod.rs`**

- [ ] **Step 4: Run model tests**

Run: `cargo test -p aria-core --locked settings`
Expected: 6 passed.

- [ ] **Step 5: Write failing store tests** (in `settings/store.rs`, using `tempfile::tempdir()`)

```rust
#[test] fn missing_file_gives_defaults_without_writing()   // load → defaults, notice None, file still absent
#[test] fn save_then_load_roundtrip()                      // theme=Dark, language=Some(Hy), accent_light=Some(..)
#[test] fn store_roundtrip_non_ascii_path()               // dir = tmp.join("Пользователь/Աria")
#[test] fn corrupt_file_is_backed_up() {
    // write "theme = [[[" ; load
    // assert settings == Settings::default()
    // assert notice == Some(Notice { level: NoticeLevel::Warning, key: "notice-settings-recovered" })
    // assert settings.toml is gone and exactly one file matching "settings.toml.bak-*" exists with the original bytes
}
#[test] fn newer_schema_is_treated_as_corrupt()           // "schema_version = 99" → defaults + recovered notice + backup
#[test] fn save_leaves_no_tmp_file()                      // after save, no "settings.toml.tmp" in dir
#[test] fn save_overwrites_existing()                     // save Dark, save Light, load → Light
#[test] fn out_of_range_values_are_clamped_on_load()      // "sidebar_width = 5000.0" → 320.0, no notice
#[test] fn missing_schema_version_loads_as_v1()          // "theme = \"dark\"" (no schema_version) → Dark, no notice; migrate() called with from = 1
```

- [ ] **Step 6: Run, expect failure**

Run: `cargo test -p aria-core --locked store`
Expected: FAIL.

- [ ] **Step 7: Implement `store.rs` and `SettingsError`**

- [ ] **Step 8: Run all core tests**

Run: `cargo test -p aria-core --locked`
Expected: all pass (17 so far).

- [ ] **Step 9: Commit**

`git commit -m "feat(core): settings model and atomic store with recovery"`

---

### Task 4: Localization

**Files:**
- Create: `locales/en/main.ftl`, `locales/ru/main.ftl`, `locales/hy/main.ftl`
- Create: `crates/aria-core/src/i18n/mod.rs`, `crates/aria-core/src/i18n/locale.rs`
- Modify: `crates/aria-core/src/error.rs` (`I18nError { Parse { lang: &'static str, detail: String } }`), `lib.rs`

**Interfaces:**
- Consumes: `Language` (Task 3).
- Produces:
  - `pub const SOURCES: [(Language, &str); 3]` built with `include_str!("../../../../locales/<code>/main.ftl")`.
  - `pub struct Localizer`, built on `fluent_bundle::concurrent::FluentBundle<FluentResource>` so that it is `Send + Sync`.
  - `Localizer::new(lang: Language) -> Result<Localizer, I18nError>`: builds the active and English bundles and calls `set_use_isolating(false)` on both.
  - `Localizer::set_language(&mut self, lang: Language) -> Result<(), I18nError>`, `fn language(&self) -> Language`
  - `Localizer::tr(&self, key: &str) -> Cow<'_, str>` and `tr_args(&self, key: &str, args: &FluentArgs) -> Cow<'_, str>`. Fallback goes active → English → the key itself. The warn-once set is a `Mutex<HashSet<String>>`, and a poisoned mutex just skips the dedup.
  - `pub fn message_ids(src: &str) -> BTreeSet<String>`: public so the parity test and Task 9 can use it.
  - `locale.rs`: `pub fn language_from_locale(tag: Option<&str>) -> Language` (prefix match `ru` → Ru, `hy` → Hy, otherwise En) and `pub fn os_language() -> Language` (`sys_locale::get_locale()`). Add the `sys-locale` dependency here.
- Key list. The English file defines exactly these, and RU/HY must define the same set:
  - nav: `nav-home nav-search nav-library nav-liked nav-playlists nav-albums nav-artists nav-local nav-settings nav-back nav-forward`
  - empty states: `empty-phase-pending` (with arg `{ $phase }`), `empty-nothing-playing`, `empty-right-panel`
  - page headings reuse the `nav-*` keys
  - player controls (tooltips): `player-play player-previous player-next player-volume player-devices player-queue player-lyrics player-coming-in` (arg `{ $phase }`)
  - settings: `settings-general settings-language settings-language-system settings-theme settings-theme-dark settings-theme-light settings-theme-system settings-accent-dark settings-accent-light settings-accent-reset settings-minimize-to-tray settings-minimize-to-tray-hint settings-advanced settings-log-level settings-open-config settings-open-logs settings-reset settings-reset-confirm-title settings-reset-confirm-body settings-cancel settings-confirm settings-about settings-version settings-license settings-third-party`
  - notices: `notice-settings-recovered notice-settings-readonly notice-settings-save-failed notice-open-folder-failed notice-dismiss`
  - language names use endonyms and are identical in every locale: `lang-en = English`, `lang-ru = Русский`, `lang-hy = Հայերեն`

- [ ] **Step 1: Write the three `.ftl` files**

English copy written plainly. Russian at production quality. Armenian as a best effort; add a header comment `# Needs native-speaker review before 1.0` to `hy/main.ftl`. `empty-phase-pending = Available in Phase { $phase }`.

- [ ] **Step 2: Write failing tests**

```rust
#[test] fn all_sources_parse() { for l in Language::ALL { Localizer::new(l).unwrap(); } }
#[test] fn key_parity_with_english() {
    let en = message_ids(SOURCES[0].1);
    for (lang, src) in &SOURCES[1..] {
        let missing: Vec<_> = en.difference(&message_ids(src)).collect();
        assert!(missing.is_empty(), "{lang:?} missing {missing:?}");
    }
}
#[test] fn falls_back_to_english_then_key() {
    let l = Localizer::new(Language::Ru).unwrap();
    assert_eq!(l.tr("definitely-not-a-key"), "definitely-not-a-key");
}
#[test] fn switches_language_live() {
    let mut l = Localizer::new(Language::En).unwrap();
    assert_eq!(l.tr("nav-settings"), "Settings");
    l.set_language(Language::Ru).unwrap();
    assert_ne!(l.tr("nav-settings"), "Settings");
}
#[test] fn tr_args_has_no_isolation_marks() {
    let l = Localizer::new(Language::En).unwrap();
    let mut a = FluentArgs::new(); a.set("phase", 2);
    let s = l.tr_args("empty-phase-pending", &a);
    assert_eq!(s, "Available in Phase 2");
    assert!(!s.contains('\u{2068}') && !s.contains('\u{2069}'));
}
#[test] fn locale_mapping() {
    assert_eq!(language_from_locale(Some("ru-RU")), Language::Ru);
    assert_eq!(language_from_locale(Some("hy-AM")), Language::Hy);
    assert_eq!(language_from_locale(Some("en-US")), Language::En);
    assert_eq!(language_from_locale(Some("fr-FR")), Language::En);
    assert_eq!(language_from_locale(Some("")), Language::En);
    assert_eq!(language_from_locale(None), Language::En);
}
```

- [ ] **Step 3: Run, expect failure**

Run: `cargo test -p aria-core --locked i18n`
Expected: FAIL.

- [ ] **Step 4: Implement `i18n/mod.rs` and `locale.rs`**

- [ ] **Step 5: Run tests**

Run: `cargo test -p aria-core --locked`
Expected: all pass.

- [ ] **Step 6: Commit**

`git commit -m "feat(core): Fluent localization with EN/RU/HY"`

---

### Task 5: Logging and panic hook

**Files:**
- Create: `crates/aria-core/src/logging.rs`
- Modify: `crates/aria-core/src/error.rs` (`LoggingError { Init(String), Io(io::Error) }`), `lib.rs`

**Interfaces:**
- Consumes: `LogLevel` (Task 3).
- Produces:
  - `pub fn filter_directive(level: LogLevel, env_override: Option<&str>) -> String`: returns the env value if it is non-empty, otherwise `"warn,aria_core=<lvl>,aria=<lvl>"`. Dependencies stay at warn so their logs don't drown out Aria's.
  - `pub struct LoggingHandle` (holds the `tracing_appender::non_blocking::WorkerGuard` and a `tracing_subscriber::reload::Handle<EnvFilter, Registry>`)
  - `pub fn init(log_dir: Option<&Path>, level: LogLevel) -> Result<LoggingHandle, LoggingError>`. It reads `ARIA_LOG`. The file layer uses `tracing_appender::rolling::Builder::new().rotation(Rotation::DAILY).filename_prefix("aria").filename_suffix("log").max_log_files(7)` through `non_blocking`. With `log_dir: None`, it logs to stderr only. In debug builds it adds a stderr layer.
  - `LoggingHandle::set_level(&self, level: LogLevel)`: a no-op if `ARIA_LOG` is set.
  - `pub fn install_panic_hook(log_dir: Option<PathBuf>)`: writes `error!` with thread name, message and location, and also appends the same line synchronously to `log_dir/panic.log`. The non-blocking writer can't be flushed from inside a hook, so the sync file guarantees the record survives. It then calls the previous hook.
  - `pub fn log_startup_banner()`: logs version (`env!("CARGO_PKG_VERSION")`), `std::env::consts::OS` and `ARCH`.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn directive_uses_level_when_no_env() {
    assert_eq!(filter_directive(LogLevel::Debug, None), "warn,aria_core=debug,aria=debug");
}
#[test] fn directive_prefers_env() {
    assert_eq!(filter_directive(LogLevel::Info, Some("trace")), "trace");
    assert_eq!(filter_directive(LogLevel::Info, Some("")), "warn,aria_core=info,aria=info");
}
#[test] fn panic_line_is_written_to_file() {
    // call the pure helper `format_panic_line(thread, msg, location) -> String` and
    // `append_panic_line(dir, &line)`; assert panic.log contains the message
}
```

Only these pure helpers are unit-tested, because the global subscriber can be set once per process. Live logging is verified in Task 11.

- [ ] **Step 2: Run, expect failure**

Run: `cargo test -p aria-core --locked logging`
Expected: FAIL.

- [ ] **Step 3: Implement `logging.rs`**

- [ ] **Step 4: Run tests**

Run: `cargo test -p aria-core --locked`
Expected: all pass.

- [ ] **Step 5: Commit**

`git commit -m "feat(core): rolling file logging, live level reload, panic hook"`

---

### Task 6: Commands, events and the backend thread

**Files:**
- Create: `crates/aria-core/src/command.rs`, `crates/aria-core/src/event.rs`, `crates/aria-core/src/runtime.rs`
- Modify: `lib.rs`

**Interfaces:**
- Consumes: `Settings`, `SettingsStore`, `SettingsError`, `Notice`, `NoticeLevel`.
- Produces:
  - `#[derive(Debug, Clone)] pub enum Command { UpdateSettings(Settings), ResetSettings, Shutdown }`
  - `#[derive(Debug, Clone, PartialEq)] pub enum Event { SettingsSaved, SettingsReplaced(Settings), Notice(Notice) }`
  - `pub trait SettingsSink: Send + 'static { fn save(&mut self, s: &Settings) -> Result<(), SettingsError>; }`, implemented for `SettingsStore`.
  - `pub type Waker = Arc<dyn Fn() + Send + Sync>;`
  - `pub struct BackendConfig { pub debounce: Duration }` with `Default` = 500 ms.
  - `Backend::start(sink: Option<Box<dyn SettingsSink>>, waker: Waker, cfg: BackendConfig) -> io::Result<BackendHandle>`. `None` means read-only mode: updates are accepted and nothing is written.
  - `BackendHandle::send(&self, cmd: Command)`: never blocks, and silently drops the command if the backend has exited.
  - `BackendHandle::drain(&self) -> Vec<Event>`: `try_recv` until empty.
  - `BackendHandle::shutdown(self, timeout: Duration) -> bool`: sends `Shutdown`, then joins with a timeout. Implement this by having the thread send on a `crossbeam_channel::bounded(1)` "done" channel and calling `recv_timeout`. Returns `true` if it exited in time.
- **Deviation from spec §3.2, decided here:** commands travel over `tokio::sync::mpsc::unbounded_channel`, whose sync `send` never blocks the UI, so the backend can `select!` on them alongside the debounce timer. Events travel over `crossbeam_channel::unbounded` so the UI can `try_recv`. Record this in a doc comment on `runtime.rs`.
- Backend loop, on a thread named `aria-backend` running a `tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_time()` runtime:
  - `pending: Option<Settings>` and `deadline: Option<Instant>`
  - `select!` between the next command and `sleep_until(deadline)`
  - `UpdateSettings(s)`: set `pending = s` and reset `deadline = now + debounce`.
  - Deadline fires: save `pending` through `spawn_blocking`.
    - On Ok: send `Event::SettingsSaved`.
    - On Err: `error!` and send `Event::Notice(Notice { level: Error, key: "notice-settings-save-failed" })`. Keep nothing pending; the next update retries naturally.
    - Either way, call `waker()`.
  - `ResetSettings`: drop pending, save `Settings::default()` immediately, send `SettingsReplaced(default)` plus the save result event, and wake.
  - `Shutdown` or channel closed: if pending, save synchronously, then exit and signal done.

- [ ] **Step 1: Write failing tests** (in `runtime.rs`, using a test sink that pushes into an `Arc<Mutex<Vec<Settings>>>` and a test waker that counts calls)

```rust
const FAST: BackendConfig = BackendConfig { debounce: Duration::from_millis(50) };
#[test] fn debounce_coalesces_bursts() {
    // send 10 UpdateSettings with sidebar_width 200.0..=209.0, sleep 300 ms
    // assert saved.len() == 1 && saved[0].sidebar_width == 209.0
    // assert drain() contains Event::SettingsSaved and waker count >= 1
}
#[test] fn shutdown_flushes_pending() {
    // send UpdateSettings(theme Dark), immediately shutdown(2s) -> true
    // assert saved == [that settings]
}
#[test] fn reset_saves_defaults_and_replaces() {
    // send ResetSettings, sleep 200 ms
    // assert saved.last() == Settings::default()
    // assert drain() contains Event::SettingsReplaced(Settings::default())
}
#[test] fn save_failure_emits_error_notice() {
    // sink returns Err(SettingsError::Io{..}) first time, Ok after
    // update, sleep 300 ms → drain() contains Event::Notice(Notice{level: Error, key:"notice-settings-save-failed"})
    // update again, sleep 300 ms → second call reached sink (retry works)
}
#[test] fn read_only_mode_accepts_updates() {
    // Backend::start(None, ..); send update; shutdown → true, no panic
}
#[test] fn dropping_handle_flushes_and_stops() {
    // send UpdateSettings(theme Light), drop(handle) without shutdown, sleep 500 ms
    // assert saved == [that settings]  (channel-closed path flushes like Shutdown)
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test -p aria-core --locked runtime`
Expected: FAIL.

- [ ] **Step 3: Implement `command.rs`, `event.rs`, `runtime.rs`**

- [ ] **Step 4: Run tests 3× to check for flakiness**

Run: `cargo test -p aria-core --locked runtime` three times
Expected: all pass every run.

- [ ] **Step 5: Commit**

`git commit -m "feat(core): backend thread with debounced settings persistence"`

---

### Task 7: Theme, fonts and icons

**Files:**
- Create: `crates/aria-app/src/ui/mod.rs`, `crates/aria-app/src/ui/theme/{mod.rs,palette.rs,visuals.rs,fonts.rs,icons.rs}`
- Create: `assets/fonts/Inter-Regular.ttf`, `assets/fonts/Inter-SemiBold.ttf`, `assets/fonts/NotoSansArmenian-Regular.ttf`, `assets/fonts/Phosphor.ttf`, `assets/fonts/LICENSE-Inter.txt`, `assets/fonts/LICENSE-NotoSansArmenian.txt`, `assets/fonts/LICENSE-Phosphor.txt`
- Create: `crates/aria-app/tests/font_coverage.rs`

**Interfaces:**
- Consumes: `Settings`, `Theme`, `Rgb` (Task 3); `SOURCES` (Task 4).
- Produces:
  - `palette.rs`: `#[derive(Clone, Copy, Debug, PartialEq)] pub struct Palette` with the fields in spec §4.1.
    - `Palette::dark()` and `Palette::light()`
    - `Palette::with_accent(self, accent: Option<Rgb>) -> Palette`: also derives `accent_hover`. For dark, mix 12% toward white; for light, mix 12% toward black.
    - `Palette::for_theme(theme: egui::Theme, s: &Settings) -> Palette`: picks `accent_dark` or `accent_light`.
    - `Palette::on_accent(&self) -> Color32`: black or white, whichever has more contrast.
    - `pub fn contrast_ratio(a: Color32, b: Color32) -> f32`: WCAG relative luminance.
  - Exact base colors (the only color literals in the codebase):

    | token | dark | light |
    |---|---|---|
    | bg | `#121417` | `#F7F8FA` |
    | surface | `#1A1D21` | `#FFFFFF` |
    | surface_raised | `#23272D` | `#EEF0F3` |
    | border | `#2E333A` | `#D9DDE3` |
    | text | `#E8EAED` | `#16181C` |
    | text_muted | `#9AA0A6` | `#5F6670` |
    | accent | `#2EC4B6` | `#13897F` |
    | focus | = accent | = accent |
    | danger | `#E5484D` | `#C62828` |

  - `visuals.rs`:
    - `pub fn visuals(p: &Palette, dark: bool) -> egui::Visuals`
    - `pub fn apply(ctx: &egui::Context, s: &Settings)`: sets both themes' visuals with `ctx.set_visuals_of`, the theme preference via `ctx.set_theme(ThemePreference)` or the 0.37 equivalent, and the style (spacing scale and text styles per spec §4.1). Call it at startup and whenever theme or accent changes.
    - `pub const SPACE: [f32; 5] = [4.0, 8.0, 12.0, 16.0, 24.0];`
  - `fonts.rs`:
    - `pub fn definitions() -> egui::FontDefinitions`: starts from `FontDefinitions::default()` and inserts `inter`, `noto-armenian` and `phosphor` at the front of `Proportional` in that order. It adds family `FontFamily::Name("semibold".into())` = [`inter-semibold`, `noto-armenian`, `phosphor`]. Monospace keeps the defaults.
    - `pub fn install(ctx: &egui::Context)`
    - `pub const FONT_FILES: [(&str, &[u8]); 4]`: embedded with `include_bytes!`, and also used by the coverage test.
  - `icons.rs`: `pub const HOUSE, MAGNIFYING_GLASS, BOOKS, HEART, PLAYLIST, VINYL_RECORD, MICROPHONE_STAGE, FOLDER_SIMPLE, GEAR, PLAY, SKIP_BACK, SKIP_FORWARD, SPEAKER_HIGH, DEVICES, QUEUE, QUOTES, CARET_LEFT, CARET_RIGHT, X, WARNING, INFO, MUSIC_NOTES: &str`. Each holds one codepoint taken from the Phosphor package's `src/regular/style.css` (`.ph-<name>:before { content: "\eXXX" }`). Also `pub const ALL: &[&str]` for the test.

- [ ] **Step 1: Ask the user's permission, then download fonts**

Ask the user, listing:
- `https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip`. Extract `extras/ttf/Inter-Regular.ttf`, `extras/ttf/Inter-SemiBold.ttf` and `LICENSE.txt`.
- `https://notofonts.github.io/armenian/fonts/NotoSansArmenian/hinted/ttf/NotoSansArmenian-Regular.ttf`, plus `OFL.txt` from `https://github.com/notofonts/armenian`. If the URL returns 404, use the latest release zip at `https://github.com/notofonts/armenian/releases`.
- `https://unpkg.com/@phosphor-icons/web@2.1.2/src/regular/Phosphor.ttf` and `style.css` from the same folder (read only for the codepoints, not committed), plus `LICENSE` from `https://unpkg.com/@phosphor-icons/web@2.1.2/LICENSE`.

Download into a fresh scratch directory, check that each `.ttf` starts with bytes `00 01 00 00` or `OTTO`, then copy into `assets/fonts/`.

- [ ] **Step 2: Write failing palette tests** (in `palette.rs`)

```rust
#[test] fn text_contrast_meets_aa() {
    for p in [Palette::dark(), Palette::light()] {
        assert!(contrast_ratio(p.text, p.bg) >= 7.0);
        assert!(contrast_ratio(p.text_muted, p.bg) >= 4.5);
        assert!(contrast_ratio(p.text, p.surface) >= 4.5);
    }
}
#[test] fn accent_is_visible_against_bg() {   // WCAG 1.4.11 non-text contrast
    for p in [Palette::dark(), Palette::light()] { assert!(contrast_ratio(p.accent, p.bg) >= 3.0); }
}
#[test] fn on_accent_is_readable() {
    for p in [Palette::dark(), Palette::light()] { assert!(contrast_ratio(p.on_accent(), p.accent) >= 4.5); }
}
#[test] fn accent_override_applies_per_mode() {
    let mut s = Settings::default();
    s.accent_dark = Some(Rgb { r: 255, g: 0, b: 128 });
    assert_eq!(Palette::for_theme(egui::Theme::Dark, &s).accent, Color32::from_rgb(255, 0, 128));
    assert_eq!(Palette::for_theme(egui::Theme::Light, &s).accent, Palette::light().accent);
}
#[test] fn visuals_follow_palette() {
    let v = visuals(&Palette::dark(), true);
    assert!(v.dark_mode);
    assert_eq!(v.panel_fill, Palette::dark().surface);
    assert_eq!(v.window_fill, Palette::dark().surface_raised);
}
```

- [ ] **Step 3: Write the failing font coverage test** (`crates/aria-app/tests/font_coverage.rs`)

```rust
#[test] fn every_locale_char_has_a_glyph() {
    // faces = FONT_FILES parsed with ttf_parser::Face::parse(bytes, 0)
    // for each (lang, src) in aria_core::i18n::SOURCES, for each char c in src
    //   (skip c.is_whitespace() || c.is_control()):
    //   assert faces.iter().any(|f| f.glyph_index(c).is_some()), "{lang:?}: no glyph for {c:?} (U+{:04X})"
}
#[test] fn every_icon_exists_in_phosphor() { /* each icons::ALL char resolves in Phosphor.ttf */ }
```

To support this, `aria-app` needs a `src/lib.rs` exposing `pub mod ui;` alongside `main.rs`, with `main.rs` using `aria_app::...`. Add `ttf-parser` as a dev-dependency.

- [ ] **Step 4: Run, expect failure**

Run: `cargo test -p aria-app --locked`
Expected: FAIL.

- [ ] **Step 5: Implement `palette.rs`, `visuals.rs`, `fonts.rs`, `icons.rs`**

- [ ] **Step 6: Run tests**

Run: `cargo test -p aria-app --locked`
Expected: all pass. If a locale char has no glyph, fix the font chain; do not edit the translation to dodge it.

- [ ] **Step 7: Commit** (fonts and license files included)

`git commit -m "feat(app): palette, visuals, bundled fonts and icons"`

---

### Task 8: Navigation model

**Files:**
- Create: `crates/aria-app/src/nav.rs`
- Modify: `crates/aria-app/src/lib.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub enum Route { Home, Search, Library, Liked, Playlists, Albums, Artists, LocalMusic, Settings }`
  - `Route::ALL: [Route; 9]` in sidebar order
  - `fn label_key(self) -> &'static str`: `nav-home` … `nav-settings`, with `LocalMusic` → `nav-local`
  - `fn icon(self) -> &'static str` from `icons`
  - `fn phase(self) -> Option<u8>`: `Home` → 2, `LocalMusic` → 2, `Search`/`Library`/`Liked`/`Playlists`/`Albums`/`Artists` → 3, `Settings` → `None`
  - `pub const HISTORY_CAP: usize = 50;`
  - `pub struct History`
    - `History::new(start: Route) -> History`
    - `fn current(&self) -> Route`
    - `fn navigate(&mut self, to: Route)`: no-op if `to == current`; otherwise push current onto back (dropping the oldest above the cap) and clear forward
    - `fn back(&mut self) -> bool` and `fn forward(&mut self) -> bool`
    - `fn can_back(&self) -> bool` and `fn can_forward(&self) -> bool`

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn navigate_back_forward() {
    let mut h = History::new(Route::Home);
    h.navigate(Route::Search); h.navigate(Route::Albums);
    assert!(h.back()); assert_eq!(h.current(), Route::Search);
    assert!(h.forward()); assert_eq!(h.current(), Route::Albums);
    assert!(!h.forward());
}
#[test] fn navigate_clears_forward() {
    let mut h = History::new(Route::Home);
    h.navigate(Route::Search); h.back(); h.navigate(Route::Artists);
    assert!(!h.can_forward());
}
#[test] fn same_route_is_noop() {
    let mut h = History::new(Route::Home); h.navigate(Route::Home); assert!(!h.can_back());
}
#[test] fn back_stack_is_capped() {
    let mut h = History::new(Route::Home);
    for i in 0..200 { h.navigate(if i % 2 == 0 { Route::Search } else { Route::Home }); }
    let mut n = 0; while h.back() { n += 1; } assert_eq!(n, HISTORY_CAP);
}
#[test] fn every_route_has_a_label_key_in_english() {
    let ids = aria_core::i18n::message_ids(aria_core::i18n::SOURCES[0].1);
    for r in Route::ALL { assert!(ids.contains(r.label_key()), "{r:?}"); }
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test -p aria-app --locked nav`
Expected: FAIL.

- [ ] **Step 3: Implement `nav.rs`**

- [ ] **Step 4: Run tests**

Run: `cargo test -p aria-app --locked nav`
Expected: 5 passed.

- [ ] **Step 5: Commit**

`git commit -m "feat(app): route enum and capped navigation history"`

---

### Task 9: App shell: layout, pages, widgets, shortcuts

**Files:**
- Create: `crates/aria-app/src/app.rs`
- Create: `crates/aria-app/src/ui/layout/{mod.rs,sidebar.rs,player_bar.rs,right_panel.rs}`
- Create: `crates/aria-app/src/ui/pages/{mod.rs,home.rs,search.rs,library.rs,liked.rs,playlists.rs,albums.rs,artists.rs,local_music.rs,settings.rs}` (`settings.rs` is a heading-only stub here; Task 10 fills it)
- Create: `crates/aria-app/src/ui/widgets/{mod.rs,nav_item.rs,section_header.rs,empty_state.rs}`
- Create: `crates/aria-app/tests/ui_shell.rs`

**Interfaces:**
- Consumes: `Settings`, `Localizer`, `BackendHandle`, `Command`, `Event`, `Notice`, `AppPaths`, `LoggingHandle` (core); `Palette`, `visuals::apply`, `fonts::install`, `icons` (Task 7); `Route`, `History` (Task 8).
- Produces:
  - `pub struct AppDeps { pub backend: Option<BackendHandle>, pub logging: Option<LoggingHandle>, pub paths: Option<AppPaths> }` with `AppDeps::none()` for tests.
  - `pub struct AriaApp`, with private fields for settings, localizer, history and deps. Toasts are added in Task 10.
  - `AriaApp::new(ctx: &egui::Context, settings: Settings, localizer: Localizer, deps: AppDeps, startup_notices: Vec<Notice>) -> AriaApp`: calls `fonts::install` and `visuals::apply`.
  - `AriaApp::show(&mut self, ctx: &egui::Context)`: the whole frame. It is the seam for both eframe and kittest. Per frame it:
    1. drains backend events
    2. handles shortcuts
    3. shows the sidebar (`SidePanel::left`, resizable, width clamped to `SIDEBAR_MIN..=SIDEBAR_MAX`; on resize end, send `UpdateSettings`)
    4. shows the player bar (`TopBottomPanel::bottom`, height 72)
    5. shows the right panel if `right_panel_open` (`SidePanel::right`, default width 300)
    6. shows the central page
  - `fn update_settings(&mut self, ctx: &egui::Context, new: Settings)`: the single place settings change. It:
    - applies the new visuals if theme or accent changed
    - calls `localizer.set_language` if the language changed
    - calls `logging.set_level` if the log level changed
    - stores the new settings
    - sends `Command::UpdateSettings`
  - `impl eframe::App for AriaApp`: delegates to `show` and sends `Shutdown` on exit (`on_exit` → `backend.take().shutdown(Duration::from_secs(2))`). Check the 0.37 trait signature on docs.rs; whatever it is, the body only calls `self.show(ctx)`.
  - `widgets::nav_item(ui, icon: &str, label: &str, selected: bool, p: &Palette) -> egui::Response`: full-width row with an accent bar on the left when selected, hover fill `surface_raised`, and accesskit label = `label`.
  - `widgets::section_header(ui, text: &str)` uses the `semibold` family at size 20.
  - `widgets::empty_state(ui, icon: &str, title: &str, body: &str)` is centered and uses the `text_muted` body color.
  - `pages::show(route: Route, ui, ctx: &mut PageCtx)`, where `pub struct PageCtx<'a> { pub tr: &'a Localizer, pub palette: Palette, pub settings: &'a Settings, pub paths_available: bool, pub out: &'a mut Vec<PageAction> }` and `pub enum PageAction { UpdateSettings(Settings), ResetSettings, OpenConfigDir, OpenLogDir }`. Non-settings pages render a heading (`tr(route.label_key())`) and `empty_state` with `empty-phase-pending { $phase }`.
  - Shortcuts, read from `ctx.input`:
    - `Ctrl+1`…`Ctrl+9` jump to `Route::ALL[n-1]`
    - `Alt+ArrowLeft` and `PointerButton::Extra1` go back; `Alt+ArrowRight` and `PointerButton::Extra2` go forward
    - `Ctrl+Alt+R` toggles the right panel
  - The player bar shows the artwork placeholder (`MUSIC_NOTES` icon in a `surface_raised` rounded square) and `tr("empty-nothing-playing")`. All transport, volume, device and lyrics buttons are `add_enabled(false, ..)` with hover text `player-coming-in { $phase }`: phase 2 for transport and volume, 3 for devices, 5 for lyrics. The queue button stays enabled and toggles the right panel.

- [ ] **Step 1: Write failing kittest tests** (`tests/ui_shell.rs`)

Build each harness from `AriaApp::new(&ctx, Settings { language: Some(Language::En), ..Default::default() }, Localizer::new(Language::En).unwrap(), AppDeps::none(), vec![])` (put this in a `fn harness(size: egui::Vec2) -> Harness<'static, AriaApp>` helper in the test file) and drive it with `egui_kittest::Harness::new_state(|ctx, app: &mut AriaApp| app.show(ctx), app)` (or the 0.37 builder equivalent), size 1200×780.

```rust
#[test] fn every_sidebar_entry_changes_heading() {
    // for each route in Route::ALL, with label = English text of route.label_key():
    //   click the sidebar node with that label; harness.run();
    //   assert_eq!(harness.state().current_route(), route);
    //   assert a node with role Heading and that label exists
    // section_header must therefore set accesskit role Heading on its label
}
#[test] fn ctrl_number_jumps_to_route()        // press Ctrl+9 → current_route() == Route::Settings
#[test] fn alt_left_goes_back()               // click Search, press Alt+Left → Route::Home
#[test] fn pending_pages_say_which_phase()    // click "Local Music" → label "Available in Phase 2" present
#[test] fn player_controls_are_disabled()     // "Play" button node is disabled (accesskit is_disabled)
#[test] fn queue_button_toggles_right_panel() // click Queue → label from empty-right-panel present
#[test] fn layout_fits_minimum_window()       // harness size 900×600: all 9 nav labels present, no panic
```

Add `pub fn current_route(&self) -> Route` and `pub fn settings(&self) -> &Settings` to `AriaApp` as public read accessors.

- [ ] **Step 2: Run, expect failure**

Run: `cargo test -p aria-app --locked --test ui_shell`
Expected: FAIL.

- [ ] **Step 3: Implement widgets, layout, pages, `app.rs`**

- [ ] **Step 4: Run tests**

Run: `cargo test -p aria-app --locked`
Expected: all pass.

- [ ] **Step 5: Commit**

`git commit -m "feat(app): layout shell, nine routes, shortcuts"`

---

### Task 10: Settings page, toasts, reset dialog

**Files:**
- Modify: `crates/aria-app/src/ui/pages/settings.rs`, `crates/aria-app/src/app.rs`
- Create: `crates/aria-app/src/ui/widgets/toast.rs`
- Create: `crates/aria-app/tests/ui_settings.rs`

**Interfaces:**
- Consumes: `PageCtx`, `PageAction`, `update_settings` (Task 9); `Event`, `Notice` (Task 6).
- Produces:
  - `pub struct Toasts`
    - `fn push(&mut self, notice: Notice, now: f64)`
    - `fn show(&mut self, ctx: &egui::Context, tr: &Localizer, p: &Palette)`: renders bottom-right with `egui::Area`. Info and Warning expire after 6.0 s. Error stays until clicked. Each toast has a dismiss button labelled `notice-dismiss`. While any toast has an expiry it calls `ctx.request_repaint_after` for the next one.
    - `fn len(&self) -> usize`
  - The settings page has three groups: General, Advanced, About (spec §8).
    - **Language:** a ComboBox with `settings-language-system` (meaning `None`) and the three endonyms.
    - **Theme:** three `selectable_value`s.
    - **Accents:** per-mode `color_edit_button_srgb` plus a `settings-accent-reset` button that sets the field to `None`.
    - **Minimize to tray:** a checkbox with hint text `settings-minimize-to-tray-hint`.
    - **Log level:** a ComboBox.
    - **Folders:** open-config and open-logs buttons, disabled when `PageCtx::paths_available` is false.
    - **Reset:** a reset button that opens `egui::Modal` with `settings-reset-confirm-title`/`-body` and Cancel/Confirm buttons.
    - **About:** version from `env!("CARGO_PKG_VERSION")`, license text `MIT OR Apache-2.0`, and a third-party line pointing at `THIRD_PARTY_NOTICES.md`.
  - In `app.rs`, `PageAction` handling:
    - `UpdateSettings` → `update_settings`
    - `ResetSettings` → send `Command::ResetSettings` and apply `Settings::default()` locally
    - `OpenConfigDir` / `OpenLogDir` → `open::that(path)`; on Err, log it and push a Warning toast with key `notice-open-folder-failed` (add the `open` dependency here)
  - Event handling: `Notice` → toast; `SettingsReplaced(s)` → apply locally without re-sending; `SettingsSaved` → `trace!` only.
  - The language "System" choice stores `None` and applies `os_language()` immediately.

- [ ] **Step 1: Write failing tests** (`tests/ui_settings.rs`, same harness setup as Task 9, starting on Settings via Ctrl+9)

```rust
#[test] fn choosing_dark_theme_switches_visuals() // click "Dark" → harness.ctx.theme() == egui::Theme::Dark && app.settings().theme == Theme::Dark
#[test] fn choosing_light_theme_switches_visuals()
#[test] fn switching_language_relabels_sidebar() // pick "Русский" → sidebar no longer has label "Settings"; has tr(ru,"nav-settings")
#[test] fn reset_cancel_keeps_settings()         // set Dark, Reset, Cancel → theme still Dark
#[test] fn reset_confirm_restores_defaults()     // set Dark, Reset, Confirm → app.settings() == &Settings::default()
#[test] fn startup_notice_shows_toast()          // AriaApp::new(..., vec![Notice{Warning,"notice-settings-recovered"}]) → its English text is visible
#[test] fn error_toast_persists_until_dismissed() // Toasts unit test: push Error at t=0; after advancing input time past 6 s still len()==1; click dismiss → 0
#[test] fn info_toast_expires()                  // push Info at t=0; frame at t=6.5 → len()==0
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test -p aria-app --locked --test ui_settings`
Expected: FAIL.

- [ ] **Step 3: Implement `toast.rs`, the settings page and action/event wiring**

- [ ] **Step 4: Run all tests**

Run: `cargo test --workspace --locked`
Expected: all pass.

- [ ] **Step 5: Commit**

`git commit -m "feat(app): settings page, toasts, reset confirmation"`

---

### Task 11: Startup wiring and hands-on verification

**Files:**
- Modify: `crates/aria-app/src/main.rs`
- Create: `scripts/screenshot-window.ps1`

**Interfaces:**
- Consumes: everything above.
- `main() -> anyhow::Result<()>` runs this sequence, timing each stage with `Instant` and logging it at info as `startup stage=<name> ms=<n>` (the cold-start baseline from spec §7):
  1. `AppPaths::resolve()`, then `ensure_dirs()`. On failure, `paths = None` and push startup notice `notice-settings-readonly`.
  2. `logging::init(paths.log_dir, Settings::default().log_level)`, `install_panic_hook`, `log_startup_banner`. If init fails, print to stderr and continue.
  3. `SettingsStore::load()` if paths exist, otherwise `Settings::default()`. Push its notice, then `logging.set_level(settings.log_level)`.
  4. Resolve the language: `settings.language.unwrap_or_else(os_language)`. Then `Localizer::new(lang)`, falling back to `Language::En`. A Localizer failure on English is fatal; return `Err`.
  5. Start the backend with sink `Some(Box::new(store))`, or `None` in read-only mode. The waker is set once the egui `Context` exists: create it with `Arc<OnceLock<egui::Context>>` that the `run_native` creator fills, and the waker calls `request_repaint` if the context is set.
  6. `eframe::run_native("Aria", options, Box::new(|cc| { ...; Ok(Box::new(AriaApp::new(&cc.egui_ctx, ...))) }))`. Options: title `Aria`, inner size 1200×780, min 900×600, `persist_window: true`, `renderer: Renderer::Glow`.
  7. Log `startup stage=first_frame` from the first `show` call. Use a `first_frame_logged: bool` field.
- If `run_native` fails, log it at error, also `eprintln!` it, and return `Err`. There is no native message box in Phase 1 because no dialog crate is in the dependency set yet.
- `scripts/screenshot-window.ps1 -Title Aria -Out <png>` finds the window with `Get-Process`, reads its rect via `Add-Type` user32 `GetWindowRect`, and saves the region with `System.Drawing` `CopyFromScreen`.

- [ ] **Step 1: Implement `main.rs`**

- [ ] **Step 2: Lint**

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: no warnings.

- [ ] **Step 3: Hands-on checks on Windows** (release build: `cargo build --release --locked`, run `target/release/aria.exe`, screenshot each with the script into the scratchpad)
  1. The window opens with the sidebar, player bar and Home empty state. Take a screenshot.
  2. `Ctrl+1…9` and `Alt+Left/Right` move between pages. Take a screenshot of one pending page.
  3. Settings → Dark, then Light, then a custom accent. Take a screenshot of each. Close the app, relaunch, and confirm the theme and accent persisted. Inspect `settings.toml` content.
  4. Switch to Русский and then Հայերեն. Take screenshots and zoom in to confirm there are no tofu boxes.
  5. Close the app, write `theme = [[[` to `settings.toml`, and relaunch. Confirm defaults loaded, a recovery toast appeared and a `.bak-*` file exists.
  6. Measure: the `first_frame` ms from the log, and the idle working set after 30 s from `(Get-Process aria).WorkingSet64`, sampled 3 times. CPU after 30 s idle comes from `Get-Counter '\Process(aria)\% Processor Time'`. Record all numbers.

  If any check fails, stop and use superpowers:systematic-debugging before changing code.

- [ ] **Step 4: Commit**

`git commit -m "feat(app): startup sequence with stage timings"`

---

### Task 12: CI, cargo-deny and repository docs

**Files:**
- Create: `.github/workflows/ci.yml`, `deny.toml`
- Create: `README.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_NOTICES.md`, `CHANGELOG.md`, `docs/ROADMAP.md`, `docs/PRIVACY.md`

**Interfaces:** none.

- [ ] **Step 1: `ci.yml`**, exactly per spec §12

Triggers: `push` and `pull_request`. Jobs:
- `check`: matrix `[windows-latest, ubuntu-latest, macos-latest]` with `fail-fast: false`. Steps:
  - `actions/checkout@v4`
  - `dtolnay/rust-toolchain@master`, which reads `rust-toolchain.toml` automatically through rustup
  - `Swatinem/rust-cache@v2`
  - Linux only: `sudo apt-get update && sudo apt-get install -y` with the packages from spec §12
  - fmt, clippy, test and release-build steps with `--locked`
- `deny`: `EmbarkStudios/cargo-deny-action@v2` with `command: check advisories bans licenses sources`.

- [ ] **Step 2: `deny.toml`**

License allow-list exactly per spec §12, `[bans] multiple-versions = "warn"`, `[sources] unknown-registry = "deny"`, `unknown-git = "deny"`.

- [ ] **Step 3: Docs**

- **README:** what Aria is, the Phase 1 status, the Spotify playback limitation in a single sentence linking to ROADMAP, build/run/test commands, Linux dev packages, and the `safe.directory` note for non-NTFS/exFAT drives.
- **ROADMAP:** the six phases.
- **PRIVACY:** per spec §13, listing the exact Windows, Linux and macOS paths from spec §5.1.
- **THIRD_PARTY_NOTICES:** Inter, Noto Sans Armenian, Phosphor.
- **CHANGELOG:** an `Unreleased` section listing the Phase 1 features.
- **Licenses:** the standard MIT and Apache-2.0 texts, copyright "2026 Aria contributors".

- [ ] **Step 4: Local equivalent of CI**

Run:
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`

Expected: all succeed. If `cargo deny` is installed locally, also run `cargo deny check`; otherwise note that it is checked in CI only.

- [ ] **Step 5: Commit**

`git commit -m "ci: GitHub Actions matrix, cargo-deny; docs and licenses"`

---

### Task 13: Publish to GitHub and confirm CI

**Files:**
- Modify: `Cargo.toml` (`repository = "https://github.com/<owner>/aria"`), `README.md` (CI badge)

- [ ] **Step 1: Ask the user** to run `gh auth login` and to choose a public or private repo, and confirm the repo name `aria`. Do not proceed without an explicit answer.

- [ ] **Step 2: Create and push**

Run: `gh repo create aria --<public|private> --source . --remote origin --push`
Expected: the repo URL is printed and `main` is pushed.

- [ ] **Step 3: Watch the first CI run**

Run: `gh run watch --exit-status` (single foreground watch, in the background if it exceeds 2 min)
Expected: `check` passes on all three OSes and `deny` passes. On failure, read `gh run view --log-failed`, fix it, push, and repeat. Report failures accurately.

- [ ] **Step 4: Add the badge and repository field, then commit and push**

`git commit -m "docs: CI badge and repository URL" && git push`

- [ ] **Step 5: Write the Phase 1 report** (in chat, not a file) covering:
  - project structure and features implemented
  - test counts
  - the measurements from Task 11
  - known limitations from spec §15
  - the dependency/license summary from `cargo deny`
  - what was verified hands-on (Windows) versus by CI only (Linux, macOS)
