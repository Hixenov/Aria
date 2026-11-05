# Roadmap

Aria is built in six phases. Each phase gets its own spec, plan and implementation cycle.

## The Spotify playback limitation

Spotify audio is not streamed or decoded inside Aria. Spotify integration (Phase 3) means
library, search and playlist access through the Web API, plus remote control of the user's
existing Spotify Connect devices. No compliant in-process streaming path exists for a native
desktop Rust app: librespot-style streaming uses private protocols and conflicts with
Spotify's developer terms. Local files get the full audio engine (Phase 2).

## Phases

1. **Foundation (current).** Window and layout shell, navigation, theme system, persistent
   settings, localization (EN/RU/HY), logging, CI and repository documentation.
2. **Local audio.** Local library and audio engine (Symphonia decoding, CPAL output), SQLite
   library storage, the working player bar.
3. **Spotify.** Library, search and playlist access through the Web API (OAuth PKCE) and
   Spotify Connect remote control. No in-app audio streaming.
4. **System integration.** System tray (including the minimize-to-tray preference stored in
   Phase 1), media keys and notifications.
5. **Lyrics, equalizer and visualizer.** LRCLIB lyrics, plus an equalizer and visualizer for
   local audio.
6. **Mini-player and installers.** A compact mini-player window and platform installers.
