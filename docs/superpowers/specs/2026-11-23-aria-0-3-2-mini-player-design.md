# Aria 0.3.2: the floating mini-player — design

## What it is for

Agreed with the user on 2026-11-23. A small window of its own, like Spotify's mini-player,
that shows what is playing and keeps the main controls at hand while the user works in other
programs. The main window stays as it is (open, moved, minimized); the mini-player is beside
it, not a shrunken form of it.

The look the user chose: **the cover fills a square window**. Moving the mouse over the window
shows, over the cover, the title and artists, previous / play-pause / next, a progress bar and
the window's own buttons. It works for local tracks and for Spotify playback (0.3.0) alike.

Failures even if it works: a blurry cover (the 128 px or 256 px thumbnail stretched over a
300 px window on a high-DPI screen), the mini-player going stale while the main window is
minimized (that is when it is most used), Spotify polled when nobody can see anything, a second
window that cannot be moved or closed, CPU or GPU work at rest.

## Approaches considered

- **A. A separate small always-on-top window** (chosen by the user). The main window keeps its
  state; the mini-player is a second OS window drawn by the same app.
- **B. The main window shrinks into the mini-player and grows back.** One window, but it loses
  the main window's place and size each time and cannot show both at once. Not chosen.

Within A, the second window is an egui **immediate viewport** (`Context::show_viewport_immediate`)
drawn from the app's own frame, so it reads the app's state directly with no shared locks. In
eframe 0.37 with glow, the root's UI keeps running while the root is minimized or hidden as
long as a child immediate viewport is visible (`is_viewport_or_descendant_visible` in
`glow_integration.rs`), so the mini-player keeps drawing while the main window is minimized. A
deferred viewport (its own callback, state behind `Arc<Mutex>`) was set aside: the player bar's
data borrows the whole app, and sharing it would mean copying it every frame.

## How it behaves

### Opening and closing

- A new button in the player bar, just left of the full-screen button ("Mini player",
  Phosphor's picture-in-picture icon) opens the mini-player; pressing it again closes it. The
  button shows as on while the mini-player is open.
- The mini-player's own close button (×, top right, shown on hover) closes it. Closing it never
  closes Aria. Closing the main window quits Aria as now, and the mini-player goes with it.
- The mini-player's "Back to Aria" button (top left, on hover) shows the main window (restores
  it if minimized, brings it forward) and leaves the mini-player open.
- The full-screen record and the mini-player are not open together: opening one closes the
  other.
- The mini-player is not reopened when Aria starts; it starts closed every time.

### The window

- Borderless, square to begin with: 300 × 300 logical pixels the first time, then the size and
  place it last had. It can be resized from its bottom-right corner (a small grip shown on hover)
  between 200 × 200 and 640 × 640, keeping no fixed shape; the cover fills it, cropped to the
  window's shape and centred.
- It is moved by dragging anywhere on it that is not a button or the progress bar.
- **Always on top** by default. A pin button (top right, beside ×, on hover) turns that off and
  on; the choice is kept.
- Not shown in the taskbar (where the system allows it); it is a part of Aria, not a second
  program. Its title, for the system and screen readers, is "Aria mini player".
- Place and size are kept in `settings.toml` and restored when it opens next. A saved place that
  is off every screen now (a monitor unplugged) is not used; the system's default place is.
- On Linux with Wayland the compositor decides the place and may ignore "always on top"; Aria
  asks and does not insist.

### What it shows

- The cover fills the window. Local tracks use the current track's large cover (read again from
  the file at up to 1024 px, as the full-screen record does); Spotify items use the item's
  largest image, fetched as the full-screen record does. The thumbnail shows until the large
  cover arrives.
- Without a cover: a plain surface in Aria's theme colour with the music-note placeholder.
- Nothing playing: the placeholder, and "Nothing playing" in the middle.
- With the mouse over the window, a dark gradient rises from the bottom and shows:
  - the title (one line, cut with an ellipsis) and the artists (muted) at the bottom left;
  - previous, play/pause and next, centred above the progress bar;
  - the progress bar along the bottom edge, which seeks on click and drag, like the bar's;
  - at the top: "Back to Aria" on the left, the pin and × on the right, over a light top
    gradient.
- Without the mouse over it only the cover shows. The overlay shows and hides with a short fade
  (about 0.15 s). It shows while the mouse is over the window and hides when it leaves, with no
  timer.
- The controls act on whichever player the bar shows (local or Spotify), with the same rules as
  the player bar: Spotify's seek and transport go to Spotify's device.
- Keys while the mini-player has focus: Space plays or pauses; Esc does nothing (it is not a
  mode). Global hotkeys and media keys keep working as always.

### Staying live while the main window is minimized

- While the mini-player is open, Aria counts as shown for Spotify: the backend keeps polling at
  the "bar shows Spotify" rate (2 s) or the slow rate (15 s), as when the main window is shown.
  Closing the mini-player with the main window minimized stops polling again.
- While the main window is minimized and the mini-player is open, Aria draws only the
  mini-player, not the hidden main window's pages.

### Cost

- At rest (overlay hidden) the mini-player repaints only when what it shows changes (track,
  cover, play state). With the overlay shown and the music playing it repaints about once a
  second for the progress bar, and at the display's rate only during the fade or a drag.
- The large cover is kept while the full-screen record or the mini-player shows it, and dropped
  when neither is open (today it is dropped when the full-screen record closes).

## Constraints

- Rust 2024, egui/eframe 0.37 (glow), as the rest of Aria. No new dependency.
- No `unwrap`/`expect` outside tests, no `unsafe`, no `#[allow]`; clippy and fmt clean.
- Every visible string and every accessible name is a Fluent key in English, Russian and
  Armenian.
- Nothing new is sent over the network. Nothing new is written to disk except the mini-player's
  settings (place, size, pin).
- Works on Windows, macOS and Linux (X11 fully; Wayland as the compositor allows).

## Left to the builder

- Exact sizes of the overlay's buttons, gradients and margins; the fade curve.
- How the window's place and size are read back (from `ViewportInfo` each frame, saved when
  they change and on close) and how settings saves are batched.
- How the resize grip is done (`ViewportCommand::BeginResize`) and the drag
  (`ViewportCommand::StartDrag`).
- How much of the player bar's and the full-screen record's code is shared (the seek bar, the
  large-cover lookup, what plays now).
- Where the module lives (`ui/mini/` beside `ui/vinyl/` is the natural place).

## Testing

- Unit tests: the overlay's fade over time; the saved place rejected when it is off every
  monitor; the settings' new fields read, written and falling back when wrong.
- UI tests (kittest) of the mini-player's drawing in a plain harness (the drawing is a function
  of a `Ui` and the bar's data, so it can be tested without a second OS window): hover shows the
  controls and their accessible names; play/pause, next, previous and a seek send the same
  actions as the bar's for local and for Spotify playback; nothing playing says so; the pin and
  × send their actions.
- App tests: the bar's button opens and closes the mini-player; opening the full-screen record
  closes it and the other way round; "Back to Aria" asks to show the main window; Spotify stays
  "shown" while the main window is minimized and the mini-player is open; the large cover is
  kept while the mini-player is open.
- A hand check for the look and for what only a real second window shows: always on top, drag,
  resize, the place restored, staying live with the main window minimized, Windows first, macOS
  and Linux as far as possible.

## Calls made after the builder check (the assistant's, open to correction)

- **Cost while the main window is shown.** An immediate viewport is painted with every pass of
  the main window, and the main window already repaints about 4 times a second while music
  plays (the player bar's clock). So with music playing the mini-player repaints at that rate
  too, overlay or not; a 300 px window, so a small cost. The rest rule holds where it matters:
  paused or stopped, with the overlay hidden, nothing repaints. A deferred viewport would avoid
  this, at the cost of copying the player's state across threads every frame; not worth it.
- **Wayland.** A compositor that stops sending frame callbacks to the minimized main window
  makes eframe run only `App::logic` (eframe `run.rs`, `MISSED_REDRAW_TIMEOUT`), so on such a
  compositor the mini-player stops updating while the main window is minimized, and updates
  again when it is shown. `RequestPaintWhileHidden` does not change that path. This is written
  down as a Wayland limit (README, CHANGELOG, the manual check), not fixed in 0.3.2. Windows,
  macOS and X11 paint minimized windows and are not affected.
- **Screen readers.** In eframe's glow backend AccessKit serves only the main window, so a
  screen reader sees the mini-player's title and not its buttons. The buttons still get
  accessible names (they serve the tests and any later backend), and everything the
  mini-player does is also in the main window, which screen readers reach. Written down as a
  limit.
- **Textures while the main window is minimized.** Texture and font-atlas changes made in the
  main window's pass are uploaded only when the main window paints, which it does not while
  minimized. So while the main window is minimized the mini-player is drawn last in the frame,
  nothing is drawn after it, and the large cover's texture is created inside the mini-player's
  own pass; the hand check covers a track change with the main window minimized.
