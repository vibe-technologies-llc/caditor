---
paths:
  - "**/*.rs"
  - "**/Cargo.toml"
---

# Rust style

## Formatting

- Format only with `rust-formatter` (`~/.cargo/bin/rust-formatter`), never `cargo fmt` or
  `rustfmt`. It runs nightly rustfmt with settings the plain tools lack, so their output differs.
- There is no `rustfmt.toml`, and none should be added. The formatter is zero-config.
- `rust-formatter --check` is read-only and exits 1 with a diff. `--since <REF>` and `--staged`
  limit a run to changed files.

## Imports

- Group imports as `std`, then external crates, then internal ones (`crate::`, `super::` and
  workspace crates), with a blank line between the groups.
- Use one `use` statement per crate, merging its paths into a single tree:
  `use std::{collections::HashMap, sync::Arc};`.
- `rust-formatter` applies both rules (`StdExternalCrate` grouping, `Crate` granularity), so
  write imports in this shape and let the formatter settle the order.

## Unsafe

- Every crate forbids unsafe code. Once a workspace exists, set this through
  `[workspace.lints.rust] unsafe_code = "forbid"` and `[lints] workspace = true` in each member.
- Prefer a safe API even when it costs a little, for example `wgpu::Instance::create_surface`
  with an owned or `Arc` window handle rather than `create_surface_unsafe`.
- If `unsafe` truly cannot be avoided, confine it to the smallest possible crate or module,
  relax that crate alone to `deny`, and allow it only at the exact item. Never relax it for the
  whole workspace.

## Language

- Edition 2024. There is no MSRV: do not set `rust-version`, and feel free to use any feature
  that is stable on the current stable toolchain.
