# Aria 0.4.2: the system tray

Status: approved by the user on 2026-11-26 ("Approve").

## Intent

Phase 4, the desktop experience: after media keys (0.4.0), Aria gets a tray icon. Then the
"Minimize to tray" switch, saved since Phase 1 but doing nothing, finally works. The point is to
keep Aria playing out of the way, off the taskbar, and to bring it back or control it from the
tray.

## What the user decided

- **A separate "Close to tray" switch, off by default.** With it off, the window's X quits Aria
  as it always has. With it on, X hides Aria to the tray and the music keeps playing.
- **All three systems**, with the `tray-icon` library. On Linux it uses that library's
  pure-Rust StatusNotifierItem backend (`ksni`), so no GTK. Its license (Unlicense) is added to
  `deny.toml`.

## Goals

- **The icon.** While Aria runs, its icon is in the tray: Windows' notification area, macOS's
  menu bar, or Linux's StatusNotifierItem tray.
  - Hovering shows "Aria" or, while something plays, "Title – Artist" (cut to the system's
    limit).
  - A left-click shows Aria (restored, in front) when it is hidden or minimized, and hides it
    otherwise. On macOS a click opens the menu, as menu bar icons do there.
  - A right-click opens the menu (see "The menu").
- **The menu.**
  - "Show Aria" or "Hide Aria" (whichever applies), a separator, "Play" or "Pause" (whichever
    applies), "Next", "Previous", a separator, and "Quit Aria".
  - The labels are in Aria's language, in en, ru and hy, and change when the language does.
  - Play, Next and Previous do what the player bar does, so they control Spotify while the bar
    shows Spotify.
  - "Quit Aria" really quits, even with "Close to tray" on.
- **"Minimize to tray" (existing setting `minimize_to_tray`, default off).** With it on,
  minimizing the main window hides it instead: it leaves the taskbar and Alt+Tab, and only the
  tray brings it back.
- **"Close to tray" (new setting `close_to_tray`, default off).**
  - With it on, the X (or Alt+F4) hides the window instead of quitting.
  - "Quit Aria" in the tray and "Restart to update" still quit, and shutting the system down is
    not affected (see item 6 below).
- **Both switches** are in Settings, General, next to each other. The hints say where they apply:
  "Needs a tray: on Linux, a desktop that shows tray icons."
- **A hidden window counts as not shown**, like a minimized one. So Aria stops polling Spotify
  while it sits in the tray, and the 0.4.0 minimized refresh for the system's media controls
  keeps them current there too.
- **Starting Aria again brings a hidden Aria back.** This already happens: the single-instance
  wake sends Visible, Restore and Focus. It is the way back on a Linux desktop without a tray.
- **The mini-player.** When the main window hides, an open mini-player stays as it is. Its
  button that returns to the main window shows the main window again.

## Non-goals

- No hiding of the mini-player itself to the tray.
- No "start minimized" or "start with the system" (that comes next in Phase 4, separately).
- No notifications (later).
- No tray-only mode with the icon off. The icon is always there while Aria runs.

## Anti-goals

- Aria must never become unreachable. It hides only after its tray icon was made. If the tray
  cannot be made (an error, or tests), both switches leave minimize and close as they are.
- On Linux the icon can be "made" without any tray showing it (plain GNOME). Aria then hides, and
  starting it again is the way back. The hint says so in short. Aria can't tell reliably, so it
  doesn't guess.
- Quitting must never be swallowed. A quit that comes from the tray, the updater or the system
  always closes.

## Constraints

- Project rules:
  - No `unwrap` or `expect` outside tests, no `unsafe`, no `#[allow]`.
  - Tests never touch the installed Aria or %APPDATA%\Aria, never open the microphone, and
    never make a real tray icon (a fake backend, as with media controls).
- `tray-icon` 0.26.1, the newest release that is at least two weeks old (0.27.0 came out
  2026-11-25).
  - Windows and macOS: default features off.
  - Linux and the BSDs: `features = ["ksni"]`, default features off (no `libappindicator`, no
    GTK).
  - `muda` (the menu) comes with it.
- The tray is made on the main thread, inside `eframe::App::logic` (eframe 0.37). The icon and
  menu callbacks queue their events and ask for a repaint. eframe 0.37 runs `logic` for an
  invisible or minimized window when a repaint is asked for (it paints invisible windows directly
  on Windows), so a click on the icon of a hidden Aria is handled.
- Windows needs the window's message loop on that same thread: winit's, already running.
- Hiding is `ViewportCommand::Visible(false)`. Showing is `Visible(true)`, then
  `Minimized(false)`, then `Focus`, as the single-instance wake already does.
- The close is cancelled with `ViewportCommand::CancelClose` when `close_requested()` is seen
  and "Close to tray" is on (and the tray exists, and this is not a real quit).

## Approaches considered

1. **`tray-icon` on all three, ksni on Linux. Chosen by the user.** It is the same family as the
   global-hotkey crate Aria already uses. Linux needs no system library, and the one new license
   is Unlicense.
2. `tray-icon` on Windows and macOS only. Smaller, but Linux gets nothing.
3. Windows only, via `windows-sys` by hand. That needs `unsafe`, which the project forbids, and
   leaves out macOS and Linux, which Aria already ships.

## The how, as far as decided

- **The backend.** A `TrayBackend` trait in a new `crates/aria-app/src/tray.rs`, as
  `media_controls.rs` does:
  - It sets the tooltip, sets the menu's labels and states, and receives events.
  - `system_factory()` makes the real one (tray-icon).
  - Tests use a fake.
  - `AppDeps.tray: Option<TrayFactory>`. `main.rs` passes the system one.
- **Events in.** `TrayEvent`: `Toggle` (left-click), `Show`, `Hide`, `TogglePlay`, `Next`,
  `Previous` and `Quit`. A pure function maps them to the app's actions, in the style of
  `media_controls::action_for`.
- **Events out.** A tray state (shown or hidden, playing, tooltip text, labels) is worked out
  each frame and sent to the backend only when it changed, like `MediaSync`.
- **Quitting.** The app keeps a `quitting` flag. It is set by "Quit Aria", by the updater's
  install, and by anything that sends `ViewportCommand::Close` itself. While it is set,
  close-to-tray never cancels.
- **The icon image.** The same 256-pixel PNG as the window's icon (`icon.rs`), downscaled by
  the library or the system.
- **Strings.** New keys in en, ru and hy:
  - The settings: `settings-close-to-tray` and `settings-close-to-tray-hint`. The rewritten
    `settings-minimize-to-tray-hint` drops "Takes effect in Phase 4".
  - The tray: `tray-show`, `tray-hide`, `tray-play`, `tray-pause`, `tray-next`, `tray-previous`
    and `tray-quit`.
- **Documents.**
  - PRIVACY: the tray sends nothing anywhere. On Linux the D-Bus tray is local.
  - CHANGELOG, README, ROADMAP.
  - `docs/testing/0.4.2-manual-check.md`.
  - THIRD_PARTY_NOTICES, if the project keeps one per dependency.
- **Version.** 0.4.2.

## Settled after the builder check (my calls, from the code)

1. **Knowing the window is hidden.**
   - Aria keeps its own `hidden` flag. It sets the flag when it sends `Visible(false)` and
     clears it when it sends `Visible(true)`, through one helper used by the tray, the
     single-instance wake, the media controls' "Raise" and the mini-player's way back.
   - egui has no reliable flag for this: `ViewportInfo::visible()` means "not minimized and not
     occluded" and is never set from `Visible`.
   - A hidden window is off the taskbar and Alt+Tab, so nothing else can show it behind Aria's
     back.
2. **What "shown" means.** For Spotify polling, it is `!(minimized || hidden) || mini open`. The
   mini-player's rule stays as it is.
3. **The mini-player while hidden.**
   - eframe 0.37 still runs `ui()` for a hidden root window as long as one of its child
     viewports is visible (`is_viewport_or_descendant_visible`). So an open mini-player keeps
     being drawn live.
   - The branch that today draws only the mini while minimized becomes
     `(minimized || hidden) && mini open`.
4. **"Minimize to tray" fires on the change only.** Minimized going from false to true while
   shown hides the window once. Showing again sends `Visible(true)`, `Minimized(false)` and
   `Focus`, in that order.
5. **"Hide Aria", "Show Aria" and the left-click act on the main window only.**
   - The label follows the main window: "Show Aria" while it is hidden or minimized, otherwise
     "Hide Aria".
   - An open mini-player is left as it is.
6. **Real quits.**
   - `quitting` is set by "Quit Aria" in the tray and by the updater's install.
   - Windows shutting down or logging off doesn't reach the window as a close request: winit
     0.30 doesn't handle `WM_QUERYENDSESSION`. So "Close to tray" cannot block it, and nothing
     changes there from today.
   - Quitting from the tray while the full-screen record is open goes through the same `Close`
     path as the X, so the window's state is saved as on any quit.
7. **"The tray exists"** means the factory returned a backend.
   - If it fails, or there is none (tests), the switches do nothing and X quits.
   - A tray that disappears later (Explorer restarting, the Linux host going away) isn't
     detected. Starting Aria again still brings the window back. Explorer restarting re-adds
     the icon by itself through tray-icon's TaskbarCreated handling, if it has that; otherwise
     this is noted in the manual check.
   - Aria makes no attempt to probe Linux for a StatusNotifierWatcher.
8. **The setting `close_to_tray`.**
   - It reads leniently like `media_controls` (a missing or odd value is `false`).
   - There is no settings version bump, and the TOML tests that list every key gain it.
9. **The tooltip and the Play/Pause state** come from the same `now_playing()` the media
   controls use: local or Spotify, whichever the player bar shows.
10. **Licenses.** cargo-deny in CI decides. Unlicense is added for `ksni`. Any other new
    license in its tree (zbus and friends) is added only if it is already a common permissive
    one, and named in the commit. CI's Linux image needs nothing new, since ksni and zbus are
    pure Rust. The BSDs are built by no one, so they are not promised.

## Left to the builder

- Private names and file layout inside `tray.rs`.
- How the fake records calls.
- The exact separator use in the menu.
- How the tooltip is shortened (Windows limits it to 127 characters).
- Russian and Armenian wording.
