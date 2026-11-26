# Aria 0.4.0: Media keys and the system's media controls

On 2026-11-26 the user said "phase 4", chose "Media keys first", then "Use souvlaki".
**This design is not approved yet.**

## Intent

Phase 4 is the desktop experience, and this is its first piece.

Today Aria is controlled only from its own window, the mini-player, and the soundboard's global
hotkeys. Keyboard media keys, headset buttons and the system's media controls do nothing for Aria:
on Windows that means the volume pop-up and the lock screen, on macOS the Now Playing menu, on
Linux the desktop's media controls.

From 0.4.0 they control Aria from anywhere, and the system shows what plays: cover, title, artist,
album and position.

## Goals

**What the system shows.**
- While something is loaded, Aria appears in the system's media controls:
  - Windows' System Media Transport Controls (SMTC);
  - macOS's Now Playing;
  - Linux's MPRIS.
- It shows title, artist, album, cover, duration, position, and playing or paused.

**It follows the player bar.**
- When the bar shows a Spotify track, the system shows that track.
- Every key or button does exactly what the bar's own button does. Commands already go to
  Spotify when it is active.

**What each event does.**

| Event from the system | What Aria does |
|---|---|
| Play | Plays, if paused |
| Pause, Stop | Pauses, if playing (Aria has no separate stop) |
| Toggle | Play or pause |
| Next, Previous | Next or previous track |
| Set position | Seeks there |
| Seek by an amount | Seeks by it |
| Seek forward or back with no amount | Seeks 10 s |
| Set volume (MPRIS only) | Sets Aria's volume |
| Raise (MPRIS) | Brings the window forward |
| Quit, Open URI | Ignored |

**Updates.**
- The title, artist, album, cover and duration are sent again when the track changes.
- Playing or paused is sent when it changes.
- The position is sent after a seek, on play and pause, and every 5 s while playing, so the
  system's timeline stays close.

**Covers.**
- A local track passes its cover's file in Aria's artwork cache as a `file://` URL, when the cache
  has one.
- A Spotify track passes its cover's `https://i.scdn.co/...` URL. The system fetches that itself.

**Nothing loaded.** The entry shows as stopped, with no metadata.

**The setting.** A new setting, `media_controls` (default on), appears in Settings, General as
"Show in the system's media controls".
- Turned off, Aria leaves the media controls entirely. Media keys then no longer reach it, unless
  another player takes them.
- Turned back on, it rejoins.

**The Spotify window.** It keeps Edge's own media session off: `--disable-features=
HardwareMediaKeyHandling`, as today. So there is only one Aria entry.

## Non-goals

- Media keys as global hotkeys.
- Showing the queue, or "liked", in the system UI.
- Shuffle and repeat buttons in the system UI.
- Taking the keys away from the Spotify desktop app. Windows picks the session used last.
- Rating or liking from the system UI.

## Anti-goals

- **Any network request by Aria itself.**
  - The controls are local OS interfaces.
  - The only new outside traffic is the system fetching a Spotify cover from Spotify's servers.
    Spotify is already connected then. PRIVACY.md says so.
- **Two Aria entries,** or a stale entry after Aria quits. The controls are dropped on exit.
- **Reading any keystroke** other than what the OS hands to the media session.
- **`unsafe`.** The window handle is passed as the pointer `souvlaki` asks for, with a cast and
  no `unsafe` block.
- **Claiming it works on a real system.** Tests use a fake backend. Nobody has pressed a key yet.
  The manual check list says so.

## Constraints

- **Crate.** `souvlaki` 0.9.3, MIT, with default features.
  - Linux uses `dbus` and `dbus-crossroads`. `dbus` 0.9 is already in `Cargo.lock`, and CI already
    installs `libdbus-1-dev`.
  - Windows uses `windows` 0.62, already in the lock.
  - macOS adds `objc`, `block`, `dispatch` and `core-graphics`.
  - `cargo deny` must pass.
- **The window handle.** On Windows, `PlatformConfig.hwnd` needs the main window's HWND. It comes
  from eframe's frame (`raw-window-handle`), so the controls are made on the first frame, not in
  `main`.
- **Platform names.** On Linux, `display_name = "Aria"` and `dbus_name = "aria"`. MPRIS appends
  the instance itself.
- **Threads.** Events arrive on the OS's thread. They are queued and wake the window with a repaint,
  as the global hotkeys already do. The app turns them into `Command`s in `logic`.
- **Code rules.** No `unwrap`/`expect` outside tests, and no `#[allow]`. Everything behind a small
  backend trait, so tests use a fake and never touch the OS.
- **Failure.** If the controls cannot be made (no D-Bus session, an old Windows), Aria logs it once
  at `warn` and carries on without them. No notice is shown.
- **Strings.** en, ru and hy.
- **Version.** 0.4.0.

## Approaches

1. **`souvlaki` (chosen by the user).**
   - One interface for SMTC, Now Playing and MPRIS, with metadata, cover and position.
   - It fits the build: the same `windows` version, and `dbus` is already there.
2. **Media keys as global hotkeys,** with `global-hotkey`, which Aria already uses.
   - Nothing shows in the system UI.
   - Windows may send the key to another player as well.
   - macOS needs an accessibility permission.
3. **SMTC by hand** with the `windows` crate. No new crates, but Windows only and more code.

## Decided by the user

- Media keys first in Phase 4: "Media keys first".
- Build them with souvlaki: "Use souvlaki".

## My calls (the user can change them)

- The setting, on by default, in Settings, General.
- The controls follow the bar's active player, Spotify included.
- Stop means pause, and a seek with no amount is 10 s.
- The position is sent every 5 s while playing.
- Volume works from MPRIS only; the other systems don't send it.
- Raise brings the window forward.
- Failures are only logged.

## Settled after the builder check (my calls)

- **When the controls are made.** In `logic`, the first time the frame gives a window handle. An
  Aria started minimized runs `logic` without drawing, so media keys work before the window is
  first shown. Without a handle yet, it tries again on the next `logic`.
- **Where the data comes from.**
  - Local tracks:
    - title, artist and album from the library's track;
    - duration and position from the player view and the playback clock.
  - Spotify:
    - title, artist and album from Spotify's playback state;
    - duration and position from its progress.
  - A field that is missing is left out. `Shown` or core grow only as far as these need.
- **Seeks.** Aria compares the clock with where it expects playback to be. A jump of more than 2 s
  counts as a seek and sends the position at once. That also catches seeks made elsewhere, such
  as in Spotify's own app.
- **A cover that arrives late.** If the cover's file is not in the cache when the track starts,
  the metadata is sent again once it is.
- **Raise** uses the same show-and-focus path as a second Aria starting. It un-minimizes the
  window, too.
- **Play and Pause** look at the bar's current state: Play while already playing does nothing, and
  so does Pause while paused.
- **Turning the setting off and on** drops the controls and makes them again (on Linux the D-Bus
  name is registered again).
- **Windows ties SMTC to the window.** Minimized is fine. Whether a hidden window, for the coming
  tray, keeps it is for the tray's design to check.

## Left to the builder

- Where it lives: an `aria-app` module, `media_controls.rs`, next to `hotkeys.rs`.
- The backend trait's shape and the fake.
- How the "what plays now" view (`ui/now_shown.rs`) feeds it.
- How the artwork cache gives a cover's file path.
- Exact copy.
