# Aria 0.4.1: update check without GitHub's API, implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Aria finds the latest release through github.com's `releases/latest` redirect and the release's download links instead of `api.github.com`, and reports GitHub refusing (403/429) as "busy" instead of "didn't make sense".

**Architecture:** `aria-update`'s `Client::latest` reads the `302` without following it and builds the release's asset URLs from the tag. Asset sizes come from `Content-Length` while downloading. `aria-core`'s update service treats a missing `SHA256SUMS.txt` as "still being published", and treats `Busy` like `Offline`.

**Tech Stack:** Rust, ureq 3.5.2 (per-request `max_redirects(0)`), the fake HTTP server in `aria_spotify::fake`.

**Spec:** docs/superpowers/specs/2026-11-26-aria-0-4-1-update-check-without-the-api-design.md

## Global Constraints

- No `api.github.com` anywhere in Aria's code. No token, no account, User-Agent stays `Aria`.
- No `unwrap`/`expect` outside tests, no `unsafe`, no `#[allow]`.
- Tests never open the real microphone, never touch the installed Aria or %APPDATA%\Aria, never start an installer, and never reach the real network (the fake server only).
- Errors never carry a URL.
- Retry period for `Busy` is the existing `retry` (an hour), automatic checks only.

## Review Focus

- A redirect whose `Location` is relative (`/Hixenov/Aria/releases/tag/v0.4.1`) is accepted only if made absolute against the same origin, never against another host. Test: `a_relative_location_is_read_against_github`.
- `Location` pointing at another repository or host is `Answer`, never followed. Test: `a_location_elsewhere_is_refused`.
- A download whose `Content-Length` is over the cap is refused before any byte is written. Test: `a_download_over_the_cap_is_refused_before_reading`.
- A download with no `Content-Length` stops at the cap. Test: `a_download_without_a_length_stops_at_the_cap`.
- 429 with a body, on any of the three requests, is `Busy`. Tests: `a_refusal_is_busy`, `a_refused_download_is_busy`.

---

### Task 1: `aria-update` reads github.com instead of the API

**Files:**
- Modify: `crates/aria-update/src/lib.rs`, `crates/aria-update/src/client.rs`, `crates/aria-update/src/release.rs`, `crates/aria-update/Cargo.toml`
- Delete: `crates/aria-update/tests/fixtures/release.json` (once nothing includes it)

**Interfaces:**
- Produces:
  - `Endpoints { latest: String, tag_prefix: String, download_prefix: String }`. `Endpoints::github()` gives `https://github.com/Hixenov/Aria/releases/latest`, `https://github.com/Hixenov/Aria/releases/tag/` and `https://github.com/Hixenov/Aria/releases/download/`.
  - `UpdateError::Busy` ("GitHub is busy") and `UpdateError::Missing` ("the release's file is not there").
  - `Asset { name: String, url: String }`, with no `size`.
  - `Release { version, page, installer: Asset, sums: Asset }` (no longer `Option`).
  - `pub fn release_for_tag(tag: &str, page: String, endpoints: &Endpoints) -> Result<Release, UpdateError>` in release.rs.
  - `Client::latest()`, `Client::text(&Asset, cap)` (a 404 is `Missing`), and `Client::download(&Asset, &Path, cap, progress)`, where progress is `(done, total)` and total is `Content-Length`, or `cap` when there is none.

- [ ] Write the tests in `client.rs`, replacing the JSON ones:
  - `the_latest_release_is_read_from_the_redirect`: a 302 with `Location: {base}/Hixenov/Aria/releases/tag/v0.3.6` gives version 0.3.6, the page equal to that location, installer `{download_prefix}v0.3.6/Aria-0.3.6-setup.exe` and sums `{download_prefix}v0.3.6/SHA256SUMS.txt`. Also: the request was not followed (no request to the tag path) and its User-Agent is `Aria`.
  - `a_relative_location_is_read_against_github`.
  - `a_location_elsewhere_is_refused` gives `Answer`.
  - `no_redirect_is_an_answer_that_makes_no_sense`: a 200 gives `Answer`.
  - `an_odd_tag_is_not_a_release`: `v0.3.6-beta` gives `NotARelease`.
  - `a_refusal_is_busy`: 403 and 429 give `Busy`.
  - `missing_sums_are_missing`: `text` on a 404 gives `Missing`.
  - `a_download_takes_its_size_from_the_answer`.
  - `a_short_download_is_not_kept`.
  - `a_download_over_the_cap_is_refused_before_reading`.
  - `a_download_without_a_length_stops_at_the_cap`.
  - `a_refused_download_is_busy`.
  - Keep the existing stall-timeout tests, adapted.
- [ ] Run `cargo test -p aria-update`. Expected: FAIL (compile errors on the new API).
- [ ] Implement it. `latest()` uses per-request config `max_redirects(0)` and reads the `Location` header. In `get()`, any status other than 2xx becomes 403/429 → `Busy`, 404 → `Missing`, else `Answer`, except that `latest` expects a 3xx. Drop `parse_release`, `RawRelease` and `serde_json`/`serde` if they are unused.
- [ ] Run `cargo test -p aria-update`. Expected: PASS.

### Task 2: the service, the app's wording, docs and version

**Files:**
- Modify: `crates/aria-core/src/update_service.rs`, `crates/aria-core/src/runtime.rs` (the update test module), `crates/aria-app/src/update.rs`, `locales/{en,ru,hy}/main.ftl`, `crates/aria-core/src/i18n/mod.rs` (KEYS), `docs/PRIVACY.md`, `CHANGELOG.md`, `docs/ROADMAP.md`, `Cargo.toml` (version 0.4.1), `docs/testing/0.4.1-manual-check.md`

**Interfaces:**
- Consumes: everything Task 1 produces.

- [ ] Change the tests in `update_service.rs`:
  - The harness serves the redirect, a helper `serve_redirect(tag)`.
  - `missing_sums_means_not_ready_and_a_retry` now uses a 404 for sums.
  - New `sums_without_the_installer_are_not_ready`.
  - New `a_busy_github_is_retried_while_on` (429, then `Failed(Busy)`, then a second request after `retry`).
  - New `a_busy_click_is_one_check`.
  - The runtime test serves a redirect.
- [ ] Change `crates/aria-app/src/update.rs`: `Busy` maps to `update-reason-busy`, and `Missing` to `update-reason-answer` (the service never shows it). Its test lists the new reason with the English text "GitHub is busy right now, so Aria will try again later".
- [ ] Run `cargo test -p aria-core update` and `-p aria-app update`. Expected: FAIL.
- [ ] Implement: in `find()`, `Missing` from `text` gives `NotReady`, and so does `sha256_for` returning `None`. `check()` adds `Busy` to the retried arm. Add the strings in en/ru/hy and KEYS.
- [ ] Docs:
  - CHANGELOG `[0.4.1]`.
  - PRIVACY "Updates" now names github.com's release page and downloads; the API is gone.
  - ROADMAP: one line.
  - A manual check list.
  - Version 0.4.1, then `cargo update --workspace --offline`.
- [ ] Run the whole workspace's tests, clippy `-D warnings` and fmt. Expected: all green.
- [ ] Try it once against the real GitHub with a throwaway example that calls `Client::latest()` and `text()` on the sums (no installer download, nothing kept). Expected: 0.4.0 and a listing that names `Aria-0.4.0-setup.exe`.
- [ ] Commit and push to main.
