# Privacy

Phase 1 of Aria sends no data over the network. It makes no network requests at runtime, has
no telemetry and no accounts.

## What Aria stores

Aria stores only three things on your machine:

1. `settings.toml`: your preferences (language, theme, accent colours, log level, and so on).
2. eframe window state: window position and size.
3. Logs: daily rolling files `aria.log.YYYY-MM-DD`; the last 7 files are kept.

## Where

| OS | settings.toml | Logs |
|----|---------------|------|
| Windows | `%APPDATA%\Aria\config\settings.toml` | `%LOCALAPPDATA%\Aria\data\logs\` |
| Linux | `$XDG_CONFIG_HOME/aria/settings.toml` | `$XDG_DATA_HOME/aria/logs/` |
| macOS | `~/Library/Application Support/Aria/settings.toml` | alongside, in `~/Library/Application Support/Aria/` |

Setting the `ARIA_CONFIG_DIR` environment variable overrides the config directory only.
The window state is kept by eframe in its own per-OS data location.

This document is extended in each phase that changes what Aria stores or sends.
