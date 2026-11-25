# Aria 0.3.5 Update Checker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Aria finds a newer GitHub release once a day, but only if the user has said yes. On an installed
Windows Aria it downloads the installer and checks its checksum, then offers "Restart to update". Everywhere
else it only says a new version is out.

**Architecture:**
- A new crate `aria-update` holds the parts that don't depend on Aria:
  - reading GitHub's release answer, comparing versions, reading `SHA256SUMS.txt`;
  - deciding whether this is an installed Aria;
  - a small `ureq` client with a size cap.
- `aria-core/src/update_service.rs` runs a worker thread, the same way the Spotify service does. It handles
  the timing, the download, the `.sha256` sidecar file, cleanup, and the check made just before installing.
- The runtime passes `Command::Update` and `Event::Update` between the app and that worker. It also passes
  settings changes to the worker.
- The app shows:
  - the consent card;
  - the Settings "Updates" row;
  - a sidebar button.
- Once the backend has shut down, the app starts the installer from `on_exit`.

**Tech Stack:** Rust 1.99.1, ureq 3.5.2 (rustls, platform verifier), sha2 0.10, hex 0.4, semver 1,
serde_json, crossbeam-channel, egui/eframe 0.37 with kittest, Fluent, and Inno Setup 6.

**Spec:** docs/superpowers/specs/2026-11-25-aria-0-3-5-update-checker-design.md

## Global Constraints

**Code rules**
- No `unsafe`. No `unwrap`/`expect` outside tests. No `#[allow]`. Clippy must pass with `-D warnings`.
- No new crates in `Cargo.lock`. `sha2 0.10.9`, `hex 0.4.3` and `semver 1.0.28` are already in it; add them as
  workspace dependencies with those versions.

**Network rules**
- No network request at all unless one of these is true:
  - `update_checks == On`;
  - the user clicked "Check for updates" (`UpdateRequest::CheckNow`).
- Every request sends the User-Agent `Aria` exactly, with no version. API calls also send
  `Accept: application/vnd.github+json`.
- API: `https://api.github.com/repos/Hixenov/Aria/releases/latest`.
- Asset URLs must start with `https://github.com/Hixenov/Aria/releases/download/`. Anything else is refused
  as `UpdateError::Answer`.
- File names:
  - installer: `Aria-{version}-setup.exe`;
  - checksums: `SHA256SUMS.txt`;
  - sidecar: `Aria-{version}-setup.exe.sha256`, which holds 64 lowercase hex characters.
- Size caps: installer 200 MiB (`200 * 1024 * 1024`), `SHA256SUMS.txt` 64 KiB, API answer 1 MiB.
- Timing: first check 10 s after start. Next one 24 h after the last check. Retry after 1 h when
  `SHA256SUMS.txt` is missing.
- A tag counts only if it parses as semver after taking off a leading `v`, has no pre-release part, and is
  higher than the running version.

**Install rules**
- "Installed" is `unins000.exe` beside the running exe, no `AriaData` folder there, and Windows. Anything else
  is `TellOnly`.
- Installer arguments: `/SILENT /SUPPRESSMSGBOXES /NORESTART /RELAUNCH=1`.
- Downloads go to `<cache_dir>/updates/`. They are written as `<name>.part` and renamed only after the
  checksum matches.

**Tests**
- Tests never open the microphone. They never start an installer or touch the installed Aria or
  `%APPDATA%\Aria`. They never reach the real network: they use the local fake (`aria_spotify::fake`).

**Copy**
- All user-facing strings are in en, ru and hy, and listed in `KEYS` in `crates/aria-core/src/i18n/mod.rs`.

## Review Focus

These are the five most likely ways this goes wrong. Each has a test in the task that owns the code.

1. **A pre-release or odd tag is published as "latest"** (`v0.4.0-beta.1`, `0.3.5`, `latest`).
   - Aria must not offer it: no pre-release part, and the leading `v` is optional.
   - Test: `odd_tags_are_not_offered`, in Task 1.
2. **The download was cut short, or is bigger or smaller than the API said.**
   - Aria must never rename the `.part` file, and the status becomes "Couldn't check".
   - Test: `a_short_download_is_not_kept`, in Task 3.
3. **Automatic checks are off, or not asked yet.**
   - Zero requests, even after the start delay has passed.
   - Test: `no_request_until_the_user_says_yes`, in Task 3.
4. **The installer in the cache was changed or damaged between the download and Restart.**
   - Install is refused, the file is deleted, and a notice is shown.
   - Test: `a_changed_installer_is_not_started`, in Task 3.
5. **Aria restarts several times within a day.**
   - The stored `update_checked_at` prevents a second check before 24 h.
   - Test: `a_recent_check_is_not_repeated`, in Task 3.

---

### Task 1: The `aria-update` crate (parsing, versions, checksums, install kind, HTTP client)

**Files:**
- Create:
  - `crates/aria-update/Cargo.toml`
  - `crates/aria-update/src/lib.rs`
  - `crates/aria-update/src/release.rs`
  - `crates/aria-update/src/client.rs`
  - `crates/aria-update/tests/fixtures/release.json`
- Modify:
  - `Cargo.toml`: add the workspace member and the deps `aria-update`, `sha2 = "0.10.9"`, `hex = "0.4.3"`,
    `semver = "1.0.28"`.

**Interfaces:**
- Produces (all `pub` from `aria_update`):
  - `struct Endpoints { latest: String, download_prefix: String }`, with `Endpoints::github()` giving the two
    production URLs above.
  - `struct Asset { name: String, size: u64, url: String }`
  - `struct Release { version: semver::Version, page: String, installer: Option<Asset>, sums: Option<Asset> }`
  - `fn parse_release(body: &str, endpoints: &Endpoints) -> Result<Release, UpdateError>`
    - Finds the installer by its exact name for the tag's version.
    - A tag that is not plain semver gives `Err(UpdateError::NotARelease)`.
  - `fn is_newer(candidate: &Version, running: &Version) -> bool`
  - `fn installer_name(version: &Version) -> String`
  - `fn sha256_for(sums: &str, file_name: &str) -> Option<[u8; 32]>`
    - Reads `sha256sum` lines: `<64 hex>  <name>` or `<64 hex> *<name>`.
  - `fn sha256_file(path: &Path) -> io::Result<[u8; 32]>`
  - `enum InstallKind { Installed, TellOnly }`
  - `fn install_kind(exe_dir: &Path, windows: bool) -> InstallKind`
  - `enum UpdateError { Offline, Answer, NotARelease, TooBig, Checksum, Disk(io::ErrorKind) }`
    - Implements `Display` without URLs. Derives `Clone, Debug, PartialEq, Eq`.
  - `struct Client`:
    - `Client::new(endpoints: Endpoints) -> Client`
    - `latest(&self) -> Result<Release, UpdateError>`
    - `text(&self, asset: &Asset, cap: u64) -> Result<String, UpdateError>`
    - `download(&self, asset: &Asset, dest: &Path, cap: u64, progress: &mut dyn FnMut(u64, u64)) -> Result<(), UpdateError>`
      - Writes `dest`.
      - Returns `TooBig` before any request when `asset.size > cap`.
      - Returns `Answer` and removes `dest` when the byte count differs from `asset.size`.
- Consumes: nothing. The dev-dependency `aria-spotify` with `features = ["test-support"]` provides
  `aria_spotify::fake::{FakeSpotify, Reply}`.

- [ ] **Step 1: Write the failing tests** in `release.rs` and `client.rs` `#[cfg(test)]` modules:

`release.json` is a trimmed GitHub answer:
- `tag_name` is `"v0.3.6"`, and `html_url` is `".../releases/tag/v0.3.6"`.
- `assets` holds `Aria-0.3.6-setup.exe` (size 11) and `SHA256SUMS.txt`.
- Their `browser_download_url` values are `{base}/Hixenov/Aria/releases/download/v0.3.6/<name>`.

```rust
#[test] fn the_release_is_read() {
    // endpoints with download_prefix = "{base}/Hixenov/Aria/releases/download/" after substituting base "http://x"
    let r = parse_release(&fixture_with_base("http://x"), &test_endpoints("http://x")).unwrap();
    assert_eq!(r.version, Version::new(0, 3, 6));
    assert_eq!(r.installer.as_ref().map(|a| (a.name.as_str(), a.size)), Some(("Aria-0.3.6-setup.exe", 11)));
    assert!(r.sums.is_some());
}
#[test] fn odd_tags_are_not_offered() {
    for tag in ["v0.4.0-beta.1", "latest", ""] { assert_eq!(parse_release(&with_tag(tag), &ep()), Err(UpdateError::NotARelease)); }
    assert_eq!(parse_release(&with_tag("0.3.6"), &ep()).unwrap().version, Version::new(0, 3, 6));
}
#[test] fn an_asset_elsewhere_is_refused() { /* browser_download_url = "https://evil.example/Aria-0.3.6-setup.exe" */ assert_eq!(parse_release(&body, &ep()), Err(UpdateError::Answer)); }
#[test] fn only_higher_versions_are_newer() {
    let run = Version::new(0, 3, 5);
    assert!(is_newer(&Version::new(0, 3, 6), &run));
    assert!(!is_newer(&Version::new(0, 3, 5), &run));
    assert!(!is_newer(&Version::new(0, 3, 4), &run));
}
#[test] fn the_checksum_line_is_found_by_exact_name() {
    let sums = "aa..(64)  Aria-0.3.6-setup.exe\nbb..(64) *aria.exe\n";
    assert_eq!(sha256_for(sums, "Aria-0.3.6-setup.exe"), Some([0xaa; 32]));
    assert_eq!(sha256_for(sums, "aria.exe"), Some([0xbb; 32]));
    assert_eq!(sha256_for(sums, "Aria-0.3.6-setup"), None);
    assert_eq!(sha256_for("zz  Aria-0.3.6-setup.exe", "Aria-0.3.6-setup.exe"), None);
}
#[test] fn only_an_inno_install_on_windows_is_installed() {
    // tempdir with unins000.exe → Installed when windows; TellOnly when !windows;
    // with unins000.exe and an AriaData folder → TellOnly; empty dir → TellOnly
}
// client.rs, against FakeSpotify:
#[test] fn the_client_sends_a_bare_user_agent() { /* latest() → request seen with header user-agent == "Aria" and accept == "application/vnd.github+json" */ }
#[test] fn a_download_too_big_is_refused_before_asking() { /* asset.size = cap + 1 → Err(TooBig), fake.requests() is empty */ }
#[test] fn a_download_is_written_and_reported() { /* 11 bytes, dest has them, progress last call == (11, 11) */ }
#[test] fn an_unreachable_github_is_offline() { /* fake.set_offline(true) → Err(Offline) */ }
```

If the fake's `Seen` does not record headers yet, add a `headers` field to it in
`crates/aria-spotify/src/fake.rs`. That is test support only.

- [ ] **Step 2: Run, to see them fail**

`cargo test -p aria-update -j 4`. Expected: compile errors for the missing items, then the tests fail.

- [ ] **Step 3: Implement**

- `ureq` agent:
  - `timeout_global` 30 s for API and text, and none for the download (use `timeout_recv_body` 60 s
    instead);
  - `http_status_as_error(false)`;
  - user agent `"Aria"`;
  - platform verifier.
- A non-2xx answer gives `Answer`.
- A transport error gives `Offline`.
- Bodies are read with `.with_config().limit(cap)`.
- The download streams to `dest` in 64 KiB chunks and calls `progress(done, total)`.

- [ ] **Step 4: Run, to see them pass**

`cargo test -p aria-update -j 4`, then `cargo clippy -p aria-update --all-targets -j 4 -- -D warnings`.
Expected: all pass, no warnings.

- [ ] **Step 5: Commit** `feat(update): release reading, checksums and a capped GitHub client`

### Task 2: Settings fields

**Files:**
- Modify: `crates/aria-core/src/settings/mod.rs`, which owns the `Settings` struct, its `Default` impl and its
  tests.

**Interfaces:**
- Produces:
  - `pub enum UpdateChecks { NotAsked, On, Off }`
    - `#[serde(rename_all = "kebab-case")]`, default `NotAsked`.
    - An unknown value reads as `NotAsked`, through a lenient deserializer like the others in this file.
  - `Settings.update_checks: UpdateChecks`
  - `Settings.update_checked_at: Option<u64>`: Unix seconds of the last finished check.

- [ ] **Step 1: Failing tests**
  - `update_checks_start_not_asked`: `Settings::default().update_checks == UpdateChecks::NotAsked` and
    `update_checked_at == None`.
  - `update_settings_round_trip`: `On` with `Some(1_700_000_000)` goes through `toml::to_string` and `parse`.
  - `an_unknown_update_choice_reads_as_not_asked`: `update_checks = "sometimes"` gives `NotAsked`, and the
    rest of the file is kept.
- [ ] **Step 2:** `cargo test -p aria-core -j 4 settings::`. Expected: FAIL to compile.
- [ ] **Step 3:** Add the enum and the two fields to `Settings` and its `Default`.
- [ ] **Step 4:** The same command. Expected: PASS.
- [ ] **Step 5: Commit** `feat(settings): update check choice and last check time`

### Task 3: The update service (`aria-core/src/update_service.rs`)

**Files:**
- Create: `crates/aria-core/src/update_service.rs`
- Modify: `crates/aria-core/src/lib.rs` (`pub mod update_service;`) and `crates/aria-core/Cargo.toml`
  (dependency `aria-update`).

**Interfaces:**
- Consumes:
  - everything Task 1 produces;
  - `UpdateChecks` from Task 2;
  - the existing `join_bounded` and `DoneGuard` helpers the Spotify service uses. Move them to a shared
    module if they are private.
- Produces:
  - `pub struct UpdateDeps`, with these fields:

    | Field | Type | Production value |
    |---|---|---|
    | `endpoints` | `Endpoints` | `Endpoints::github()` |
    | `running` | `semver::Version` | `CARGO_PKG_VERSION` |
    | `cache_dir` | `Option<PathBuf>` | |
    | `kind` | `InstallKind` | |
    | `start_delay` | `Duration` | 10 s |
    | `interval` | `Duration` | 24 h |
    | `retry` | `Duration` | 1 h |
    | `now` | `Arc<dyn Fn() -> u64 + Send + Sync>` | `SystemTime` |

    `UpdateDeps::system(cache_dir: Option<PathBuf>, kind: InstallKind) -> UpdateDeps` fills these in.
  - `pub enum UpdateRequest`:
    - `Settings { checks: UpdateChecks, checked_at: Option<u64> }`
    - `CheckNow`
    - `Install`
    - `Shutdown`
  - `pub enum UpdateStatus`:
    - `Idle`, `Checking`, `UpToDate`
    - `NotReady(Version)`
    - `Downloading { version: Version, percent: u8 }`
    - `Ready(Version)`
    - `Available { version: Version, page: String }`
    - `Failed(UpdateError)`
  - `pub enum UpdateEvent`:
    - `Status(UpdateStatus)`
    - `Checked(u64)`: the app stores it in `update_checked_at`.
    - `Install(PathBuf)`: verified, and ready to start.
    - `Notice(Notice)`
  - `pub struct UpdateService;` with
    `UpdateService::start(deps, on_event: Box<dyn Fn(UpdateEvent) + Send>) -> io::Result<UpdateHandle>`.
  - `UpdateHandle::send(&self, UpdateRequest)` and `UpdateHandle::join(self, Duration) -> bool`.

**Behavior** (which the tests pin):

- **At start,** in `cache_dir/updates`:
  - Delete every `*.part` file.
  - Delete every `Aria-X-setup.exe` (and its `.sha256`) whose `X` is not newer than `running`.
  - If a newer installer is there, its sidecar matches `sha256_file`, and `kind == Installed`, emit
    `Status(Ready(X))` with no request.
  - Otherwise, delete that pair.
- **Due:**
  - Nothing is due until the first `Settings` arrives with `checks == On`.
  - Then the check is due at whichever is later: start + `start_delay`, or `checked_at` + `interval`. The
    second is measured with `now()`.
  - `Off` or `NotAsked` cancels the timer.
  - `CheckNow` always checks at once, whatever the setting.
- **A check:**
  1. Emit `Status(Checking)` and call `latest()`.
  2. If the release is not newer, emit `UpToDate` and `Checked(now)`.
  3. If it is newer and `TellOnly`, emit `Available { version, page }` and `Checked(now)`.
  4. If it is newer and `Installed` but has no sums or no installer asset, emit `NotReady(v)` and retry
     after `retry`. Do not emit `Checked` here, so a restart checks again.
  5. Otherwise:
     - fetch the sums (64 KiB cap) and find the installer's line, or give `Answer`;
     - download to `.part`, emitting `Downloading` only when the percent changes;
     - hash the file. If it does not match, delete it and give `Checksum`;
     - write the sidecar, rename the `.part`, then emit `Ready(v)`, `Checked(now)` and
       `Notice(Info, "notice-update-ready")` with arg `version`.
  - Any `Err(e)` emits `Failed(e)` and sets the next due time to `retry`.
- **`Install`:**
  - Hash the ready installer again and compare it with the sidecar.
  - If they match, emit `Install(path)`.
  - Otherwise, delete both, emit `Notice(Warning, "notice-update-broken")`, set `Status(Idle)` and make a
    check due at once (it runs only if `On`).
- **When `Ready`,** later checks are skipped until the installer is used.

- [ ] **Step 1: Failing tests** (fake server, `tempfile` dir, short durations: `start_delay` 0,
  `retry` 50 ms):
  - `no_request_until_the_user_says_yes`
    - Start, then send `Settings{NotAsked, None}`. Wait 300 ms.
    - Then send `Settings{Off, None}` and wait again.
    - `fake.requests()` is empty both times.
  - `on_checks_after_the_delay_and_reports_up_to_date`
    - The release is `v0.3.5` and `running` is 0.3.5.
    - Events are `Checking`, `UpToDate` and `Checked(now)`.
  - `a_recent_check_is_not_repeated`
    - `checked_at = now − 3600` with `interval` 24 h. No request within 300 ms.
  - `check_now_works_with_checks_off`
    - One request, then `UpToDate`.
  - `a_newer_release_downloads_verifies_and_is_ready`
    - The installer is 11 bytes and the sums line is correct.
    - At the end, the dir holds `Aria-0.3.6-setup.exe` and `.sha256`, with no `.part`.
    - The events end with `Ready(0.3.6)`, `Checked` and the `notice-update-ready` Notice.
  - `a_short_download_is_not_kept`
    - The fake returns 5 bytes while the API says 11.
    - Result: `Failed(Answer)`, and the dir is empty.
  - `a_wrong_checksum_is_not_kept`
    - Result: `Failed(Checksum)`, and the dir is empty.
  - `missing_sums_means_not_ready_and_a_retry`
    - Result: `NotReady(0.3.6)`, then a second `latest` request after the retry. No `Checked`.
  - `tell_only_never_downloads`
    - `kind = TellOnly`. Result: `Available{0.3.6, page}`, with only the API request seen.
  - `a_ready_installer_from_before_is_offered_without_asking`
    - A valid pair is already in the dir. Result: `Ready` with zero requests.
  - `old_installers_and_parts_are_deleted_at_start`
    - The dir holds `Aria-0.3.5-setup.exe`, its sidecar, and `x.part`. All are gone after start.
  - `install_sends_the_verified_path`
    - Result: `Install(dir/Aria-0.3.6-setup.exe)`.
  - `a_changed_installer_is_not_started`
    - One byte of the ready file is changed, then `Install` is sent.
    - Result: no `Install` event, the `notice-update-broken` Notice, and both files deleted.
- [ ] **Step 2:** `cargo test -p aria-core -j 4 update_service`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** a single thread that loops on `requests.recv_timeout(until_due)`, in the shape of
  the Spotify service.
- [ ] **Step 4:** The same command, then the whole `aria-core` suite. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): update service checks, downloads and verifies releases`

### Task 4: Runtime wiring

**Files:**
- Modify:
  - `crates/aria-core/src/runtime.rs`
  - `crates/aria-core/src/command.rs`
  - `crates/aria-core/src/event.rs`

**Interfaces:**
- Consumes: Task 3.
- Produces:
  - `BackendServices.update: Option<UpdateDeps>`, where `None` leaves it out (tests);
  - `Command::Update(UpdateRequest)`;
  - `Event::Update(UpdateEvent)`.
- The runtime sends `UpdateRequest::Settings{checks, checked_at}`:
  - once at start, from the startup settings;
  - after every `UpdateSettings` whose `update_checks` or `update_checked_at` changed.
- On `shutdown`, it joins the update handle within the same timeout as the others.

- [ ] **Step 1: Failing tests** in the runtime's tests:
  - `update_events_reach_the_app`
    - The backend runs with fake `UpdateDeps`, `CheckNow` is sent, and `UpToDate` is drained.
  - `turning_update_checks_on_reaches_the_service`
    - Send `UpdateSettings` with `On`, and a request reaches the fake within 2 s.
- [ ] **Step 2:** Run `cargo test -p aria-core -j 4 runtime::`. Expected: FAIL.
- [ ] **Step 3:** Implement this following `SpotifyLink`: an `UpdateLink` with an `UnboundedReceiver` arm in
  the `select!`.
- [ ] **Step 4:** Run the aria-core suite. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): wire the update service into the backend`

### Task 5: The app (card, Settings row, sidebar button, restart)

**Files:**
- Create: `crates/aria-app/src/update.rs`, which defines `UpdateUi { status: UpdateStatus }` and
  `apply(&mut self, UpdateEvent) -> Option<UpdateEvent>`. It keeps the status and passes on anything else.
- Modify:
  - `crates/aria-app/src/app.rs`: the event loop, `on_exit`, the card above the page in the central panel,
    and the handling of `PageAction`.
  - `crates/aria-app/src/ui/pages/mod.rs`, which gains these `PageAction`s:
    - `UpdateChecks(UpdateChecks)`
    - `CheckForUpdates`
    - `RestartToUpdate`
    - `OpenReleasePage(String)`
  - `crates/aria-app/src/ui/pages/settings/mod.rs`, the `about` Updates rows.
  - `crates/aria-app/src/ui/layout/sidebar.rs`, for the bottom button.
  - `crates/aria-app/src/main.rs`: `UpdateDeps::system`, and `install_kind` on the exe's folder with
    `cfg!(windows)`.
  - `crates/aria-core/src/i18n/mod.rs`, with the new keys in `KEYS`. The placeable test also sets `version`
    and `percent`.
  - `locales/{en,ru,hy}/main.ftl`.

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces:
  - The app field `launcher: Box<dyn Fn(&Path) -> io::Result<()>>`.
    - In production it runs `std::process::Command::new(path).args(INSTALLER_ARGS).spawn()`.
    - Tests record the call instead.
  - The app field `install_on_exit: Option<PathBuf>`.
- **When the app gets an update event:**
  - `Event::Update(Checked(at))` sets `settings.update_checked_at = Some(at)` and sends `UpdateSettings`.
  - `Event::Update(Install(path))` sets `install_on_exit` and sends `ViewportCommand::Close`.
- **`on_exit`:** after `backend.shutdown`, if `install_on_exit` is set, call `launcher(&path)`. A failure is
  only logged at `warn`, with the path's file name and not the full path.
- **The card** appears only while `settings.update_checks == NotAsked`, above the page in the main panel.
  - "Turn on" sends `UpdateChecks(On)`. "Not now" sends `UpdateChecks(Off)`.
  - Both save the settings.
- **The Settings Updates group,** replacing today's single button:
  - a checkbox `settings-update-auto`, ticked when the setting is `On`;
  - the button `settings-check-updates`, which sends `CheckForUpdates`;
  - a status line;
  - then **Restart to update** when `Ready`, or **Open the release page** when `Available`.
- **The sidebar** shows one button above the pinned entry, and only when the status is `Ready` or
  `Available`:
  - `Ready`: the label is `update-restart` and the button sends `RestartToUpdate`.
  - `Available`: the label is `update-available` and the button sends `OpenReleasePage(page)`.
  - The mini-player does not show it.
- `RELEASES_URL` is kept, and the app falls back to it when there is no page.

**English copy** (exact). Write ru and hy translations with the same placeables.

```
update-card-title = Keep Aria up to date?
update-card-body = Once a day Aria can ask GitHub whether a new version is out, and download it for you. GitHub sees your computer's IP address; nothing else is sent.
update-card-on = Turn on
update-card-later = Not now
settings-update-auto = Check for updates automatically
settings-update-auto-hint = Once a day Aria asks GitHub for its newest release. GitHub sees your IP address; nothing else is sent.
settings-check-updates-hint = Asks GitHub now whether a newer Aria is out.
update-idle = Not checked yet
update-checking = Checking…
update-up-to-date = Aria is up to date
update-downloading = Downloading { $version }… { $percent }%
update-ready = Aria { $version } is ready
update-restart = Restart to update
update-available = Aria { $version } is out
update-open-page = Open the release page
update-not-ready = Aria { $version } is being published; trying again soon
update-failed = Couldn't check: { $reason }
update-reason-offline = no connection to GitHub
update-reason-answer = GitHub's answer didn't make sense
update-reason-too-big = the download is too big
update-reason-checksum = the download didn't match its checksum
update-reason-disk = it couldn't be saved on this computer
notice-update-ready = Aria { $version } is downloaded. Click Restart to update when it suits you.
notice-update-broken = The downloaded update was damaged and has been deleted.
```

`settings-check-updates-hint` replaces the old text, which promised no network use.
`UpdateError::NotARelease` is shown with `update-reason-answer`.

- [ ] **Step 1: Failing tests**, kittest with `fonts::install`, in the files the existing tests use:
  - `the_card_shows_until_answered`: with `NotAsked` the card is there. Clicking "Not now" sends
    `UpdateSettings` with `Off`, and the card is gone.
  - `turn_on_saves_on`
  - `the_settings_row_shows_the_status`: one case each for `Downloading{0.3.6, 42}`
    ("Downloading 0.3.6… 42%"), `Ready` (a "Restart to update" button) and `Failed(Offline)`.
  - `check_for_updates_sends_check_now`
  - `the_sidebar_button_appears_only_when_ready_or_available`
  - `checked_is_stored_in_settings`
  - `install_closes_and_launches_after_shutdown`:
    - the app gets `Install(p)`;
    - a Close viewport command is sent;
    - the launcher has not been called before `on_exit`;
    - `on_exit` calls it once with `p`.
  - `the_mini_player_has_no_update_button`
- [ ] **Step 2:** Run in the background:
  `cargo test -p aria-app -j 4 > .local/build-logs/t5.log 2>&1`. Expected: FAIL.
- [ ] **Step 3: Implement**
- [ ] **Step 4:** The aria-app suite and clippy. Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): update card, Settings row, sidebar button and restart to update`

### Task 6: Installer relaunch, docs and version

**Files:**
- Modify:
  - `packaging/windows/aria.iss`
  - `Cargo.toml`: version `0.3.5`
  - `CHANGELOG.md`: `[0.3.5] - <date>`
  - `README.md`
  - `docs/PRIVACY.md`
  - `ROADMAP.md`
- Create: `docs/testing/0.3.5-manual-check.md`

- [ ] **Step 1:** In `aria.iss`, keep the existing `[Run]` line and add:

```
Filename: "{app}\aria.exe"; Flags: nowait skipifnotsilent; Check: Relaunch

[Code]
function Relaunch: Boolean;
begin
  Result := ExpandConstant('{param:relaunch|0}') = '1';
end;
```

- [ ] **Step 2:** Check the installer script compiles without installing anything. Build the release exe the
  usual way, then compile the script:
  `& "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" /Q /DAppVersion=0.3.5 packaging\windows\aria.iss`.
  Expected: exit code 0.
  - The installer produced is **not run**.
  - The relaunch cannot be tested until 0.3.5 updates to a later version.
- [ ] **Step 3: Docs**
  - `docs/PRIVACY.md`:
    - The first sentence becomes "Aria sends nothing over the network unless you connect Spotify or turn
      on update checks (since 0.3.5)".
    - A new "Updates" section lists the URLs, the User-Agent `Aria`, what GitHub sees (your IP address),
      what is stored (the choice and the last check time in `settings.toml`, and the installer and
      `.sha256` in the cache's `updates` folder), and that the checksum is in the same release. So the
      checksum catches broken downloads, not a taken-over GitHub account. The installer is not
      code-signed.
  - `README.md`: one short paragraph covering the same points.
  - `CHANGELOG.md`: a 0.3.5 entry.
  - `ROADMAP.md`: tick the item.
  - `docs/testing/0.3.5-manual-check.md`: every user-visible step, all marked pending. That includes "the
    first real update from 0.3.5 to the next release relaunches Aria".
- [ ] **Step 4:** Run the full workspace tests and clippy. They take about 20 minutes, so run them in the
  background with the log in `.local/build-logs`. Expected: all pass.
- [ ] **Step 5: Commit** `chore: 0.3.5 installer relaunch, privacy notes and docs`
