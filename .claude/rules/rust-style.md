---
paths:
  - "**/*.rs"
  - "**/Cargo.toml"
---

# Rust style

- Format only with `rust-formatter`, never `cargo fmt` or `rustfmt`; it is zero-config, so never
  add a `rustfmt.toml`. `--check` is read-only; `--since <REF>` and `--staged` limit a run to
  changed files. CI builds it from `RUST_FORMATTER_REV` on `FORMAT_TOOLCHAIN` (both in
  `.github/versions.env`).
- Imports: `std`, external crates, then internal (`crate::`, `super::`, workspace crates), blank
  lines between, one `use` tree per crate. The formatter enforces both.
- No comments in `.rs` or `.toml` (`//`, `///`, `//!`, `/* */`, `#`). Say it in a name, a type or an
  extracted function; design rationale goes in `CLAUDE.md`, a rules file or `docs/`. Delete comments
  in code you touch. `conventions_tests.rs` fails on any comment.
- Pick the collection by access pattern; `BTreeMap`/`BTreeSet` where deterministic order matters,
  as with ID-keyed model data.
- Hash maps and sets are `ahash::AHashMap`/`AHashSet`; locks are `parking_lot`; `clippy.toml`
  enforces both. Anything keyed by what a file or other outside input chose uses `std`'s
  DoS-resistant hasher through `caditor-file`'s `UntrustedMap` and `UntrustedSet`, the only place
  the ban is lifted.
- `unsafe_code = "forbid"` workspace-wide and every member sets `[lints] workspace = true`; prefer
  a safe API even at a small cost. Unavoidable `unsafe` lives in the smallest crate possible,
  relaxed to `deny` there and allowed only at the exact item (`caditor-zstd`, which lists the
  workspace clippy lints itself). `conventions_tests.rs` checks both.
- Edition 2024, no MSRV: never set `rust-version`; any feature stable on the current toolchain is
  fair game.
