# Privacy

Phase 1 of Aria sends no data over the network. It makes no network requests at runtime, has
no telemetry and no accounts.

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
   are kept and older ones are deleted.
6. `panic.log`, in the same folder as the logs: one line per crash, with the thread, source
   location and message. It is appended to and never rotated or deleted by Aria.
7. The cache folder. Aria creates it at startup, but Phase 1 stores nothing in it.

## Where

| OS | settings.toml, its `.tmp` and backups | app.ron | Logs and panic.log | Cache (empty in Phase 1) |
|----|---------------------------------------|---------|--------------------|--------------------------|
| Windows | `%APPDATA%\Aria\config\` | `%APPDATA%\Aria\data\app.ron` | `%APPDATA%\Aria\data\logs\` | `%LOCALAPPDATA%\Aria\cache\` |
| Linux (expected, not yet verified on that OS) | `$XDG_CONFIG_HOME/aria/` | `$XDG_DATA_HOME/aria/app.ron` | `$XDG_DATA_HOME/aria/logs/` | `$XDG_CACHE_HOME/aria/` |
| macOS (expected, not yet verified on that OS) | `~/Library/Application Support/Aria/` | `~/Library/Application Support/Aria/app.ron` | `~/Library/Application Support/Aria/logs/` | `~/Library/Caches/Aria/` |

The Windows paths were observed on a real machine. On Linux, when `XDG_CONFIG_HOME`,
`XDG_DATA_HOME` or `XDG_CACHE_HOME` is unset, `~/.config`, `~/.local/share` and `~/.cache`
are used instead. `app.ron` lives in eframe's storage directory for the app id "Aria", which
is the data directory of the `directories` crate.

Setting the `ARIA_CONFIG_DIR` environment variable to a non-empty path overrides the config
directory only. An empty value is ignored.

This document is extended in each phase that changes what Aria stores or sends.
