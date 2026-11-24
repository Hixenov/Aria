# Aria 0.3.3 — Spotify plays in Aria: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** With no Spotify app running, pressing play on a Spotify item in Aria plays it in a small Edge or Chrome window that Aria opens, runs and closes itself. The window runs Spotify's Web Playback SDK.

**Architecture:**
- `aria-spotify` gains three things:
  - the `streaming` scope;
  - a browser finder and launcher (`browser.rs`);
  - a loopback page server with a one-time session (`player_page/`), serving a page built into the binary that runs the SDK.
- `aria-core` gains a player state machine (`spotify_player.rs`), driven by the Spotify thread's existing `select!` loop. It starts the window on play when no device is active (or when "Aria" is chosen), waits for `ready`, then moves playback there.
- `aria-app` adds:
  - the setting;
  - the "Aria (this computer)" entry in both device pickers;
  - the page's strings;
  - the Reconnect button.

**Tech Stack:** Rust; egui/eframe 0.37; crossbeam-channel; ureq; serde_json; getrandom and base64 (already in `aria-spotify`); std `TcpListener` and `std::process::Command`.

**Spec:** `docs/superpowers/specs/2026-11-24-aria-0-3-3-spotify-in-aria-design.md` (approved 2026-11-24, "Approve, go ahead").

## Global Constraints

- No `unwrap`/`expect` outside tests, no `unsafe`, no `#[allow]`/`#[expect]`. `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` must be clean.
- "Never log access tokens. Never expose credentials in crash reports. Store credentials in OS-protected storage." Tokens stay in `Secret`, and no log line carries the key, the session or the token.
- The page never puts the token, or anything else, in `localStorage`, `sessionStorage`, IndexedDB or cookies, and never logs it. No analytics.
- "Do not: Circumvent DRM. Extract Spotify encryption keys. Record or download Spotify streams. Bypass Premium requirements. Bypass advertisements. Reverse-engineer private Spotify endpoints. Spoof official Spotify clients or devices." The SDK device is named `Aria`.
- "Do not claim that an artifact is supported unless it has actually been built and tested." macOS and Linux are called untested in README, CHANGELOG and release notes.
- Servers bind `127.0.0.1` only.
- Tests never start a real browser or open a URL, never open the real microphone, and never touch `%APPDATA%\Aria`. Never run the built `aria.exe` to check it.
- The marker for "Aria (this computer)" in `spotify_device` is the string `aria` (`aria_core::spotify_player::ARIA_DEVICE`).
- The setting is `spotify_play_in_aria: bool`, default `true`, read leniently.
- Waits:
  - `ready`: 20 s, or 90 s when the profile folder doesn't exist yet;
  - lost: 15 s without any page request;
  - the page's beat: `alive` every 1 s (ruling: the spec's "5 s" heartbeat number is left to the builder);
  - the close long-poll answers after 20 s.
- Builds are on E: with `CARGO_INCREMENTAL=0` and `-j 4`, using the environment from `env.sh` in the session scratchpad. Logs go to `.local/build-logs/`.
- Commit and push to `main`, with no branches. Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A leftover browser on Aria's profile**, from a crash or a false "lost". Pressing play must reuse the old "Aria" device if Spotify still lists it, not open a second window. Test: `a_lost_player_still_listed_is_reused` (Task 5).
2. **"Play in Aria" turned off, or a sign-out, while the window is open.** The window closes (through the page), the server stops, and the device entry goes. Test: `turning_it_off_closes_the_window` (Task 5).
3. **Play pressed several times while the player is starting.** One launch only; the newest request plays once `ready` comes. Test: `plays_while_starting_launch_once` (Task 5).
4. **A `Host` that names the right port by another name** (`localhost:<port>`, `127.0.0.1` with no port, another case). Refused with 403. Test: `a_host_by_another_name_is_refused` (Task 3).
5. **The page asks for a token while the session can't refresh** (no network, ended session). The page gets 503, nothing hangs past 10 s, and nothing panics. Test: `a_token_that_cannot_be_had_answers_503` (Task 5).

---

### Task 1: Scopes, granted scopes, fresh access token

**Files:**
- Modify: `crates/aria-spotify/src/lib.rs` (`SCOPES`)
- Modify: `crates/aria-spotify/src/token.rs` (`Tokens`, `TokenAnswer`)
- Modify: `crates/aria-spotify/src/client.rs` (`fresh_access`)
- Modify: every place that builds `Tokens` (tests and fake)

**Interfaces:**
- Produces:
  - `Tokens.scopes: Option<String>`: the `scope` of the last answer that had one. A refresh answer without `scope` keeps the old value.
  - `Tokens::can_stream(&self) -> bool`: true iff `scopes` splits on whitespace into a list containing `streaming`.
  - `Client::fresh_access(&mut self, within: Duration) -> Result<Secret, SpotifyError>`: refreshes first when `expires_at` is less than `within` away, then returns a clone of the access token.

- [ ] **Step 1: Write the failing tests**
  - In `lib.rs` tests, `the_scopes_include_streaming_and_email`: `SCOPES.split(' ')` contains `"streaming"` and `"user-read-email"`, and still contains `"user-read-private"`.
  - In `token.rs` tests:
    - `granted_scopes_are_kept`: an exchange answer with `"scope":"streaming user-read-private"` gives `can_stream() == true`.
    - `a_refresh_without_scope_keeps_the_old_ones`.
    - `no_streaming_scope_cannot_stream`: `"scope":"user-read-private"` gives false, and `None` gives false.
  - In `client.rs` tests, `fresh_access_refreshes_when_near_expiry`, using the fake: with `expires_at = now + 30 s` and `within = 60 s`, one `POST /api/token` is seen and the new token is returned. With `expires_at = now + 10 min`, there is no request.
- [ ] **Step 2: Run them; they fail to compile** (no `scopes` field, no `fresh_access`).
  - Run: `cargo test -p aria-spotify -j 4 > .local/build-logs/t1-red.log 2>&1; tail -20 .local/build-logs/t1-red.log`
- [ ] **Step 3: Implement.**
  - `SCOPES` gains ` streaming user-read-email` at the end.
  - `TokenAnswer` gains `scope: Option<String>`.
  - `refresh(..., previous_scopes)` keeps the old scopes when the answer has none. Choose the signature; keep callers compiling.
  - `fresh_access` reuses the client's existing refresh path.
- [ ] **Step 4: Run** `cargo test -p aria-spotify -j 4`. Expected: all pass.
- [ ] **Step 5: Commit:** `feat(spotify): ask for streaming, keep the granted scopes`.

### Task 2: Finding and starting the browser

**Files:**
- Create: `crates/aria-spotify/src/browser.rs`
- Modify: `crates/aria-spotify/src/lib.rs` (`pub mod browser;`)

**Interfaces:**
- Produces:
  - `pub enum Os { Windows, MacOs, Linux }`
  - `pub fn find_browser(os: Os, var: &dyn Fn(&str) -> Option<OsString>, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf>`
  - `pub fn chromium_args(profile: &Path, url: &str) -> Vec<OsString>`
  - `pub trait Running: Send { fn exited(&mut self) -> bool; fn kill(&mut self); }`
  - `pub enum Launched { Window(Box<dyn Running>), Tab }`
  - `pub trait Launcher: Send { fn launch(&mut self, url: &str, profile: &Path) -> io::Result<Launched>; }`
  - `pub struct SystemLauncher;`, which implements `Launcher`. It looks for a browser for the current `Os` using the real environment and `Path::exists`, then spawns it with `chromium_args`, stdin/stdout/stderr null, and returns `Window`. With no browser, it calls `open::that(format!("{url}?tab=1"))` and returns `Tab`.
  - `fn current_os() -> Os`.

- [ ] **Step 1: Write the failing tests** in `browser.rs`, with `var` and `exists` as closures over a `HashMap` and a `HashSet`:
  - **Windows:**
    - `windows_prefers_edge`: with `ProgramFiles(x86)=C:\PF86`, `ProgramFiles=C:\PF`, `LOCALAPPDATA=C:\L`, and both `C:\PF86\Microsoft\Edge\Application\msedge.exe` and `C:\PF\Google\Chrome\Application\chrome.exe` existing, it returns the Edge path.
    - `windows_falls_back_to_chrome_in_local_app_data`: only `C:\L\Google\Chrome\Application\chrome.exe` exists, and that is returned.
    - The order is: Edge (x86), Edge (PF), Chrome (PF), Chrome (x86), Chrome (LOCALAPPDATA).
  - **macOS:** `macos_looks_in_applications_then_home`:
    - `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` wins over Edge.
    - With only `$HOME/Applications/Chromium.app/Contents/MacOS/Chromium` present, that path is returned.
    - The order is: Chrome, Edge (`Microsoft Edge.app/Contents/MacOS/Microsoft Edge`), Chromium, first in `/Applications` and then in `$HOME/Applications`.
  - **Linux:** `linux_searches_path_in_order`: with `PATH=/a:/b`, where `/b/chromium` and `/a/microsoft-edge` exist, it returns `/b/chromium`, because names are tried in order across the whole PATH: `google-chrome`, `google-chrome-stable`, `chromium`, `chromium-browser`, `microsoft-edge`, `microsoft-edge-stable`. PATH is split with `std::env::split_paths`.
  - `nothing_found_is_none`.
  - `the_arguments_are_exact`: `chromium_args(Path::new("/p"), "http://127.0.0.1:5/p/k/")` equals, in this order:
    ```
    --user-data-dir=/p
    --app=http://127.0.0.1:5/p/k/
    --window-size=420,260
    --no-first-run
    --no-default-browser-check
    --autoplay-policy=no-user-gesture-required
    --disable-features=HardwareMediaKeyHandling
    --disable-background-timer-throttling
    --disable-renderer-backgrounding
    --disable-backgrounding-occluded-windows
    --hide-crash-restore-bubble
    ```
    Ruling: `--hide-crash-restore-bubble` is added beyond the spec's list, so that a window killed after the close grace never shows "restore pages".
- [ ] **Step 2: Run** `cargo test -p aria-spotify browser -j 4`. Expected: compile errors (module missing).
- [ ] **Step 3: Implement** the functions and `SystemLauncher`. `SystemLauncher` isn't run in tests; its `Running` wraps `std::process::Child`:
  - `exited` is `try_wait` returning `Some`;
  - `kill` is `child.kill()`, then `wait()`, with errors ignored and noted in a comment.
  - `open` is a dependency of `aria-core`. If `aria-spotify` doesn't have it, add `open = { workspace = true }` to `aria-spotify`.
- [ ] **Step 4: Run** the tests. Expected: pass.
- [ ] **Step 5: Commit:** `feat(spotify): find and start Aria's player browser`.

### Task 3: The player page server and the page

**Files:**
- Create: `crates/aria-spotify/src/player_page/mod.rs`
- Create: `crates/aria-spotify/src/player_page/page.html`
- Create: `crates/aria-spotify/src/player_page/player.js`
- Modify: `crates/aria-spotify/Cargo.toml` (add `crossbeam-channel = { workspace = true }`)
- Modify: `crates/aria-spotify/src/lib.rs`

**Interfaces:**
- Consumes: `Secret`, and `callback::bind_error`.
- Produces:
  - Page events:
    ```rust
    pub enum PageEvent { Ready { device_id: String }, NotReady { device_id: String }, Error { kind: PageError, message: String }, AutoplayFailed, Alive }
    pub enum PageError { Initialization, Authentication, Account, Playback }
    ```
    Both derive `Clone, Debug, PartialEq, Eq`.
  - Messages to the Spotify thread:
    ```rust
    pub enum PageMsg { Token(crossbeam_channel::Sender<Option<Secret>>), Event(PageEvent) }
    ```
  - The server:
    ```rust
    pub struct PageServer
    impl PageServer {
        pub fn start(strings_json: String, to_service: crossbeam_channel::Sender<PageMsg>) -> Result<PageServer, SpotifyError>;
        pub fn url(&self) -> String;
        pub fn port(&self) -> u16;
        pub fn close_page(&self);
        pub fn stop(self);
    }
    impl Drop for PageServer { /* stops */ }
    ```
    - `url` is `http://127.0.0.1:{port}/p/{key}/`.
    - `close_page` sets a flag: every `/wait` answers `{"close":true}` at once, and so does every later `/event`.
  - `#[cfg(any(test, feature = "test-support"))] pub struct PageClient`: a minimal HTTP client that plays the page, for Task 5's tests. It has `PageClient::new(url: &str) -> PageClient` and these methods: `session() -> Option<String>`, `token() -> (u16, String)`, `event(json: &str) -> (u16, String)` and `wait() -> (u16, String)`. It sends `Host: 127.0.0.1:<port>`.

**Routes.** All are under `/p/<key>/`, where the key is 32 bytes from `getrandom`, base64url with no padding. Any other path answers 404.

| Route | Answer |
|---|---|
| `GET ""` (the page) | `page.html`, `text/html; charset=utf-8` |
| `GET player.js` | `text/javascript; charset=utf-8` |
| `GET strings` | `strings_json`, `application/json` |
| `POST session` | `{"session":"<32 random bytes, base64url>"}` the first time; 403 after |
| `GET token` | asks the service (`PageMsg::Token`), waits ≤ 10 s; `{"access_token":"…"}` or 503 |
| `POST event` | body parsed as below; `{}` or `{"close":true}` |
| `GET wait` | held until `close_page` (`{"close":true}`) or 20 s (`{}`) |

- `token`, `event` and `wait` need the header `X-Aria-Session: <session>`, or they answer 403.
- **Every answer:** `Cache-Control: no-store`, `Referrer-Policy: no-referrer`, `Connection: close`. HTML answers also send `Content-Security-Policy: script-src 'self' https://sdk.scdn.co`. No `Access-Control-*` header is ever sent.
- **Refused before routing:**
  - `Host` not exactly `127.0.0.1:<port>`: 403;
  - an `Origin` present and not `http://127.0.0.1:<port>`: 403;
  - head plus body over 8 KiB: 413;
  - a `Content-Length` above 4 KiB: 413.
- **Event bodies:**
  ```
  {"type":"ready","device_id":"…"}
  {"type":"not_ready","device_id":"…"}
  {"type":"error","kind":"initialization|authentication|account|playback","message":"…"}
  {"type":"autoplay_failed"}
  {"type":"alive"}
  ```
  Anything else answers 400 and sends nothing on.
- **Every accepted** `token`, `event` or `wait` also sends `PageMsg::Event(PageEvent::Alive)`, unless the event already was a page event. Any page request is a sign of life.
- **Connections:** the accept loop is non-blocking with a 50 ms poll, like `Callback`. Each connection is handled on its own thread, at most 8 at once; beyond that, answer 503. The read/write timeout is 2 s, except `wait`. `stop` sets the stop flag and joins the accept thread for at most 1 s.
- **The page** (`page.html` + `player.js`):
  - **Loading order.** `page.html` loads `player.js` and then `https://sdk.scdn.co/spotify-player.js`, using relative `player.js`. It shows a title, one paragraph and a status line, with Aria's dark colours inline in a `<style>` (CSS isn't a script, so the CSP allows it).
  - **Strings.** `player.js` fetches `strings` and fills the texts.
  - **Session.** It POSTs `session` once and keeps the value in a variable. If that fails, it shows `player-page-reload` and stops.
  - **The player.** It defines `window.onSpotifyWebPlaybackSDKReady`, which creates `new Spotify.Player({ name: "Aria", getOAuthToken: cb => fetch token → cb(access_token), volume: 0.5 })`.
  - **Listeners.** `ready` and `not_ready` post their events. `initialization_error`, `authentication_error`, `account_error` and `playback_error` post `error` with `message`. `autoplay_failed` posts its event.
  - **Connecting.** With `?tab=1` in `location.search`, it shows a Start button that calls `player.activateElement()` and then `player.connect()`. Otherwise it calls `connect()` at once.
  - **Beat and close.** It posts `alive` every 1000 ms and keeps one `wait` request open, renewed on `{}`. On `{"close":true}` from `wait` or `event`, it calls `player.disconnect()` and then `window.close()`.
  - **No console output** at all.

- [ ] **Step 1: Write the failing tests** in `player_page/mod.rs`, using `PageClient` and raw `TcpStream` for bad requests. A test thread answers `PageMsg::Token` with a fixed `Secret("tok")`.
  - `the_page_and_script_are_served_with_the_headers`: 200, the CSP is exact, `no-store`, and there is no `Access-Control`.
  - `a_wrong_key_is_404`.
  - `a_host_by_another_name_is_refused`: `localhost:<port>`, `127.0.0.1` with no port, and `127.0.0.1:<port+1>` each answer 403.
  - `a_foreign_origin_is_refused`: `Origin: https://evil.example` answers 403. A right `Origin` passes.
  - `the_session_is_given_once`: the first `session` is 200; the second is 403.
  - `token_needs_the_session`: with no header, or a wrong one, it answers 403 and the body doesn't contain `tok`. The right header gives 200 and `{"access_token":"tok"}`.
  - `events_are_parsed_and_sent_on`: `ready` arrives as `PageMsg::Event(Ready{device_id:"d1"})`, and an `error` with kind `account` as `Error{kind:Account,..}`. Bad JSON answers 400 and nothing arrives.
  - `an_oversized_request_is_413`.
  - `close_page_answers_wait_and_event`: a `wait` in flight returns `{"close":true}` within 500 ms of `close_page()`. The next `event` answer is `{"close":true}`.
  - `it_listens_on_loopback_only`: `url()` starts with `http://127.0.0.1:`.
  - `the_page_script_never_logs_or_stores`: `player.js` doesn't contain `console.`, `localStorage`, `sessionStorage`, `indexedDB` or `document.cookie`.
- [ ] **Step 2: Run** `cargo test -p aria-spotify player_page -j 4`. Expected: compile errors.
- [ ] **Step 3: Implement** the server and write the page as specified.
- [ ] **Step 4: Run** the tests, then the whole crate. Expected: pass.
- [ ] **Step 5: Commit:** `feat(spotify): the player page and its loopback server`.

### Task 4: Settings, the device marker and the strings

**Files:**
- Modify: `crates/aria-core/src/settings/mod.rs`
- Modify: `crates/aria-core/src/i18n/mod.rs` (the KEYS list)
- Modify: `locales/{en,ru,hy}/main.ftl`

**Interfaces:**
- Produces:
  - `Settings::spotify_play_in_aria: bool`, default `true`, read with `lenient_or_default` like the other booleans;
  - the i18n keys below.

**English strings** (ru and hy translated with the same meaning):

| Key | Text |
|---|---|
| `settings-spotify-play-in-aria` | `Play in Aria` |
| `settings-spotify-play-in-aria-hint` | `Spotify plays in a small Edge or Chrome window that Aria opens and closes. You can minimize it. Needs Spotify Premium.` |
| `settings-spotify-reconnect` | `Reconnect` |
| `settings-spotify-reconnect-hint` | `Reconnect once so Spotify lets Aria play music itself.` |
| `spotify-device-aria` | `Aria (this computer)` |
| `spotify-player-starting` | `Starting Aria's Spotify player…` |
| `spotify-player-reconnect` | `Reconnect Spotify in Settings to play it in Aria.` |
| `spotify-player-premium` | `Playing Spotify in Aria needs Spotify Premium.` |
| `spotify-player-no-drm` | `This browser can't play Spotify (it lacks the copy-protection part). Install Chrome or Edge.` |
| `spotify-player-failed` | `Spotify couldn't play this in Aria.` |
| `spotify-player-timeout` | `Aria's Spotify player didn't start.` |
| `spotify-player-launch-failed` | `Aria couldn't open its player window.` |
| `spotify-player-closed` | `The Spotify player window was closed; press play to open it again.` |
| `spotify-player-tab` | `Edge or Chrome wasn't found, so Spotify plays in a tab of your browser. Click Start there.` |
| `spotify-player-autoplay` | `Click Start in Aria's Spotify page to let it play.` |
| `player-page-title` | `Aria — Spotify` |
| `player-page-playing` | `Aria is playing Spotify in this window. You can minimize it; closing it stops Spotify in Aria.` |
| `player-page-starting` | `Starting…` |
| `player-page-ready` | `Ready` |
| `player-page-failed` | `Spotify couldn't start here. Aria says why.` |
| `player-page-start` | `Start` |
| `player-page-reload` | `Close this window and press play in Aria.` |

- [ ] **Step 1: Write the failing tests.**
  - `spotify_play_in_aria_defaults_on_and_reads_leniently`:
    - default `true`;
    - `spotify_play_in_aria = false` reads false;
    - `= "x"` reads `true`;
    - a round trip keeps it.
  - `the_aria_marker_survives_sanitizing`: `spotify_device = "aria"` stays `Some("aria")` after `sanitized()`.
  - The existing i18n tests (planned keys exist in every locale, placeables) cover the new keys once they're in `KEYS`.
- [ ] **Step 2: Run** `cargo test -p aria-core settings i18n -j 4`. Expected: fail.
- [ ] **Step 3: Implement.** Add the field, the keys to `KEYS`, and the strings to the three `.ftl` files.
- [ ] **Step 4: Run.** Expected: pass.
- [ ] **Step 5: Commit:** `feat(settings): play Spotify in Aria, and its strings`.

### Task 5: The player state machine in the Spotify thread

**Files:**
- Create: `crates/aria-core/src/spotify_player.rs`
- Modify: `crates/aria-core/src/spotify_service.rs`
- Modify: `crates/aria-core/src/runtime.rs` (route and persist `PlayInAria`)
- Modify: `crates/aria-core/src/lib.rs`

**Interfaces:**
- Consumes:
  - `PageServer`, `PageMsg`, `PageEvent` and `PageError` (Task 3);
  - `Launcher`, `Launched` and `Running` (Task 2);
  - `Tokens::can_stream` and `Client::fresh_access` (Task 1);
  - `spotify_play_in_aria` (Task 4).
- Produces:
  - `pub const ARIA_DEVICE: &str = "aria";`
  - The player's dependencies:
    ```rust
    pub struct PlayerDeps {
        pub launcher: Box<dyn Launcher>,
        pub profile_dir: Option<PathBuf>,
        pub ready: Duration,
        pub first_ready: Duration,
        pub lost: Duration,
        pub close_grace: Duration,
    }
    ```
    `PlayerDeps::system(profile_dir)` gives `SystemLauncher`, 20 s, 90 s, 15 s and 700 ms. `SpotifyDeps` gains `pub player: PlayerDeps`.
  - `SpotifyDeps::system(cache_dir: Option<PathBuf>)`: the profile is `cache_dir/spotify-player`. With `None`, play in Aria is unavailable and Aria acts as 0.3.2 does.
  - `SpotifyRequest::PlayInAria { on: bool, strings: String }`. `strings` is the page's JSON.
  - `SpotifyEvent::Player(AriaPlayerStatus)`, where:
    ```rust
    pub enum AriaPlayerStatus { Off, Starting, Ready { device_id: String }, Lost, NeedsReconnect }
    ```
    Derives `Clone, Debug, PartialEq, Eq`. It is emitted on every change.

**Behaviour** (from the spec, §5.4 and §5.5):
- **What the player tracks:** a state (`Off` / `Starting` / `Ready` / `Lost`), the server, the `Running`, the last device id, the last page request's `Instant`, the pending `PlayRequest`, and the deadline.
- **When the player is wanted.** It is "wanted" for a play when:
  - `on`, the client is signed in and `profile_dir` is `Some`; and
  - either the chosen device is `ARIA_DEVICE`, or the chosen device is `None` and the op answered `NoActiveDevice`.
- **Play while wanted and not `Ready`:**
  - Without `can_stream`: emit `NeedsReconnect` and notice `spotify-player-reconnect`, and do nothing else.
  - If `Lost` and Spotify's device list (refreshed now) still has the last id, go back to `Ready` and play there.
  - Otherwise:
    1. stop any old server and kill any old child;
    2. start a server and launch with the server's url and the profile;
    3. on `Tab`, notice `spotify-player-tab`;
    4. on an error, notice `spotify-player-launch-failed` and go to `Off`;
    5. notice `spotify-player-starting` and go to `Starting`, with the deadline `ready`, or `first_ready` if the profile folder doesn't exist yet. Keep the request as pending; a newer play replaces it.
- **Play while `Ready`:** the target is the ready device id.
- **`PageEvent::Ready`:**
  - go to `Ready` and note the id;
  - with a pending request, call `transfer(id, true)`, then `play(Some(id), request)` unless the request is `Resume`;
  - refresh the devices.
- **Page messages.**
  - `PageMsg::Token(reply)`: reply with `fresh_access(60 s)` mapped to `Some`. On an error, reply `None`, and the error goes through the usual `fail`.
  - Every page message updates the last-request time. A `Lost` player that hears from its page goes back to `Ready`.
- **Errors from the page:**
  - `Authentication`: notice `spotify-player-reconnect`;
  - `Account`: notice `spotify-player-premium`;
  - `Initialization`: notice `spotify-player-no-drm`;
  - `Playback`: notice `spotify-player-failed`;
  - `AutoplayFailed`: notice `spotify-player-autoplay`;
  - the page's message is logged at debug level.
- **Timers** (`run_due`; the player's next deadline joins the loop's `min`):
  - `Starting` past its deadline: notice `spotify-player-timeout`, then close.
  - `Ready` with no page request for `lost`: go to `Lost`, and notice `spotify-player-closed` if the child has exited.
- **Close** (setting off, sign-out, `Shutdown`):
  1. `close_page()`;
  2. wait up to `close_grace` for `exited()`, then `kill()`;
  3. stop the server and go to `Off`.
  
  `Shutdown` must finish within the runtime's 1 s Spotify join.
- **The devices list.** The ready device id is removed from `SpotifyEvent::Devices`; the UI shows its own entry instead. `target()` maps `ARIA_DEVICE` to the ready id.
- **The runtime** routes `SpotifyRequest::PlayInAria` to the service, and persists `spotify_play_in_aria = on` like `Device`.

- [ ] **Step 1: Write the failing tests** in `spotify_service.rs` tests, with the fake Spotify, a signed-in session whose tokens have `streaming`, and a `FakeLauncher`. The `FakeLauncher` records URLs and returns `Window(FakeRunning)`, which has a shared `exited` flag and a `killed` counter. "The page" is a `PageClient` driven from the test.
  - `play_with_no_device_starts_the_player`: `/me/player/play` first answers 404 `NO_ACTIVE_DEVICE`. A launch is recorded, and `Player(Starting)` is emitted. After the page posts `session` and `ready{device_id:"web1"}`, the fake sees `PUT /me/player` with `"device_ids":["web1"]`, then `PUT /me/player/play?device_id=web1`.
  - `choosing_aria_starts_the_player_on_play`: `Device(Some("aria"))` followed by `Play` launches without first calling play on the active device.
  - `a_chosen_phone_does_not_start_the_player`: `Device(Some("phone"))` with a `NoActiveDevice` answer leads to the transfer to the phone, as today, and no launch.
  - `turning_it_off_closes_the_window`: once ready, `PlayInAria{on:false}` makes the page's `wait` get `{"close":true}`, and the status becomes `Off`. With `exited` never set, `killed == 1` after the grace.
  - `play_in_aria_off_is_0_3_2`: `on:false` with `NoActiveDevice` gives notice `spotify-no-device` and no launch.
  - `without_streaming_it_asks_to_reconnect`: tokens without `streaming` give `NeedsReconnect`, notice `spotify-player-reconnect`, and no launch.
  - `plays_while_starting_launch_once`: two `Play`s before `ready` give one launch, and the second request is the one played.
  - `no_ready_in_time_is_a_timeout`: with `ready = 100 ms`, notice `spotify-player-timeout` and `Off`.
  - `a_quiet_page_is_lost`: with `lost = 200 ms` and no page traffic after `ready`, the status is `Lost`.
  - `a_lost_player_still_listed_is_reused`: when `Lost` and `/me/player/devices` lists `web1`, a play needs no new launch and plays on `web1`.
  - `the_ready_device_is_not_listed_twice`: `Devices` emitted after ready doesn't contain `web1`.
  - `a_token_that_cannot_be_had_answers_503`: the fake's token endpoint is offline and the access token has expired, so `PageClient::token()` gives 503 within 11 s.
  - `shutdown_closes_the_window`: `Shutdown` while ready kills the child, or it exits; the thread ends within 1 s.
- [ ] **Step 2: Run** `cargo test -p aria-core spotify -j 4 > .local/build-logs/t5-red.log 2>&1`. Expected: compile errors.
- [ ] **Step 3: Implement** `spotify_player.rs` and wire it into `Service`:
  - a new `recv(page)` arm in `select!`;
  - the player's deadline added to the loop's deadlines;
  - `handle` for `PlayInAria`;
  - `command()` changed so that wanted plays go through the player;
  - closing in `sign_out` and at loop exit;
  - `SpotifyDeps::system` takes `cache_dir`, and its callers are updated (`main.rs` passes `paths.as_ref().map(|p| p.cache_dir.clone())`).
- [ ] **Step 4: Run** the `aria-core` tests, then the workspace tests. Expected: pass.
- [ ] **Step 5: Commit:** `feat(spotify): play in Aria's own player window`.

### Task 6: The app: setting, device entries, strings, reconnect

**Files:**
- Modify: `crates/aria-app/src/spotify.rs` (`player_page_strings`, `SpotifyUi.player`)
- Modify: `crates/aria-app/src/ui/pages/settings/spotify.rs`
- Modify: `crates/aria-app/src/ui/layout/player_bar.rs` (`devices_button`)
- Modify: `crates/aria-app/src/app.rs` (send `PlayInAria` at the first frame and when the setting or the language changes; handle Reconnect)

**Interfaces:**
- Consumes: `SpotifyRequest::PlayInAria`, `SpotifyEvent::Player(AriaPlayerStatus)`, `ARIA_DEVICE` and the i18n keys.
- Produces:
  - `pub fn player_page_strings(tr: &Translator) -> String`: a JSON object keyed by the `player-page-*` ids, built with `serde_json`. Use the translator type the module already uses.
  - `SpotifyUi.player: AriaPlayerStatus`.
  - `PageAction::Spotify(SpotifyRequest::PlayInAria{..})` from the checkbox.
  - Reconnect is `SignOut` followed by `SignIn` with the usual pages.

- [ ] **Step 1: Write the failing tests** (kittest, as the existing settings and player bar tests do):
  - `the_aria_device_is_listed_first_after_active`: signed in, with `spotify_play_in_aria` on. The settings combo and the player bar's devices menu show `en("spotify-device-aria")`. Choosing it pushes `SpotifyRequest::Device(Some("aria"))` or `PlayerAction::SpotifyDevice(Some("aria"))`. With the setting off, the entry is absent.
  - `the_chosen_aria_shows_its_name`: `spotify_device = Some("aria")` makes the combo's selected text `en("spotify-device-aria")`.
  - `the_play_in_aria_checkbox_sends_the_request`: unticking pushes `PlayInAria{on:false, ..}`.
  - `reconnect_shows_when_needed`: `SpotifyUi.player = NeedsReconnect` shows the `settings-spotify-reconnect` button, and clicking it pushes `SignOut` then `SignIn{..}`. With `Ready`, there is no button.
  - `page_strings_are_json_with_every_key`: it parses as an object with all seven `player-page-*` keys.
  - In `app.rs` tests: `play_in_aria_is_sent_at_start`: the first frame sends `Command::Spotify(PlayInAria{on:true, ..})`.
- [ ] **Step 2: Run** `cargo test -p aria-app -j 4 > .local/build-logs/t6-red.log 2>&1`. Expected: fail.
- [ ] **Step 3: Implement.**
  - The checkbox and its hint go in the signed-in group, under the device row. Reconnect has its hint line.
  - The device entries go in both pickers, using the same helper for the name.
  - `apply` handles `SpotifyEvent::Player`.
- [ ] **Step 4: Run** the workspace tests, clippy and fmt. Expected: clean.
- [ ] **Step 5: Commit:** `feat(app): play in Aria in Settings and the device pickers`.

### Task 7: Version, documents, hand check

**Files:**
- `Cargo.toml` and `Cargo.lock` → 0.3.3; the version example in `packaging/windows/aria.iss` → 0.3.3.
- `CHANGELOG.md` gets `## [0.3.3] - <date> (Spotify plays in Aria)` with Added, Changed and Limits, and the test count from the last run.
- `README.md`: the Spotify bullet says it plays in Aria's own window with Premium, and that macOS and Linux are untested.
- `docs/PRIVACY.md`, in the Spotify section, covers the spec's §5.7 points:
  - the new scopes, and that the email is never read;
  - the local page on `127.0.0.1` with its per-launch key and one-time session;
  - the browser profile at `<cache>/spotify-player`;
  - `sdk.scdn.co`, and that Spotify's player talks to Spotify itself;
  - the token is handed only to that page, and is kept in memory.
- `docs/ROADMAP.md`: the "no in-app streaming" line gains "Aria itself still plays no Spotify audio; since 0.3.3 Spotify's own Web Playback SDK plays it in a browser window Aria opens".
- `docs/testing/0.3.3-manual-check.md`, every line pending:
  - **Dashboard:** tick "Web Playback SDK" for the app.
  - **Reconnect:** it asks once for the new permissions.
  - **First play:**
    - with the Spotify app closed everywhere, play a track: the window opens within 90 s the first time and the music plays with no click;
    - the second time, it starts within 20 s.
  - **Controls:** pause, next, seek, volume, shuffle and repeat work; the mini-player and the full-screen record follow.
  - **Minimized:** minimized and paused for 10 minutes, it still plays on the next press, with no second window.
  - **Closing:**
    - closing the window by hand gives the notice, and play reopens it;
    - quitting Aria closes the window, and the next start shows no "restore pages".
  - **Devices:** choosing a phone in the picker plays there and opens no window.
  - **Without Edge or Chrome:** with no Edge or Chrome on the PATH (another machine), a tab opens and Start is needed.
  - **Global hotkeys:** the media keys still go to Aria.
  - **Other systems:** macOS and Linux, untested.

- [ ] **Step 1:** Make the edits. Run the workspace tests, clippy and fmt, and record the count in the CHANGELOG.
- [ ] **Step 2: Commit and push:** `docs: 0.3.3 — Spotify plays in Aria`. Wait for CI to be green on all three systems and `deny`.
