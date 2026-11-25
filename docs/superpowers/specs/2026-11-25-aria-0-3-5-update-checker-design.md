# Aria 0.3.5: Update checker that downloads and installs

The user asked for this on 2026-11-24 ("add auto update checker"), then chose "Download and install"
and "Restart button". **This design is not approved yet.**

## Intent

Today the "Check for updates" button in Settings only opens the GitHub releases page in the browser.
Users find out about a new Aria only if they think to look.

From 0.3.5 on, Aria can find a new release, download it and check it, all by itself. When it is
ready, a "Restart to update" button appears. The update installs when the user clicks it, so
the music never stops on its own.

## Goals

- **Consent first.**
  - A new setting, "Check for updates automatically", is off until the user says yes.
  - The first time a 0.3.5+ Aria starts (fresh installs and upgrades both), a card at the top of the
    main page asks once: "Keep Aria up to date?" with "Turn on" and "Not now".
  - The card says what is sent: a request to GitHub, which sees the computer's IP address. Nothing
    else is sent.
  - Either answer is stored and the card never comes back. Settings can change it later.
- **When automatic checks are on:**
  - Aria asks GitHub for the latest full release: no drafts, no pre-releases.
  - It asks shortly after start (about 10 s, so start-up is not slowed), then at most once every
    24 hours while it runs.
  - The time of the last check is stored, so restarting Aria does not trigger a new check.
- **"Check for updates" in Settings:**
  - The button checks right away, whether or not automatic checks are on. A click is the user
    asking.
  - The Updates row shows where things stand: "Aria is up to date", "Checking…",
    "Downloading 0.3.6… 42%", "Aria 0.3.6 is ready" with **Restart to update**, or
    "Couldn't check: …" with the reason.
- **What happens when a newer version is found:**
  - "Newer" means a higher semver than the running version. An equal or older tag is ignored.
  - **On an installed Aria on Windows:**
    - Aria downloads that release's `Aria-<version>-setup.exe` and `SHA256SUMS.txt`.
    - It checks the installer against its SHA256 line. Only a match is kept.
    - It then shows **Restart to update** in two places: a button always visible in the main
      window, and the Settings row. One Info notice also says so.
  - **On portable, a loose `aria.exe`, macOS and Linux:**
    - Nothing is downloaded.
    - The same visible spot says "Aria 0.3.6 is out", with a button that opens that release's
      page.
- **Restart to update:**
  1. Aria checks the stored installer against its checksum again.
  2. It saves its state the way a normal quit does, and starts the installer:
     `/SILENT /SUPPRESSMSGBOXES /NORESTART /RELAUNCH=1`.
  3. Aria then quits.
  4. The installer shows its own progress window, replaces the files, and starts the new Aria.
  5. If the user cancels the installer, the old Aria is still installed. The user starts it again
     by hand.
- **Leftovers:** at start, any downloaded installer whose version is not newer than the running one
  is deleted.
- **If nothing was installed:** if Aria is quit without clicking Restart, nothing happens. The
  downloaded installer stays in the cache, and the button is back on the next start. The checksum
  is checked again before it is shown.
- **The installer script** gains a `[Run]` entry that starts Aria after a silent install only when
  `/RELAUNCH=1` was passed. A silent install anyone runs by hand still starts nothing, as today.

## Non-goals

- Installing by itself when Aria quits. The user chose the button.
- Delta updates or downgrades. Pre-release or beta channels.
- Updating macOS/Linux/portable builds in place.
- Code signing.
- A "skip this version" choice. "Not now" or turning the setting off covers it.
- Checking more than once a day.

## Anti-goals

- **Any network request before the user has said yes.**
  - The automatic check runs only when the setting is on.
  - A Settings click is that single check, nothing more.
  - Turning the setting off stops all automatic checks at once. A download that is already running
    finishes. Fine, since the user asked earlier.
- **Sending anything beyond what fetching a public file needs.**
  - No version or ID goes in the request: the User-Agent is plain `Aria`.
  - No analytics, no "update installed" ping.
- **Running anything that failed its checksum,** or was not fully downloaded, or is too big:
  anything over 200 MB is refused.
- **Saying it is safer than it is.** `SHA256SUMS.txt` sits in the same GitHub release. It catches a
  broken or incomplete download. It does not protect against someone who takes over the GitHub
  account, and the installer is not code-signed. PRIVACY.md and the README say exactly this.
- **Claiming the restart flow was tried.**
  - Tests use a local fake server and never start an installer. A 0.3.5 can only update itself to
    a later release, so the first real test is going from 0.3.5 to the next version. It stays on
    the manual check list.
  - The installed Aria and `%APPDATA%\Aria` are never touched by tests.

## Constraints

- Rust, no `unsafe`, no `unwrap`/`expect` outside tests, no `#[allow]`.
- HTTP goes through the `ureq` 3.5.2 Aria already uses: rustls with platform verifier, HTTPS only.
- Hashing and version parsing use `sha2`, `hex` and `semver`. These are already in `Cargo.lock`
  through other crates, so no new crates are added; `cargo deny` must still pass.
- **Endpoints, all public, no sign-in:**
  - `GET https://api.github.com/repos/Hixenov/Aria/releases/latest`.
    - Fields used: `tag_name`, `html_url`, and `assets[].name`, `.size`, `.browser_download_url`.
  - The two asset downloads: their `browser_download_url` and the redirect to GitHub's file host.
  - An asset URL must be on `https://github.com/Hixenov/Aria/releases/download/`, or it is refused.
- **A release that has no `SHA256SUMS.txt` yet** is not ready. The release workflow adds it a few
  minutes after publishing.
  - Aria tries again in 1 hour, not 24.
  - The Settings row says "Aria 0.3.6 is being published; trying again soon".
- **"Installed"** means a `unins000.exe` sits beside the running `aria.exe` and there is no
  `AriaData` folder. Anything else counts as not installed.
- **Files:** downloads go to `<cache_dir>/updates/`. Each is written as `.part` and renamed only after
  the checksum matches.
- **Strings** go in en, ru and hy, and the i18n key test covers them.
- **Privacy notes:** PRIVACY.md's opening promise changes to: "Aria sends nothing over the network
  unless you connect Spotify or turn on update checks". A new "Updates" section lists exactly what
  is sent and stored. The old Settings hint, "Aria itself sends nothing over the network", is
  rewritten.

## Approaches

1. **Aria checks, downloads and verifies itself, and runs the Inno installer silently on Restart
   (chosen).**
   - It uses the HTTP and settings code Aria already has.
   - It keeps the one installer people already use, so the AppId, "Open with" entries and
     uninstall all stay the same.
   - It adds one `[Run]` line to the installer.
2. **A ready-made updater (Velopack, or the `self_update` crate).**
   - Velopack replaces Inno Setup and the install folder, so existing installs would need a
     one-time migration.
   - `self_update` swaps `aria.exe` in place and skips the installer, so the uninstall entry and
     "Open with" go stale.
   - Both pull in many new crates.
3. **Only tell, never download.** It is simpler, but the user chose "Download and install".

## Decided by the user

- Download and install, not just tell: "Download and install".
- Install when a button is clicked, not on quit: "Restart button".

## My calls (the user can change them)

- Off until asked once at first start.
- At most daily, and 1 hour when the checksums are not up yet.
- Full releases only.
- Portable, macOS and Linux only tell.
- The User-Agent carries no version.
- The 200 MB cap.
- Leftover cleanup at start.
- Where the visible button sits: next to the window's other always-visible controls, not in the
  mini-player.
- The setting is stored as one of three states: not asked yet, on, or off. Only "not asked yet"
  shows the card.
- How the relaunch works with the single-instance guard:
  - Because of `CloseApplications=yes`, the installer waits until the old Aria has exited before
    it copies files.
  - So when the new Aria starts, the old one's pipe is already gone, and the new Aria opens
    normally.
  - The relaunch entry is added alongside the existing `postinstall skipifsilent` entry. It does
    not replace it.

## Left to the builder

- Which crate the checker lives in: a new `aria-update` crate, or a module beside the Spotify
  client.
- How its worker thread talks to the UI, following the Spotify service pattern.
- Exact copy and layout, within the strings above.
- How progress is reported.
- Fake-server test setup.
