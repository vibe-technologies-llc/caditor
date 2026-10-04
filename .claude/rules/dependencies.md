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
- Crates whose defaults reach beyond Linux or beyond what caditor uses list their features in the
  workspace entry (`wgpu`, `egui-wgpu`, `egui-winit`, `env_logger`, `zbus`); a new backend or
  feature is added only when code uses it.
