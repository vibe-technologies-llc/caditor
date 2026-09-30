---
paths:
  - "**/*.rs"
  - "**/Cargo.toml"
---

# Rust style

## Formatting

- Format only with `rust-formatter`, never `cargo fmt` or `rustfmt`: it runs nightly rustfmt with
  settings the plain tools lack. It is zero-config; never add a `rustfmt.toml`.
- `--check` is read-only and exits 1 with a diff; `--since <REF>` and `--staged` limit a run to
  changed files. CI builds it from `RUST_FORMATTER_REV` on the nightly `FORMAT_TOOLCHAIN` in
  `.github/workflows/ci.yml`; raise both when the local tools move on.

## Imports

- Groups `std`, external crates, then internal (`crate::`, `super::`, workspace crates), blank
  lines between; one `use` per crate as a single tree: `use std::{fmt::Debug, sync::Arc};`. The
  formatter enforces both (`StdExternalCrate`, `Crate`).

## Comments

- No comments: `//`, `///`, `//!`, `/* */`, nor `#` in TOML. Put what one would say into a name, a
  type or an extracted function; design rationale goes in `CLAUDE.md`, a rules file or `docs/`.
- Remove existing comments in code you touch.

## Collections and locks

- Pick the collection by access pattern; `BTreeMap`/`BTreeSet` where ordered or deterministic
  iteration matters, as with ID-keyed model data.
- Hash maps and sets are `ahash::AHashMap`/`AHashSet`, never `std`'s; locks are
  `parking_lot::{Mutex, RwLock, Condvar}`. `clippy.toml`'s `disallowed-types` enforces both.

## Unsafe

- `unsafe_code = "forbid"` in `[workspace.lints.rust]`; every member, new ones included, sets
  `[lints] workspace = true`.
- Prefer a safe API even at a small cost (`wgpu::Instance::create_surface` with an owned or `Arc`
  handle, not `create_surface_unsafe`).
- Unavoidable `unsafe` lives in the smallest crate possible, relaxed to `deny` there alone and
  allowed only at the exact item. `caditor-zstd` is that crate for zstd; it lists the workspace
  clippy lints itself, since it cannot inherit them with `unsafe_code` changed.

## Language

- Edition 2024, no MSRV: never set `rust-version`; any feature stable on the current toolchain is
  fair game.
