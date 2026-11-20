# Aria Phase 3 (0.3.0) Spotify Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sign in to Spotify from Aria, browse your saved music and search it, and play it on one
of your Spotify devices from Aria's player bar.

**Architecture:** A new crate `aria-spotify` holds everything that talks to Spotify: PKCE sign-in
with a loopback listener, the token exchange and refresh, a typed Web API client over blocking
`ureq`, and the refresh token's store in the system keyring. `aria-core` gets a
`spotify_service` thread (built like `library_service`: a request channel in, an `on_event`
callback out) that owns the client, polls playback, debounces search and fetches covers. The
backend worker routes the player bar's commands to the local player or to Spotify, whichever
the bar shows. The app turns the four Phase 3 placeholder pages into Spotify pages, adds a
Spotify card to Settings and a Spotify mode to the player bar.

**Tech Stack:** Rust 2024, egui/eframe 0.37, `ureq` 3 (rustls + platform verifier),
`serde_json`, `keyring` 3, `getrandom` 0.3, `sha2` 0.10, `base64` 0.22, `form_urlencoded` 1,
`image` 0.25 (already used).

**Spec:** `docs/superpowers/specs/2026-11-20-aria-phase3-spotify-design.md`

## Global Constraints

- Client ID `250bd9732a644668911a74bdd329e66e`, built in. Redirect URI exactly
  `http://127.0.0.1:43819/callback`. No client secret anywhere, ever.
- Scopes, exactly: `user-read-private user-library-read user-follow-read playlist-read-private
  playlist-read-collaborative user-read-playback-state user-modify-playback-state
  user-read-currently-playing`.
- Never ask for or store the Spotify password. Never log, `Debug`-print or display an access
  token, refresh token, authorization code or PKCE verifier: they live in `Secret`, whose
  `Debug`/`Display` print `***`.
- The refresh token goes to the system keyring (service `Aria`, account `spotify`); the access
  token stays in memory. Nothing Spotify returns is written to disk.
- No network request before sign-in or after sign-out. Hosts: `accounts.spotify.com`,
  `api.spotify.com`, and image URLs from Spotify's answers, HTTPS only.
- Public Web API only, as checked on 2026-11-20 for a Development-mode app: search `limit` ≤ 10,
  no `/artists/{id}/top-tracks`, no batch lookups, no `/playlists/{id}/tracks` (and Aria never
  reads playlist items), `product` absent from `GET /me`, `popularity`/`followers`/`label` absent.
- No `unwrap`/`expect` outside tests, no `unsafe`, no `#[allow]`. `cargo clippy --workspace
  --all-targets --locked -- -D warnings` and `cargo fmt --all --check` stay clean.
- No test talks to the real Spotify, opens a real browser or touches the real keyring.
- Every new user-visible string is a Fluent key in `locales/en`, `ru` and `hy/main.ftl` and in
  the `KEYS` list of `crates/aria-core/src/i18n/mod.rs`.
- Shell for every command (Git Bash):
  `cd /e/claude/aria; export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory
  GIT_CONFIG_VALUE_0=E:/claude/aria CARGO_TARGET_DIR=C:/Users/hixenpc/aria-target
  CMAKE="C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe"
  CMAKE_GENERATOR="Visual Studio 17 2022"`, and `-j 4` on cargo. Commit and push straight to
  `main`. cargo-deny is not installed locally; the CI `deny` job checks licences.

## Review Focus

1. **Tokens leaking through `Debug` or errors.** A `Client`, `Tokens`, `Pkce` or `SpotifyError`
   printed with `{:?}` must not contain a token, code or verifier, and a failed request's error
   must not carry the `Authorization` header. Test in Task 3 (`nothing_secret_is_ever_printed`).
2. **Port 43819 already taken.** Sign-in must give the "port busy" notice and return to signed
   out, not hang or panic. Test in Task 2 (`a_taken_port_is_reported`) and Task 6.
3. **Big libraries.** 5,000 saved tracks means 100 pages: paging must follow `next` to the end,
   and the pages must draw only the visible rows. Tests in Task 4 (`paging_follows_next`) and
   Task 10 (rows drawn with `show_rows`).
4. **What Spotify plays that is not a track.** `GET /me/player` can answer 204, or an episode, or
   a local file with no URI and no images. None may break the bar. Tests in Task 4
   (`nothing_playing_is_none`, `an_episode_or_local_file_is_shown_by_name`).
5. **A network that drops while signed in.** Polling every 2 seconds must give one notice, not
   one every poll, and recover silently when the network returns. Test in Task 6
   (`a_lost_network_gives_one_notice`).

---

### Task 1: The `aria-spotify` crate, its secrets and PKCE

**Files:**
- Create: `crates/aria-spotify/Cargo.toml`, `crates/aria-spotify/src/lib.rs`,
  `crates/aria-spotify/src/secret.rs`, `crates/aria-spotify/src/auth.rs`,
  `crates/aria-spotify/src/error.rs`
- Modify: `Cargo.toml` (workspace dependencies), `deny.toml` if needed

**Interfaces:**
- Produces:
  - `pub const CLIENT_ID: &str`, `pub const REDIRECT_PORT: u16 = 43819`,
    `pub const SCOPES: &str` (space-separated, as in Global Constraints),
    `pub fn redirect_uri(port: u16) -> String` (`http://127.0.0.1:{port}/callback`).
  - `pub struct Endpoints { pub accounts: String, pub api: String }` with
    `Endpoints::spotify()` = `https://accounts.spotify.com`, `https://api.spotify.com/v1`.
  - `pub struct Secret(String)`: `Secret::new(String)`, `expose(&self) -> &str`; `Debug` and
    `Display` print `***`; `Clone`, `PartialEq`.
  - `pub struct Pkce { pub verifier: Secret, pub challenge: String, pub state: String }`:
    `Pkce::new() -> Result<Pkce, SpotifyError>` (64 random bytes → base64url verifier of 86
    chars; 16 random bytes → base64url `state`; challenge = base64url-no-pad of SHA-256 of the
    verifier's ASCII), `Pkce::from_parts(verifier: &str, state: &str) -> Pkce`.
  - `pub fn authorize_url(endpoints: &Endpoints, pkce: &Pkce, redirect_uri: &str) -> String`:
    `{accounts}/authorize?client_id=…&response_type=code&redirect_uri=…&code_challenge_method=S256&code_challenge=…&state=…&scope=…`,
    query encoded with `form_urlencoded`.
  - `pub enum SpotifyError` (`thiserror`): `Network(String)`, `SessionEnded`, `SignInDenied`,
    `SignInTimedOut`, `SignInCancelled`, `PortBusy`, `NotAllowListed`,
    `RateLimited { retry_after: Duration }`, `QuotaExceeded { retry_after: Duration }`,
    `PremiumRequired`, `NoActiveDevice`, `Api { status: u16, message: String }`,
    `Store(String)`, `Random`, `BadAnswer(String)`.

- [ ] **Step 1: Add the crate and its dependencies.** Workspace deps: `ureq = { version = "3",
  default-features = false, features = ["rustls", "platform-verifier", "json"] }`,
  `serde_json = "1"`, `keyring = { version = "3", features = ["apple-native",
  "windows-native", "sync-secret-service", "crypto-rust"] }`, `getrandom = "0.3"`,
  `base64 = "0.22"`, `form_urlencoded = "1"`; `sha2` is already there. Take the newest release
  of each major line with `cargo search`. The crate has `[lints] workspace = true` and a
  `test-support` feature (used in Task 3). Run `cargo tree -p aria-spotify -e normal --prefix
  none --format "{p} {l}"` and compare every licence with `deny.toml`'s `allow` list. If
  `webpki-roots` or `webpki-root-certs` is pulled in, add `"CDLA-Permissive-2.0"` with a comment
  saying why (Mozilla's root certificates, a permissive data licence). Add nothing else without
  a comment.

- [ ] **Step 2: Write the failing tests** in `auth.rs`:

```rust
#[test]
fn the_challenge_is_rfc_7636s_example() {
    // RFC 7636, appendix B.
    let pkce = Pkce::from_parts("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", "s");
    assert_eq!(pkce.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
}

#[test]
fn new_verifiers_are_long_random_and_url_safe() {
    let (a, b) = (Pkce::new().unwrap(), Pkce::new().unwrap());
    assert_eq!(a.verifier.expose().len(), 86);
    assert_ne!(a.verifier.expose(), b.verifier.expose());
    assert_ne!(a.state, b.state);
    assert!(a.verifier.expose().bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
}

#[test]
fn the_authorize_url_asks_for_s256_with_the_built_in_client() {
    let pkce = Pkce::from_parts("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", "st4te");
    let url = authorize_url(&Endpoints::spotify(), &pkce, &redirect_uri(REDIRECT_PORT));
    assert!(url.starts_with("https://accounts.spotify.com/authorize?"));
    for part in ["client_id=250bd9732a644668911a74bdd329e66e", "response_type=code",
        "code_challenge_method=S256", "state=st4te",
        "redirect_uri=http%3A%2F%2F127.0.0.1%3A43819%2Fcallback",
        "code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        "scope=user-read-private+user-library-read"] {
        assert!(url.contains(part), "{part} in {url}");
    }
    assert!(!url.contains("dBjftJeZ"), "the verifier never leaves Aria");
}
```

  And in `secret.rs`: `secrets_print_as_stars` (`format!("{:?} {}", s, s) == "*** ***"`).

- [ ] **Step 3: Run them.** `cargo test -j 4 -p aria-spotify` → FAIL (items not defined).
- [ ] **Step 4: Implement** `secret.rs`, `error.rs`, `auth.rs` and `lib.rs` as in Interfaces.
- [ ] **Step 5: Run them.** `cargo test -j 4 -p aria-spotify` → 4 passed.
- [ ] **Step 6: Commit** `feat(spotify): new crate with PKCE and the authorize URL`.

### Task 2: The loopback callback

**Files:**
- Create: `crates/aria-spotify/src/callback.rs`

**Interfaces:**
- Consumes: `SpotifyError`, `Secret` (Task 1).
- Produces: `pub struct Callback`; `Callback::bind(port: u16) -> Result<Callback, SpotifyError>`
  (binds `127.0.0.1` only; `AddrInUse` → `PortBusy`); `fn port(&self) -> u16`;
  `fn wait(self, state: &str, timeout: Duration, cancel: &AtomicBool, done_page: &str)
  -> Result<Secret, SpotifyError>` returning the authorization code.

Behaviour `wait` must have: non-blocking accept polled every 50 ms so `cancel` and `timeout`
are seen; each connection read with a 2 s read timeout, at most 8 KiB; only `GET /callback?…`
counts. A request whose `state` differs is answered `400` with a one-line text page and waiting
goes on (a stray or forged request must not end the sign-in). Matching `state` with `code` →
`200` with `done_page` as `text/html; charset=utf-8`, returns the code. Matching `state` with
`error=access_denied` → `SignInDenied`; another `error` → `Api { status: 400, message }`. Any
other path → `404`, waiting goes on. Never log the request line.

- [ ] **Step 1: Write the failing tests** (each binds port 0 and plays the browser with a
  `TcpStream` from a second thread):
  `the_code_comes_back_and_the_page_is_shown` (asserts the returned code and that the response
  is `200` and contains `done_page`), `a_wrong_state_is_refused_and_waiting_goes_on` (first
  request wrong state → `400`; second right → code), `a_denied_sign_in_is_reported`
  (`error=access_denied` → `SignInDenied`), `other_paths_are_not_found` (`GET /favicon.ico` →
  `404`, then the right request still works), `it_gives_up_after_the_timeout` (timeout 200 ms →
  `SignInTimedOut` within 1 s), `cancelling_stops_it` (flag set after 100 ms →
  `SignInCancelled`), `a_taken_port_is_reported` (bind a `TcpListener` on a port, then
  `Callback::bind(that_port)` → `PortBusy`).
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-spotify callback` → FAIL.
- [ ] **Step 3: Implement** `callback.rs`; parse the query with `form_urlencoded::parse`.
- [ ] **Step 4: Run them.** → 7 passed.
- [ ] **Step 5: Commit** `feat(spotify): one-shot loopback listener for the sign-in callback`.

### Task 3: HTTP, tokens and the fake Spotify for tests

**Files:**
- Create: `crates/aria-spotify/src/http.rs`, `crates/aria-spotify/src/token.rs`,
  `crates/aria-spotify/src/fake.rs` (behind `cfg(any(test, feature = "test-support"))`),
  `crates/aria-spotify/tests/fixtures/token.json`, `token-refreshed.json`

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub struct Http` wrapping a `ureq::Agent` with a 10 s global timeout,
    `http_status_as_error(false)`, user agent `Aria/<version>`, platform-verifier TLS;
    `Http::new() -> Http`.
  - `pub struct Tokens { pub access: Secret, pub refresh: Secret, pub expires_at: Instant }`
    (`Debug` derived; the fields print `***`).
  - `pub fn exchange_code(http: &Http, endpoints: &Endpoints, code: &Secret, pkce: &Pkce,
    redirect_uri: &str) -> Result<Tokens, SpotifyError>`: `POST {accounts}/api/token`, form
    `grant_type=authorization_code, code, redirect_uri, client_id, code_verifier`.
  - `pub fn refresh(http: &Http, endpoints: &Endpoints, refresh: &Secret)
    -> Result<Tokens, SpotifyError>`: form `grant_type=refresh_token, refresh_token,
    client_id`. An answer without `refresh_token` keeps the given one. `400` with
    `"error":"invalid_grant"` → `SessionEnded`. `expires_at = now + expires_in − 60 s`.
  - `fake::FakeSpotify` (feature `test-support`): a std `TcpListener` HTTP/1.1 server on
    `127.0.0.1:0`, one thread, `Connection: close`. `FakeSpotify::start() -> FakeSpotify`,
    `fn endpoints(&self) -> Endpoints` (`http://127.0.0.1:{port}` and `…/v1`),
    `fn reply(&self, method: &str, path: &str, reply: Reply)` (queued per method+path; the last
    one repeats), `fn requests(&self) -> Vec<Seen>`; `pub struct Reply { status: u16, headers:
    Vec<(String, String)>, body: String }` with `Reply::json(status, &str)` and
    `Reply::fixture(status, name)` (reads `tests/fixtures/{name}.json`);
    `pub struct Seen { pub method, pub path, pub query: Vec<(String, String)>,
    pub authorization: Option<String>, pub body: String }`. Unscripted paths answer `404`.

Fixtures are written by hand in the shape of the examples in Spotify's Web API reference, with
made-up ids; they are not captured from real traffic.

- [ ] **Step 1: Write the failing tests** in `token.rs`:
  `the_code_is_exchanged_with_the_verifier_and_no_secret` (asserts the form holds `client_id`,
  `code_verifier`, `redirect_uri`, `grant_type=authorization_code`, and that `Seen.authorization`
  is `None`: PKCE sends no Basic header), `a_refresh_without_a_new_refresh_token_keeps_the_old`,
  `invalid_grant_means_the_session_ended`, `the_access_token_expires_a_minute_early`
  (`expires_in: 3600` → `expires_at` within 3540 s ± 2 s of now),
  `nothing_secret_is_ever_printed` (formats `Tokens`, a `Pkce` and every `SpotifyError` produced
  by a failing exchange with `{:?}`; none contains the fixture's token strings, the code or the
  verifier).
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-spotify token` → FAIL.
- [ ] **Step 3: Implement** `http.rs`, `token.rs`, `fake.rs`. A transport failure maps to
  `Network(<ureq error kind only>)`, never the request.
- [ ] **Step 4: Run them.** → 5 passed.
- [ ] **Step 5: Commit** `feat(spotify): token exchange and refresh, and a fake Spotify for tests`.

### Task 4: The Web API client and its models

**Files:**
- Create: `crates/aria-spotify/src/client.rs`, `crates/aria-spotify/src/model.rs`,
  fixtures `me.json`, `saved-tracks-1.json` … `-3.json`, `saved-albums.json`,
  `followed-artists.json`, `playlists.json`, `search.json`, `player.json`,
  `player-episode.json`, `player-local.json`, `devices.json`, `error-401.json`,
  `error-premium.json`, `error-no-device.json`, `error-not-registered.json`, `error-quota.json`

**Interfaces:**
- Consumes: Tasks 1 and 3.
- Produces (`model.rs`, all `Clone, Debug, PartialEq`):
  - `pub struct Account { pub id: String, pub name: String }` (`display_name`, else `id`).
  - `pub struct SpTrack { pub uri: Option<String>, pub name: String, pub artists: Vec<String>,
    pub album: String, pub album_uri: Option<String>, pub duration_ms: u64,
    pub image: Option<String> }` (`uri` is `None` for local files).
  - `pub struct SpAlbum { pub uri: String, pub name: String, pub artists: Vec<String>,
    pub image: Option<String> }`, `pub struct SpArtist { pub uri: String, pub name: String,
    pub image: Option<String> }`, `pub struct SpPlaylist { pub uri: String, pub name: String,
    pub owner: String, pub image: Option<String> }`.
  - `pub struct SearchResults { pub tracks: Vec<SpTrack>, pub albums: Vec<SpAlbum>,
    pub artists: Vec<SpArtist>, pub playlists: Vec<SpPlaylist> }` (Spotify can return `null`
    entries in these lists; skip them).
  - `pub struct SpDevice { pub id: Option<String>, pub name: String, pub kind: String,
    pub active: bool, pub restricted: bool, pub volume: Option<u8>, pub supports_volume: bool }`.
  - `pub enum RepeatState { Off, Context, Track }` (serde `off`/`context`/`track`).
  - `pub struct PlaybackState { pub device: Option<SpDevice>, pub item: Option<NowItem>,
    pub progress_ms: u64, pub playing: bool, pub shuffle: bool, pub repeat: RepeatState }`;
    `pub enum NowItem { Track(SpTrack), Other { name: String } }` (episodes and anything that is
    not a track).
  - `pub fn pick_image(images: &[Image]) -> Option<String>`: the smallest with `width >= 64`,
    else the widest; only `https://` URLs.
  - `pub enum PlayRequest { Context { uri: String, offset_uri: Option<String> },
    Tracks { uris: Vec<String>, start: usize }, Resume }`.
- Produces (`client.rs`): `pub struct Client` holding `Http`, `Endpoints`, `Tokens`; built with
  `Client::new(http, endpoints, tokens)`; `fn tokens(&self) -> &Tokens` (so the service can save a
  rotated refresh token). Methods, all `-> Result<_, SpotifyError>`:
  `me() -> Account`; `saved_tracks() -> Vec<SpTrack>` (`/me/tracks?limit=50`, follows `next`);
  `saved_albums() -> Vec<SpAlbum>` (`/me/albums?limit=50`); `followed_artists() -> Vec<SpArtist>`
  (`/me/following?type=artist&limit=50`, cursor `after`); `playlists() -> Vec<SpPlaylist>`
  (`/me/playlists?limit=50`); `search(query: &str) -> SearchResults`
  (`/search?q=…&type=track,album,artist,playlist&limit=10`); `player() -> Option<PlaybackState>`
  (`/me/player?additional_types=episode`, `204` → `None`); `devices() -> Vec<SpDevice>`;
  `play(device: Option<&str>, request: &PlayRequest)` (`PUT /me/player/play[?device_id=]`, body
  `{"context_uri","offset":{"uri"}}`, `{"uris","offset":{"position"}}` or none);
  `pause(device)`; `next(device)` (`POST`); `previous(device)` (`POST`);
  `seek(device, position_ms: u64)`; `volume(device, percent: u8)`;
  `shuffle(device, on: bool)`; `repeat(device, state: RepeatState)`;
  `transfer(device_id: &str, play: bool)` (`PUT /me/player`); `image(url: &str) -> Vec<u8>`
  (no `Authorization` header, `https://` only unless the URL starts with the fake's base, at
  most 2 MiB).

Every API call: `Authorization: Bearer <access>`. Refresh first when `expires_at` has passed.
`401` → refresh once and repeat once; a second `401` → `SessionEnded`. `429` with a body whose
`reason` is `QUOTA_EXCEEDED` → `QuotaExceeded { retry_after }`. Any other `429` with
`Retry-After` ≤ 5 s → sleep and repeat once, otherwise `RateLimited { retry_after }` (missing
header: 30 s). `403` with reason `PREMIUM_REQUIRED` → `PremiumRequired`. `403` whose message
contains `not registered` or `may not be registered` → `NotAllowListed`. `404` with reason
`NO_ACTIVE_DEVICE` → `NoActiveDevice`. Other non-2xx → `Api { status, message }` with Spotify's
`error.message`.

- [ ] **Step 1: Write the failing tests** against `FakeSpotify`:
  `me_uses_the_display_name`, `paging_follows_next` (three pages of 50, 50 and 7 → 107 tracks,
  three requests, each with the bearer header), `followed_artists_follow_the_cursor`,
  `search_asks_for_ten_of_four_types` (asserts the query: `limit=10`,
  `type=track,album,artist,playlist`, `q` encoded; null entries skipped),
  `nothing_playing_is_none` (204), `an_episode_or_local_file_is_shown_by_name`
  (`player-episode.json` → `NowItem::Other`; `player-local.json` → `Track` with `uri: None`,
  `image: None`), `play_sends_the_context_and_offset`, `play_sends_tracks_with_a_position`,
  `next_and_previous_are_posts`, `a_401_refreshes_once_and_repeats` (first `401`, refresh, then
  `200`; a second `401` in a row → `SessionEnded`), `a_short_retry_after_is_waited_once`
  (`429` with `Retry-After: 1`, then `200`), `a_long_retry_after_is_returned`
  (`Retry-After: 120` → `RateLimited { 120 s }`, one request), `quota_exceeded_is_its_own_error`,
  `premium_no_device_and_allow_list_errors_are_named`, `images_are_https_only`
  (`http://example.com/x.jpg` → `BadAnswer`, no request made), `pick_image_prefers_the_smallest_over_64`.
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-spotify client` → FAIL.
- [ ] **Step 3: Implement** `model.rs` (serde structs for Spotify's JSON, converted into the
  public types; accept both `items` and `tracks` on simplified playlists) and `client.rs`.
- [ ] **Step 4: Run them.** → 16 passed; `cargo clippy -j 4 -p aria-spotify --all-targets
  -- -D warnings` clean.
- [ ] **Step 5: Commit** `feat(spotify): Web API client for the library, search and playback`.

### Task 5: The token store

**Files:**
- Create: `crates/aria-spotify/src/store.rs`

**Interfaces:**
- Produces: `pub trait TokenStore: Send { fn load(&self) -> Result<Option<Secret>, SpotifyError>;
  fn save(&self, token: &Secret) -> Result<(), SpotifyError>; fn delete(&self)
  -> Result<(), SpotifyError>; fn persistent(&self) -> bool; }`;
  `pub struct KeyringStore` (`keyring::Entry::new("Aria", "spotify")`; `NoEntry` on load or
  delete is `Ok(None)` / `Ok(())`; `persistent() == true`); `pub struct MemoryStore` (a
  `Mutex<Option<Secret>>`; `persistent() == false`);
  `pub fn system_store() -> Box<dyn TokenStore>`: a `KeyringStore` if a probe
  (`Entry::new` then `get_password`, where `NoEntry` counts as working) succeeds, else a
  `MemoryStore`, logging only the error kind.

- [ ] **Step 1: Write the failing tests:** `memory_store_round_trips_and_deletes`,
  `memory_store_is_not_persistent`. The keyring itself is not touched by tests (Global
  Constraints); the hand check covers it.
- [ ] **Step 2: Run them.** → FAIL.
- [ ] **Step 3: Implement** `store.rs`. Add `libdbus-1-dev pkg-config` to the Linux `apt-get`
  line of `.github/workflows/ci.yml` and of the Linux job in `.github/workflows/release.yml`.
- [ ] **Step 4: Run them.** → 2 passed.
- [ ] **Step 5: Commit** `feat(spotify): keep the refresh token in the system keyring`.

### Task 6: `spotify_service` in the core

**Files:**
- Create: `crates/aria-core/src/spotify_service.rs`
- Modify: `crates/aria-core/Cargo.toml` (`aria-spotify`, `image`; dev: `aria-spotify` with
  `test-support`), `crates/aria-core/src/lib.rs`, the three `main.ftl`, `i18n/mod.rs` `KEYS`

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces:
  - `pub struct SpotifyDeps { pub endpoints: Endpoints, pub store: Box<dyn TokenStore>,
    pub open_browser: Box<dyn Fn(&str) -> io::Result<()> + Send>, pub redirect_port: u16,
    pub sign_in_timeout: Duration, pub search_quiet: Duration, pub fast: Duration,
    pub slow: Duration, pub after_command: Duration }` with
    `SpotifyDeps::system()` = Spotify's endpoints, `system_store()`, `open::that`, port 43819,
    5 min, 300 ms, 2 s, 15 s, 400 ms.
  - `pub enum Cadence { Fast, Slow, Off }`.
  - `pub enum SpotifyRequest { SignIn { done_page: String }, CancelSignIn, SignOut, Refresh,
    Search(String), Play(PlayRequest), TogglePlay, Pause, Next, Previous, Seek(u64), SetVolume(u8),
    SetShuffle(bool), SetRepeat(RepeatState), Device(Option<String>), ListDevices,
    Cadence(Cadence), Cover(String), Shutdown }` (`Device` is the chosen device id; the worker
    also saves it in the settings, Task 7).
  - `pub enum SpotifyAccount { SignedOut { keyring: bool }, SigningIn, SignedIn { name: String } }`.
  - `pub struct SpotifyLibrary { pub tracks: Vec<SpTrack>, pub albums: Vec<SpAlbum>,
    pub artists: Vec<SpArtist>, pub playlists: Vec<SpPlaylist>, pub loading: bool }`.
  - `pub struct SpotifySearch { pub query: String, pub results: SearchResults }`.
  - `pub struct SpotifyPlayback { pub state: PlaybackState, pub at: Instant }`.
  - `pub struct Cover { pub url: String, pub width: u32, pub height: u32, pub rgba: Vec<u8> }`
    (decoded with `image`, scaled down to at most 128 px on its longer side).
  - `pub enum SpotifyEvent { Account(SpotifyAccount), Library(Arc<SpotifyLibrary>),
    Search(Arc<SpotifySearch>), Playback(Option<Arc<SpotifyPlayback>>),
    Devices(Vec<SpDevice>), Cover(Arc<Cover>), Notice(Notice) }`.
  - `pub struct SpotifyHandle` with `send(&self, SpotifyRequest)` and `join(self, timeout)`;
    `SpotifyService::start(deps: SpotifyDeps, on_event: Box<dyn Fn(SpotifyEvent) + Send>)
    -> io::Result<SpotifyHandle>`: thread `aria-spotify`, plus thread `aria-spotify-covers` fed
    by a bounded channel of 64 URLs (a full channel drops the request; the UI asks again).

Behaviour:
- Start: `store.load()`. No token → `Account(SignedOut)`, and no request is made. A token →
  `refresh` → `me` → `SignedIn`, save a rotated refresh token, load the library
  (`Library { loading: true }` first, then the four lists) and `devices`. `SessionEnded` →
  delete the token, `SignedOut`, notice `spotify-session-ended`.
- `SignIn`: `Pkce::new`, `Callback::bind(redirect_port)`, `open_browser(authorize_url)`,
  `Account(SigningIn)`, then `wait` with a cancel flag that `CancelSignIn`/`SignOut`/`Shutdown`
  set (requests are read on while waiting: `wait` runs on a scoped helper thread). Code →
  `exchange_code` → save → as on start. `PortBusy`, `SignInDenied`, `SignInTimedOut`, a failed
  browser open each give their notice and `SignedOut`; `SignInCancelled` gives none.
- `SignOut`: `store.delete()`, drop the client, forget library, search, playback, devices;
  `Account(SignedOut)`, `Library(empty)`, `Playback(None)`, `Devices([])`. No request.
- `Search(q)`: trimmed empty → `Search` with empty results, no request. Otherwise wait
  `search_quiet` without a newer `Search`, then one request; send its results only if `q` is
  still the newest query.
- Polling: `Off` never polls; `Fast`/`Slow` poll `player()` at that interval while signed in;
  every playback command is followed by a poll after `after_command`. `Playback` is sent only
  when the state differs from the last one sent, ignoring `progress_ms` drift under 1.5 s.
- Playback commands go to the chosen device if `Device(Some(id))` was set and the device list
  still has it, else to none (Spotify's active device). `NoActiveDevice` with a chosen device →
  `transfer(id, true)` then the command once more.
- Errors to notices, at most one notice per error kind until a request succeeds again:
  `Network` → `spotify-offline`; `RateLimited`/`QuotaExceeded` → `spotify-rate-limited`
  (`seconds`) and no polling until `retry_after` has passed; `PremiumRequired` →
  `spotify-premium-required`; `NoActiveDevice` → `spotify-no-device`; `NotAllowListed` →
  `spotify-not-allow-listed` and sign out (keeping no token); `SessionEnded` as above;
  `Api` → `spotify-error` (`message`); `Store` on save → `spotify-keyring-failed` (stays signed
  in for the session).

Fluent keys (English text; Russian and Armenian written alongside, as for every key):
`spotify-session-ended = Your Spotify session has ended. Connect again in Settings.`,
`spotify-port-busy = Aria can't receive the sign-in: port 43819 on this computer is in use.`,
`spotify-sign-in-denied = Spotify sign-in was cancelled.`,
`spotify-sign-in-timed-out = Spotify sign-in took too long. Try again.`,
`spotify-browser-failed = Aria couldn't open your browser for the Spotify sign-in.`,
`spotify-offline = Aria can't reach Spotify. Check your connection.`,
`spotify-rate-limited = Spotify asked Aria to wait { $seconds } s before asking again.`,
`spotify-premium-required = Playing on Spotify devices needs Spotify Premium.`,
`spotify-no-device = No Spotify device is active. Open Spotify on a device first.`,
`spotify-not-allow-listed = This Spotify account isn't on Aria's allow-list yet. Ask the app's owner to add it in the Spotify developer dashboard.`,
`spotify-error = Spotify said: { $message }`,
`spotify-keyring-failed = Aria couldn't save the Spotify sign-in; you'll connect again next time.`,
`spotify-signed-in-page = You're connected to Aria. You can close this tab.`

- [ ] **Step 1: Write the failing tests** with `FakeSpotify`, a `MemoryStore`, `open_browser`
  that hands the URL to the test (which plays the browser against the callback), port 0
  handled by passing `redirect_port: 0` and reading the bound port from the authorize URL, and
  short durations (search quiet 50 ms, fast 100 ms, slow 1 s, after-command 20 ms):
  `signed_out_at_start_makes_no_request` (the fake sees zero requests in 300 ms),
  `a_stored_token_signs_in_and_loads_the_library`, `sign_in_saves_the_refresh_token`,
  `sign_out_deletes_the_token_and_forgets_everything`,
  `cancelling_sign_in_returns_to_signed_out_quietly`, `a_busy_port_gives_its_notice`,
  `search_waits_for_quiet_and_drops_stale_answers` (three searches 10 ms apart → one request,
  for the last), `cadence_off_stops_polling`, `fast_polls_more_than_slow`,
  `a_command_is_followed_by_a_poll`, `the_chosen_device_is_used_and_woken`
  (`NoActiveDevice` → a transfer, then the command again), `a_lost_network_gives_one_notice`
  (five failing polls → one `spotify-offline`; then success; then a failure → a second notice),
  `a_long_rate_limit_pauses_polling`, `not_allow_listed_signs_out`,
  `covers_are_decoded_and_scaled` (a 300×300 PNG fixture → `Cover` 128×128).
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-core spotify_service` → FAIL.
- [ ] **Step 3: Implement** `spotify_service.rs` and the keys. The service loop uses
  `crossbeam_channel::select!` with `recv_timeout`-style deadlines for the next poll and the
  search.
- [ ] **Step 4: Run them.** → 15 passed; `cargo test -j 4 -p aria-core i18n` passes with the
  new keys.
- [ ] **Step 5: Commit** `feat(core): Spotify service with sign-in, library, search and polling`.

### Task 7: The backend routes the player to Spotify or to Aria

**Files:**
- Modify: `crates/aria-core/src/command.rs`, `event.rs`, `runtime.rs`,
  `settings/mod.rs`, `crates/aria-app/src/main.rs`

**Interfaces:**
- Consumes: Task 6.
- Produces:
  - `Command::Spotify(SpotifyRequest)` (the UI never sends `Cadence` or `Shutdown`; the worker
    ignores them from the UI) and `Command::WindowShown(bool)`.
  - `pub enum ActivePlayer { Local, Spotify }` in `event.rs`; `Event::ActivePlayer(ActivePlayer)`
    and `Event::Spotify(SpotifyEvent)`.
  - `BackendServices.spotify: Option<SpotifyDeps>` (`None` in existing tests;
    `Some(SpotifyDeps::system())` in `main.rs`).
  - `Settings.spotify_device: Option<String>`, sanitized to `None` when empty or over 128
    characters. No schema change (the struct is `#[serde(default)]`).

Rules in the worker:
- `active` starts `Local`. `Command::Spotify(Play(_))` → `active = Spotify`, and if the local
  player is playing, pause it. `PlayContext` and `OpenPaths` → `active = Local`, and if the last
  `SpotifyPlayback` was playing, send `SpotifyRequest::Pause` (never `TogglePlay`, which could
  start Spotify if the poll is stale).
- `TogglePlay`, `Next`, `Previous`, `Seek(ms)` go to Spotify while `active == Spotify`, else to
  the local player as today.
- A `Playback` event showing Spotify playing while the local player is not playing makes
  `active = Spotify` (you started it on your phone: the bar follows it).
- `Event::ActivePlayer` is sent whenever `active` changes, and once at start.
- Cadence: `Off` while the window is not shown, else `Fast` if `active == Spotify`, else `Slow`.
  Sent to the service whenever it changes.
- `Command::Spotify(Device(id))` also writes `spotify_device` into the settings (through the
  same path as `UpdateSettings`) and `SignOut` clears it.
- At start the service gets `Device(settings.spotify_device)`. Shutdown sends
  `SpotifyRequest::Shutdown` and joins with the library's timeout.

- [ ] **Step 1: Write the failing tests** in `runtime.rs`'s test module, with a
  `SpotifyDeps` pointed at a `FakeSpotify` signed in through a `MemoryStore` holding a token:
  `spotify_play_pauses_local_and_shows_spotify`, `local_play_pauses_spotify_and_shows_local`,
  `transport_goes_to_the_player_shown`, `playback_started_elsewhere_takes_the_bar`,
  `a_hidden_window_stops_polling`, `choosing_a_device_is_saved_in_the_settings`,
  `signing_out_clears_the_device`, `without_spotify_deps_nothing_changes` (existing behaviour,
  `Command::Spotify` answered with no event and no panic). In `settings`:
  `spotify_device_is_sanitized`.
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-core runtime` → FAIL.
- [ ] **Step 3: Implement** the commands, events and worker rules; wire `main.rs`.
- [ ] **Step 4: Run them**, then the whole workspace: `cargo test -j 4 --workspace --locked`
  → all pass (1142 before this plan, plus the new ones).
- [ ] **Step 5: Commit** `feat(core): route the player bar to Spotify or to Aria`.

### Task 8: Spotify state in the app, and the Settings card

**Files:**
- Create: `crates/aria-app/src/spotify.rs` (UI state and the cover textures),
  `crates/aria-app/src/ui/pages/settings/spotify.rs`
- Modify: `crates/aria-app/src/app.rs`, `crates/aria-app/src/lib.rs`,
  `crates/aria-app/src/ui/pages/mod.rs`, `crates/aria-app/src/ui/pages/settings/mod.rs`,
  `.ftl` files, `KEYS`

**Interfaces:**
- Consumes: Task 7.
- Produces:
  - `pub struct SpotifyUi { pub account: SpotifyAccount, pub library: Arc<SpotifyLibrary>,
    pub search: Arc<SpotifySearch>, pub search_text: String, pub playback:
    Option<Arc<SpotifyPlayback>>, pub devices: Vec<SpDevice>, pub active: ActivePlayer }` with
    `fn apply(&mut self, event: SpotifyEvent)` and `fn signed_in(&self) -> bool`.
  - `pub struct RemoteCovers` (`url → TextureHandle`, at most 400; the oldest is dropped past
    that): `fn get(&mut self, url: &str, out: &mut Vec<PageAction>) -> Option<&TextureHandle>`
    (asks once per URL with `PageAction::Spotify(SpotifyRequest::Cover(url))`) and
    `fn insert(&mut self, ctx: &egui::Context, cover: &Cover)`.
  - `PageCtx.spotify: &'a SpotifyUi`, `PageCtx.covers: &'a mut RemoteCovers`;
    `PageAction::Spotify(SpotifyRequest)`. The app sends `Command::Spotify` for it; for
    `SignIn` it fills `done_page` with `spotify-signed-in-page`.
  - The app sends `Command::WindowShown(!minimized)` when
    `ctx.input(|i| i.viewport().minimized)` changes, and `true` at the first frame.

The Settings **Spotify** card, placed after Library: signed out → a "Connect Spotify" button
and the caption `settings-spotify-allow-list`; signing in → "Waiting for the browser…" and a
"Cancel" button; signed in → "Connected as {name}", a "Device" combo (first entry "The active
device", then `devices` by name, restricted ones disabled), "Refresh library" and a
"Disconnect" button. Disconnect asks first with the existing confirm dialog. If the store is
not persistent, the caption `settings-spotify-no-keyring` is shown.

Keys: `settings-spotify = Spotify`, `settings-spotify-connect = Connect Spotify`,
`settings-spotify-waiting = Waiting for the browser…`, `settings-spotify-cancel = Cancel`,
`settings-spotify-connected = Connected as { $name }`, `settings-spotify-device = Play on`,
`settings-spotify-active-device = The active device`,
`settings-spotify-refresh = Refresh library`, `settings-spotify-disconnect = Disconnect`,
`settings-spotify-disconnect-confirm = Disconnect Spotify? Aria forgets the sign-in and your Spotify library.`,
`settings-spotify-allow-list = While Aria's Spotify app is in development mode, only accounts its owner has added can connect (up to 5).`,
`settings-spotify-no-keyring = This system has no keyring, so you'll connect again each time Aria starts.`,
`settings-spotify-premium = Playing on your devices needs Spotify Premium.`

- [ ] **Step 1: Write the failing tests** (kittest, in `settings/mod.rs`'s tests and
  `crates/aria-app/tests/ui_settings.rs`): `connect_sends_sign_in_with_the_page_text`,
  `signing_in_shows_cancel`, `signed_in_shows_the_name_and_devices`,
  `choosing_a_device_sends_it`, `disconnect_asks_first`, `the_no_keyring_caption_shows`; in
  `app.rs`: `spotify_events_reach_the_ui_state`, `minimizing_tells_the_backend`,
  `covers_are_asked_for_once`.
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-app` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them.** → pass; whole workspace passes.
- [ ] **Step 5: Commit** `feat(app): Spotify card in Settings`.

### Task 9: The player bar in Spotify mode

**Files:**
- Modify: `crates/aria-app/src/ui/layout/player_bar.rs`, `crates/aria-app/src/app.rs`, `.ftl`,
  `KEYS`
- Test: `crates/aria-app/tests/ui_player.rs`

**Interfaces:**
- Consumes: Task 8.
- Produces: `PlayerBar.spotify: Option<&'a SpotifyPlayback>` (set by the app while
  `active == Spotify`), `PlayerBar.spotify_devices: Option<&'a [SpDevice]>` (set while signed
  in), `PlayerAction::SpotifyDevice(Option<String>)`; `pub fn spotify_position_ms(playback:
  &SpotifyPlayback, now: Instant) -> u64` (`progress_ms` plus the time since `at` while playing,
  never past the duration).

In Spotify mode: title and artists from `NowItem` (`Other` shows its name and "Spotify"),
covers from `RemoteCovers`; the line under the title reads `player-spotify-on` with the device
name; play/pause, next, previous, seek, shuffle and repeat send their `SpotifyRequest`s through
the app (`CycleRepeat`: off → context → track → off); the volume shows the device's volume and
sends `SetVolume` on release only, disabled if the device does not support volume; mute, the
heart and the sleep timer are disabled with the tooltip `player-spotify-not-here`. The devices
button is enabled while signed in and opens a popup listing "The active device" and the devices
(the chosen one ticked); choosing sends `PlayerAction::SpotifyDevice`, which the app turns into
`SpotifyRequest::Device`. Signed out, it keeps its tooltip, now `player-devices-connect`.
`DEVICES_PHASE` goes.

Keys: `player-spotify-on = On { $device }`, `player-spotify-not-here = Not for Spotify playback`,
`player-devices-connect = Connect Spotify in Settings to choose a device`.

- [ ] **Step 1: Write the failing tests:** `spotify_position_moves_while_playing_and_stops_at_the_end`,
  `the_bar_shows_spotifys_track_and_device`, `transport_in_spotify_mode_sends_spotify_requests`,
  `volume_is_sent_on_release_only`, `heart_mute_and_sleep_are_disabled_for_spotify`,
  `the_devices_popup_lists_devices_and_sends_the_choice`, `an_episode_shows_its_name`.
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-app --test ui_player` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them.** → pass; whole workspace passes.
- [ ] **Step 5: Commit** `feat(app): the player bar controls Spotify`.

### Task 10: The Search, Your Library, Albums and Artists pages

**Files:**
- Create: `crates/aria-app/src/ui/widgets/spotify_row.rs`
- Modify: `crates/aria-app/src/ui/pages/search.rs`, `library.rs`, `albums.rs`, `artists.rs`,
  `crates/aria-app/src/nav.rs` (`phase()` returns `None` for the four routes), `.ftl`, `KEYS`
- Test: `crates/aria-app/tests/ui_spotify.rs`

**Interfaces:**
- Consumes: Tasks 8 and 9.
- Produces: `pub fn spotify_row(ui: &mut Ui, ctx: &mut PageCtx<'_>, image: Option<&str>,
  title: &str, subtitle: &str) -> Response` (a 48 px row: 40 px cover or placeholder, title,
  muted subtitle; a click on it is the response's click).

Play rules (all `PageAction::Spotify(SpotifyRequest::Play(…))`):
- A search track → `Context { uri: album_uri, offset_uri: track uri }`; a track with no album
  URI → `Tracks { uris: [uri], start: 0 }`. Tracks with `uri: None` are drawn muted and do not
  play.
- An album, an artist or a playlist → `Context { uri, offset_uri: None }`.
- A Liked Songs track at index `i` → `Tracks { uris: the playable URIs from i, at most 100,
  start: 0 }`.

Pages, each with its heading: signed out → an empty state with `spotify-page-signed-out` and a
"Connect Spotify" button (sends `SignIn`); `library.loading` with nothing yet → `spotify-loading`.
**Search**: a text field (`spotify-search-hint`) bound to `search_text`; each change sends
`Search(text)`; results in four sections (Songs, Albums, Artists, Playlists) with
`spotify-search-none` when all are empty for a non-empty query. **Your Library**: two sections,
Liked Songs (count in the heading) and Playlists. **Albums**, **Artists**: one list each, empty
→ `spotify-empty`. Long lists draw with `ScrollArea::show_rows`.

Keys: `spotify-page-signed-out = Connect Spotify to see your music here.`,
`spotify-loading = Loading your Spotify library…`, `spotify-search-hint = Search Spotify`,
`spotify-search-none = Nothing on Spotify matches that.`, `spotify-songs = Songs`,
`spotify-albums = Albums`, `spotify-artists = Artists`, `spotify-playlists = Playlists`,
`spotify-liked = Liked Songs ({ $count })`, `spotify-empty = Nothing here yet.`,
`spotify-by = by { $owner }`.

- [ ] **Step 1: Write the failing tests:** `signed_out_pages_offer_to_connect` (all four),
  `typing_in_search_sends_the_query`, `a_search_track_plays_in_its_album`,
  `an_album_artist_or_playlist_plays_as_a_context`,
  `a_liked_song_plays_with_the_ones_after_it` (150 liked → 100 URIs from the clicked one),
  `a_local_file_does_not_play`, `a_long_list_draws_only_what_is_visible` (5,000 liked songs:
  the harness finds fewer than 100 row labels), `no_results_says_so`,
  `the_four_routes_are_no_longer_pending`.
- [ ] **Step 2: Run them.** `cargo test -j 4 -p aria-app --test ui_spotify` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run them.** → pass; whole workspace passes; clippy clean.
- [ ] **Step 5: Commit** `feat(app): Spotify search and library pages`.

### Task 11: Docs, version 0.3.0 and the hand check

**Files:**
- Modify: `Cargo.toml` (`version = "0.3.0"`), `Cargo.lock`, `CHANGELOG.md`, `README.md`,
  `docs/PRIVACY.md`, `docs/ROADMAP.md`, `THIRD_PARTY_NOTICES.md`, `packaging/windows/aria.iss`
  (the example version in its comments)
- Create: `docs/testing/0.3.0-manual-check.md`

- [ ] **Step 1: PRIVACY** gets a "Spotify" section: nothing is sent until you connect; then
  requests to `accounts.spotify.com`, `api.spotify.com` and Spotify's image servers, with your
  access token; what is kept (the refresh token in Windows Credential Manager / macOS Keychain /
  the Secret Service, under `Aria` / `spotify`; nothing else on disk); how to remove it
  (Settings, Spotify, Disconnect; or delete that credential); that Aria plays no Spotify audio
  and records nothing. The line "Aria sends nothing over the network" elsewhere in PRIVACY and
  README is changed to say "unless you connect Spotify".
- [ ] **Step 2: README, ROADMAP, CHANGELOG [0.3.0]:** what Phase 3 does, that it needs Premium
  for playback, the five-account allow-list while in development mode, and what is not in
  0.3.0 (the spec's last section). THIRD_PARTY_NOTICES lists the new crates and their licences.
- [ ] **Step 3: The hand check** `docs/testing/0.3.0-manual-check.md`: connect (browser opens,
  page says connected, Settings shows the name); restart Aria (still connected, no browser);
  library pages fill; search; play an album on the phone and on the desktop app; pause, next,
  previous, seek, volume, shuffle, repeat from the bar; start a local track (Spotify pauses);
  start Spotify (local pauses); minimize for a minute (no polling: the log at debug shows none);
  pull the network cable (one notice; it recovers); disconnect (the credential is gone from
  Credential Manager); an account not on the allow-list (its notice).
- [ ] **Step 4: Verify.** `cargo test -j 4 --workspace --locked`, `cargo clippy -j 4 --workspace
  --all-targets --locked -- -D warnings`, `cargo fmt --all --check`, `cargo build -j 4 --release
  --locked` → all pass. Push, and wait for CI (Windows, macOS, Linux, deny) to go green; a
  `deny` failure on a licence is fixed in `deny.toml` with a comment, not by dropping the check.
- [ ] **Step 5: Commit** `docs: 0.3.0 Spotify` and push. Tagging and publishing wait for the
  user's yes.
