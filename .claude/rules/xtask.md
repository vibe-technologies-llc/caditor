---
paths:
  - "crates/caditor-xtask/**"
  - ".cargo/**"
---

# The check runner (`cargo xtask`)

- `caditor-xtask` is a binary (`xtask`) behind the `.cargo/config.toml` alias
  `cargo xtask = run --quiet --package caditor-xtask --`. It depends on no workspace crate and on
  `thiserror` alone, so it builds in seconds and never waits on the app. Nothing ships it:
  packaging builds `-p caditor`.
- Tasks: `fmt [--check]` (`rust-formatter` over the workspace and `fuzz/`), `lint [--windows]`,
  `test [-- <nextest args>]`, `deny` (the root and fuzz workspaces, `deny.toml` for both) and
  `check [--windows]`, the gate before a push: `fmt --check`, `lint`, `deny`, `test`. Each step is
  the line CI runs (`ci.md`), so a green `check` means a green `check` job barring the GPU, Windows
  and packaging steps; `--windows` adds `cargo xwin clippy --target x86_64-pc-windows-msvc`, the
  only lint of `cfg(windows)` code off Windows. `--locked` passes `--locked` to cargo.
- Output: each step prints one line (`clippy ... ok (41.2s)`, and nextest's summary under a test
  step) and keeps its output in `target/xtask/NN-<step>.log`, emptied at each run. A failure prints
  its last 60 lines without cargo's and nextest's progress lines (`Compiling`, `PASS`, ...) and the
  log's path. `-v` streams instead. Steps stop at the first failure unless `--keep-going`, which
  also gives nextest `--no-fail-fast`.
- `--changed` (against `origin/master`, `--since <rev>` for another) narrows `lint` and `test`:
  the files differing from the merge base, staged or not, plus untracked ones, map to the member
  whose folder holds them, then `cargo tree --invert` (all targets, normal, build and dev edges)
  adds every package depending on one. A change to a root manifest, the lock, `clippy.toml`,
  `.config/` or `.cargo/` is every package. Narrowed nextest runs take `--no-tests=warn`, since a
  selected crate may have no tests on this platform (`caditor-windows` off Windows).
- Files read from outside a crate's folder select their reader through `OUTSIDE_READS`
  (`changed.rs`): `packaging/` (the app's about box, logo and Windows resources, the MIME test of
  `caditor-file`), `fuzz/seeds/` (the seed tests of `caditor-file`, `caditor-step` and `caditor-checks`) and the Inter fonts in
  `crates/caditor/assets/fonts/` (compiled into `caditor-lettering`, read by `caditor-file`'s
  tests). `every_file_a_crate_reads_outside_its_folder_selects_it_when_changed` scans every
  `include_bytes!`, `include_str!` and `env!("CARGO_MANIFEST_DIR")).join(...)` and fails on one
  outside its crate that neither a dependency nor the table covers.
- The conventions tests (`caditor`'s `conventions_tests`) read the whole tree, so under
  `--changed` any change runs them as a step of their own (`-p caditor -E
  'test(/^conventions_tests::/)'`) unless `caditor` is selected anyway: a rules file or a doc alone
  still has its rules-table and coverage checks run, without the app's other tests.
- Under `--changed`, `check` runs `deny` only when a `Cargo.toml`, a `Cargo.lock` or `deny.toml`
  changed. `fmt` always covers everything: `rust-formatter` is fast and its single-file mode
  formats imports differently.
- `fuzz/`'s own build is not a step: CI's `fuzz-build` covers it, and the Stop hook keeps
  `fuzz/Cargo.lock` fresh (`ci.md`).
