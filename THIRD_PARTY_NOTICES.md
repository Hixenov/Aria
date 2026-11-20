# Third-party notices

Aria bundles the following fonts. Full license texts are in `assets/fonts/`.

| Font | License | Text |
|------|---------|------|
| Inter | SIL Open Font License 1.1 | [assets/fonts/LICENSE-Inter.txt](assets/fonts/LICENSE-Inter.txt) |
| Noto Sans Armenian | SIL Open Font License 1.1 | [assets/fonts/LICENSE-NotoSansArmenian.txt](assets/fonts/LICENSE-NotoSansArmenian.txt) |
| Phosphor (icons) | MIT | [assets/fonts/LICENSE-Phosphor.txt](assets/fonts/LICENSE-Phosphor.txt) |

egui's default fonts are also compiled into the binary through the `epaint_default_fonts`
crate (SPDX: `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`).

## Audio and library components (Phase 2a)

| Component | License | Used for |
|-----------|---------|----------|
| libopus, bundled through `opusic-sys` and `symphonia-adapter-libopus` | BSD-3-Clause (opus); the adapter crate is MIT OR Apache-2.0 | Opus decoding |
| SQLite, bundled through `rusqlite` and `libsqlite3-sys` | Public domain (SQLite); the Rust bindings are MIT | Library database |
| `symphonia` 0.6 | MPL-2.0 | Decoding MP3, FLAC, WAV, AAC, ALAC, Vorbis |
| `cpal` 0.18 | Apache-2.0 | Audio output |
| `rubato` 5 | MIT OR Apache-2.0 | Resampling |
| `rtrb` 0.4 | MIT OR Apache-2.0 | Lock-free ring buffer |
| `lofty` 0.25 | MIT OR Apache-2.0 | Reading tags and embedded artwork |
| `image` 0.25 | MIT OR Apache-2.0 | Artwork thumbnails (JPEG, PNG) |
| `sha2` 0.10 | MIT OR Apache-2.0 | Artwork cache file names |
| `pollster` 0.4 | Apache-2.0 / MIT | Minimal futures executor |

The libopus license text is in `opusic-sys`'s `opus/COPYING`; the licenses above are those declared
in each crate's `Cargo.toml`. This table lists the crates added in Phase 2a; the full set is in
`Cargo.lock` and checked in CI.

## Exclusive output and global hotkeys (Phase 2c)

| Component | License | Used for |
|-----------|---------|----------|
| `wasapi` 0.27 | MIT | WASAPI exclusive output (Windows only) |
| `global-hotkey` 0.10 | Apache-2.0 OR MIT | Global hotkeys for the soundboard |
| `keyboard-types` 0.8, a dependency of `global-hotkey` | MIT OR Apache-2.0 | Key and modifier types |

The licenses above are those declared in each crate's `Cargo.toml`. These are the crates added in Phase 2c; the full set is in `Cargo.lock` and checked in CI.

Rust crate dependencies are under their own licenses, checked in CI with `cargo-deny`
(see `deny.toml`).

## Watching folders (0.2.7)

| Component | License | Used for |
|-----------|---------|----------|
| `notify` 8.2 | CC0-1.0 | Watching the library's folders for changes |

## Spotify (Phase 3)

| Component | License | Used for |
|-----------|---------|----------|
| `ureq` 3 | MIT OR Apache-2.0 | HTTPS requests to Spotify |
| `rustls` 0.23, with `ring` 0.17 | Apache-2.0 OR ISC OR MIT (rustls); Apache-2.0 AND ISC (ring) | TLS |
| `rustls-platform-verifier` 0.6 | MIT OR Apache-2.0 | Checking certificates against the system's store |
| `webpki-roots` 1 | CDLA-Permissive-2.0 | Mozilla's root certificates, which `ureq` carries |
| `keyring` 3 | MIT OR Apache-2.0 | Keeping the Spotify refresh token in the system's protected store |
| `serde_json` 1 | MIT OR Apache-2.0 | Reading Spotify's answers |
| `getrandom` 0.3, `base64` 0.22, `form_urlencoded` 1 | MIT OR Apache-2.0 | The sign-in's PKCE values and addresses |

This table lists the crates added in Phase 3; their own dependencies are in `Cargo.lock` and
checked in CI.
