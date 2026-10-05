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
- A dependency only one platform uses goes in the member's `[target.'cfg(unix)'.dependencies]` or
  `[target.'cfg(windows)'.dependencies]` (`rustix`, `xattr`, `signal-hook`, `zbus` on Unix;
  `caditor-windows`, `rfd` on Windows). A build dependency stays unconditional, since `cfg` there
  would test the host: `build.rs` checks `CARGO_CFG_TARGET_OS` instead.
