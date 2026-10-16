---
paths:
  - "**/*.rs"
  - "**/Cargo.toml"
---

# Rust style

- Format only with `rust-formatter`, never `cargo fmt` or `rustfmt`; it is zero-config, so no
  `rustfmt.toml`. `--check` is read-only; `--since <REF>` and `--staged` limit a run to changed
  files. CI builds it from `RUST_FORMATTER_REV` on `FORMAT_TOOLCHAIN` (`.github/versions.env`).
- Imports: `std`, external crates, then internal (`crate::`, `super::`, workspace crates), blank
  lines between, one `use` tree per crate.
- No comments in `.rs` or `.toml`; say it in a name, a type or an extracted function, and put design
  rationale in `CLAUDE.md`, a rules file or `docs/`. `conventions_tests.rs` fails on any comment.
- Pick the collection by access pattern; `BTreeMap`/`BTreeSet` where deterministic order matters,
  as with ID-keyed model data.
- Hash maps and sets are `ahash::AHashMap`/`AHashSet`; locks are `parking_lot`; `clippy.toml`
  enforces both. Anything keyed by what a file or other outside input chose uses `std`'s
  DoS-resistant hasher through `caditor-file`'s `UntrustedMap` and `UntrustedSet`, the only place
  the ban is lifted.
- `unsafe_code = "forbid"` workspace-wide and every member sets `[lints] workspace = true`; prefer a
  safe API even at a small cost. Unavoidable `unsafe` lives in the smallest crate possible
  (`caditor-zstd`, `caditor-windows`, `caditor-wayland`), relaxed to `deny` there with its own list
  of the workspace clippy lints, and expected (`#[expect(unsafe_code, reason = "…")]`) only at the
  exact item. `conventions_tests.rs` checks both.
- Platform code is whole `cfg(unix)` and `cfg(windows)` items behind one interface
  (`caditor-file`'s `os` module; `portal`, `crash` in `caditor`), with a `cfg` at a call site only
  for a step one platform alone takes; Windows details are in `windows.md`.
- Tests bound cost by work done, never by time: a count a test can read (interrupt polls through the
  kernel's `test_support::polls_during`, or a `tally` of work units in `caditor-kernel`,
  `caditor-file` and the solver's `solve/tally.rs`, free outside tests) with a bound that holds
  for a stated growth of the input. A wait for another thread is bounded by a deadline fixed before
  the loop. `conventions_tests.rs` (`no_assertion_reads_the_clock`) refuses `elapsed` inside an
  assert.
- Errors are enums of specific variants (`thiserror`), one per failure, carrying typed data and
  worded by their `Display`; a caller wraps a callee's error with `#[from]` or a field, never its
  text, and the text is made only where it is shown (a notice, a callout, a tool answer). A user's
  own names or typed text may stay a `String` field. `conventions_tests.rs`
  (`errors_are_typed_rather_than_text`) refuses `Result<_, String>`, `From<…> for String` and a
  variant whose only payload is a `reason`, `message`, `detail`, `words` or `refusal` string
  outside test code; in the app crate it also refuses `Result<_, &'static str>`, such a payload
  held as `&'static str` and an `#[error]` tuple variant holding one. A refusal that only says the
  model is not ready is a `queued::Pending` the error answers through `Reason::pending`
  (`app-input.md`).
- Edition 2024, no MSRV: never set `rust-version`; any feature stable on the current toolchain is
  fair game.
