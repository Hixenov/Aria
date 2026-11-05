# Aria — Phase 1 (Foundation) Design

- **Date:** 2026-11-05
- **Status:** Approved in conversation; pending written-spec review
- **Scope:** Phase 1 of 6. Each later phase gets its own spec → plan → implementation cycle.

## 1. Context and intent

Aria is a native, lightweight, cross-platform desktop music player written in Rust
(egui/eframe). Over six phases it will combine a local music player (Symphonia + CPAL),
Spotify Web API library access and Spotify Connect remote control (OAuth PKCE), LRCLIB
lyrics, an equalizer and visualizer for local audio, system tray and media keys, and a
mini-player. It takes Spotifast's usability as a reference but has its own original code
and visual identity.

Phase 1 delivers the foundation every later phase builds on: a working native window with
the final layout shell, navigation, a theme system, persistent settings, localization
(EN/RU/HY), logging, and CI.

### Product constraint recorded now

Spotify audio is **not** streamed or decoded inside Aria. Spotify integration (Phase 3)
means library, search and playlist access through the Web API, plus remote control of the
user's existing Spotify Connect devices. No compliant in-process streaming path exists for
a native desktop Rust app: librespot-style streaming uses private protocols and conflicts
with Spotify's developer terms. Local files get the full audio engine (Phase 2). This
limitation is stated up front in `docs/ROADMAP.md` and the README.

## 2. Scope

### In Phase 1

- A launchable Aria window with the final layout shell:
  - left sidebar: Home, Search, Your Library, Liked Songs, Playlists, Albums, Artists,
    Local Music, Settings
  - main content area
  - bottom player bar (static, disabled controls with no fake playback)
  - collapsible right panel
- Every sidebar entry is a real route. Pages whose features belong to later phases show a
  clear localized empty state ("Available in Phase N"). Nothing pretends to work.
- A functional **Settings** page for Phase-1-owned settings: language, theme (dark /
  light / system), separate accent colors for dark and light, minimize-to-tray preference
  (stored now, enforced in Phase 4), log level, reset settings, and buttons that open the
  config and log folders.
- Theme system, settings persistence, i18n, file logging, panic logging, CI, and
  repository documentation.

### Not in Phase 1

Audio, local library, SQLite, Spotify, lyrics, tray, media keys, notifications, mini-player,
installers. Each is covered by a later phase.

## 3. Architecture

### 3.1 Workspace layout

```
aria/
  Cargo.toml                 workspace: [workspace.dependencies], [workspace.lints], release profile
  rust-toolchain.toml        channel = "1.99.1", components = ["clippy", "rustfmt"]
  Cargo.lock                 committed
  deny.toml                  cargo-deny config
  crates/
    aria-core/               no GUI dependencies
      src/lib.rs
      src/paths.rs           config / cache / data / log directories
      src/settings/mod.rs    Settings struct, defaults, schema version
      src/settings/store.rs  load, atomic save, corrupt-file recovery, migration
      src/i18n/mod.rs        Localizer, tr / tr_args, fallback
      src/i18n/locale.rs     OS locale detection → supported Language
      src/command.rs         Command enum (UI → backend)
      src/event.rs           Event enum (backend → UI)
      src/runtime.rs         Backend: Tokio runtime on its own thread, command routing
      src/redact.rs          Redacted<T>
      src/logging.rs         tracing setup, rolling files, panic hook
    aria-app/                the binary (UI only)
      src/main.rs            startup: paths → logging → settings → backend → eframe
      src/app.rs             AriaApp: eframe::App impl, top-level update
      src/nav.rs             Route enum + back/forward history
      src/ui/layout/         sidebar.rs, player_bar.rs, right_panel.rs
      src/ui/pages/          home.rs, search.rs, library.rs, liked.rs, playlists.rs,
                             albums.rs, artists.rs, local_music.rs, settings.rs
      src/ui/theme/          palette.rs (tokens), visuals.rs (tokens → egui), fonts.rs, icons.rs
      src/ui/widgets/        nav_item.rs, section_header.rs, empty_state.rs, toast.rs
  locales/en/main.ftl
  locales/ru/main.ftl
  locales/hy/main.ftl
  assets/fonts/              Inter, Noto Sans Armenian, Phosphor icon font (+ license files)
  docs/                      ROADMAP.md, PRIVACY.md (stub stating Phase 1 sends nothing), specs/
  scripts/
  .github/workflows/ci.yml
```

Later phases add `aria-audio`, `aria-library`, `aria-spotify` and `aria-platform` crates.
UI code never depends on networking, audio or persistence internals directly. It talks to
them only through `aria-core`'s `Command`/`Event` types.

### 3.2 Runtime model

1. `main.rs` resolves paths, initializes logging, loads settings, starts the `Backend`, then
   runs eframe.
2. `Backend::start(ctx_waker)` spawns a dedicated thread that owns a multi-thread Tokio
   runtime. It returns a `BackendHandle` containing a `crossbeam_channel::Sender<Command>`
   and a `Receiver<Event>`.
3. The UI thread, once per frame, drains pending `Event`s with `try_recv` (never blocking),
   updates its view state, and renders.
4. The backend sends `Event`s and calls a repaint waker (wrapping
   `egui::Context::request_repaint`) so the UI wakes only when there is something new.
5. Shutdown: on window close the UI sends `Command::Shutdown`. The backend flushes any
   pending settings save and exits, and the UI joins the thread with a 2-second timeout.

Phase 1 backend responsibilities:

- `Command::UpdateSettings(Settings)` → debounced (500 ms) atomic save →
  `Event::SettingsSaved` or `Event::Notice { level: Error, key }`.
- `Command::ResetSettings` → write defaults → `Event::SettingsReplaced(Settings)`.
- `Command::Shutdown` → flush and exit.

The UI applies settings changes to its own copy immediately so changes look instant. The
backend is responsible only for durability.

### 3.3 Lints and build profile

Workspace `[lints]`:

- `rust.unsafe_code = "forbid"`
- `clippy.unwrap_used = "deny"` and `clippy.expect_used = "deny"` (allowed in `#[cfg(test)]`)
- `clippy.all = "warn"` with CI running `-D warnings`

Release profile: `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`, `panic = "unwind"`
(so the panic hook can log).

## 4. Theming

### 4.1 Palette tokens

```rust
pub struct Palette {
    pub bg: Color32,             // window background
    pub surface: Color32,        // sidebar, player bar
    pub surface_raised: Color32, // hover rows, popovers
    pub border: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub focus: Color32,
    pub danger: Color32,
}
```

- There are two base palettes, `Palette::dark()` and `Palette::light()`, plus an optional
  per-mode accent override (`accent_dark`, `accent_light` in settings). `accent_hover` is
  derived from the accent: lighter in dark mode, darker in light mode.
- The default accent is an original teal-cyan (`#2EC4B6` dark / `#13897F` light). Exact
  values are validated for WCAG AA contrast against `bg` for text-on-accent.
- `visuals.rs` turns a `Palette` into `egui::Visuals` and `egui::Style` in one function:
  spacing scale (4/8/12/16/24), corner radius (6 for controls, 8 for panels), widget
  hover/active/inactive states, selection color, text styles (Heading 20, Body 14,
  Small 12, Button 14, Monospace 13).
- Widgets read colors only from the active `Palette`. No color literals appear outside
  `palette.rs`.

### 4.2 Theme preference

`Theme::{Dark, Light, System}` maps to `egui::ThemePreference`. **System** uses egui's
built-in OS theme tracking, so no extra crate is needed. Visuals for both themes are
installed via `ctx.set_visuals_of(egui::Theme::Dark, …)` and
`ctx.set_visuals_of(egui::Theme::Light, …)`, and they are re-applied when an accent changes.

### 4.3 Fonts and icons

- **Inter** (OFL-1.1): primary proportional font, covering Latin and Cyrillic.
- **Noto Sans Armenian** (OFL-1.1): fallback in the same family chain for Armenian script.
- **Phosphor** icon font (MIT), regular weight: added as a further fallback. `icons.rs`
  defines `pub const` codepoints for icons in use. No wrapper crate is used, because
  `egui-phosphor` 0.14 does not track egui 0.37.
- Fonts are embedded with `include_bytes!`. License texts ship in `assets/fonts/` and are
  listed in `THIRD_PARTY_NOTICES.md`.

## 5. Settings

### 5.1 Location

Directories come from `directories::ProjectDirs::from("", "", "Aria")`:

- Windows: `%APPDATA%\Aria\config\settings.toml`, logs in `%APPDATA%\Aria\data\logs\` (corrected: `data_dir()` is Roaming on Windows; the only Local Aria folder is the cache, `%LOCALAPPDATA%\Aria\cache`, which Phase 1 creates and leaves empty)
- Linux: `$XDG_CONFIG_HOME/aria/settings.toml`, logs in `$XDG_DATA_HOME/aria/logs/`
- macOS: `~/Library/Application Support/Aria/settings.toml`, logs alongside

`ARIA_CONFIG_DIR` overrides the config directory. It is used by tests and for portable mode
later.

### 5.2 Schema

```rust
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,          // 1
    pub language: Option<Language>,   // None = detect from OS at startup
    pub theme: Theme,                 // System
    pub accent_dark: Option<Rgb>,     // None = palette default
    pub accent_light: Option<Rgb>,
    pub minimize_to_tray: bool,       // false; enforced in Phase 4
    pub log_level: LogLevel,          // Info
    pub right_panel_open: bool,       // false
    pub sidebar_width: f32,           // 220.0, clamped 180..=320
}
```

`Rgb` serializes as a `"#RRGGBB"` string. `Language` is one of `En`, `Ru`, `Hy` and
serializes as `"en"`, `"ru"`, `"hy"`.

### 5.3 Load

1. If the file is missing, use defaults (with no write until the first change).
2. If it parses, `#[serde(default)]` fills any missing fields. If `schema_version` is lower
   than current, migrations run in order. Out-of-range values (such as `sidebar_width`)
   are clamped.
3. If it fails to parse, or `schema_version` is newer than supported, rename the file to
   `settings.toml.bak-<unix_ts>`, use defaults, and emit
   `Notice { Warning, "notice-settings-recovered" }`.
4. If the config dir can't be created, run with in-memory settings and emit
   `Notice { Warning, "notice-settings-readonly" }`.

### 5.4 Save

A save is atomic: serialize to `settings.toml.tmp` in the same directory, `sync_all`, then
rename over `settings.toml`. On Windows, rename-over-existing is done with
`std::fs::rename`, which replaces the target. Saves are debounced by 500 ms on the
backend, and a pending save is flushed on shutdown.

### 5.5 Window state

Window size, position and maximized state go through eframe's `persistence` feature
(`persist_window = true`), stored by eframe in its own app storage, kept separate from
`settings.toml`.

## 6. Localization

- Uses `fluent-bundle` 0.16 + `unic-langid` 0.9. One `main.ftl` per locale is embedded via
  `include_str!`.
- `Localizer` holds bundles for the active locale and English. `tr(key)` and
  `tr_args(key, args)` return a `Cow<str>`.
- Missing key in the active locale → English. Missing in English → the key itself, and a
  `warn!` is logged once per key.
- Language changes apply live: the next frame renders with the new bundle.
- First-launch language comes from the OS locale (`sys-locale` crate; latest version
  pinned via `Cargo.lock` at implementation). `ru*` maps to Russian, `hy*` to Armenian, everything else to
  English.
- Key naming: kebab-case and area-prefixed (`nav-home`, `settings-theme-dark`,
  `empty-phase-pending`, `notice-settings-recovered`).
- Translation quality: Russian strings are written to production quality. Armenian strings
  are a best effort and are listed as a known limitation needing native-speaker review
  before 1.0.

## 7. Logging and diagnostics

- `tracing` + `tracing-subscriber` (env-filter, fmt) + `tracing-appender`, with daily
  rolling files `aria.YYYY-MM-DD.log` in the log dir (corrected: the appender is built
  with prefix `aria` and suffix `log`, which names files `aria.<date>.log`; the date is
  UTC). The last 7 files are kept, and older ones are deleted at startup and on rollover.
- Debug builds also log to stderr.
- Level: `ARIA_LOG` env var if set, otherwise `settings.log_level`. A level change in
  Settings applies live through a `reload` handle.
- `Redacted<T>`: `Debug`/`Display` print `***`, with access only via `.expose()`. Phase 3
  tokens must be wrapped in it.
- Panic hook: logs thread name, message and location at `error!`, flushes the
  non-blocking writer guard, then calls the default hook.
- Startup logs the version, OS and arch, renderer, and timing for each startup stage,
  which serves as the baseline for cold-start measurement.

## 8. UI behavior

- **Sidebar:** fixed nav list with icon + label. The active route is highlighted with the
  accent. Width is resizable (180–320) and persisted. Keyboard: `Ctrl+1…9` jumps to
  entries, `Alt+Left` / `Alt+Right` go back and forward, and mouse back/forward buttons are
  supported.
- **Navigation:** `Route` enum with `History { back: Vec<Route>, current, forward:
  Vec<Route> }`. The back stack is capped at 50. Navigating to the current route is a no-op.
- **Player bar:** artwork placeholder, "Nothing playing" text, and transport, volume,
  device, queue and lyrics controls rendered **disabled** with a tooltip saying when they
  arrive.
- **Right panel:** toggled from the player bar's queue button and `Ctrl+Alt+R`. It shows
  an empty state in Phase 1. Its open state is persisted.
- **Pages:** each page renders a heading and either real content (Settings) or the shared
  `empty_state` widget with icon, title, and "Available in Phase N" text.
- **Settings page:** sections are General (language, theme, accents, minimize-to-tray),
  Advanced (log level, open config folder, open log folder, reset settings with a
  confirmation dialog), and About (version, license, third-party notices). The folders
  open in the OS file manager through the `open` crate. Only paths Aria itself computed
  are passed, never user-supplied strings.
- **Toasts:** `Event::Notice` shows a bottom-right toast that auto-dismisses after 6
  seconds, or can be dismissed with a click. Errors stay until dismissed.
- **Repaint:** no continuous repaint. Frames are driven by input, backend wakes and toast
  timers (`request_repaint_after`).

## 9. Error handling

- `aria-core` defines `thiserror` enums: `PathError`, `SettingsError` (Io, Parse, Serialize,
  UnsupportedVersion) and `I18nError` (Parse).
- `aria-app/main.rs` uses `anyhow::Result` only for fatal startup failures, such as eframe
  failing to create a window. Those are logged and reported through a native message
  box if possible, otherwise via stderr.
- No `unwrap()` / `expect()` in production paths (enforced by clippy).
- User-facing errors carry Fluent keys, and their details go to the log.

## 10. Renderer and performance

- eframe 0.37 with `default-features = false` and features `glow`, `persistence`,
  `accesskit`, `clipboard`, `links`, `default_fonts`, `wayland`, `x11`. This drops `wgpu`
  and `system_fonts`. `default_fonts` stays on so Hack covers monospace and emoji have a
  fallback. Inter, Noto Sans Armenian and Phosphor are inserted ahead of the defaults in
  the proportional family.
- Targets for Phase 1, measured on this Windows machine and reported honestly:
  - cold start to first frame under 1 s
  - idle RAM under 120 MB
  - idle CPU about 0% with no continuous repaint
- Measurements come from startup-stage log timings plus Task Manager / `Get-Process`
  working-set samples.

## 11. Testing

### aria-core

- settings: defaults round-trip; partial file fills defaults; old schema migrates; corrupt
  file is backed up and replaced by defaults; newer schema is treated as corrupt;
  out-of-range values clamp; atomic save leaves no `.tmp` behind and the original is intact
  if serialization fails; the debounce coalesces 10 rapid updates into 1 write; shutdown
  flushes a pending save.
- i18n: every `.ftl` parses; key-set parity (`ru` ⊇ `en`, `hy` ⊇ `en`); fallback to `en`;
  missing-everywhere returns the key; locale mapping (`ru-RU`, `hy-AM`, `en-US`, `fr-FR`,
  empty).
- redact: `format!("{:?}")` and `format!("{}")` never contain the inner value.
- paths: `ARIA_CONFIG_DIR` override respected.
- Tests that use the filesystem use `tempfile` directories and never touch real user
  config.

### aria-app

- nav: push, back, forward, cap at 50, no-op on same route, forward cleared on push.
- theme: dark and light palettes map to visuals with the expected fill/text colors; accent
  override applied; generated text-on-accent pairs meet WCAG AA contrast.
- fonts: every char in `locales/*/main.ftl` resolves to a glyph in the bundled fonts (via a
  `ttf-parser` cmap lookup over the embedded font bytes; dev-dependency only).
- UI (`egui_kittest` 0.37, headless, AccessKit queries):
  - clicking each sidebar entry changes the page heading
  - switching theme in Settings changes `ctx.theme()`
  - switching language relabels the sidebar
  - the reset confirmation dialog cancels and confirms correctly

## 12. CI and reproducibility

`.github/workflows/ci.yml`, on push and PR:

- `check` job on a matrix of `windows-latest`, `ubuntu-latest`, `macos-latest`, with these
  steps:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - `cargo test --workspace --locked`
  - `cargo build --release --locked`
- Linux installs `libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev
  libxcb-xfixes0-dev libgl1-mesa-dev`.
- `deny` job: `cargo-deny` 0.20 checks advisories, bans and licenses. The license
  allow-list is MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2/3-Clause, ISC,
  Zlib, Unicode-3.0, OFL-1.1 and MPL-2.0 (crates only; reviewed if hit).
- Caching uses `Swatinem/rust-cache`, with actions pinned to major versions.
- `rust-toolchain.toml` pins `1.99.1`, `Cargo.lock` is committed, and every command uses
  `--locked`.

## 13. Repository and documentation

- `README.md`: what Aria is, current status (Phase 1), build/run/test commands, platform
  notes, Linux dev package list.
- `LICENSE-MIT`, `LICENSE-APACHE` (dual license, `MIT OR Apache-2.0`).
- `THIRD_PARTY_NOTICES.md`: fonts and their licenses.
- `CHANGELOG.md` (Keep a Changelog format) with an `Unreleased` section.
- `docs/ROADMAP.md`: the six phases, including the Spotify playback limitation.
- `docs/PRIVACY.md`: states that Phase 1 sends no data over the network and lists
  everything it stores, with exact paths: `settings.toml` with its transient `.tmp` and its
  `.bak-<secs>` backups, eframe window state, logs, `panic.log` and the (empty) cache
  folder (corrected: an earlier version said "only" settings, window state and logs). The
  document is extended each phase.
- `.gitignore`, `.editorconfig`, `rustfmt.toml` (edition 2024 defaults).
- GitHub repository: created after `gh auth login`. Public or private is confirmed with the
  user before creation.

## 14. Definition of done (Phase 1)

1. `cargo run -p aria-app` opens the Aria window on this Windows 11 machine. Verified with
   screenshots.
2. All nine routes reachable by mouse and `Ctrl+1…9`. Back and forward work via `Alt+Arrows`
   and mouse buttons.
3. Dark, light and system themes plus custom accents apply live and persist across a
   restart.
4. EN/RU/HY switch live, and Armenian and Russian render with no missing glyphs.
5. A deliberately corrupted `settings.toml` is backed up, defaults load, and a toast
   appears.
6. `cargo fmt --check`, `clippy -D warnings` and `cargo test` pass locally.
7. CI is green on Windows, Linux and macOS once the GitHub repo exists.
8. Startup time and idle RAM are measured and recorded in the Phase 1 report.

Hands-on verification is Windows only. Linux and macOS are verified by CI build and tests,
and the Phase 1 report will say so.

## 15. Known limitations (Phase 1)

- The Armenian translation needs native-speaker review.
- Linux and macOS are not manually tested in Phase 1.
- The player bar, right panel and most pages are intentionally empty states until their
  phase lands.
- Spotify in-app audio streaming is out of scope for the whole project (see §1).
