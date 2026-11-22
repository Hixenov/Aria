# Aria 0.3.1 Full-Screen Record Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A full-screen "now playing" mode, opened from the player bar's bottom-right corner,
showing the playing track as a record turning on a turntable, with five record finishes.

**Architecture:** Pure pieces first, each with unit tests: the turntable's motion
(`ui/vinyl/motion.rs`), the layout (`ui/vinyl/layout.rs`) and the finish renderer
(`ui/vinyl/finish.rs`, drawing into an `image::RgbaImage` off the UI thread). Large covers come
from a local loader in the app (reading the file's embedded picture) and from a new Spotify
request decoded at 1024 px. The mode (`ui/vinyl/mod.rs`) draws the textures as rotated quads
and reuses the player bar's own transport, seek and volume code, so it behaves the same for
local and Spotify playback. The app owns the mode's state, the OS full-screen command and the
keys.

**Tech Stack:** Rust 2024, egui/eframe 0.37 (glow), `image` 0.25, `egui_kittest`.

**Spec:** `docs/superpowers/specs/2026-11-22-aria-0-3-1-vinyl-fullscreen-design.md`

## Global Constraints

- No `unwrap`/`expect` outside tests, no `unsafe`, no `#[allow]`; `cargo clippy --workspace
  --all-targets --locked -- -D warnings` and `cargo fmt --all --check` clean.
- No new dependency (`image` 0.25 and `egui` already do everything here).
- Every new visible string is a Fluent key in `locales/en`, `ru`, `hy/main.ftl` and in `KEYS`
  (`crates/aria-core/src/i18n/mod.rs`); every new control has an accessible name.
- Nothing new is sent over the network except Spotify's larger image for the Spotify item;
  nothing new is written to disk except `vinyl_finish` in `settings.toml`.
- Nothing of MD Vinyl is copied: no text, label printing or artwork.
- Shell: Git Bash, `cd /e/claude/aria; export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory
  GIT_CONFIG_VALUE_0=E:/claude/aria CARGO_TARGET_DIR=C:/Users/hixenpc/aria-target`, `-j 4`.
  Build on C: (E: is exFAT and ~20× slower). Commit and push straight to `main`.

## Review Focus

1. **The window stuck in OS full screen.** Closing the mode by any path (Esc, button, F11, the
   track list emptying, the app closing) must leave the window as it was before. Test in Task 6
   (`closing_restores_a_windowed_window`, `a_window_already_full_screen_stays_so`).
2. **Repainting when nothing moves.** With the mode closed, or open but paused and settled, no
   repaint is requested by the mode. Test in Task 1 (`at_rest_nothing_is_busy`) and Task 6
   (`a_paused_record_asks_for_no_repaint`).
3. **A cover that is huge, broken, or missing.** A 6000×6000 or corrupt embedded picture must
   not stall the UI or crash; the label falls back. Test in Task 4 (`a_broken_picture_gives_none`,
   `a_large_picture_is_scaled_to_1024`).
4. **Tracks changing fast.** Finishes and large covers that arrive for a track no longer
   current are dropped. Test in Task 5 (`a_stale_render_is_dropped`) and Task 4
   (`a_stale_cover_is_dropped`).
5. **Keys in the mode.** Esc closes the finish panel before the mode; page shortcuts (Ctrl+1…0,
   Alt+arrows) do nothing in the mode. Test in Task 6 (`esc_closes_the_panel_then_the_mode`,
   `page_shortcuts_are_off_in_the_mode`).

---

### Task 1: The turntable's motion

**Files:**
- Create: `crates/aria-app/src/ui/vinyl/mod.rs` (module root, `pub mod motion;` for now),
  `crates/aria-app/src/ui/vinyl/motion.rs`
- Modify: `crates/aria-app/src/ui/mod.rs` (`pub mod vinyl;`)

**Interfaces:**
- Produces: `pub const RPM: f32 = 100.0 / 3.0;` `pub struct Turntable { pub angle: f32, pub speed:
  f32, pub arm: f32 }` (`angle` in radians mod τ; `speed` 0..=1 of full speed; `arm` 0 = rest,
  1 = on the record), `Default` = all 0; `fn step(&mut self, dt: f32, playing: bool)`;
  `fn busy(&self, playing: bool) -> bool` (true while turning or while speed/arm are moving).
  Spin-up 0.6 s, spin-down 0.8 s (smooth, e.g. smoothstep on speed), arm 0.5 s each way.

- [ ] **Step 1: Write the failing tests** in `motion.rs`: `full_speed_is_33_and_a_third_rpm`
  (after 1 s of spin-up then 6 s at speed, the angle advanced over the last 6 s is
  `6.0 * RPM / 60.0 * TAU` ± 1e-3, mod τ handled); `play_speeds_up_over_0_6_s` (speed < 1 at
  0.3 s, == 1 at 0.6 s); `pause_slows_down_without_a_jump` (no step changes the angle by more
  than `RPM / 60 * TAU * dt * 1.001`; speed 0 after 0.8 s); `the_arm_moves_on_and_off`
  (arm 1 after 0.5 s playing, 0 after 0.5 s paused); `at_rest_nothing_is_busy`
  (`Turntable::default().busy(false) == false`; `busy(true)` true).
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-app --lib vinyl::motion` → FAIL.
- [ ] **Step 3: Implement** `motion.rs`.
- [ ] **Step 4: Run them.** → 5 passed.
- [ ] **Step 5: Commit** `feat(app): the record's motion for the full-screen mode`.

### Task 2: The finish setting

**Files:**
- Modify: `crates/aria-core/src/settings/mod.rs`

**Interfaces:**
- Produces: `pub enum VinylFinish { #[default] Black, Tinted, Splatter, Marble, Random }`
  (serde `lowercase`, `Clone, Copy, Debug, PartialEq, Eq, Default`), `pub const ALL: [VinylFinish; 5]`
  in the order Random, Black, Tinted, Splatter, Marble (the panel's order);
  `Settings.vinyl_finish: VinylFinish` with `#[serde(deserialize_with = "lenient_or_default")]`.

- [ ] **Step 1: Write the failing test** `vinyl_finish_reads_and_falls_back`:
  `toml::from_str("vinyl_finish = \"marble\"")` → `Marble`; `"vinyl_finish = 3"` → `Black`;
  missing → `Black`; a round trip through `toml::to_string` keeps `Splatter`.
- [ ] **Step 2: Run it.** `cargo test -j 4 -p aria-core --lib settings` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run it**, and the whole `aria-core` suite → pass.
- [ ] **Step 5: Commit** `feat(core): the record finish setting`.

### Task 3: Rendering the finishes

**Files:**
- Create: `crates/aria-app/src/ui/vinyl/finish.rs`

**Interfaces:**
- Consumes: `VinylFinish` (Task 2).
- Produces:
  - `pub enum Finish { Black, Tinted, Splatter, Marble }` (what is drawn; `Random` resolves to
    one of these).
  - `pub struct CoverColours { pub main: [u8; 3], pub accents: [[u8; 3]; 2], pub neutral: bool }`
    and `pub fn cover_colours(rgba: &[u8], width: u32, height: u32) -> CoverColours`: from a
    downscale to 32×32, the most common colour among pixels with HSV saturation ≥ 0.25 and value
    in 0.15..=0.95 (main), the next two distinct ones (accents); `neutral = true` and greys when
    fewer than 5 % of pixels qualify. `CoverColours::neutral()` for no cover.
  - `pub fn resolve(finish: VinylFinish, identity: &str) -> Finish`: `Random` → FNV-1a 64-bit of
    `identity` modulo 4 over [Black, Tinted, Splatter, Marble]; others map directly.
  - `pub fn render(finish: Finish, colours: &CoverColours, size: u32) -> image::RgbaImage`:
    a disc of diameter `size`, transparent outside it and inside the label radius
    (`LABEL_RATIO = 0.36` of the diameter, where the cover is drawn separately); concentric
    grooves with a few wider gaps, fine grain; Tinted and Splatter partly transparent (alpha
    about 0.75 in the grooves); Splatter's streaks and Marble's swirls from a fixed-seed
    generator (a small xorshift; no `rand` dependency) so the output is deterministic.
  - `pub const LABEL_RATIO: f32 = 0.36;` `pub const TEXTURE_SIZE: u32 = 1024;`

- [ ] **Step 1: Write the failing tests:** `each_finish_is_a_disc_of_the_size` (for all four:
  1024×1024, pixel (0,0) alpha 0, the centre pixel alpha 0, a pixel at 0.45 of the diameter
  from the centre alpha > 0); `rendering_is_deterministic` (two renders equal byte for byte);
  `tinted_is_see_through` (mean alpha in the groove area < 230, Black's == 255);
  `a_grey_cover_is_neutral` (a 64×64 mid-grey image → `neutral == true`); `a_red_cover_gives_red`
  (64×64 of (200, 30, 30) → `main` within 20 of it per channel); `random_is_stable_per_track`
  (`resolve(Random, "spotify:track:x")` equal twice; over 40 distinct identities at least 3 of the
  4 finishes appear; non-Random passes through).
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-app --lib vinyl::finish` → FAIL.
- [ ] **Step 3: Implement** `finish.rs` (render in under ~150 ms in release for 1024 px; it runs on
  a worker thread, Task 6).
- [ ] **Step 4: Run them.** → 6 passed.
- [ ] **Step 5: Commit** `feat(app): record finishes rendered into textures`.

### Task 4: Large covers

**Files:**
- Modify: `crates/aria-library/src/artwork.rs` (`pub fn large_cover`),
  `crates/aria-library/src/lib.rs` (re-export), `crates/aria-spotify/src/model.rs`
  (`image_large`), `crates/aria-spotify/src/client.rs` tests, `crates/aria-core/src/spotify_service.rs`
  (`LargeCover` request), `crates/aria-app/src/spotify.rs` (`RemoteCovers` keeps large ones apart)
- Create: `crates/aria-app/src/ui/vinyl/cover.rs` (the local loader)

**Interfaces:**
- Produces:
  - `aria_library::large_cover(path: &Path, max: u32) -> Option<image::RgbaImage>`: the file's
    embedded picture (`read_tags(path)?.picture`), decoded with `image` limits (4096 px a side),
    scaled down to fit `max` (never up). `None` for no picture, a broken one, or a missing file.
  - `SpTrack.image_large: Option<String>`: the widest `https://` image (`pick_large(&[Image])`
    beside `pick_image`).
  - `SpotifyRequest::LargeCover(String)`: fetched like `Cover`, decoded to fit `LARGE_COVER =
    1024` (const in `spotify_service.rs`), sent as `SpotifyEvent::Cover` with that URL. The
    covers queue carries `(url, max)`.
  - In the app: `RemoteCovers::get_large(&mut self, url, out) -> Option<&TextureHandle>`, kept
    in a separate slot that holds only the latest large cover.
  - `pub struct LocalCover` in `ui/vinyl/cover.rs`: `fn request(&mut self, ctx: &egui::Context,
    key: TrackId, path: PathBuf)` (spawns one thread per new key, named `aria-cover`, which
    calls `large_cover(path, 1024)` and sends `(key, Option<RgbaImage>)` back on a channel),
    `fn texture(&mut self, ctx, key: TrackId) -> Option<&TextureHandle>` (takes in arrived
    results, keeps only the one for `key`), `fn clear(&mut self)`.

- [ ] **Step 1: Write the failing tests:**
  - `aria-library/tests/tags.rs` or `artwork.rs` tests: `a_large_picture_is_scaled_to_1024`
    (a temp copy of `cover.flac` with a 2048×1024 PNG written in by lofty → result 1024×512);
    `cover_flac_gives_its_picture` (the fixture → `Some`, no side over 1024);
    `a_broken_picture_gives_none` (a file whose picture bytes are garbage → `None`);
    `a_missing_file_gives_none`.
  - `aria-spotify` client test `the_largest_image_is_kept_too` (images of 640, 300 and 64 px
    plus an `http://` one of 1000 px → `image` unchanged from `pick_image` (the 64 px one),
    `image_large` the 640 px one).
  - `aria-core` service test `a_large_cover_is_decoded_to_1024` (a 2000×1000 PNG served by the
    fake → `Cover` 1024×512 with that URL).
  - `aria-app` `vinyl::cover` test `a_stale_cover_is_dropped` (request key 1 then key 2, both
    with paths to the fixture; after both arrive `texture(ctx, TrackId(2))` is `Some` and the
    loader holds one texture).
- [ ] **Step 2: Run them** (`cargo test -j 4 -p aria-library`, `-p aria-spotify`,
  `-p aria-core --lib spotify_service`, `-p aria-app --lib vinyl::cover`) → FAIL.
- [ ] **Step 3: Implement.** The file is only read, never written. The Spotify large cover goes
  through the existing covers thread and its rules (HTTPS only, no token, 2 MiB).
- [ ] **Step 4: Run them**, then the workspace → pass.
- [ ] **Step 5: Commit** `feat: large covers for the full-screen record`.

### Task 5: The layout and the record drawing

**Files:**
- Create: `crates/aria-app/src/ui/vinyl/layout.rs`, `crates/aria-app/src/ui/vinyl/draw.rs`

**Interfaces:**
- Produces:
  - `pub struct Layout { pub sleeve: Option<Rect>, pub record: Rect, pub arm_pivot: Pos2,
    pub title: Pos2 }` and `pub fn layout(screen: Rect) -> Layout`: wide when
    `width >= WIDE_RATIO * height` (`WIDE_RATIO = 1.4`): sleeve a square of 0.62 × height,
    left of centre; the record the same size, its centre at the sleeve's right edge minus a
    quarter of its diameter. Narrow: the record's diameter 1.15 × the smaller side, its centre
    left of the screen's centre so that about a fifth is off the left edge. The arm's pivot
    at the record's top right, outside it. The title at the bottom left with a margin.
  - `pub fn draw_record(painter: &Painter, record: Rect, angle: f32, finish: Option<&TextureHandle>,
    label: Label<'_>, sheen: Color32)` where `pub enum Label<'a> { Cover(&'a TextureHandle),
    Plain { colour: Color32, title: &'a str, artist: &'a str } }`: the finish texture and the
    label as two quads rotated by `angle` about the record's centre (a 4-vertex `epaint::Mesh`
    with rotated positions and full UVs); the label a circle of `LABEL_RATIO` of the diameter
    (cover cut to a circle with a mesh fan, not a square quad) with a spindle hole; then a
    still, soft sheen; `None` finish draws a plain black disc.
  - `pub fn draw_arm(painter: &Painter, layout: &Layout, arm: f32, colours: ArmColours)`:
    the tonearm rotated between rest (`arm = 0`, off the record) and play (`arm = 1`, its
    stylus on the outer grooves).
  - `pub struct FinishCache` (in `draw.rs`): `fn want(&mut self, ctx: &egui::Context,
    finish: Finish, colours: CoverColours, key: u64)` (renders on a thread named
    `aria-vinyl` when `key` is new; `key` = hash of finish and colours), `fn texture(&mut self,
    ctx) -> Option<&TextureHandle>` (the latest arrived for the current key, else the last one
    shown), `fn clear(&mut self)`.

- [ ] **Step 1: Write the failing tests:** `layout_is_wide_from_1_4` (1400×1000 → `sleeve`
  `Some`; 1399×1000 → `None`); `the_record_is_on_screen_enough` (narrow 800×1200: at least 70 %
  of the record's area inside the screen; wide 1920×1080: record and sleeve overlap and both
  are within the screen vertically); `a_stale_render_is_dropped` (`want` key A then key B;
  after both renders arrive, `texture` is B's: compare sizes or an id kept beside it).
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-app --lib vinyl` → FAIL.
- [ ] **Step 3: Implement** `layout.rs` and `draw.rs`. Arm and sheen colours come from the
  theme (`Palette`): light theme a cream arm, dark theme a black one.
- [ ] **Step 4: Run them.** → pass.
- [ ] **Step 5: Commit** `feat(app): lay out and draw the turning record`.

### Task 6: The mode in the app

**Files:**
- Modify: `crates/aria-app/src/ui/vinyl/mod.rs` (the mode), `crates/aria-app/src/ui/layout/player_bar.rs`
  (the button; `transport`, `seek_scrub`, the volume parts made `pub(crate)` for reuse),
  `crates/aria-app/src/app.rs`, `crates/aria-app/src/shortcuts.rs`, `.ftl` files, `KEYS`,
  `crates/aria-app/src/ui/theme/icons.rs` (`CORNERS_OUT`, `CORNERS_IN`, `PALETTE` from Phosphor)
- Test: `crates/aria-app/tests/ui_vinyl.rs`

**Interfaces:**
- Consumes: Tasks 1–5; `PlayerBar` and its control functions.
- Produces:
  - `PlayerAction::ToggleFullScreen`; the bar's button at its far right (`player-full-screen`).
  - `pub struct VinylMode` (owned by `AriaApp`): `open: bool`, `was_fullscreen: Option<bool>`,
    `turntable: Turntable`, `panel_open: bool`, `last_move: f64`, `finishes: FinishCache`,
    `local_cover: LocalCover`; `fn show(&mut self, ui: &mut Ui, bar: &mut PlayerBar<'_>,
    settings: &Settings) -> Vec<VinylAction>` where `pub enum VinylAction { Player(PlayerAction),
    Close, SetFinish(VinylFinish) }`.
  - `Shortcuts.toggle_full_screen` (F11) and `Shortcuts.escape` (Esc, not consumed while a text
    field has focus).

Behaviour (from the spec): opening sends `ViewportCommand::Fullscreen(true)` after remembering
`ctx.input(|i| i.viewport().fullscreen)`; closing sends `Fullscreen(false)` only if that was
`Some(false)` or `None`; a reported `fullscreen == Some(false)` while open (after it was seen
`true`) closes the mode. While open, the app draws only the mode (and toasts); page shortcuts
are not read. Controls and cursor hide after `HIDE_AFTER = 3.0` s without pointer movement,
unless the pointer is over them. The panel lists `VinylFinish::ALL` as round swatches (each a
small render of its finish, Random a shuffle icon), the chosen one ringed; choosing saves
`settings.vinyl_finish`. Repaint: `request_repaint()` while `turntable.busy(playing)` or the
controls are fading; nothing otherwise. Identity for Random: the Spotify URI, or the track's
path. The label's cover: local large cover → local thumbnail → plain label; Spotify large →
Spotify small → plain.

Keys: `player-full-screen = Full screen`, `player-exit-full-screen = Exit full screen`,
`vinyl-finish = Record finish`, `vinyl-finish-random = Random`, `vinyl-finish-black = Black`,
`vinyl-finish-tinted = Tinted`, `vinyl-finish-splatter = Splatter`,
`vinyl-finish-marble = Marble`. "Nothing playing" reuses `empty-nothing-playing`.

- [ ] **Step 1: Write the failing tests** in `tests/ui_vinyl.rs` (the whole app in a harness, as
  `ui_player.rs` does): `the_bar_button_opens_the_mode_and_esc_closes_it` (asserts the
  viewport commands sent, via `harness.output().viewport_output`, include `Fullscreen(true)`,
  then `Fullscreen(false)`); `closing_restores_a_windowed_window`;
  `a_window_already_full_screen_stays_so` (input reports `fullscreen = Some(true)` before
  opening → no `Fullscreen(false)` on close); `leaving_os_full_screen_closes_the_mode`;
  `controls_show_on_movement_and_hide_after_3_s` (step the harness clock);
  `esc_closes_the_panel_then_the_mode`; `choosing_a_finish_saves_it` (settings'
  `vinyl_finish` becomes `Marble`); `page_shortcuts_are_off_in_the_mode` (Ctrl+9 does not
  navigate); `the_mode_controls_spotify_like_the_bar` (with `ActivePlayer::Spotify` and a
  playback, clicking play/pause sends `Command::Spotify(SpotifyRequest::TogglePlay)`);
  `nothing_playing_says_so`; `a_paused_record_asks_for_no_repaint` (paused, after the
  spin-down and the controls' fade have settled, `harness.ctx.has_requested_repaint()` is
  false after a frame).
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-app --test ui_vinyl` → FAIL.
- [ ] **Step 3: Implement** the mode, the button, the keys and the app wiring.
- [ ] **Step 4: Run them**, the whole workspace, clippy and fmt → all pass.
- [ ] **Step 5: Commit** `feat(app): the full-screen record`.

### Task 7: Docs, version 0.3.1 and the hand check

**Files:**
- Modify: `Cargo.toml` (`version = "0.3.1"`), `Cargo.lock`, `CHANGELOG.md`, `README.md`,
  `docs/PRIVACY.md` (Spotify's larger image), `packaging/windows/aria.iss` (example version)
- Create: `docs/testing/0.3.1-manual-check.md`

- [ ] **Step 1: CHANGELOG [0.3.1]** (the mode, the finishes, the larger covers; test count; what
  is unverified), README's feature list gains the mode in one bullet, PRIVACY says the larger
  Spotify image is fetched only for the item in full screen.
- [ ] **Step 2: The hand check:** open and close (button, F11, Esc) on Windows; the record turns
  smoothly at 33⅓ rpm and eases on pause; the tonearm swings; each finish, Random over a few
  tracks; the sleeve on a wide screen, the narrow layout on a tall window; a local track's sharp
  cover; a Spotify item's cover; the controls hide after 3 s; CPU near zero when paused
  (Task Manager); macOS and Linux full screen where available.
- [ ] **Step 3: Verify.** `cargo test -j 4 --workspace --locked --no-fail-fast`, clippy, fmt,
  `cargo build -j 4 --release --locked` → pass. Push; CI green on all three systems.
- [ ] **Step 4: Commit** `docs: 0.3.1 full-screen record` and push. Tagging and publishing wait
  for the user's yes.
