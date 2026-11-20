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
player-opened-no-heart = Opened from outside your library, so it has no heart. Add its folder in Settings, Library to keep it.
drop-hint = Drop to play. Folders are added to your library.
player-sleep = Sleep timer
player-sleep-15 = 15 minutes
player-sleep-30 = 30 minutes
player-sleep-45 = 45 minutes
player-sleep-60 = 1 hour
player-sleep-90 = 1 hour 30 minutes
player-sleep-end-of-track = After this track
player-sleep-off = Turn off
# $minutes (Number) - whole minutes until the sleep timer pauses the music, rounded up
player-sleep-left = { $minutes } min left
sleep-timer-done = The sleep timer paused the music.
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
settings-watch-folders = Watch folders for changes
settings-watch-folders-hint = Music added, removed or changed in these folders shows up by itself, a couple of seconds after the copying stops. Some network folders do not report changes; use Rescan for them.
settings-replay-gain = Volume levelling (ReplayGain)
settings-replay-gain-off = Off
settings-replay-gain-track = By track
settings-replay-gain-album = By album
settings-replay-gain-hint = Plays tracks at an even level, as the ReplayGain tags in their files say, never louder than they can go without clipping. Files without these tags play as they are. It takes effect from the next track, and a levelled track is no longer bit-perfect.
settings-back-up = Back up…
settings-restore = Restore…
settings-backup-hint = A backup holds your folders, the tracks' tags, favorites, playlists, history, sounds and settings, but no music: the files stay where they are. A restore takes place when Aria starts again, and keeps the current library beside it.
# $name (String) - the backup folder's name, e.g. Aria backup 2026-11-20 14.05
backup-done = Backed up to the folder { $name }.
backup-failed = The backup could not be made.
restore-staged = The backup is ready. Close Aria and start it again to restore it.
restore-not-a-backup = That folder holds no Aria backup. Choose the folder named "Aria backup" with the date.
restore-newer = That backup was made by a newer version of Aria, and this one cannot restore it.
restore-failed = The backup could not be restored.
restore-done = Restored from the backup. The library you had before is kept beside it, named "library.db.before-restore".
playlist-import = Import…
playlist-export = Export…
# $name (String) - the playlist's name; $count (Number) - how many entries it got
playlist-imported = { $count ->
    [one] Made the playlist { $name }, with { $count } entry.
   *[other] Made the playlist { $name }, with { $count } entries.
}
playlist-import-empty = That playlist file lists no files.
# $name (String) - the playlist's name
playlist-exported = Saved the playlist { $name }.
playlist-file-failed = The playlist file could not be read or written.
settings-updates = Updates
settings-check-updates = Check for updates
settings-check-updates-hint = Opens the page of the newest release in your browser. Aria itself sends nothing over the network.
settings-history = Keep playback history
settings-history-hint = History stays on this computer.
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
settings-history-clear = Clear playback history
settings-history-clear-confirm = Clear your playback history?
settings-history-clear-body = Recently played is emptied. Favorites and playlists stay. This can't be undone.
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
notice-pick-sounds-failed = Couldn't open the file picker.
notice-dismiss = Dismiss

## Library notices (toasts)

# $count (Number) - how many files and folders the scan could not read
library-scan-skipped = { $count ->
    [one] { $count } file or folder couldn't be read and was skipped.
   *[other] { $count } files or folders couldn't be read and were skipped.
}
# $count (Number) - how many of the files opened were skipped
open-skipped = { $count ->
    [one] { $count } file was skipped: it isn't music Aria plays, or it couldn't be read.
   *[other] { $count } files were skipped: they aren't music Aria plays, or they couldn't be read.
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
track-favorite = Add to favorites
track-unfavorite = Remove from favorites
track-add-to-playlist = Add to playlist
playlist-new = New playlist…
playlist-name-label = Name
playlist-create = Create
playlist-rename = Rename

## Playback notices (toasts)

# $title (String) - the track's title, e.g. Blue in Green
playback-track-failed = Couldn't play { $title }, so it was skipped.
playback-stopped-after-failures = Playback stopped because several tracks in a row couldn't be played.
playback-device-fallback = The audio device isn't available, so Aria switched to another one.
playback-no-device = No audio device is available, so playback is paused.
exclusive-unavailable = Exclusive mode isn't available on this device right now, so Aria is using the system mixer.

## Soundboard

sound-name-invalid = A sound name needs 1 to 50 characters.
sound-limit = The soundboard holds up to { $max } sounds, so some files weren't added.
sound-missing = The file of this sound can't be found, so nothing was played.
sound-failed = This sound couldn't be played. The music wasn't affected.
sound-device-failed = The second device for sounds couldn't be used.
microphone-failed = The microphone couldn't be opened. The music and sounds go on.
microphone-denied = The system refused Aria the microphone. Allow microphone access for desktop apps in the system's privacy settings.
microphone-on = Microphone on
microphone-off = Microphone off
microphone-needs-device = Choose a second device in Settings, Soundboard, to send the microphone to the chat.
microphone-is-cable = The microphone chosen is a virtual cable, so it wasn't opened: the chat would hear itself. Choose your real microphone in Settings, Soundboard.
hotkey-not-allowed = A hotkey needs Ctrl, Alt or Win, unless its key is an F key or a numpad key.
hotkey-taken = This hotkey is already used in Aria.
hotkey-unavailable = The system refused this hotkey, perhaps because another program uses it. It wasn't saved.
# $count (Number) - how many hotkeys the system refused to register
hotkeys-inactive = { $count ->
    [one] { $count } hotkey doesn't work: the system refused it, perhaps because another program uses it.
   *[other] { $count } hotkeys don't work: the system refused them, perhaps because other programs use them.
}
hotkey-inactive-hint = The system refused this hotkey, perhaps because another program uses it. It doesn't work now.
hotkey-dialog-title = Set a hotkey
hotkey-dialog-hint = Press the keys together. Escape cancels; Delete or Backspace clears the hotkey.
hotkey-dialog-clear = Clear
hotkeys-unavailable = Global hotkeys are unavailable on this system, so none of them work.
hotkey-dialog-numpad = Numpad key

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
instance-unreachable-title = Aria is already open
instance-unreachable-body = Another Aria window is already open, but it didn't answer. Switch to it, or close it (in Task Manager if it doesn't respond), and start Aria again.

## Language names: endonyms, identical in every locale

lang-en = English
lang-ru = Русский
lang-hy = Հայերեն

# Home page
home-recent = Recently played
home-added = Recently added
home-empty = Your library is empty
home-empty-hint = Add a folder with your music in Settings, Library.
home-recent-empty = Nothing played yet.

# Liked Songs page
liked-empty = No favorites yet
liked-empty-hint = Click the heart next to a track.

# Playlists page
playlists-empty = No playlists yet
playlists-empty-hint = Make one here, or choose Add to playlist in a track's menu.
playlist-empty = This playlist is empty
playlist-delete = Delete playlist
# $name (String) - the playlist's name
playlist-delete-confirm = Delete the playlist { $name }? Its tracks stay in your library.
playlist-remove-entry = Remove from playlist
# $count (Number) - how many entries of the playlist can't be shown because their files are gone or their folder was removed
playlist-unavailable = { $count ->
    [one] { $count } track unavailable
   *[other] { $count } tracks unavailable
}
# $count (Number) - how many tracks a playlist has
playlist-track-count = { $count ->
    [one] { $count } track
   *[other] { $count } tracks
}

## Output path, exclusive mode and quality
output-bit-perfect = Bit-perfect
output-bit-perfect-hint = Aria sends the track's samples to the device unchanged. Starting, pausing, resuming and seeking fade over 5 ms; gapless joins between tracks have no fade.
output-resampled = Resampled
output-resampled-hint = Exclusive output at a rate the track does not have, so the samples are resampled.
output-converted = Converted
output-converted-hint = Exclusive output, but the samples are not the file's: a lossy or float source, a depth that is unknown, above 24 bits or reduced, changed channels, or muted.
output-mixed = Mixed with a sound
output-mixed-hint = A soundboard sound is mixed into the stream, so it is not bit-perfect.
output-system-mixer = System mixer
output-system-mixer-hint = Playing through the system mixer, which may change the samples.
player-volume-locked = Volume is set on the device while exclusive mode is on.
player-mic-on = Microphone on: your voice goes to the second device. Click to turn it off.
player-mic-failed = The microphone couldn't be opened. Click to turn it off.
player-mic-device = Microphone: { $name }
settings-exclusive = Exclusive mode
settings-exclusive-hint = Other programs are silent on the device while Aria plays. Set the level on the device.
settings-exclusive-windows-only = Exclusive mode is available on Windows only.
quality-lossless = Lossless
quality-hi-res = Hi-Res
local-col-quality = Quality
local-lossless-only = Lossless only

## Soundboard page and settings

nav-soundboard = Soundboard
soundboard-empty = No sounds yet
soundboard-empty-hint = Add short audio files to play them with a click or a hotkey.
soundboard-add = Add sounds…
soundboard-stop-all = Stop all
sound-rename = Rename
sound-hotkey = Hotkey…
sound-hotkey-none = None
sound-volume = Volume
sound-remove = Remove
# $name (String) - the sound's name
sound-remove-confirm = Remove { $name } from the soundboard? The file is not deleted.
settings-soundboard = Soundboard
settings-sound-device = Second device
settings-sound-device-none = None
settings-sound-target = Play sounds to
settings-sound-target-mine = My device
settings-sound-target-second = Second device
settings-sound-target-both = Both
settings-sound-target-note = No second device is chosen, so sounds play on your device.
settings-global-hotkeys = Global hotkeys
settings-global-hotkeys-hint = Hotkeys work while another program has the focus. Aria receives only the combinations you set.
settings-stop-hotkey = Hotkey for Stop all
settings-voice-chat-hint = Aria doesn't install a virtual cable and never plays into your real microphone directly. With a cable as the second device and as the chat's microphone, the chat hears the sounds, the shared music and your microphone.
settings-share-music = Music also to the second device
settings-share-music-hint = Friends in the chat hear what you play. Your own volume and mute don't change it.
settings-share-music-volume = Music volume in the chat
settings-mic-device = Microphone
settings-mic-is-cable = This is a virtual cable, not a microphone: sending it into the cable would loop the chat into itself, so Aria won't open it. Choose your real microphone.
settings-mic-default = System default
settings-mic-send = Send my microphone to the second device
settings-mic-send-hint = The microphone is open only while this is on and a second device is chosen. Its sound goes only there; Aria doesn't record or keep it.
settings-mic-volume = Microphone volume
settings-mic-hotkey = Hotkey to turn the microphone on or off
settings-chat-needs-device = Choose a second device to send music or your microphone to a chat.
settings-chat-same-device = The second device is the one the music plays on, so nothing reaches a chat. Choose a virtual cable instead.
# $name (String) - the name of the virtual cable found
settings-cable-found = Found a virtual cable: { $name }
settings-cable-use = Use it
settings-cable-guide-1 = To send sound to a chat, install a free virtual cable, for example VB-CABLE:
settings-cable-guide-link = VB-CABLE, official page
settings-cable-guide-2 = Then restart Aria, or open Settings again.
settings-cable-guide-3 = Choose the cable here as the second device. In the chat program, choose its recording side (for VB-CABLE, "CABLE Output") as the microphone.
settings-cable-hint-vb = In the chat program, choose "CABLE Output" as the microphone.
settings-cable-hint-other = In the chat program, choose the cable's recording device as the microphone.

## Spotify notices
spotify-session-ended = Your Spotify session has ended. Connect again in Settings.
spotify-port-busy = Aria can't receive the sign-in: port 43819 on this computer is in use.
spotify-sign-in-denied = Spotify sign-in was cancelled.
spotify-sign-in-timed-out = Spotify sign-in took too long. Try again.
spotify-browser-failed = Aria couldn't open your browser for the Spotify sign-in.
spotify-offline = Aria can't reach Spotify. Check your connection.
# $seconds (Number) - how long Spotify asked Aria to wait
spotify-rate-limited = Spotify asked Aria to wait { $seconds } s before asking again.
spotify-premium-required = Playing on Spotify devices needs Spotify Premium.
spotify-no-device = No Spotify device is active. Open Spotify on a device first.
spotify-not-allow-listed = This Spotify account isn't on Aria's allow-list yet. Ask the app's owner to add it in the Spotify developer dashboard.
# $message (String) - Spotify's own error message, in English
spotify-error = Spotify said: { $message }
spotify-keyring-failed = Aria couldn't save the Spotify sign-in; you'll connect again next time.
spotify-signed-in-page = You're connected to Aria. You can close this tab.

## Settings: Spotify
settings-spotify = Spotify
settings-spotify-connect = Connect Spotify
settings-spotify-waiting = Waiting for the browser…
settings-spotify-cancel = Cancel
# $name (String) - the Spotify account's display name
settings-spotify-connected = Connected as { $name }
settings-spotify-device = Play on
settings-spotify-active-device = The active device
settings-spotify-refresh = Refresh library
settings-spotify-disconnect = Disconnect
settings-spotify-disconnect-confirm = Disconnect Spotify? Aria forgets the sign-in and your Spotify library.
settings-spotify-allow-list = While Aria's Spotify app is in development mode, only accounts its owner has added can connect (up to 5).
settings-spotify-no-keyring = This system has no keyring, so you'll connect again each time Aria starts.
settings-spotify-premium = Playing on your devices needs Spotify Premium.

## Player bar: Spotify
# $device (String) - the name of the Spotify device playing
player-spotify-on = On { $device }
player-spotify-not-here = Not for Spotify playback
player-devices-connect = Connect Spotify in Settings to choose a device

## Spotify pages
spotify-page-signed-out = Connect Spotify to see your music here.
spotify-loading = Loading your Spotify library…
spotify-search-hint = Search Spotify
spotify-search-none = Nothing on Spotify matches that.
spotify-songs = Songs
spotify-albums = Albums
spotify-artists = Artists
spotify-playlists = Playlists
# $count (Number) - how many saved tracks there are
spotify-liked = Liked Songs ({ $count })
spotify-empty = Nothing here yet.
# $owner (String) - the playlist owner's name
spotify-by = by { $owner }
