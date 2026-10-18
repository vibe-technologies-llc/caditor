---
paths:
  - "**/Cargo.toml"
  - "Cargo.lock"
---

# Dependencies

- Full three-part versions (`x.y.z`) for normal, dev, build and workspace dependencies alike.
- Add or bump to the latest release, looked up with `cargo info <crate>` or
  `cargo search <crate> --limit 1`, never from memory.
- Declare each dependency once under `[workspace.dependencies]`; members use
  `<crate>.workspace = true` plus only the features they need. Internal crates are `path`-only
  entries with no version (`publish = false`).
- Commit `Cargo.lock`; run `rust-formatter` after editing a `Cargo.toml` (it formats TOML).
- Crates whose defaults reach beyond Linux and Windows or beyond what caditor uses list their
  features in the workspace entry (`wgpu`, `egui-wgpu`, `egui-winit`, `env_logger`, `zbus`,
  `windows-sys`, `rfd`); a new backend or feature is added only when code uses it. wgpu has `dx12`
  for Windows; jiff has `tzdb-bundle-platform`, bundling the time zone database only where the
  system has none (Windows).
- `wayland-client` takes `system` and `dlopen` (in `caditor-wayland`'s manifest): only the system
  backend can adopt winit's foreign `wl_display`, and winit already loads that library at run time,
  so nothing new is linked. `wayland-protocols` takes `client` and `unstable` only, for
  `xdg-foreign`.
- A dependency only one platform uses goes in the member's `[target.'cfg(unix)'.dependencies]` or
  `[target.'cfg(windows)'.dependencies]` (`rustix`, `xattr`, `signal-hook`, `zbus`,
  `caditor-wayland` on Unix; `caditor-windows`, `rfd` on Windows). A build dependency stays
  unconditional, since `cfg` there tests the host: `build.rs` checks `CARGO_CFG_TARGET_OS`.
- `skrifa` (`std` only, through its `raw` re-export of `read-fonts`) reads outlines, the `wght` axis and
  GPOS kerning for SVG text in `caditor-file`; it is the copy epaint builds, so no second font parser
  is compiled. Its version follows epaint's rather than the latest release (`cargo deny` refuses the
  second copy a newer one would add). `ttf-parser` is left only through winit's `sctk-adwaita`, which is why `deny.toml`
  ignores its unmaintained advisory; nothing of ours uses it.
- `zvariant`, `zbus_names` and `zvariant_derive` (merged into `zbus` 6) come in only through
  `egui-winit` → `accesskit_unix` 0.24 → `atspi` → `zbus` 5, so `deny.toml` ignores their
  unmaintained advisories until an `accesskit_unix` on `zbus` 6 is published.
- `egui` takes `default_fonts` (no longer a default since 0.37): its fallbacks follow Inter in every
  family and `caditor-notices` lists `epaint_default_fonts`. `egui-phosphor` supplies only the font
  bytes (`Variant::font_bytes`) and glyph constants, never its `FontDefinitions` helpers, so it may
  lag egui; while it does, `deny.toml` skips its older egui copy.
- `half` (no default features) packs the environment picture's texels as `f16`; it is the copy
  `naga` and `tiff` already build.
- Reference pictures are decoded by `image` (no default features; `bmp`, `gif`, `hdr` (the rendered
  view's environment pictures), `jpeg` through the
  pure-Rust `zune-jpeg`, `png`, `qoi`, `tga`, `tiff`, `webp` through `image-webp`) and `jxl-oxide`
  (no default features, so no rayon; its `lcms2` and `moxcms` stay off), both pure Rust and the
  copies `arboard` already builds. AVIF and HEIC have no mature pure-Rust decoder and no C one is
  taken in for them: `image`'s `avif` features stay off and such files are refused in words.
  `tiff` is a dev-dependency of `caditor-file` only, for writing a two-page test file; its `zstd`
  feature, which links C, stays off.
- Crates the UI tests spend their time in are built at `opt-level = 3` in the dev profile
  (`[profile.dev.package.*]`): `egui`, `emath`, `epaint` and its glyph stack (`skrifa`,
  `read-fonts`, `font-types`, `harfrust`, `vello_cpu`, `vello_common`, `fearless_simd`, the Unicode
  tables), the PNG encoder (`png`, `fdeflate`, `miniz_oxide`, `flate2`, `crc32fast`,
  `simd-adler32`) and the kernel's `nalgebra`, `spade`, `robust`. Unoptimised, font rasterising
  dominated every UI test. A crate topping a profile of the UI tests joins the list.
