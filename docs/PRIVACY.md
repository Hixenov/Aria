# Privacy

Aria sends nothing over the network unless you connect Spotify (since 0.3.0; see "Spotify"
below) or turn on update checks (since 0.3.5; see "Updates" below). Until then it makes no
network requests at runtime, and it has no telemetry. Nothing
from your music library, tags or folders is sent anywhere, Spotify included.

## What Aria stores

Everything Aria writes stays on your machine:

1. `settings.toml`: your preferences (language, theme, accent colours, log level, and so on).
2. `settings.toml.tmp`: a temporary file used while saving. Each save writes it, then
   renames it over `settings.toml`; it exists only for that moment, and a failed save
   removes it.
3. `settings.toml.bak-<secs>` (or `settings.toml.bak-<secs>-<N>` when that name is taken):
   a backup of a settings file that could not be used, made before the defaults replace it.
   `<secs>` is the Unix time of the backup. Backups hold your old settings and are kept
   indefinitely; Aria never deletes them, so remove them yourself when you no longer need
   them.
4. `app.ron`: eframe's persisted state: window position/size and UI layout state (egui memory).
5. Logs: daily rolling files `aria.YYYY-MM-DD.log` (the date is in UTC); the last 7 files
   are kept and older ones are deleted. Logs at debug level contain the full paths of music files.
6. `panic.log`, in the same folder as the logs: one line per crash, with the thread, source
   location and message. It is appended to and never rotated or deleted by Aria.
7. `library.db`, your music library, in the data folder. It holds the full paths of the music
   folders you added and of every track found in them, the tags read from those files (title,
   artist, album and so on), durations, your saved play queue and the playback position, and,
   since 0.2.1, your favorites, playlists and playback history (below). It
   stays on your machine. Next to it, SQLite keeps `library.db-wal` and `library.db-shm` while
   Aria runs (and may leave them afterwards). If the database is damaged, Aria moves it aside
   as `library.db.bak` (with `library.db.bak-wal` and `library.db.bak-shm` when those existed),
   replacing an older `.bak`, and starts a new one. These files are never deleted by Aria
   except as that replacement; remove them yourself when you no longer need them.

   Since 0.2.1 the same database also holds three more kinds of personal data:
   - **Favorites**: the full path of each track you marked, and when.
   - **Playlists**: their names, when they were created, and the full paths of their tracks in order.
   - **Playback history**: the full path of each track you played, and when. A play is recorded after 30 seconds of playing, or when the track ends if it is shorter than 30 seconds or its length is not known. Aria keeps the newest 10,000 plays, and "Recently played" on the Home page is built from them.

   Favorites, playlists and history are kept by path, so they come back if you remove a music
   folder and add it again. History can be turned off ("Keep playback history") and emptied
   ("Clear playback history") in Settings, Playback. Turning it off stops new plays from being recorded
   and keeps the old ones until you clear them. Clearing deletes the rows of the history from the
   database, but SQLite does not overwrite the space they took: their old contents can stay in
   `library.db` and in `library.db-wal` on disk until SQLite reuses that space for new data.
   Deleting `library.db` (and its `-wal` and `-shm` files, and any `library.db.bak` files) while
   Aria is closed removes all of it, together with the rest of the library. Favorites and
   playlists are removed only by you (unmark a track, remove an entry, delete a playlist). All
   of this stays in `library.db` on your machine; nothing is sent anywhere.
8. The artwork cache, `artwork\` inside the cache folder: small thumbnails made from the
   cover art embedded in your audio files. "Clear artwork cache" in Settings empties it, and it
   is rebuilt by a rescan.
9. `aria.lock`, in the data folder (since 0.2.4): an empty file that the running copy of Aria
   holds locked, so that a second copy knows it is open. It holds no data and may be deleted
   while Aria is closed.

Aria only reads your audio files; it never writes to or deletes them. Removing a folder from
the library removes its tracks' rows from `library.db`, not the files. Favorites, playlist
entries and plays are stored by file path and are not removed with the folder: they stay in
`library.db`, hidden while the file is not in the library, and show again if you add the folder
back. To delete them, unmark the tracks, remove the entries or delete the playlists, and clear
the playback history.

## Where

| OS | settings.toml, its `.tmp` and backups | app.ron and library.db (and its `-wal`, `-shm`, `.bak` files) | Logs and panic.log | Cache (holds `artwork\`) |
|----|---------------------------------------|---------|--------------------|--------------------------|
| Windows | `%APPDATA%\Aria\config\` | `%APPDATA%\Aria\data\` | `%APPDATA%\Aria\data\logs\` | `%LOCALAPPDATA%\Aria\cache\` |
| Linux (expected, not yet verified on that OS) | `$XDG_CONFIG_HOME/aria/` | `$XDG_DATA_HOME/aria/` | `$XDG_DATA_HOME/aria/logs/` | `$XDG_CACHE_HOME/aria/` |
| macOS (expected, not yet verified on that OS) | `~/Library/Application Support/Aria/` | `~/Library/Application Support/Aria/` | `~/Library/Application Support/Aria/logs/` | `~/Library/Caches/Aria/` |

The Windows config, data, log and cache locations were observed on a real machine in Phase 1; the location of `library.db` and `artwork\` follows from the code (the data and cache folders) and has not been looked up on a default install. On Linux, when `XDG_CONFIG_HOME`,
`XDG_DATA_HOME` or `XDG_CACHE_HOME` is unset, `~/.config`, `~/.local/share` and `~/.cache`
are used instead. `app.ron` lives in eframe's storage directory for the app id "Aria", which
is the data directory of the `directories` crate. `library.db` is in the same folder.

Setting the `ARIA_CONFIG_DIR` environment variable to a non-empty path overrides the config
directory only; the library database, logs and cache still go to the folders above. An empty value is ignored.

## Soundboard and hotkeys (since 0.2.2)

- **Sounds.** For each sound Aria stores the full path of its audio file, its name, its volume and its hotkey, in the `sounds` table of `library.db` (the hotkey of "Stop all", the chosen second output device, where sounds go, and the global-hotkeys switch are in `settings.toml`). The files are never copied, changed or deleted by Aria. Removing a sound removes its row, not the file; as with other deleted rows, SQLite may leave the old contents on disk in `library.db` and `library.db-wal` until the space is reused. Deleting `library.db` (and its `-wal`, `-shm` and `.bak` files) while Aria is closed removes all sounds with the rest of the library.
- **Hotkeys.** Global hotkeys are registered with the operating system. Aria receives only the combinations you registered, when you press them. It installs no keyboard hook and does not see any other key you press, in Aria or in other programs. While a combination is registered, the system gives it to Aria alone: other programs no longer receive it until Aria lets it go (when you change or clear the hotkey, turn "Global hotkeys" off, or quit).
- **Second device.** The second output device is an output; for a voice chat it must be a virtual audio cable installed separately.

## One copy at a time (since 0.2.4)

- **The lock.** The running copy holds a lock on `aria.lock` in the data folder. The operating system lets go of it when Aria ends, however it ends.
- **The pipe.** While Aria runs, it listens on a local named pipe (on Windows, `\\.\pipe\aria-<number>.sock`, where the number is worked out from the data folder's path; on Linux and macOS, a local socket). A second copy of Aria sends it the word `show`, and, since 0.2.7, the full paths of the files and folders it was started with (as with a double-click or "Open with"), one per line. The open copy brings its window forward and opens them: files play, and folders are added to the library. Nothing else is accepted, nothing is sent back, and the paths are not stored, except that a folder becomes part of the library as Add folder makes it. The pipe is local to your computer: it is not a network connection. Other programs running as you on this computer could send the same, and could do no more than bring Aria's window forward, play files, and add folders to the library.

## Files opened from outside the library (since 0.2.7)

- **What is read.** A file opened with Aria that is not in the library is read for its tags and played. Its path, tags and length are held in memory until Aria closes. They are not written to `library.db`, to the queue, to the history or to `settings.toml`; the path may appear in logs at the debug level, as library paths do.
- **"Open with".** The installer writes, for your user only, the registry entries that put Aria in "Open with" for audio files (under `HKEY_CURRENT_USER\Software\Classes`: the `Aria.AudioFile` type, `Applications\aria.exe`, and an `OpenWithProgids` value for each file type). Uninstalling removes them. They change no default program.

## Watching the library's folders (since 0.2.7)

While "Watch folders for changes" is on (Settings, Library; on by default), Aria asks the operating system to tell it of changes in the library's folders. It learns only the paths of the files that change, and acts on them only by scanning the library again, as Rescan does. Nothing about them is stored beyond what a scan stores.

## Portable mode, backups and playlist files (since 0.2.9)

- **Portable mode.** With a folder named `AriaData` beside `aria.exe`, everything listed above is stored in it instead of in your user folders: `AriaData\config` (the settings), `AriaData\data` (the library, the logs, the lock) and `AriaData\cache` (the artwork). The window state may still be kept where eframe keeps it.
- **Backups.** A backup is written only when you ask for one, into the folder you choose: `library.db` (everything the library holds, listed above) and `settings.toml`. It stays where you put it, and Aria never reads it unless you restore it. A restore copies it into `restore` in the data folder until the next start, and keeps the files it replaces as `library.db.before-restore` and `settings.toml.before-restore`, which Aria never deletes.
- **Playlist files.** "Export…" writes the playlist's file paths, and each track's length, artist and title, to the `.m3u8` file you choose. "Import…" reads the file you choose; nothing else is read from it.
- **Check for updates.** The button opens the releases page in your web browser. Aria itself makes no request for it.

## Spotify (since 0.3.0)

Aria talks to Spotify only after you connect it in Settings, Spotify. Before that, and after you
disconnect, it makes no request to Spotify or anyone else.

- **Signing in.** "Connect Spotify" opens Spotify's own sign-in page in your browser. Aria never
  sees or stores your Spotify password. The sign-in uses OAuth with PKCE: Aria has no client
  secret, and Spotify hands the result back to Aria through `http://127.0.0.1:43819/callback`, a
  one-time listener on your own computer that stops as soon as it has the answer, or after five
  minutes. While the app is in Spotify's development mode, only the accounts its owner added (up
  to five) can connect.
- **What Aria may do.** It asks Spotify for these permissions: your display name
  (`user-read-private`), your saved tracks and albums (`user-library-read`), the artists you
  follow (`user-follow-read`), your playlists (`playlist-read-private`,
  `playlist-read-collaborative`), and to see and control playback on your Spotify devices
  (`user-read-playback-state`, `user-modify-playback-state`, `user-read-currently-playing`). Since
  0.3.3 also `streaming`, for Spotify's own player in Aria's window, and `user-read-email`, which
  that player requires; Aria never reads, keeps or sends your email address. Since 0.3.4 also
  `user-library-modify` and `playlist-modify-public`, `playlist-modify-private`: Aria changes
  your Liked Songs and playlists only when you click a heart, "Add to playlist", "New
  playlist…" or "Remove from this playlist", and nothing else in your Spotify account.
- **What is sent, and to whom.** Requests go to Spotify only: `accounts.spotify.com` (sign-in and
  token refresh) and `api.spotify.com` (your library, search, playback), with your access token.
  Covers are fetched from the image addresses Spotify's answers give (Spotify's image servers,
  such as `i.scdn.co`), over HTTPS and without your token. The full-screen record and the
  mini-player fetch the largest image Spotify offers (about 640 pixels) for the item they show,
  and only while one of them is open. What you type in Search is sent to
  Spotify as you type. Opening one of your playlists asks Spotify for its tracks; liking a track
  or changing a playlist sends that track's Spotify address and the playlist's id. While Aria is connected it asks Spotify what is playing every 2 seconds
  while the player bar shows Spotify and every 15 seconds otherwise, and not at all while the
  window is minimized.
- **What is kept.** Only the refresh token, in your system's protected store: Windows
  Credential Manager, the macOS Keychain, or the Secret Service on Linux, under `Aria` /
  `spotify`. The access token, your Spotify library, an opened playlist, search results and covers stay in memory
  and are gone when Aria closes; nothing of them is written to `library.db` or anywhere else.
  The chosen Spotify device's id is kept in `settings.toml` (`spotify_device`). A system without
  a protected store keeps nothing, and you connect again each time. Tokens are never written to
  the logs; at debug level the logs name Spotify's errors, not your data.
- **Aria's player window (since 0.3.3).** With "Play in Aria" on (Settings, Spotify), playing
  with no Spotify device open starts Edge or Chrome with a browser profile of Aria's own, kept
  in Aria's cache folder under `spotify-player` (cookies and cache of that window only; your own
  browser profile is not touched). The window shows a page that Aria serves on `127.0.0.1`
  only, under an address with a random key that changes at each launch; the page takes a second
  random secret once, as the first thing it does, keeps it in memory, and only with it gets the
  access token from Aria, so once the page has loaded, a program that reads the window's address
  can get nothing with it. The page writes nothing to the
  browser's storage and logs nothing. It loads Spotify's player from `sdk.scdn.co`, and that
  player talks to Spotify as Spotify's own code decides (it is Spotify's, not Aria's). The
  access token is handed only to that page; Aria keeps no copy on disk. Turning "Play in Aria"
  off, disconnecting or quitting closes the window; deleting the `spotify-player` folder removes
  the profile.
- **Removing it.** Settings, Spotify, "Disconnect" deletes the stored token and forgets your
  Spotify library. You can also delete the `Aria` / `spotify` entry from the system store, and
  remove Aria's access on your Spotify account page (Apps).
- **Audio.** Aria plays no Spotify audio and records nothing: Spotify plays on your own devices,
  and Aria only asks it to.

## Updates (since 0.3.5)

- **Asked first.** The first time 0.3.5 or later starts, a card asks "Keep Aria up to date?". Until
  you answer "Turn on", Aria checks nothing by itself. "Not now" and Settings, Updates, "Check for
  updates automatically" turn it off; "Check for updates" checks once, when you click it.
- **What is sent.** With checks on, at most once a day (and 10 seconds after start, if a day has
  passed), Aria opens `https://github.com/Hixenov/Aria/releases/latest` without following it:
  GitHub answers with the address of the latest full release's page, and its tag is the version.
  (Until 0.4.0 Aria asked GitHub's API, `api.github.com`, which allows only 60 questions an hour
  per IP address; 0.4.1 no longer uses it.) When GitHub cannot be reached, is busy (it refused for
  now), gives an answer Aria cannot use, or the release is still being published, the next
  automatic check is an hour later instead. A click on "Check
  for updates" is that one check and is not repeated. On the installed Windows Aria, when that release is newer, it then downloads that
  release's `SHA256SUMS.txt` and `Aria-<version>-setup.exe` from
  `https://github.com/Hixenov/Aria/releases/download/` (GitHub sends the files from its own file
  servers). Each request carries the User-Agent `Aria`, with no version, and nothing else about
  you or your music. GitHub sees your IP address, as any website does. A portable Aria, the loose
  `aria.exe`, macOS and Linux only ask about the latest release and show its page; they download
  nothing.
- **What is stored.** In `settings.toml`: your choice (`update_checks`) and when the last check
  finished (`update_checked_at`, seconds since 1970). In the cache folder's `updates` folder: the
  downloaded installer and `Aria-<version>-setup.exe.sha256`, the checksum it matched. A download
  in progress is a `.part` file. Aria deletes installers that are not newer than itself, and any
  `.part` file, when it starts.
- **Installing.** Nothing installs until you click "Restart to update". Aria then checks the
  installer against its checksum again, closes as on any quit, and starts it with `/SILENT
  /SUPPRESSMSGBOXES /NORESTART /RELAUNCH=1`; the installer shows its progress and starts the new
  Aria.
- **What the checksum proves.** `SHA256SUMS.txt` is in the same GitHub release as the installer.
  It catches a broken or incomplete download; it does not protect against someone who takes over
  the GitHub account, who could change both. The installer is not code-signed.

## System media controls (since 0.4.0)

- **What is handed over.** While "Show in the system's media controls" is on (Settings, General;
  on by default), Aria tells the operating system's media session what plays: the title, artist,
  album, length and position, whether it plays, and a link to the cover. For a local track that
  link is the cover's thumbnail file in Aria's cache; for a Spotify track it is Spotify's own
  image address (`https://i.scdn.co/…`), the same cover Aria already shows, which is then loaded
  from Spotify's image server once more: by the system on Windows and Linux, by Aria's own
  process on macOS (souvlaki loads it there). This is Windows' System Media Transport Controls,
  macOS's Now Playing, or MPRIS over the local D-Bus on Linux. Nothing else goes over the network
  for it.
- **Who sees it.** Whatever the system shows it in: the volume pop-up and the lock screen on
  Windows, Now Playing on macOS, the desktop's media controls on Linux, and other programs that
  read the system's media session (on Linux, any program on the session's D-Bus).
- **What Aria receives.** Only what the system's media session hands it: play, pause, next,
  previous, seek, and on Linux volume and "raise". Aria reads no other keys.
- **What is kept.** Nothing new beyond the setting `media_controls` in `settings.toml`. Turning it
  off makes Aria leave the system's media controls (on macOS, Now Playing may keep showing the
  last track until something else plays).
- **While minimized with Spotify playing.** So that the system's media controls follow Spotify,
  Aria asks Spotify for what plays every 15 seconds, and when the track should have ended, while
  its window is minimized (otherwise it stops asking while minimized).

## Start with the system (since 0.4.3)

- **What is written.** Only while "Start with the system" is on (Settings, General; off by
  default), Aria writes one entry for your user account that starts it at login with
  `--autostart`:
  - Windows: the value `Aria` under `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`.
  - macOS: `~/Library/LaunchAgents/io.github.hixenov.aria.plist`.
  - Linux: `~/.config/autostart/io.github.hixenov.aria.desktop`.
  It holds the path to Aria's program and nothing else.
- **When it changes.** At every start, Aria checks the entry: it rewrites it if Aria moved, or
  removes it if the switch is off. It never touches an entry that isn't its own. Uninstalling on
  Windows removes it too.
- **Nothing is sent anywhere** for it.

## The tray (since 0.4.2)

- **What it shows.** The tray icon's tooltip shows the title and artist of what plays, and its
  menu shows whether it plays. That is handed to the operating system's tray on this computer
  only: Windows' notification area, macOS's menu bar, or, on Linux, the desktop's tray through
  the session's D-Bus (the StatusNotifierItem protocol). Nothing goes over the network for it.
- **What is kept.** Nothing new beyond the setting `close_to_tray` in `settings.toml`
  (`minimize_to_tray` was there already).
- **While in the tray.** Aria counts as not shown, as when minimized, so it stops asking Spotify
  for what plays, apart from the occasional question described under "System media controls".

## The microphone (since 0.2.3)

Before 0.2.3 Aria never opened a microphone. Since 0.2.3 it can, and only like this:

- **When.** The microphone is open only while "Send my microphone to the second device" is on in Settings, Soundboard, and a second device is chosen (and is not the device the music plays on). It is off by default. Turning the switch off, or its hotkey, closes it at once. While it is open the player bar shows a microphone icon, and Windows normally shows its own microphone-in-use mark (not yet checked with Aria).
- **Where its sound goes.** Only into the mix of the second device, usually a virtual cable that a chat program listens to. It never goes to your own speakers or headphones. It is held only in memory, for at most about one second while Aria moves it along (less in normal running), and at most 40 ms of it waits in the mix; older audio is dropped.
- **What is kept.** Nothing. Aria never writes what it captures to disk, never logs it or its level, and never sends it over a network. The choice of microphone, its volume, its switch and its hotkey are stored in `settings.toml`; the microphone's name may appear in logs.
- **The music copy.** "Music also to the second device" sends a copy of the music you play to that device. It is sound, not data about you; nothing about it is stored beyond the switch and its volume in `settings.toml`.
- **A cable as the microphone.** Aria never opens a virtual cable's recording side (for example "CABLE Output") as the microphone, chosen or as the system default: it would send the chat's sound back into itself.
- **The cable guide.** When no virtual cable is found, Settings shows a link to the official page of a free cable. Aria makes no request to it; clicking it opens your web browser. Aria installs and changes no driver.
- Nothing about sounds or hotkeys is sent anywhere.
- Since 0.2.2 `library.db` also holds each track's bit depth, read from your audio files.

This document is extended in each phase that changes what Aria stores or sends.
