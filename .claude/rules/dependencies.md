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
- Crates whose defaults reach beyond Linux or beyond what caditor uses list their features:
  `wgpu` builds `vulkan`, `gles`, `wgsl`, `std` and `parking_lot` only (its own dependency on
  `wgpu-core` still enables RenderDoc), `egui-wgpu` none, `egui-winit` `clipboard`, `wayland` and
  `x11` plus `accesskit` in the app (no `links`, so no `webbrowser`), and `env_logger`
  `auto-color` and `humantime` (no `regex` filters). A new backend or feature is added to the
  workspace entry only when code uses it.
