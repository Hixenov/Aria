# Privacy

Phase 1 of Aria sends no data over the network. It makes no network requests at runtime, has
no telemetry and no accounts.

## What Aria stores

Aria stores only three things on your machine:

1. `settings.toml`: your preferences (language, theme, accent colours, log level, and so on).
2. `app.ron`: eframe's persisted state: window position/size and UI layout state (egui memory).
3. Logs: daily rolling files `aria.log.YYYY-MM-DD`; the last 7 files are kept.

## Where

| OS | settings.toml | app.ron | Logs |
|----|---------------|---------|------|
| Windows | `%APPDATA%\Aria\config\settings.toml` | `%APPDATA%\Aria\data\app.ron` | `%APPDATA%\Aria\data\logs\` |
| Linux (expected, not yet verified on that OS) | `$XDG_CONFIG_HOME/aria/settings.toml` | `$XDG_DATA_HOME/aria/app.ron` | `$XDG_DATA_HOME/aria/logs/` |
| macOS (expected, not yet verified on that OS) | `~/Library/Application Support/Aria/settings.toml` | `~/Library/Application Support/Aria/app.ron` | `~/Library/Application Support/Aria/logs/` |

The Windows paths were observed on a real machine. On Linux, when `XDG_CONFIG_HOME` or
`XDG_DATA_HOME` is unset, `~/.config` and `~/.local/share` are used instead. `app.ron` lives in
eframe's storage directory for the app id "Aria", which is the data directory of the
`directories` crate.

Setting the `ARIA_CONFIG_DIR` environment variable overrides the config directory only.

This document is extended in each phase that changes what Aria stores or sends.
