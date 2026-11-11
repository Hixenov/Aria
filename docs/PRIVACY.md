# Privacy

Aria sends no data over the network. It makes no network requests at runtime, has no
telemetry and no accounts. Nothing from your music library, tags or folders is sent anywhere.

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
   - **Playback history**: the full path of each track you played for at least 30 seconds, and when. Aria keeps the newest 10,000 plays, and "Recently played" on the Home page is built from them.

   Favorites, playlists and history are kept by path, so they come back if you remove a music
   folder and add it again. History can be turned off ("Keep playback history") and emptied
   ("Clear playback history") in Settings, Playback. Turning it off stops new plays from being recorded
   and keeps the old ones until you clear them. Favorites and playlists are removed only by you
   (unmark a track, remove an entry, delete a playlist). All of this stays in `library.db` on
   your machine; nothing is sent anywhere.
8. The artwork cache, `artwork\` inside the cache folder: small thumbnails made from the
   cover art embedded in your audio files. "Clear artwork cache" in Settings empties it, and it
   is rebuilt by a rescan.

Aria only reads your audio files; it never writes to or deletes them. Removing a folder from
the library removes its rows from `library.db`, not the files.

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

This document is extended in each phase that changes what Aria stores or sends.
