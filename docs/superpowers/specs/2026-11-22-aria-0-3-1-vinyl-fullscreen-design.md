# Aria 0.3.1: full-screen "now playing" with a spinning record — design

## What it is for

Agreed with the user on 2026-11-22. A full-screen mode, opened like Spotify's from a button in
the bottom-right corner of the player bar, that shows what is playing as a record turning on a
turntable. The reference is the MD Vinyl app (a turntable with a tonearm, the cover as the
record's label, on a wide screen the sleeve beside the record, a choice of record finishes).
Aria takes the idea, not the app's look: no text, layout details, label printing or artwork of
MD Vinyl are copied.

It is what plays, made beautiful, with the controls at hand when the mouse moves: Spotify's
full-screen mode, with a record instead of a flat cover. It works for local tracks and for
Spotify playback (0.3.0) alike.

A floating mini-player window comes next, in 0.3.2; it is not part of this design. The bar's
new button leaves room for its button beside it, as Spotify's bar does.

## Scope (the user chose the middle option)

- The background and the turntable follow Aria's theme (light or dark). No colour themes of
  its own.
- Record finishes: **Black**, **Tinted** (translucent, its colour taken from the cover),
  **Splatter**, **Marble**, and **Random** (a finish picked per track). The choice is made in a
  small panel inside the mode and kept in the settings.
- On a wide window the sleeve (the large cover) lies beside the record, partly under it.
- Everything is drawn by Aria: no image assets.

Not in 0.3.1: custom colour themes, the user's own image as a record, the floating window
(0.3.2), lyrics, a visualizer.

Failures even if it works: a blurry cover (a small thumbnail stretched), CPU or GPU work while
the mode is closed or nothing plays, a copy of MD Vinyl's branding, the window left in OS full
screen after the mode closes.

## Approaches considered

- **A. Draw the record from egui shapes every frame** (grooves as circles, splatter and marble
  as strokes, rotated geometry). No stored images, but thousands of shapes per frame, coarser
  grooves and marble, more CPU while turning.
- **B. Render each finish once into a texture, then rotate it** (chosen by the user). A
  finish is drawn once, off the UI thread, into an RGBA image of about 1024×1024 with
  antialiased grooves, grain, splatter or marble; on screen the record and its label are two
  rotated textured quads, and a still highlight over them gives the light that stays put while
  the record turns. Picture quality at almost no cost per frame, about 4 MB per finish, held
  only while the mode is open.
- (A separate OS window for the mode was not considered further: the main window going full
  screen does the job, and a second window is 0.3.2's subject.)

## How it behaves

### Opening and closing

- A button at the right end of the player bar ("Full screen", Phosphor's corners-out icon)
  opens the mode; in the mode a button at the top right ("Exit full screen") and **Esc** close
  it. **F11** toggles it too.
- Opening sends the window to OS full screen; closing restores what it was (full screen or not,
  as before). If the user leaves OS full screen another way (Windows key, macOS green button),
  the mode closes.
- While the mode is open the sidebar, pages, right panel and player bar are not drawn; toasts
  still show. Global hotkeys and media keys keep working.
- With nothing current (local player stopped with no track, Spotify shown with nothing
  playing), the mode shows the record at rest, the tonearm off, and "Nothing playing".

### The turntable

- The record turns at 33⅓ rpm while playing. Play speeds it up over about 0.6 s, pause slows it
  to a stop over about 0.8 s (no jump of the angle). The angle is kept while paused.
- The tonearm rests off the record when paused or stopped and swings onto the outer grooves
  over about 0.5 s when playback starts; it moves back on pause. It does not track progress
  across the record.
- The label is the cover, cut to a circle, with a spindle hole. Without a cover the label is a
  plain disc in a colour from the finish, with the title and artist printed on it.
- A finish's highlight (a soft sheen across the grooves) does not rotate.
- Layout: when the window is wider than about 1.4 times its height, the sleeve (a square with
  the full cover and a slight shadow) sits left of centre and the record overlaps its right
  edge, the tonearm at the record's top right; otherwise the record is large and partly off the
  left edge, as on a phone in portrait, and the tonearm is at the top right. The title (large)
  and the artists (muted) sit at the bottom left.

### Controls

- Moving the mouse shows, at the bottom: shuffle, previous, play/pause, next, repeat, the seek
  bar with times, and the volume; at the top right: the finish button and "Exit full screen".
  After 3 s without movement they fade out with the cursor; hovering over them keeps them.
  The keyboard works without showing them: Space plays or pauses, the arrow keys seek and
  change the volume, as the bar's do now.
- The controls act on whichever player the bar shows (local or Spotify), with the same rules
  as the player bar in 0.3.0 (Spotify's volume sent on release, mute not offered for
  Spotify).
- The finish button opens a small panel along the bottom: the five finishes as round swatches
  (Random first), the chosen one ringed. Choosing one applies at once and is saved
  (`vinyl_finish` in `settings.toml`, default Black).

### Finishes

- **Black**: deep black with fine concentric grooves, a few wider gaps between "tracks", grain.
- **Tinted**: translucent coloured vinyl; the colour is the cover's dominant colour, darkened
  enough to read as vinyl; the background shows faintly through. Without a cover, a neutral
  smoky grey.
- **Splatter**: a translucent smoky base with white and cover-coloured streaks thrown outwards
  from near the centre.
- **Marble**: swirls of two or three colours from the cover over black.
- **Random**: for each new track, one of the four above, chosen from the track's identity so
  that the same track gets the same finish.
- Finishes that use the cover's colours are rendered again when the cover changes; Black is
  rendered once. Rendering is deterministic for a given finish and colours, so tests can check
  it.

### Large covers

- Local tracks: the current track's embedded cover is read again from the file, off the UI
  thread, and scaled to fit 1024 px (the library keeps only 256 px thumbnails). The thumbnail
  shows until it arrives. Opened files (outside the library) work the same way. Aria still only
  reads audio files.
- Spotify: the item's largest image (usually 640 px) is fetched, through the existing covers
  thread and its rules (HTTPS only, no token, size limit), and kept in memory.
- Only the current track's large cover is held, and only while the mode is open.

### Cost

- The window repaints at the display's rate only while the mode is open and the record is
  turning or a transition is running; at rest it repaints as the rest of Aria does.
- Finish textures and the large cover are dropped when the mode closes.

## Constraints

- Rust 2024, egui/eframe 0.37 (glow), as the rest of Aria; `image` 0.25 for decoding and for
  rendering finishes. No new dependency unless it is needed and its licence passes cargo-deny.
- No `unwrap`/`expect` outside tests, no `unsafe`, no `#[allow]`; clippy and fmt clean.
- Every visible string is a Fluent key in English, Russian and Armenian; every control has an
  accessible name.
- Nothing new is sent over the network except Spotify's larger image for the Spotify item, from
  Spotify's image servers, as in 0.3.0. Nothing new is written to disk except `vinyl_finish`.
- Works on Windows, macOS and Linux (OS full screen through eframe's viewport command).

## Left to the builder

- The exact drawing of each finish (groove count, grain, streak shapes), within the
  descriptions above; the colour extraction from the cover.
- Easing curves, exact sizes and margins, the shadow of the sleeve.
- How the large-cover request is carried (a new library request and a new Spotify request, or a
  size on the existing cover request).
- Where the mode lives in the code (a new module beside the player bar is the natural place)
  and how much of the player bar's control code it shares.

## Calls made after the builder check (the assistant's, open to correction)

- **Spotify's large image.** The Spotify model keeps one image URL today (the smallest of at
  least 64 px). It gains a second, `image_large`: the widest image, HTTPS only. It travels with
  the track wherever the track does. When Spotify has only a small image, the mode shows the
  largest there is; that is Spotify's limit, not a thumbnail chosen by Aria.
- **Local large cover.** It is read from the current track's file, whose path the library
  snapshot or the opened tracks hold. A file that is gone or has no embedded cover keeps the
  256 px thumbnail if there is one, else the label falls back to the plain disc with the title.
  Folder images (`cover.jpg`) are not read: Aria reads only audio files.
- **Full screen state.** The mode uses the full screen state eframe reports for the window
  (`ViewportInfo::fullscreen`). On entering it remembers that state. On leaving it asks for
  `Fullscreen(false)` only if the window was not full screen before. If the state turns off
  while the mode is open, the mode closes; if a platform never reports it, the mode simply stays
  until Esc, the button or F11.
- **While a finish renders.** The record shows the finish last rendered, or Black, which is
  rendered first and fast. A render that finishes for a cover no longer current is dropped. The
  render size is 1024×1024 RGBA.
- **Transparency.** Where a translucent record (Tinted, Splatter) overlaps the sleeve, the
  sleeve shows faintly through it: the sleeve is drawn first, the record over it.
- **Colours from the cover.** Taken from a small downscale of the cover, preferring saturated
  colours. A cover with no colour worth using (greyscale, near black, near white) gives the
  neutral smoky grey for Tinted and greys for Marble and Splatter.
- **Layout.** The wide layout starts at a width of 1.4 times the height, to be tuned by eye. The
  record's size follows the smaller of the window's sides; there is no minimum beyond Aria's own
  minimum window size.
- **Minimized window.** Nothing repaints while the window is minimized. A second launch's "show
  the window" request brings the window forward as before and leaves the mode as it was. Aria
  has no tray yet (Phase 4).
- **Random's choice.** A stable hash (FNV-1a) of the track's identity: the Spotify URI, or the
  file path for a local or opened track. The same track gets the same finish across restarts.
- **Keys.** Esc closes the finish panel first if it is open, then the mode. While the mode is
  open the page and navigation shortcuts do nothing; Space, the arrows and F11 work as described.
- **Strings.** About 12 to 15 new keys, written in English, Russian and Armenian by the
  assistant as for every earlier key.

## Testing

- Unit tests: the angle under play, pause and resume (no jump, 33⅓ rpm), the tonearm's
  position over time, the layout's choice of wide and narrow, Random's choice being stable per
  track, each finish rendering to the right size with a transparent hole and outside, and the
  same output twice for the same input.
- UI tests (kittest): the bar's button opens the mode and Esc closes it; the controls appear on
  mouse movement and hide after the delay; the finish panel saves the choice; controls in the
  mode send the same requests as the bar's for local and for Spotify playback; with nothing
  playing the mode says so.
- A hand check for the look: the record turning smoothly, the sleeve on a wide screen, each
  finish, OS full screen entered and left on Windows (macOS and Linux as far as possible).
