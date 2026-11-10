### Aria UI strings: English, the source locale.
### Every other locale defines exactly the same message ids and placeables.

## Navigation: sidebar entries, page headings and history buttons

nav-home = Home
nav-search = Search
nav-library = Your Library
nav-liked = Liked Songs
nav-playlists = Playlists
nav-albums = Albums
nav-artists = Artists
nav-local = Local Music
nav-settings = Settings
nav-back = Back
nav-forward = Forward

## Empty states

# $phase (Number) - the roadmap phase that delivers the page, e.g. 2
empty-phase-pending = Available in Phase { $phase }
empty-nothing-playing = Nothing playing
empty-right-panel = Your queue is empty

## Player controls (tooltips)

player-play = Play
player-previous = Previous
player-next = Next
player-volume = Volume
player-devices = Devices
player-queue = Queue
player-lyrics = Lyrics
player-pause = Pause
player-shuffle = Shuffle
player-repeat-off = Repeat: off
player-repeat-all = Repeat: all
player-repeat-one = Repeat: one track
player-mute = Mute
player-unmute = Unmute
player-seek = Seek
player-unknown-title = Unknown title
# $phase (Number) - the roadmap phase that delivers the control, e.g. 2
player-coming-in = Coming in Phase { $phase }

## Queue panel

queue-now-playing = Now playing
queue-next-user = Next in queue
queue-next-list = Next from list
queue-clear = Clear queue
# $title - the track's title
queue-remove = Remove from queue: { $title }

## Settings page

settings-general = General
settings-language = Language
settings-language-system = System default
settings-theme = Theme
settings-theme-dark = Dark
settings-theme-light = Light
settings-theme-system = System
settings-accent-dark = Dark theme accent
settings-accent-light = Light theme accent
settings-accent-reset = Reset to default
settings-minimize-to-tray = Minimize to tray
settings-minimize-to-tray-hint = Minimizing the window sends Aria to the system tray instead of the taskbar. Takes effect in Phase 4.
settings-playback = Playback
settings-output-device = Output device
settings-device-system-default = System default
settings-device-unavailable = Unavailable device
settings-gapless = Gapless playback
settings-library = Library
settings-folders-empty = No folders yet. Add a folder with your music.
settings-folder-unavailable = Can't be reached
settings-remove = Remove
settings-remove-named = Remove { $folder }
settings-remove-folder-title = Remove this folder?
settings-remove-folder-body = The folder is taken out of your library. Its files stay on disk.
settings-clear-artwork = Clear artwork cache
settings-clear-artwork-title = Clear the artwork cache?
settings-clear-artwork-body = Saved covers are deleted. They are rebuilt the next time your library is scanned.
settings-advanced = Advanced
settings-log-level = Log level
settings-log-level-error = Error
settings-log-level-warn = Warning
settings-log-level-info = Info
settings-log-level-debug = Debug
settings-log-level-trace = Trace
settings-open-config = Open settings folder
settings-open-logs = Open log folder
settings-reset = Reset settings
settings-reset-confirm-title = Reset all settings?
settings-reset-confirm-body = All settings will return to their defaults. This can't be undone.
settings-cancel = Cancel
settings-confirm = Confirm
settings-about = About
settings-version = Version
settings-license = License
settings-third-party = Third-party licenses are listed in THIRD_PARTY_NOTICES.md

## Notices (toasts)

notice-settings-recovered = Your settings file couldn't be read, so defaults were restored. A backup of the old file was kept.
notice-settings-readonly = Couldn't access the settings file, so defaults are in use. Changes may not be saved.
notice-settings-save-failed = Couldn't save settings. Aria will try again after your next change.
notice-open-folder-failed = Couldn't open the folder.
notice-pick-folder-failed = Couldn't open the folder picker.
notice-dismiss = Dismiss

## Library notices (toasts)

# $count (Number) - how many files and folders the scan could not read
library-scan-skipped = { $count ->
    [one] { $count } file or folder couldn't be read and was skipped.
   *[other] { $count } files or folders couldn't be read and were skipped.
}
# $folder (String) - the last part of the folder's path, e.g. Music
library-folder-unavailable = The folder { $folder } can't be reached, or all its files were moved or deleted. Its tracks are kept, and you can remove the folder in Settings.
library-folder-nested = This folder is inside a folder that's already in your library.
library-folder-duplicate = This folder is already in your library.
library-recovered = The library database was damaged, so a new one was started. The old file was kept as a backup.
library-newer = The library was saved by a newer version of Aria, so it's turned off. Update Aria to use it.
library-unavailable = The library couldn't be opened, so it's turned off for now. The rest of Aria keeps working.
library-action-failed = Something went wrong in the library, and that action didn't finish. Your music files weren't changed.
playlist-name-invalid = A playlist name needs 1 to 100 characters.

## Playback notices (toasts)

# $title (String) - the track's title, e.g. Blue in Green
playback-track-failed = Couldn't play { $title }, so it was skipped.
playback-stopped-after-failures = Playback stopped because several tracks in a row couldn't be played.
playback-device-fallback = The audio device isn't available, so Aria switched to another one.
playback-no-device = No audio device is available, so playback is paused.

## Local Music page

local-add-folder = Add folder
local-rescan = Rescan
local-tab-tracks = Tracks
local-tab-albums = Albums
local-tab-artists = Artists
local-search = Search your music
local-col-number = #
local-col-title = Title
local-col-artist = Artist
local-col-album = Album
local-col-duration = Duration
local-play = Play
local-play-next = Play next
local-add-to-queue = Add to queue
local-back = All tracks
local-empty-title = No music yet
local-empty-body = Add a folder with your music and Aria will find it.
local-no-tracks-title = No tracks found
local-no-tracks-body = Tracks show up here once Aria has read your folders.
local-no-results = No tracks match your search.
# $count (Number) - how many tracks an album or an artist has
local-track-count = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}
# $count (Number) - how many audio files the running scan has looked at so far
library-scanning = { $count ->
    [one] Scanning your folders, { $count } file so far
   *[other] Scanning your folders, { $count } files so far
}
library-unknown-artist = Unknown artist
library-unknown-album = Unknown album

## Fatal startup errors (a message box shown before Aria quits)

error-startup-title = Aria couldn't start
error-startup-window = Aria couldn't open its window.

## Language names: endonyms, identical in every locale

lang-en = English
lang-ru = Русский
lang-hy = Հայերեն
