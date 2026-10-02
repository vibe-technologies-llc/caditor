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
  (without that variable the offscreen render tests skip when no adapter exists); clippy with
  `--all-features`, so the `fuzzing` module is linted too; `rust-formatter --check` on the nightly
  pinned by `FORMAT_TOOLCHAIN`; `cargo deny` on the root and the fuzz workspace; a snapshot archive
  checked by `packaging/check-install.sh`; a minute of fuzzing per target.
- The check job runs cargo as the unprivileged `builder` user through `as-builder`, since root
  ignores the file modes that the tests of unreadable and unwritable files rely on. After the tests
  it fails if the checkout has any change or untracked file, so a test writing beside the sources
  (rather than in a `TempDir`) is caught.
- Cargo registries and `target` are cached per job, keyed by toolchain and lock file, with
  `CARGO_INCREMENTAL=0` to keep the cache small; `rust-formatter` is cached by its revision. Each
  fuzz target's corpus is cached and restored from its latest run, so coverage accumulates.
- Ubuntu 22.04's `desktop-file-validate` (0.26) rejects keys newer than its spec, such as
  `SingleMainWindow`; `packaging/caditor.desktop` uses only keys it knows.

## Pinning and cargo-deny

- Actions are pinned by commit. `cargo-deny`, `cargo-fuzz` and `cargo-about` are their release
  binaries, checked against pinned SHA-256 sums. `rust-formatter` is built from the commit pinned by
  `RUST_FORMATTER_REV`.
- `deny.toml` covers licences, sources, advisories and bans; each ignored advisory, licence
  exception and skipped duplicate carries its reason. Duplicate versions are denied except the
  listed ones, which come from upstream crates; C-backed crates with a Rust alternative
  (`openssl-sys`, `native-tls`, `zstd-sys`, `libz-sys` and the like) are banned. The fuzz workspace
  is checked with the same file (`--config deny.toml`); `libfuzzer-sys`'s NCSA licence is allowed
  there alone.

## Fuzzing

- `fuzz/` is its own cargo workspace for `cargo fuzz` on nightly. Targets:
  - `expression`: the expression parser;
  - `dxf`: DXF;
  - `model`: model files as they are (mostly exercises the container's damage scan);
  - `model_sealed`: model files with every chunk checksum recomputed (reaches the value decoder, the
    record loaders and the version history);
  - `journal`: the recovery journal and its replay (also resealed);
  - `zstd`: `caditor-zstd` with and without a prefix (also checks that frames round-trip);
  - `journal_torn`: the recovery journal as it is, not resealed, so a torn or damaged tail stays
    torn;
  - `step`: `read_step` (covers the Part 21 parser);
  - `step_import`: STEP import end to end;
  - `preferences`: preferences JSON through `Preferences::from_settings`, which must reach a fixed
    point after one save, and stored shortcuts through `commands::parse_stored`, which must
    round-trip;
  - `recent_files`: the recent-files list, which must round-trip once read.
- Structured targets decode their bytes with `arbitrary::Unstructured` (re-exported by
  `libfuzzer-sys`) through the generators in `fuzz/src`: points on a 0.5 mm grid nudged by 1e-6 to
  1e-4 mm, angles in 15° steps, so near-coincident and aligned cases come up often.
  - `profile_sweep`: profiles of lines, circles, arcs, rectangles, polygons and splines through
    `Profile::new`, region selection, triangulation and `extrude` or `revolve`;
  - `boolean`: two swept solids, the second placed by a rigid transform, through `boolean`;
  - `sketch_solve`: sketches with every constraint kind through `solve`, a re-solve of the solved
    geometry and a drag;
  - `document`: sequences of document edits, undo and redo on an `Editor`, then a recompute. Each
    applied transaction must be undone exactly by its inverse, `transaction_to` must reach the
    applied state, a refused one must leave the document unchanged, and undo and redo must not fail;
  - `save_load`: the same edit sequences saved after each step over the previous file, each save
    loading back to the same content with no issues and every kept version loading to a saved
    state.
  Kernel and solver work runs under a 3 s interrupt (`WORK_BUDGET`), so slow inputs cancel rather
  than trip libFuzzer's timeout; an `Ok` solid must validate, and every panic counts, including
  ones recompute would contain, since libFuzzer's panic hook aborts.
- The byte-based entry points are `caditor_file::fuzzing`, behind `caditor-file`'s `fuzzing`
  feature, and `caditor::fuzzing`, behind the app's `fuzzing` feature (which enables
  `caditor-file`'s).
- Seeds are committed in `fuzz/seeds/<kind>` and dictionaries in `fuzz/dictionaries/<kind>.dict`,
  shared by targets reading the same kind of input. The model, journal and zstd seeds are written by
  `CADITOR_WRITE_FUZZ_SEEDS=1 cargo test -p caditor-file write_fuzz_seeds -- --ignored`; tests keep
  every seed loading cleanly. The structured targets share `seeds/structured`, fixed pseudo-random
  bytes; the preferences and recent-files seeds are hand-written JSON sharing `json.dict`.
  `fuzz/corpus` is not committed; CI caches it.
- `fuzz/Cargo.lock` is committed and CI fails when it is stale (`cargo metadata --locked`): after
  changing a dependency of a crate the fuzz targets use, run `cargo update -w` in `fuzz/`.
