# Aria 0.3.2 Mini-Player Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A floating, always-on-top mini-player window whose cover fills a square and whose controls appear on hover, working for local and Spotify playback, live while the main window is minimized.

**Architecture:** A second OS window, drawn as an egui immediate viewport from `AriaApp::show`, last in the frame. The mini-player is a `MiniPlayer` state plus a `show` function in `crates/aria-app/src/ui/mini/`, fed the same `PlayerBar` data the bar and the full-screen record get, returning `MiniAction`s the app carries out. What plays (title, artists, large and small cover) is shared with the full-screen record through a function moved out of `ui/vinyl/`.

**Tech Stack:** Rust 2024, egui/eframe 0.37 (glow), egui_kittest 0.37, Fluent.

**Spec:** `docs/superpowers/specs/2026-11-23-aria-0-3-2-mini-player-design.md`

## Global Constraints

- No new dependency. No `unwrap`/`expect` outside tests, no `unsafe`, no `#[allow]`; `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` clean.
- Every visible string and accessible name is a Fluent key in `locales/en/main.ftl`, `locales/ru/main.ftl`, `locales/hy/main.ftl`.
- Nothing new over the network. Nothing new on disk except the `[mini_player]` settings (place, size, pin).
- First size 300 × 300 points; resizable between 200 × 200 and 640 × 640; always on top by default.
- The overlay fades in and out over about 0.15 s, shown while the pointer is over the window, no timer.
- Build environment: `source /c/Users/hixenpc/AppData/Local/Temp/claude/7be4b8b77034ae5d/c6ff30fe-202a-4a81-a3b1-d98bb34108a6/scratchpad/env.sh` before every cargo command (sets `CARGO_TARGET_DIR` on E:, CMake). Commit to `main` and push; no branches.
- Never open the real microphone in tests; never touch `%APPDATA%\Aria` or the installed Aria.

## Review Focus

- A track change while the main window is minimized: the mini-player's title and cover must change (texture and font uploads go with the last pass drawn). Test in Task 4: with ROOT minimized and the mini-player open, a frame draws the mini-player and not the pages.
- A saved place on a monitor that is gone: the window must open on a screen. Test in Task 3: `place_on_screen` rejects a place off every monitor.
- Dragging the window: settings must not be written every frame. Test in Task 4: a moved window is saved once, after it has been still for 1 s, and on close.
- An OS close of the mini-player (Alt+F4 while it has focus) closes the mini-player only. Test in Task 3: a close request in the mini-player's input gives `MiniAction::Close`.
- Opening and closing over and over must not keep large covers: Test in Task 4: the large Spotify cover is dropped once neither the record nor the mini-player is open.

---

### Task 1: Share what plays and the bar's controls

**Files:**
- Create: `crates/aria-app/src/ui/now_shown.rs`
- Modify: `crates/aria-app/src/ui/mod.rs`, `crates/aria-app/src/ui/vinyl/mod.rs` (`what_plays`, `Shown`), `crates/aria-app/src/ui/layout/player_bar.rs` (`center`, `transport`)

**Interfaces:**
- Produces:
  - `pub struct Shown { pub title: String, pub artist: String, pub identity: String, pub large: Option<TextureHandle>, pub small: Option<TextureHandle>, pub colours: CoverColours }` (moved from vinyl, `Default` kept).
  - `pub fn what_plays(ctx: &egui::Context, bar: &mut PlayerBar<'_>, local: Option<(TrackId, &Path)>, local_cover: &mut LocalCover, player: &mut Vec<PlayerAction>) -> Shown` in `ui/now_shown.rs`, the body of today's `VinylMode::what_plays` with `input.local` and `self.local_cover` as parameters.
  - `pub(crate) fn controls(ui: &mut Ui, column: Rect, bar: &mut PlayerBar<'_>, out: &mut Vec<PlayerAction>, sides: bool)` in `player_bar.rs`: the transport row (height `PLAY_BUTTON[0]`) at the top of `column` and the seek row (height `SEEK_ROW`) at its bottom, as `center` draws them today; `sides: false` leaves out shuffle and repeat and centres previous / play / next. `center` computes its column as now and calls `controls(.., true)`.

- [ ] **Step 1: Write the failing test** in `player_bar.rs` tests:

```rust
#[test]
fn controls_without_sides_have_no_shuffle_or_repeat() {
    // A harness drawing `controls(ui, ui.max_rect(), &mut bar, &mut out, false)` in 260 × 60.
    // assert: a button labelled "Previous", "Play" and "Next" exists;
    // assert: no button labelled "Shuffle" or "Repeat: off".
}
```
Build the `PlayerBar` with the module's existing test helper for bars.

- [ ] **Step 2: Run it to verify it fails**
Run: `cargo test -p aria-app --locked controls_without_sides`
Expected: FAIL to compile, `controls` not found.

- [ ] **Step 3: Implement** `controls` and the move of `what_plays`/`Shown` to `ui/now_shown.rs`; `VinylMode` calls `now_shown::what_plays(ctx, bar, input.local, &mut self.local_cover, &mut player)`.

- [ ] **Step 4: Run the whole aria-app suite**
Run: `cargo test -p aria-app --locked`
Expected: all pass, the vinyl tests unchanged.

- [ ] **Step 5: Commit** `refactor: share what plays and the bar's controls`

---

### Task 2: The mini-player's settings

**Files:**
- Modify: `crates/aria-core/src/settings/mod.rs`

**Interfaces:**
- Produces:
  - `pub struct MiniPlayerSettings { pub place: Option<[f32; 2]>, pub size: [f32; 2], pub on_top: bool }` with `Serialize, Deserialize, Clone, Copy, Debug, PartialEq`, `#[serde(default)]`; `Default` is `place: None, size: [300.0, 300.0], on_top: true`.
  - `pub const MINI_SIZE_MIN: f32 = 200.0; pub const MINI_SIZE_MAX: f32 = 640.0;`
  - `Settings::mini_player: MiniPlayerSettings` with `#[serde(deserialize_with = "lenient_or_default")]`, written as the `[mini_player]` table. A size outside `MINI_SIZE_MIN..=MINI_SIZE_MAX` is clamped where the other clamped settings are (the sidebar width's place); a place with a non-finite number reads as `None`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn mini_player_defaults() {
    let s = Settings::default();
    assert_eq!(s.mini_player, MiniPlayerSettings { place: None, size: [300.0, 300.0], on_top: true });
}

#[test]
fn mini_player_reads_clamps_and_falls_back() {
    let read = |text: &str| toml::from_str::<Settings>(text).unwrap().sanitized().mini_player;
    let m = read("[mini_player]\nplace = [40.0, 50.0]\nsize = [900.0, 120.0]\non_top = false");
    assert_eq!(m, MiniPlayerSettings { place: Some([40.0, 50.0]), size: [640.0, 200.0], on_top: false });
    assert_eq!(read("mini_player = 3"), MiniPlayerSettings::default());
}

#[test]
fn mini_player_round_trips() {
    // Settings with place Some([10.0, 20.0]), size [320.0, 280.0], on_top false → toml → back equal.
}
```
The clamp goes in `Settings::sanitized`.

- [ ] **Step 2: Run to verify they fail**
Run: `cargo test -p aria-core --locked mini_player`
Expected: FAIL to compile, `MiniPlayerSettings` not found.

- [ ] **Step 3: Implement** the struct, the field, the clamp.

- [ ] **Step 4: Run** `cargo test -p aria-core --locked` — Expected: all pass (the settings files written in older tests still read).

- [ ] **Step 5: Commit** `feat(core): mini-player settings`

---

### Task 3: The mini-player's drawing and state

**Files:**
- Create: `crates/aria-app/src/ui/mini/mod.rs`, `crates/aria-app/src/ui/mini/place.rs`
- Modify: `crates/aria-app/src/ui/mod.rs`, `crates/aria-app/src/ui/theme/icons.rs`, `locales/{en,ru,hy}/main.ftl`

**Interfaces:**
- Consumes: `now_shown::what_plays`, `Shown`, `player_bar::controls` (Task 1); `MiniPlayerSettings`, `MINI_SIZE_MIN/MAX` (Task 2); `vinyl::cover::LocalCover`.
- Produces:
  - `pub const MINI_ID: &str = "aria_mini_player";` and `pub fn viewport_id() -> egui::ViewportId` (`ViewportId::from_hash_of(MINI_ID)`).
  - `pub enum MiniAction { Player(PlayerAction), Close, ShowMain, SetOnTop(bool) }` (`Clone, Debug, PartialEq`).
  - `pub struct MiniInput<'a> { pub local: Option<(TrackId, &'a Path)>, pub on_top: bool }`.
  - `#[derive(Default)] pub struct MiniPlayer` with `pub fn is_open(&self) -> bool`, `pub fn open(&mut self)`, `pub fn close(&mut self)` (drops its `LocalCover`), `pub fn show(&mut self, ui: &mut Ui, bar: &mut PlayerBar<'_>, input: &MiniInput<'_>) -> Vec<MiniAction>`, `pub fn window_rect(&self) -> Option<Rect>` (the outer rect last read from the viewport's `ViewportInfo::outer_rect` inside `show`).
  - `place.rs`: `pub fn place_on_screen(place: [f32; 2], size: [f32; 2], monitors: &[Rect]) -> bool` — true when at least a 48 × 48 piece of the window at `place` lies inside one monitor rect; an empty `monitors` (unknown) gives true.
  - Icons, found in the bundled Phosphor font and added to `icons::ALL`: `PICTURE_IN_PICTURE`, `PUSH_PIN`, `PUSH_PIN_SLASH`, `ARROW_SQUARE_OUT`.
  - Strings: `mini-title` (Aria mini player / Мини-плеер Aria / Aria մինի նվագարկիչ), `mini-back` (Back to Aria / Вернуться в Aria / Վերադառնալ Aria), `mini-on-top` (Keep on top / Поверх всех окон / Միշտ վերևում), `mini-close` (Close mini player / Закрыть мини-плеер / Փակել մինի նվագարկիչը).

Drawing, as the spec says: the cover (`shown.large` or else `shown.small`) fills `ui.max_rect()` cropped to its shape and centred (UV crop on one textured rect); without a cover, `palette.surface_raised` with `icons::MUSIC_NOTES`; with nothing playing also `empty-nothing-playing` centred. The overlay's opacity is `ctx.animate_bool_with_time(id, hovered, 0.15)` where `hovered` is the pointer being inside the window; while it is above 0: a bottom gradient (transparent to black at 75 %), title (one line, ellipsis) and artists (muted) at the bottom left, `player_bar::controls(.., false)` above the bottom edge, a top gradient with `mini-back` (ARROW_SQUARE_OUT) on the left, `mini-on-top` (PUSH_PIN when on, PUSH_PIN_SLASH when off, `.selected(on_top)`) and `mini-close` (X) on the right, and a resize grip at the bottom-right corner. A drag starting on the background sends `ViewportCommand::StartDrag`; one on the grip sends `ViewportCommand::BeginResize(ResizeDirection::SouthEast)`. Space toggles play as `PlayerAction::TogglePlay` through the bar's transport rule. A viewport close request (`ui.input(|i| i.viewport().close_requested())`) gives `MiniAction::Close`. Overlay text and icons are white whatever the theme. While hovered and playing, `ctx.request_repaint_after(Duration::from_millis(250))`.

- [ ] **Step 1: Write the failing tests** — `place.rs`:

```rust
#[test]
fn a_place_on_a_monitor_is_kept() {
    let monitors = [Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0))];
    assert!(place_on_screen([100.0, 100.0], [300.0, 300.0], &monitors));
    assert!(place_on_screen([1880.0, 1040.0], [300.0, 300.0], &monitors) == false);
}

#[test]
fn a_place_on_a_gone_monitor_is_dropped() {
    let monitors = [Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0))];
    assert!(!place_on_screen([2200.0, 100.0], [300.0, 300.0], &monitors));
    assert!(!place_on_screen([-900.0, 100.0], [300.0, 300.0], &monitors));
}

#[test]
fn unknown_monitors_keep_the_place() {
    assert!(place_on_screen([5000.0, 5000.0], [300.0, 300.0], &[]));
}
```

`mod.rs` (kittest harness 300 × 300 drawing `MiniPlayer::show` with a local bar and with a Spotify bar, built like the vinyl module's test bars):

```rust
#[test] fn without_the_pointer_only_the_cover_shows()  // no button "Next" in the tree after 1 s of steps without hover
#[test] fn hovering_shows_the_controls()               // pointer moved to the centre; buttons "Previous", "Play", "Next", "Back to Aria", "Keep on top", "Close mini player" exist
#[test] fn the_controls_act_like_the_bars()            // hover, click "Next" → [MiniAction::Player(PlayerAction::Next)]; with the Spotify bar → Player(PlayerAction::Spotify(SpotifyRequest::Next)) (whatever `transport_action` gives the bar)
#[test] fn space_plays_and_pauses()                    // key Space → Player(TogglePlay) (local)
#[test] fn the_window_buttons_send_their_actions()    // click "Back to Aria" → ShowMain; "Keep on top" with on_top true → SetOnTop(false); "Close mini player" → Close
#[test] fn nothing_playing_says_so()                   // no current track → label "Nothing playing" exists
#[test] fn an_os_close_request_closes_it()             // harness input event ViewportEvent::Close on its viewport → contains MiniAction::Close
```

- [ ] **Step 2: Run to verify they fail**
Run: `cargo test -p aria-app --locked mini::`
Expected: FAIL to compile, module `mini` not found.

- [ ] **Step 3: Implement** `place.rs`, the icons, the strings and `MiniPlayer`.

- [ ] **Step 4: Run** `cargo test -p aria-app --locked` — Expected: all pass, including the icon-font test over `icons::ALL` and the locale-parity test.

- [ ] **Step 5: Commit** `feat(app): the mini-player's drawing`

---

### Task 4: The mini-player in the app

**Files:**
- Modify: `crates/aria-app/src/app.rs`, `crates/aria-app/src/ui/layout/player_bar.rs`, `locales/{en,ru,hy}/main.ftl`

**Interfaces:**
- Consumes: Task 3's `MiniPlayer`, `MiniAction`, `MiniInput`, `viewport_id`, `place_on_screen`; Task 2's settings.
- Produces:
  - `PlayerAction::ToggleMiniPlayer`; `PlayerBar` gains `pub mini_open: bool`. The bar draws `IconButton::new(icons::PICTURE_IN_PICTURE, &tr("player-mini-player"), p).selected(bar.mini_open)` right after the full-screen button in `extras`. String `player-mini-player` (Mini player / Мини-плеер / Մինի նվագարկիչ).
  - `AriaApp` gains `mini: MiniPlayer`, `monitors: Vec<Rect>`, `mini_moved: Option<(Rect, f64)>` (rect, time it last changed), and for tests `pub fn mini_open(&self) -> bool` and `pub fn main_window_asked(&self) -> u32` (count of "show the main window" requests).
  - `fn show_mini(&mut self, ctx: &egui::Context, palette: &Palette)`: builds the bar data as `show_vinyl` does, and `ctx.show_viewport_immediate(viewport_id(), builder, |ui, _| self.mini.show(ui, &mut bar, &input))`. The builder is made once on opening and kept unchanged while open: title `mini-title`, `with_decorations(false)`, `with_resizable(true)`, `with_inner_size(settings.size)`, min/max `MINI_SIZE_MIN/MAX`, `with_taskbar(false)`, `with_window_level(AlwaysOnTop)` when `on_top`, and `with_position(place)` only when `place_on_screen(place, size, &self.monitors)`.

Behaviour to wire:
- `ToggleMiniPlayer` opens or closes it; opening closes the full-screen record (`vinyl.close`), and `vinyl.toggle`/F11 opening the record closes the mini-player.
- `MiniAction::Player(a)` → `handle_player_action(ctx, a)`; `Close` → close (and save, below); `ShowMain` → `ctx.send_viewport_cmd_to(ViewportId::ROOT, …)` with `Visible(true)`, `Minimized(false)`, `Focus`, and count it; `SetOnTop(on)` → settings `on_top`, and `ctx.send_viewport_cmd_to(viewport_id(), ViewportCommand::WindowLevel(..))`.
- Place and size: each frame while open, when `mini.window_rect()` differs from the last one, set `mini_moved = Some((rect, now))`; when `now - changed >= 1.0` s, write `place`/`size` to settings with `update_settings` and clear `mini_moved`; on close, write any pending one.
- `tell_window_shown`: shown is `!minimized || self.mini.is_open()`.
- The large cover: `covers.clear_large()` only when neither `vinyl` nor `mini` is open.
- Root minimized with the mini-player open: `show` runs events, shortcuts that are not page shortcuts, hotkeys and toasts' timers, draws no panels, and draws the mini-player last; with the root shown, the mini-player is drawn after everything else in `show`.
- Monitors: `eframe::App::ui` fills `self.monitors` from `frame.winit_window()`'s `available_monitors()` (each monitor's position and size divided by its `scale_factor()`) on the frame after `mini.open()` (a `monitors_wanted` flag), before `show`.

- [ ] **Step 1: Write the failing tests** in `app.rs` tests, a `mod mini` beside `mod vinyl` using its harness helpers:

```rust
#[test] fn the_bar_button_opens_and_closes_the_mini_player()   // click "Mini player" → mini_open(); again → !mini_open()
#[test] fn the_record_and_the_mini_player_are_not_open_together() // open mini, F11 → vinyl_open && !mini_open; open mini again from... (F11 closes the record first, then the bar button) → mini_open && !vinyl_open
#[test] fn back_to_aria_asks_for_the_main_window()             // open mini, hover it, click "Back to Aria" → main_window_asked() == 1, mini still open
#[test] fn spotify_stays_shown_while_the_mini_player_is_open() // open mini, ROOT minimized → no WindowShown(false) sent; close mini → WindowShown(false) sent
#[test] fn the_minimized_main_window_draws_only_the_mini_player() // open mini, ROOT minimized, step → no "Settings" nav label in the tree; "Mini player"'s cover area / a mini button reachable after hover
#[test] fn a_moved_mini_player_is_saved_once_it_is_still()     // drive `mini_moved` through a test seam: report a new rect, step 0.5 s → settings place unchanged; step past 1 s → settings place == new rect; only one settings write
#[test] fn closing_saves_a_pending_place()                     // report a new rect, close at once → settings place == new rect
#[test] fn the_large_cover_stays_while_the_mini_player_is_open() // open mini, insert a large Spotify cover (as in vinyl's test) → has_large(); close → !has_large()
```
For the moved-window tests, add `#[cfg(test)] pub fn report_mini_rect(&mut self, rect: Rect)` that sets what `mini.window_rect()` returns, since kittest's embedded viewport has no OS window.

- [ ] **Step 2: Run to verify they fail**
Run: `cargo test -p aria-app --locked app::tests::mini`
Expected: FAIL to compile, `ToggleMiniPlayer` / `mini_open` not found.

- [ ] **Step 3: Implement** the wiring above.

- [ ] **Step 4: Run** `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo fmt --check` — Expected: all pass and clean.

- [ ] **Step 5: Commit** `feat(app): the floating mini-player`

---

### Task 5: Version, docs and the hand check

**Files:**
- Modify: `Cargo.toml` (workspace version 0.3.2), `Cargo.lock`, `packaging/windows/aria.iss` (example version 0.3.2), `CHANGELOG.md`, `README.md`
- Create: `docs/testing/0.3.2-manual-check.md`

- [ ] **Step 1:** Version 0.3.2 in `Cargo.toml` and `aria.iss`'s comments; `cargo update -w --offline` to refresh `Cargo.lock`.
- [ ] **Step 2:** CHANGELOG `## [0.3.2] - <date> (the mini-player)`: Added (the mini-player as the spec says, the `[mini_player]` settings), Changed (Spotify polled while the mini-player is open with the main window minimized; the large cover kept while either view shows it), Limits (Wayland may freeze it while the main window is minimized and may ignore its place and "on top"; screen readers see only its title), and the real test count from the last run. README: a "Mini player" bullet beside "Full-screen record", with the Wayland and screen-reader limits in a line.
- [ ] **Step 3:** `docs/testing/0.3.2-manual-check.md` in the 0.3.1 file's form, every line `pending`: opening and closing (bar button, ×, Alt+F4 on it, closing the main window quits), on top and the pin, drag and resize within 200–640, place and size restored after closing and after restarting, a place on an unplugged monitor, "Back to Aria" from minimized, the cover sharp at 300 and 640 px, hover overlay and its fade, controls for local and Spotify, a track change with the main window minimized (title and cover change), Spotify paused on the phone with the main window minimized, CPU near zero when paused and not hovered, macOS and Linux X11/Wayland lines.
- [ ] **Step 4:** Run `cargo test --workspace --locked` — Expected: all pass; put the count in CHANGELOG and the manual check's Automated line.
- [ ] **Step 5: Commit** `docs: 0.3.2 mini-player` and push.
