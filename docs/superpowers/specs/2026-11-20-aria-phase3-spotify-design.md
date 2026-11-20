# Aria Phase 3 (0.3.0): Spotify — design

## What it is for

You use Spotify on your phone, a speaker or the Spotify app, and want to browse your Spotify
library and search from Aria, start what you pick on one of your Spotify devices, and control it
from Aria's player bar — beside your local music, in one window. Aria never plays Spotify audio
itself (see the roadmap: no in-app streaming, no private protocols).

Decided with the user on 2026-11-20:

- The account has **Premium**, so remote control is built and is the point of it. A free
  account gets everything but playback, with the controls greyed out and the reason given.
- The **Client ID is built into Aria** (`250bd9732a644668911a74bdd329e66e`, the user's Spotify
  developer app, Web API only). While that app is in Spotify's Development mode, only the few
  accounts on its allow-list (developer.spotify.com, Users and Access) can sign in; the docs and
  the sign-in error say so.
- Spotify has been taking endpoints away from Development-mode apps. Before the plan is written,
  every endpoint below is checked against Spotify's current Web API reference; one that is not
  available is dropped from 0.3.0 and the page says less, never worked around.
- **Scope of 0.3.0: the core.** Sign in and out; saved tracks, saved albums, followed artists and
  your playlists; search; play on a device you choose; play/pause, next, previous, seek, volume,
  shuffle and repeat; what is playing, in the player bar. Liking tracks and editing Spotify
  playlists from Aria come later (0.3.x).

## Rules this design keeps

- Never asks for or stores the Spotify password. Never uses a client secret: OAuth with PKCE.
- Never logs tokens or puts them in crash reports. The refresh token is kept in the system's
  protected store (Windows Credential Manager, macOS Keychain, the Secret Service on Linux);
  the access token lives in memory only.
- No telemetry. Aria talks to Spotify only after you sign in, and only to `accounts.spotify.com`
  and `api.spotify.com`; before that, and after signing out, it makes no request at all.
- No private endpoints, no device spoofing, no audio: the public Web API only.

## Pieces

### A new crate, `aria-spotify`

What talks to Spotify, with nothing of the UI in it:

- **`auth`**: PKCE. Makes a 64-byte random verifier (`getrandom`), its SHA-256 challenge
  (`sha2`, already used) in base64url, and a random `state`. Opens
  `https://accounts.spotify.com/authorize` in the browser with the scopes below. A one-shot
  listener on `127.0.0.1:43819` (std `TcpListener`, the registered redirect URI) takes the one
  `GET /callback`, checks `state`, answers with a small page ("You can close this tab"), and
  stops; it gives up after 5 minutes. The code is exchanged at `/api/token` with the verifier.
  Refresh: before the access token expires, or on a 401, once.
- **`client`**: the Web API calls, as typed requests and results (serde). Pages of 50, followed
  with `next`. A 429 waits for `Retry-After`, then tries once more; a 401 refreshes once; any
  other failure is an error the UI can name (no network, signed out, not Premium, no device,
  Spotify's own error).
- **`store`**: the refresh token in the system store (`keyring` crate), under the service name
  `Aria` and the account `spotify`. A system without a store (some Linux setups) means signing in
  each session, and Settings says so.
- **HTTP**: `ureq` 3 with rustls, blocking, run on the backend's blocking pool. Its licences are
  checked with cargo-deny before it is added.

Scopes: `user-read-private` (Premium or not), `user-library-read`, `user-follow-read`,
`playlist-read-private`, `playlist-read-collaborative`, `user-read-playback-state`,
`user-modify-playback-state`, `user-read-currently-playing`.

### In the core: `spotify_service`

On the backend, like the library service:

- State: signed out / signing in / signed in (with the display name and whether it is Premium).
  Sent to the UI as `Event::Spotify(Arc<SpotifyState>)`.
- The library: saved tracks, saved albums, followed artists and playlists, fetched after sign-in
  and on Refresh, kept in memory only (`Arc<SpotifyLibrary>`), never written to `library.db`.
- Search: as you type, after 300 ms of quiet, tracks, albums, artists and playlists, as many of
  each as Spotify allows in one request (up to 20).
- Devices and playback: `GET /me/player` every 2 seconds while the player bar shows a Spotify
  item, every 15 seconds otherwise, and not at all while signed out or while the window is
  minimized or hidden in the tray. Commands go to the device chosen, or to the active one.

### Which player the bar shows

The bar follows what you last started: start a local track and it shows Aria's own player (and
asks Spotify to pause, if it was playing); start a Spotify item and it shows Spotify's playback
(and pauses the local player). The transport buttons, the seek bar and the volume act on the
player shown. The local queue is not mixed with Spotify's.

### Pages

The four pages that say "Available in Phase 3" today:

- **Search**: one field, results in four groups. Click to play; a track plays within its album.
- **Your Library**: Liked Songs (your saved tracks) and your playlists. Click to play.
- **Albums** and **Artists**: your saved albums and followed artists. Click to play the album, or
  the artist's top tracks.
- Signed out, each says so and has a "Connect Spotify" button.
- Covers: fetched from Spotify's image URLs, kept in memory for the session (no cache on disk).

Settings gets a **Spotify** card: Connect / Disconnect, the account name and Premium or not, the
device to play on (Spotify's list, with "the active one"), and a line on the Development-mode
allow-list. Disconnect deletes the stored token and forgets the library.

The player bar's devices button, a placeholder since Phase 1, opens the Spotify device list.

## Errors, as you see them

Each is a notice in your language: not on the allow-list (with what to do), sign-in cancelled or
timed out, no network, Spotify's limit reached (with how long to wait), no active device ("open
Spotify on a device first"), not Premium (for playback only), a session that has ended (sign in
again).

## Testing

- `aria-spotify` against a fake Web API: a local HTTP server in the tests, playing Spotify's
  answers from recorded JSON. Paging, 429 with Retry-After, 401 and refresh, PKCE values (known
  test vectors), the callback listener (state checked, wrong state refused, timeout).
- The core service with a fake client: sign-in states, polling cadence, which player the bar
  shows, the commands sent.
- UI tests for the pages and the Settings card, signed in and out.
- No test talks to the real Spotify. A hand check list covers the real sign-in, a real device and
  Premium.

## Privacy and docs

PRIVACY gets a Spotify section: what is sent (the requests above, with the token), to whom
(Spotify), what is kept (the refresh token in the system store; nothing else on disk), and how to
remove it (Disconnect, or delete the credential). README and ROADMAP say what Phase 3 is and is
not.

## Not in 0.3.0

Liking and unliking tracks, adding to or editing Spotify playlists, the Spotify queue, podcasts,
Spotify in the global hotkeys, local files in Spotify playlists, offline caching of anything.
