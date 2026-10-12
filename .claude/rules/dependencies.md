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
- Crates the UI tests spend their time in are built at `opt-level = 3` in the dev profile
  (`[profile.dev.package.*]`): `egui`, `emath`, `epaint` and its glyph stack (`skrifa`,
  `read-fonts`, `font-types`, `harfrust`, `vello_cpu`, `vello_common`, `fearless_simd`, the Unicode
  tables), the PNG encoder (`png`, `fdeflate`, `miniz_oxide`, `flate2`, `crc32fast`,
  `simd-adler32`) and the kernel's `nalgebra`, `spade`, `robust`. Unoptimised, font rasterising
  dominated every UI test. A crate topping a profile of the UI tests joins the list.
