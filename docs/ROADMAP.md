# Roadmap

Aria is built in six phases. Each phase gets its own spec, plan and implementation cycle.

## The Spotify playback limitation

Spotify audio is not streamed or decoded inside Aria. Spotify integration (Phase 3) means
library, search and playlist access through the Web API, plus remote control of the user's
existing Spotify Connect devices. No compliant in-process streaming path exists for a native
desktop Rust app: librespot-style streaming uses private protocols and conflicts with
Spotify's developer terms. Local files get the full audio engine (Phase 2). Aria itself still
plays no Spotify audio; since 0.3.3 Spotify's own Web Playback SDK plays it in a browser window
Aria opens, the one route Spotify allows for other apps.

## Phases

1. **Foundation (current).** Window and layout shell, navigation, theme system, persistent
   settings, localization (EN/RU/HY), logging, CI and repository documentation.
2. **Local audio.** Local library and audio engine (Symphonia decoding, CPAL output), SQLite
   library storage, the working player bar. Split into parts: 2a, the local player (0.2.0,
   released as a pre-release); 2b, favorites, playlists, playback history and Home (0.2.1,
   released as a pre-release, not yet checked by hand); 2c, lossless output and the soundboard (0.2.2, released, not yet checked by hand); then the
   music and the microphone to a voice chat through a virtual cable (0.2.3, released,
   not yet checked by hand); then one Aria at a time and fixes to the chat features (0.2.4,
   released, mostly not checked by hand); then Aria's own icon (0.2.5, released); then reliability fixes (0.2.6, released, not yet checked by hand); then opening files with Aria and watching the library's folders (0.2.7, released, not yet checked by hand); then volume levelling and a sleep timer (0.2.8, released, with macOS and Linux files); then portable mode, backups and M3U playlists (0.2.9, released).
3. **Spotify.** Library, search and playlist access through the Web API (OAuth PKCE) and
   Spotify Connect remote control. No in-app audio streaming. 0.3.0 brings the core: sign-in,
   your saved tracks, albums, followed artists and playlists, search, and playing on the device
   you choose from the player bar. Liking tracks and adding to or removing from your own Spotify
   playlists came in 0.3.4. An update checker that downloads and installs new versions, once
   the user agrees, came in 0.3.5. Spotify's limits for apps in development mode apply: five allow-listed accounts, search
   results capped at 10 per kind, no artist top tracks (an artist plays as a whole).
4. **Desktop experience.** System tray (including the minimize-to-tray preference stored in
   Phase 1), media keys, shortcuts, notifications, startup integration and the mini-player.
   The mini-player came in 0.3.2; media keys and the system's media controls in 0.4.0. Next:
   the tray, starting with the system, and track-change notifications.
5. **Advanced functionality.** LRCLIB lyrics, an equalizer and audio visualizer for local
   playback, crossfading between tracks, additional themes, advanced queue management, and optional Winamp skin support.
6. **Production release.** Full testing, performance profiling, security and dependency
   audits, cross-platform builds and installers, documentation, and the 1.0 release.
