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
- Commit `Cargo.lock`, and run `rust-formatter` after editing a `Cargo.toml` (it formats TOML).
- Crates whose defaults reach beyond Linux and Windows or beyond what caditor uses list their
  features in the workspace entry (`wgpu`, `egui-wgpu`, `egui-winit`, `env_logger`, `zbus`,
  `windows-sys`, `rfd`); a new backend or feature is added only when code uses it. wgpu has
  `dx12` for Windows; jiff has `tzdb-bundle-platform`, which bundles the time zone database only
  where the system has none (Windows).
- `wayland-client` takes `system` (libwayland-client's backend) and `dlopen`: `caditor-wayland`
  works on winit's own Wayland connection, and only the system backend can adopt a foreign
  `wl_display` (the pure-Rust backend owns its socket). winit already loads that library at run
  time through the same backend, so nothing new is linked or loaded. `wayland-protocols` takes
  `client` and `unstable` only, for `xdg-foreign` (the window's export for portal dialogs).
- A dependency only one platform uses goes in the member's `[target.'cfg(unix)'.dependencies]` or
  `[target.'cfg(windows)'.dependencies]` (`rustix`, `xattr`, `signal-hook`, `zbus`,
  `caditor-wayland` on Unix; `caditor-windows`, `rfd` on Windows). A build dependency stays
  unconditional, since `cfg` there would test the host: `build.rs` checks `CARGO_CFG_TARGET_OS`
  instead.
- The crates egui spends the UI tests' time in are built optimised in the dev profile
  (`[profile.dev.package.*]` with `opt-level = 3`, as for `nalgebra`, `spade` and `robust`):
  `epaint` and its glyph stack (`skrifa`, `read-fonts`, `font-types`, `harfrust`, `vello_cpu`,
  `vello_common`, `fearless_simd`, the Unicode tables). An unoptimised debug build spent most of
  every UI test rasterising the font, so a test cost 0.7 s before it did anything; a frame is now
  a few milliseconds and the whole UI suite takes about 200 s on one thread instead of about 15
  minutes. A new crate showing up at the top of a profile of the UI tests joins the list.
