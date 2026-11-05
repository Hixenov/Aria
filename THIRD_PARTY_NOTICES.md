# Third-party notices

Aria bundles the following fonts. Full license texts are in `assets/fonts/`.

| Font | License | Text |
|------|---------|------|
| Inter | SIL Open Font License 1.1 | [assets/fonts/LICENSE-Inter.txt](assets/fonts/LICENSE-Inter.txt) |
| Noto Sans Armenian | SIL Open Font License 1.1 | [assets/fonts/LICENSE-NotoSansArmenian.txt](assets/fonts/LICENSE-NotoSansArmenian.txt) |
| Phosphor (icons) | MIT | [assets/fonts/LICENSE-Phosphor.txt](assets/fonts/LICENSE-Phosphor.txt) |

egui's default fonts are also compiled into the binary through the `epaint_default_fonts`
crate (SPDX: `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`).

Rust crate dependencies are under their own licenses, checked in CI with `cargo-deny`
(see `deny.toml`).
