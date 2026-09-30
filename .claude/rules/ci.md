---
paths:
  - ".github/**"
  - "fuzz/**"
  - "deny.toml"
  - "crates/caditor-file/src/fuzzing.rs"
---

# CI, dependency policy and fuzzing

## Workflow

- `.github/workflows/ci.yml` runs on every push to `master` and every pull request, and is called by
  the release workflow before it builds. Jobs run in Ubuntu 22.04 containers on the toolchain pinned
  by `RUST_TOOLCHAIN` (shared with the release workflow), each with a timeout; a newer push to a
  pull request cancels its older run.
- Jobs: tests with `--locked` and `CADITOR_REQUIRE_GPU=1` on the lavapipe software Vulkan driver
  (without that variable the offscreen render tests skip when no adapter exists); clippy;
  `rust-formatter --check` on the nightly pinned by `FORMAT_TOOLCHAIN`; `cargo deny`; a snapshot
  archive checked by `packaging/check-install.sh`; a minute of fuzzing per target.

## Pinning and cargo-deny

- Actions are pinned by commit. `cargo-deny`, `cargo-fuzz` and `cargo-about` are their release
  binaries, checked against pinned SHA-256 sums. `rust-formatter` is built from the commit pinned by
  `RUST_FORMATTER_REV`.
- `deny.toml` covers licences, sources and advisories; each ignored advisory carries its reason.

## Fuzzing

- `fuzz/` is its own cargo workspace for `cargo fuzz` on nightly. Targets:
  - `expression`: the expression parser;
  - `dxf`: DXF;
  - `model`: model files as they are (mostly exercises the container's damage scan);
  - `model_sealed`: model files with every chunk checksum recomputed (reaches the value decoder, the
    record loaders and the version history);
  - `journal`: the recovery journal and its replay (also resealed);
  - `zstd`: `caditor-zstd` with and without a prefix (also checks that frames round-trip);
  - `step`: `read_step` (covers the Part 21 parser);
  - `step_import`: STEP import end to end.
- The byte-based entry points are `caditor_file::fuzzing`, behind `caditor-file`'s `fuzzing`
  feature.
- Seeds are committed in `fuzz/seeds/<kind>` and dictionaries in `fuzz/dictionaries/<kind>.dict`,
  shared by targets reading the same kind of input. The model, journal and zstd seeds are written by
  `CADITOR_WRITE_FUZZ_SEEDS=1 cargo test -p caditor-file write_fuzz_seeds -- --ignored`; tests keep
  every seed loading cleanly. `fuzz/corpus` stays local.
