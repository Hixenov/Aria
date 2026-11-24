# Aria 0.3.3 — Spotify plays in Aria, without the Spotify app

Date: 2026-11-24. Status: draft, for the user's approval. A builder check was done; its answers are marked "my call".

## 1. Intent

The user, in their words: "просто чтобы aria играла без запуска spotify" ("just so Aria plays without launching Spotify"). Today Aria only controls Spotify devices that already exist (the Spotify app, a phone, a speaker). With none open, pressing play on a Spotify item shows "No Spotify device is active. Open Spotify on a device first." After 0.3.3, pressing play on a Spotify item plays it, with no Spotify app running anywhere.

Success: with a Premium account and the Spotify app closed everywhere, the user picks a Spotify track in Aria, presses play, and hears it. They don't type anything, click anything in another window, or keep a browser tab open. The player bar, the mini-player and the full-screen record show and control it as they do any Spotify device.

## 2. The only allowed route

Spotify allows other apps to play its music only through the **Spotify Web Playback SDK**. That is Spotify's own JavaScript player, which runs in a real web browser and appears in Spotify as a Connect device. It needs:
- full Premium (mobile-only Premium plans are excluded);
- the scopes `streaming`, `user-read-email` and `user-read-private`;
- a browser with Widevine (copy protection): Chrome, Edge, Firefox or Safari.

**Dashboard step (the account owner only).** Aria's Spotify app (client ID `250bd973…`) was made on 2026-11-18 with "Web API" only. In the Spotify Developer Dashboard, the app's settings must also tick **"Web Playback SDK"**, or the SDK refuses the token. Nothing in the code can do this; the manual check starts with it.

Sources checked 2026-11-24:
- developer.spotify.com/documentation/web-playback-sdk
- the developer blog of 2026-02-06 (Development Mode: Premium required, up to five users, one client ID per developer)

The SDK is not deprecated.

The user's rules apply in full:
- "Never request or store a user's Spotify password. Never hardcode client secrets or authentication tokens."
- "Never log access tokens. Never expose credentials in crash reports. Store credentials in OS-protected storage."
- "Do not implement telemetry without explicit user consent."
- "Do not: Circumvent DRM. Extract Spotify encryption keys. Record or download Spotify streams. Bypass Premium requirements. Bypass advertisements. Reverse-engineer private Spotify endpoints. Spoof official Spotify clients or devices."
- "Do not claim that an artifact is supported unless it has actually been built and tested."

Aria itself still decodes, captures and stores no Spotify audio. The browser plays it through Spotify's own player.

## 3. Approaches

| | Approach | Trade-off | Verdict |
|---|---|---|---|
| A | Aria's player page in a **tab of the user's own browser** | Any browser and any OS, the least work. But the tab must stay open, is easy to close with other tabs, and needs one click each time it opens (browsers block sound until the page is clicked). | Kept only as the fallback when no Chromium browser is found |
| **B** | **A small window of Aria's own**: Edge or Chrome started by Aria with a separate profile, in app mode (no tabs, no address bar), sound allowed at start, closed by Aria when it quits | No click and no tab to look after; closest to "Aria plays". Needs Edge or Chrome (Edge is always on Windows), and more to build. | **Chosen by the user ("B: own window")** |
| C | A web view inside Aria (WebView2, wry) | WebView2 and the system web views have no Widevine, so the SDK cannot play there. | Ruled out |
| D | librespot-style streaming in Aria | Private Spotify protocol: forbidden by the user's rules and Spotify's terms. | Ruled out |

## 4. Goals, non-goals, anti-goals

**Goals**
- Play any Spotify item from Aria with no Spotify app running, on Windows with Edge or Chrome, and on macOS and Linux with Chrome, Edge or Chromium.
- Everything Aria already does with a Spotify device works on this one too: play, pause, next, previous, seek, volume, shuffle, repeat, what is playing, and the cover.
- Fall back to a browser tab when no Chromium browser is found.
- Clear messages for each way it can fail: not Premium, needs reconnecting, a browser without Widevine, the window closed.

**Non-goals**
- Spotify sound through Aria's audio engine. It plays through the browser to the system's default output, so it gets no ReplayGain, no output-device choice, no exclusive mode, no soundboard and no chat sharing.
- Hiding the window. It is shown small, and the user may minimize it.
- Starting the player when Aria starts. It starts on the first Spotify play.
- Offline playback, downloads, mobile.

**Anti-goals** (a failure even if it plays)
- Anything that touches DRM, captures the browser's audio or saves a stream.
- A device that claims to be an official Spotify client. The device is named "Aria".
- Use of private endpoints.
- Touching the user's own browser profile, tabs or cookies.
- The access token reaching a log, a crash report, disk or any process that doesn't hold the per-launch key.

## 5. Design

### 5.1 Permissions and reconnecting

- `SCOPES` gains `streaming` and `user-read-email`.
- `Tokens` keeps the scopes Spotify granted: the `scope` field of the token answer, both from the code exchange and from each refresh.
- A session restored or refreshed without `streaming`:
  - keeps working for everything it did before;
  - playing in Aria shows "Reconnect Spotify in Settings to play it in Aria", and Settings, Spotify, gets a "Reconnect" button. That button is a plain sign-out followed by sign-in.
- The email is never read, deserialized or stored. `Account` gains no field for it.

### 5.2 The player page server

A small HTTP server on `127.0.0.1` (never another interface) on a free port. It runs only while the player runs.

- **Key.** Each launch makes a key: 32 bytes from the OS random source, base64url. All routes are under `/p/<key>/`; any other path answers 404. The key travels on the browser's command line, which other local users can read (`ps`), so it is only good for loading the page.
- **Session.** On load, the page calls `POST /p/<key>/session` once. That answers a second random secret, the session, which the page keeps in a JavaScript variable only. `/token` and `/event` require it in an `X-Aria-Session` header. A second `/session` call for the same launch is refused, so a process that read the key afterwards gets nothing. (My call, after the builder check.) A reloaded page can't get a session; it says to close the window and press play in Aria, which starts a fresh launch.
- **Routes:**
  - `GET /p/<key>/` serves the page (HTML).
  - `GET /p/<key>/player.js` serves the page's script.
  - `GET /p/<key>/strings` serves the page's text in Aria's current language (JSON).
  - `POST /p/<key>/session` answers `{"session": "..."}`, once per launch.
  - `GET /p/<key>/token` answers `{"access_token": "..."}`. The token is fresh: the Spotify thread refreshes it first if it expires within 60 s. The answer carries `Cache-Control: no-store`.
  - `POST /p/<key>/event` takes the page's events as JSON (§5.3). Its answer is `{}`, or `{"close": true}` when Aria wants the window closed (§5.4).
- **Checks on every request:**
  - `Host` must be exactly `127.0.0.1:<port>`, which stops DNS rebinding;
  - an `Origin` header, if present, must be `http://127.0.0.1:<port>`;
  - the request may be at most 8 KiB;
  - no CORS headers are sent.
- **Security headers.** Responses send `Content-Security-Policy` with `script-src 'self' https://sdk.scdn.co`, plus `Referrer-Policy: no-referrer`. The SDK's other needs (frames, media, connections) are not documented, so other directives stay open. That is the builder's call; it must be checked by hand.
- **Embedded files.** The page and script are written for Aria and built into the binary (`include_str!`). They load only `https://sdk.scdn.co/spotify-player.js`.
- **What the page must not do:**
  - log the token;
  - put the token, or anything else, in `localStorage`, `sessionStorage`, IndexedDB or cookies;
  - send analytics.
- **Logging.** The server logs no request line that holds the key or the token.

### 5.3 The page

- **The player.** It creates `Spotify.Player({ name: "Aria", getOAuthToken, volume })`, where `getOAuthToken` fetches `/token` each time the SDK asks. It then connects.
- **What it shows,** in Aria's language and colours:
  - a line saying Aria is playing Spotify in this window, that it can be minimized, and that closing it stops Spotify in Aria;
  - its state: starting, ready, or the error.
- **Events it posts to `/event`:**
  - `ready {device_id}`
  - `not_ready {device_id}`
  - `error {kind: "initialization" | "authentication" | "account" | "playback", message}`. The message is the SDK's text, shown only at debug level.
  - `autoplay_failed`
  - `alive`, every 5 s.
- **The Start button.** In the fallback tab, the page shows a "Start" button that calls `player.activateElement()`, then connects.

### 5.4 The browser window

**Finding a browser.** The first one found is used.
- Windows:
  - Edge in `%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe`, then `%ProgramFiles%\…`;
  - Chrome in `%ProgramFiles%\Google\Chrome\Application\chrome.exe`, `%ProgramFiles(x86)%\…`, then `%LOCALAPPDATA%\…`.
- macOS: `Google Chrome.app`, `Microsoft Edge.app` and `Chromium.app`, in `/Applications` and then `~/Applications`.
- Linux: on `PATH`, `google-chrome`, `google-chrome-stable`, `chromium`, `chromium-browser`, `microsoft-edge` and `microsoft-edge-stable`.

**Starting it.** Aria starts the browser as its own child process with:
- `--user-data-dir=<cache_dir>/spotify-player`: Aria's own profile, kept between runs so the browser fetches its Widevine part only once; portable mode keeps it under the portable root's cache;
- `--app=http://127.0.0.1:<port>/p/<key>/`;
- `--window-size=420,260`;
- `--no-first-run` and `--no-default-browser-check`;
- `--autoplay-policy=no-user-gesture-required`;
- `--disable-features=HardwareMediaKeyHandling`, so the browser doesn't take the media keys from Aria's hotkeys;
- `--disable-background-timer-throttling`, `--disable-renderer-backgrounding` and `--disable-backgrounding-occluded-windows`, so a minimized, paused window keeps sending `alive` (my call, after the builder check: otherwise Chromium slows its timers to about once a minute after 5 minutes hidden).

The builder may add further flags only with a ledgered reason; the tests pin the list.

**Owning it.**
- Aria closes the window when Aria quits, when Spotify is disconnected, and when the player is turned off. It first answers the page's next event with `{"close": true}`, and the page calls `window.close()`, so the browser shuts down cleanly with no "restore pages" prompt next time. If the page doesn't post within 3 s, Aria kills the child it holds. (My call, after the builder check. A browser that took over the launch can then only be closed through the page.)
- The window counts as **gone** when the page has posted nothing for 15 s, or the child exited and no `alive` came within 15 s afterwards. The second case covers a leftover browser on the same profile taking over the launch.
- When the window is gone, a new launch gets a new key and port. Before launching, Aria looks in Spotify's device list for a device named "Aria" with the last known id; if it is there, it is used, and no second window opens.

**No Chromium browser found.** Aria opens the same page in the default browser (approach A), with the Start button. Settings says Edge or Chrome wasn't found, so Aria uses a tab instead.

### 5.5 When it plays there

- **Settings, Spotify, "Play in Aria".** On by default. When it is off, behaviour is that of 0.3.2.
- **The device picker** always lists **"Aria (this computer)"** while the setting is on, even before the window exists. Choosing it starts the window if needed and moves playback there. The choice is kept in `spotify_device` as the marker `aria`. The real device id changes on every launch, so it isn't saved.
- **Pressing play** on a Spotify item when no device is chosen and none is active, or when "Aria" is chosen:
  1. Aria starts the window if it isn't running;
  2. it waits for `ready`: 20 s, or 90 s on the profile's first launch, while the browser fetches Widevine; the player bar shows "Starting Aria's Spotify player…";
  3. it then moves playback there with `transfer(id, true)` and plays the item on that device id. A fresh device may have nothing to play, so a bare play isn't used.

  Only play starts the window; pause, next, previous and seek don't.
  
  This replaces "No Spotify device is active" in that case.
- **One entry.** The SDK's own "Aria" entry in Spotify's device list is shown as "Aria (this computer)", matched by the id from `ready`, not as a second entry.
- **Another device.** If the user chose another device, such as a phone, Aria uses it as today and doesn't start the window.
- **Staying open.** The window stays open between tracks and while paused. Closing it by hand ends Spotify in Aria until the next play. A notice says the player window was closed.
- **Volume.** Aria's volume slider sets the SDK device's volume through the Web API, as for any device.

### 5.6 Messages (en, ru, hy)

| Case | Message |
|---|---|
| Starting | "Starting Aria's Spotify player…" |
| Needs reconnecting | "Reconnect Spotify in Settings to play it in Aria." |
| `account` error | "Playing Spotify in Aria needs Spotify Premium." |
| `initialization` error | "This browser can't play Spotify (it lacks the copy-protection part). Install Chrome or Edge." |
| `playback` error | "Spotify couldn't play this in Aria." |
| No `ready` in time | "Aria's Spotify player didn't start." |
| Window couldn't be launched | "Aria couldn't open its player window." |
| Window closed | "The Spotify player window was closed; press play to open it again." |
| Fallback tab | "Edge or Chrome wasn't found, so Spotify plays in a tab of your browser. Click Start there." |

### 5.7 Documents

- **PRIVACY:**
  - the new permissions, and that the email is not read;
  - the local page and its key;
  - Aria's browser profile and where it is;
  - the browser loads Spotify's player from `sdk.scdn.co`, and Spotify's own player talks to Spotify as Spotify decides;
  - the token is handed only to that page.
- **README and CHANGELOG:**
  - the feature;
  - its limits: Premium, the window must stay open, sound goes to the system default output, Linux Chromium builds often lack Widevine.
- **Version** 0.3.3.
- **A manual-check document.** None of this can be tried in CI against real Spotify. It starts with the dashboard step (§2).
- **macOS and Linux.** The browser search is built for them, but README, CHANGELOG and the release notes call them untested until someone checks them by hand on those systems (the user's rule). (My call, after the builder check.)

## 6. Tests (automated)

- **Server:**
  - a wrong key, a wrong `Host` and a wrong `Origin` each answer 404 or 403, with no token in the body;
  - an over-size request is refused;
  - `/token` is `no-store` and fresh after a refresh;
  - events parse, and bad JSON is ignored;
  - the server only binds `127.0.0.1`.
- **Browser search and arguments:** run over a fake filesystem and environment for each OS, with the exact argument list.
- **Service flow,** with a fake launcher and a fake page (an HTTP client playing the page):
  - play with no device starts the launcher, waits for `ready`, then plays on that id;
  - a timeout gives the message;
  - losing `alive` marks the window gone;
  - quit and sign-out end the child;
  - a chosen phone doesn't start the launcher;
  - the setting off gives 0.3.2's behaviour.
- **Scopes:** a token answer without `streaming` leads to the reconnect message. `SCOPES` holds the two new scopes.
- **i18n:** the new keys exist in en, ru and hy.
- **The token never shows up in the output of `Debug` or in a log line.**

The real SDK, Widevine and real browsers are checked by hand: `docs/testing/0.3.3-manual-check.md`.

## 7. Left to the builder

- Module placement. A guess: the page server and the launcher in `aria-spotify`, the flow in `spotify_service`.
- The page's exact look.
- The exact CSP beyond `script-src`.
- Heartbeat numbers within the spirit of §5.4.
- The waits (`ready`, `/token` refresh) run as a state machine inside the Spotify thread's loop, not on threads of their own (the builder check's suggestion).
- First launch (the 90 s wait) is the profile folder not existing yet.
- How Settings words the fallback.
