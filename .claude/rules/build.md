---
paths:
  - "Cargo.toml"
  - "crates/*/Cargo.toml"
  - ".config/**"
---

# Builds and test runs

A full test run must leave the machine usable: it once froze a 16-core, 30 GB desktop. Each choice
below is measured; keep the measurements in mind before undoing one.

## Profiles (root `Cargo.toml`)

- `dev` keeps `debug = "line-tables-only"` for workspace crates and `debug = false` for every
  dependency (`[profile.dev.package."*"]`): backtraces still name our lines, while objects stay
  smaller and links faster and lighter.
- The workspace crates whose tests compute (geometry, expression, kernel, sketch, document, file,
  step, lettering) build at `opt-level = 3` in `dev`, and the numeric dependencies (`nalgebra`,
  `spade`, font, PNG and deflate crates, egui's painting) too. Against `opt-level = 1` this cut the
  test run's CPU time by a third (1,630 s to 1,100 s) and a pipe along a helix measured through the
  MCP server from 70 s to 14 s, for about 26 s more on a clean rebuild of the workspace (110 s to
  136 s) and about a second on an incremental one, with no more memory. Debug assertions and
  overflow checks stay on. The app crate (`caditor`) stays at 0: its 285,000 lines would cost far
  more to optimise than its interface tests spend running.

## What a test build compiles

- The app crate is compiled once per test build, in test mode. Nothing else needs it outside test
  mode: its bin target has `test = false` (`main.rs` has no tests), every lib sets
  `doctest = false` (no item can carry a doc comment, `rust-style.md`, so rustdoc would only parse
  285,000 lines for nothing), and the crash test that runs a child process lives in `caditor-file`
  (`tests/crash_flush.rs`, `file-journal.md`) beside the code it tests. An integration test under
  `crates/caditor/tests/` would bring back a second full compile of the app crate; put such a test
  in the lowest crate holding what it tests.

## Running tests (`.config/nextest.toml`)

- Tests run under cargo-nextest: one process per test, so a test's threads, statics and GPU device
  die with it, a hung test is reported and ended on its own, and the binaries' tests run side by
  side rather than one binary after another. Before the changes above, `cargo test --workspace`
  took 223 s and nextest 147 s; with them both take about 93 s, with no test past 25 s. `cargo test`
  still works and must keep passing; packagers (`packaging/arch/PKGBUILD`) and the named ignored
  runs (`.github/run-ignored.sh`, the seed and corpus writers, screenshots) use it.
- The `gpu` test group lets four tests at a time hold a device: the offscreen render tests, the
  interface screenshots and the import timing, so a run does not fight the desktop's compositor for
  the GPU.
- `slow-timeout` reports a test past a minute. A test that long holds back the whole run, since
  everything else finishes around it: split it into tests of a few cases each (the turned tori in
  `boolean/tests.rs` share `a_torus_and_its_turned_copies_combine`), shrink its input while it
  pins the same property, or make it an ignored survey run in release by the `stress` job. Never
  weaken an assertion to make it faster.
- The `ci` profile never stops at the first failure and ends a test still running after twenty
  minutes (`ci.md`).
- Helper threads inside a test process come from the kernel's one budget (`kernel.md`), so tests
  that recompute and tessellate share the cores rather than each starting one thread per core.
