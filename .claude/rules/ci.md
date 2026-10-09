---
paths:
  - ".github/**"
  - "fuzz/**"
  - "deny.toml"
  - "crates/caditor-file/src/fuzzing.rs"
---

# CI, dependency policy and fuzzing

## Workflow

- `ci.yml` runs on every push to `master` and every pull request, and the release workflow calls it
  before building. Linux jobs run in Ubuntu 22.04 containers, Windows jobs on `windows-2025`, all
  with a timeout; a newer push to a pull request cancels its older run. The concurrency group is
  `ci-<ref>`, not the workflow's name: inside a called workflow `github.workflow` is the caller's,
  and GitHub would take the release's own group as a deadlock and cancel it.
- `windows` tests every crate even after one fails (`--no-fail-fast`, with `CADITOR_REQUIRE_GPU=1`
  on the runner's software adapter), checks the tree stays clean and lints on Windows, the only
  place `cfg(windows)` code is compiled in CI.
  `package-windows` builds a snapshot MSI (`packaging/windows/build-release.ps1 -Snapshot`) and
  runs `check-install.ps1` on it. Windows jobs check out with `core.autocrlf` off, since tests
  compare committed text byte for byte. `.github/install-windows-packaging.sh` installs WiX,
  checked against its pinned SHA-256 (from its NuGet package, installed from a local source only).
- Tests run `--locked` with `CADITOR_REQUIRE_GPU=1` on lavapipe (without it the offscreen render
  tests skip when no adapter exists). Clippy runs `--all-features` so the `fuzzing` modules are
  linted. Also: `rust-formatter --check`, `cargo deny` on the root and fuzz workspaces, a snapshot
  archive checked by `packaging/check-install.sh`, and a minute of fuzzing per target.
- The `stress` job runs only nightly and on demand (`schedule`, `workflow_dispatch`): the ignored
  kernel and sketch stress tests and the large STEP import benchmark (`app-tests.md`) in release. `random_placements_of_every_fixture` asserts that every
  boolean of its seed succeeds, so a kernel change that breaks one fails the job.
- The check job runs cargo as the unprivileged `builder` user (`as-builder`), since root ignores the
  file modes the unreadable-file tests rely on. It fails if the tests leave any change or untracked
  file in the checkout, so a test writing beside the sources instead of a `TempDir` is caught.
- `deny.toml` resolves both release targets, Linux and Windows.
- Ubuntu 22.04's `desktop-file-validate` rejects keys newer than its spec; `caditor.desktop` uses
  only keys it knows.
- `build-release.sh` fails on any `appstreamcli` finding except `accepted_metainfo_findings`, which
  follow from publishing no identity (`docs/RELEASING.md`). The release build attests the archive's
  provenance (Sigstore). The release builds the MSI on Windows the same way and publishes it beside
  the archive.

## Pinning and cargo-deny

- Every version pin lives once, in `.github/versions.env`, loaded into `GITHUB_ENV` after each
  checkout by both workflows. Never copy a pin into a workflow's `env`.
- Rust comes from `.github/actions/install-rust`, which verifies `rustup-init` (`rustup-init.exe`
  on Windows, `RUSTUP_WINDOWS_SHA256`) against its pinned checksum; never `curl | sh`. Tool binaries (`cargo-deny`, `cargo-fuzz`, WiX) are checked
  against pinned SHA-256 sums; `rust-formatter` is built from `RUST_FORMATTER_REV`.
- Actions are pinned by commit with their version in the step name.
- `.github/bump-pins.sh` raises every pin to its latest release; review the diff and let CI pass
  before committing it.
- `deny.toml` covers licences, sources, advisories and bans, each exception with its reason.
  Duplicate versions are denied except upstream-caused ones; C-backed crates with a Rust
  alternative are banned. The fuzz workspace is checked with the same file (`--config deny.toml`).
  Licence exceptions take no `reason` key, so theirs are here: `libfuzzer-sys` may be NCSA, since
  it is LLVM's libFuzzer, linked only into the fuzz workspace and never into a release.

## Fuzzing

- `fuzz/` is its own cargo workspace for `cargo fuzz` on nightly, one target per file in
  `fuzz/fuzz_targets`. CI runs them with `--target x86_64-unknown-linux-gnu`, since the released
  cargo-fuzz binary is built for musl and otherwise builds for musl, which AddressSanitizer refuses. Byte targets feed parsers and loaders (expression, DXF, SVG, model files, journal,
  zstd, STEP, preferences, recent files); `model_sealed` and the resealed journal target recompute
  chunk checksums so the decoders behind them are reached, while `journal_torn` leaves a damaged
  tail torn.
- Structured targets (`profile_sweep`, `boolean`, `sketch_solve`, `document`, `save_load`) decode
  bytes with `arbitrary::Unstructured` through the generators in `fuzz/src`, biased to
  near-coincident and aligned geometry. They hold the invariants of their subject: an `Ok` solid
  validates, each transaction is undone exactly by its inverse, every saved version loads back.
- Kernel and solver work runs under `WORK_BUDGET`, so slow inputs cancel instead of tripping
  libFuzzer's timeout. Every panic counts, including ones recompute would contain.
- The byte entry points are `caditor_file::fuzzing` and `caditor::fuzzing`, behind their `fuzzing`
  features.
- Seeds are committed in `fuzz/seeds/<kind>` and dictionaries in `fuzz/dictionaries/<kind>.dict`,
  shared by targets reading the same kind of input. The model, journal and zstd seeds are written by
  `CADITOR_WRITE_FUZZ_SEEDS=1 cargo test -p caditor-file write_fuzz_seeds -- --ignored`; tests keep
  every seed loading. `fuzz/corpus` is not committed; CI caches it.
- `fuzz/Cargo.lock` is committed and CI fails when it is stale: after any change to the root
  dependencies, run `cargo update -w` in `fuzz/`. A Stop hook in `.claude/settings.json` runs
  `cargo metadata --locked` on the fuzz workspace and refuses to finish while the lock is stale.
