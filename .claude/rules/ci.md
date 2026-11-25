---
paths:
  - ".github/**"
  - "fuzz/**"
  - "deny.toml"
  - "crates/caditor-file/src/fuzzing.rs"
  - "crates/caditor-sketch/src/fuzzing.rs"
---

# CI, dependency policy and fuzzing

## Workflow

- `ci.yml` runs on every push to `master` and every pull request; the release workflow calls it before
  building. Linux jobs run in Ubuntu 22.04 containers, Windows jobs on `windows-2025`, all with a
  timeout; a newer push to a pull request cancels its older run. The concurrency group is `ci-<ref>`,
  not the workflow's name: inside a called workflow `github.workflow` is the caller's, and GitHub
  would take the release's own group as a deadlock and cancel it.
- Tests run under cargo-nextest (`cargo nextest run --profile ci`, `build.md`), `--locked`, with
  `CADITOR_REQUIRE_GPU=1` on lavapipe (without it the offscreen render tests skip). The `ci` profile
  never stops at the first failure and ends a test still running after twenty minutes. Clippy runs
  `--all-features` so the `fuzzing` modules are linted. Also: `rust-formatter --check`, `cargo deny` on the root and fuzz workspaces, a snapshot archive checked by
  `packaging/check-install.sh`, the `.deb`, `.rpm` and AppImage built from it by
  `packaging/build-packages.sh` and checked (every file of the archive, `THIRD-PARTY-NOTICES.txt`
  included, must be in each), the `.deb` installed and removed with `apt-get` by
  `packaging/check-packages.sh --install`, and a minute of fuzzing per target.
- `fuzz-build` builds every fuzz target once (`cargo fuzz build`), keeping `fuzz/target` and the
  cargo registry under `fuzz-build-<toolchain>-<lock hash>` so registry dependencies are not
  rebuilt, and uploads the binaries as one `fuzz-targets` artifact: a zstd tarball, since a zip
  drops the executable bit. Each `fuzz` matrix job `needs` it, downloads the artifact and runs its
  one binary directly with `ASAN_OPTIONS=detect_odr_violation=0` and the `-artifact_prefix` that
  `cargo fuzz run` sets: no Rust, no `cargo-fuzz`, no build cache. A job that ran `cargo fuzz run`
  itself spent four minutes extracting the 8-10 GB build cache and 13 more recompiling every
  workspace crate and its target, since a fresh checkout's file times are newer than the cached
  fingerprints, which with a minute of fuzzing met the 20-minute limit. The `Fuzz` step has its own
  10-minute timeout and the job 15, so a hang fails the step with its log instead of cancelling
  the job.
- The check job runs cargo as the unprivileged `builder` user (`as-builder`), since root ignores the
  file modes the unreadable-file tests rely on, with its own `XDG_RUNTIME_DIR=/run/user/1001` (Mesa's
  Vulkan stack probes for Wayland and libwayland prints an error each time when unset). It fails if
  the tests leave any change or untracked file in the checkout, so a test writing beside the sources
  instead of a `TempDir` is caught.
- The check job also runs `caditor-render`'s `offscreen_tests` with `WGPU_BACKEND=gl`,
  `LIBGL_ALWAYS_SOFTWARE=1` and `CADITOR_REQUIRE_GPU=1` on llvmpipe through surfaceless EGL, since
  `packaging/INSTALL.md` promises OpenGL; a test in that file asserts the adapter is the backend asked
  for, so the step cannot pass on Vulkan by accident.
- `windows` tests every crate even after one fails (the `ci` profile, `CADITOR_REQUIRE_GPU=1` on the
  software adapter), checks the tree stays clean and lints, the only place `cfg(windows)` code is
  compiled in CI. `package-windows` builds a snapshot MSI
  (`packaging/windows/build-release.ps1 -Snapshot`) and runs `check-install.ps1` on it. Windows jobs
  check out with `core.autocrlf` off, since tests compare committed text byte for byte.
  `.github/install-windows-packaging.sh` installs WiX, checked against its pinned SHA-256 (from its
  NuGet package, installed from a local source only).
- The package and release jobs install Xvfb and the Mesa drivers (`.github/linux-display.sh`, which
  also removes them before `check-packages.sh --install`, so the `.deb` still has to bring its own
  libraries) and run `check-install.sh` with `CADITOR_REQUIRE_DISPLAY=1`. Besides the files and the
  menu entry it runs `check-binary.sh` (`NEEDED` libraries are glibc and `libgcc_s` only, `ldd`
  resolves them all, highest `GLIBC_` symbol at most 2.35; `CADITOR_MAX_GLIBC` previews on a newer
  system) and `check-run.sh`: the installed program starts under Xvfb, first on lavapipe
  (`VK_ICD_FILENAMES`, `WGPU_BACKEND=vulkan`), then on llvmpipe (`WGPU_BACKEND=gl`,
  `LIBGL_ALWAYS_SOFTWARE=1`; a `WGPU_BACKEND` in the environment limits it to that one). On each it
  runs two checks with their own state directory. With `CADITOR_STARTUP_CHECK=1` it logs its first
  frame and its journalled edit (`app.md`), is killed with SIGKILL and leaves a journal; restarted
  with `CADITOR_STARTUP_CHECK=restore` it logs the recovery offer and that the restored document holds
  the edit. The first check uses an untitled document (journal under the state directory); the second
  has the program save a model first (`CADITOR_STARTUP_CHECK=save:<path>`), then checks the journal
  sits beside it as `.model.caditor.journal` and the restore lands in `model.caditor`. Both scripts
  also run alone: without `CADITOR_REQUIRE_DISPLAY`, `check-install.sh` skips the start when Xvfb is
  missing.
- The `stress` job runs only nightly and on demand (`schedule`, `workflow_dispatch`): the property
  suites of `caditor-checks` over `CADITOR_CASES=10000` cases in release, the ignored
  kernel and sketch surveys and the large STEP import benchmark (`app-tests.md`) in release, each
  through `.github/run-ignored.sh`, which runs the named ignored tests with libtest's `--exact` (a
  handful of named release runs gain nothing from a process each) and fails when fewer ran than were
  named, so a renamed test cannot leave the job green running nothing.
  `random_placements_of_every_fixture` asserts every boolean of its seed succeeds. An ignored test
  pinning a known failure (`vertical_edges_are_rounded_after_the_top_rim_is_chamfered`) or only
  timing (`display_mesh_costs`) is left out.
- `deny.toml` resolves both release targets, Linux and Windows.
- Ubuntu 22.04's `desktop-file-validate` rejects keys newer than its spec; `caditor.desktop` uses only
  keys it knows.
- The AppImage runtime is a pin like the tools (`APPIMAGE_RUNTIME_VERSION`, `APPIMAGE_RUNTIME_SHA256`):
  `build-packages.sh` reads `versions.env` itself outside the workflows and checks the download
  against the sum; `bump-pins.sh` picks the newest dated release of the runtime, not its rolling
  `continuous` one.
- `build-release.sh` fails on any `appstreamcli` finding except `accepted_metainfo_findings`, which
  follow from publishing no identity (`docs/RELEASING.md`). The release attests provenance (Sigstore)
  of the archive and of the `.deb`, `.rpm` and AppImage made from it, and builds the MSI on Windows
  the same way, publishing it beside the archive.

## Pinning and cargo-deny

- Every version pin lives once, in `.github/versions.env`, loaded into `GITHUB_ENV` after each checkout
  by both workflows. Never copy a pin into a workflow's `env`.
- Rust comes from `.github/actions/install-rust`, which verifies `rustup-init` (`rustup-init.exe` on
  Windows, `RUSTUP_WINDOWS_SHA256`) against its pinned checksum; never `curl | sh`. Tool binaries
  (`cargo-deny`, `cargo-fuzz`, `cargo-nextest` through `.github/actions/install-nextest`, WiX) are
  checked against pinned SHA-256 sums; `rust-formatter` is built from `RUST_FORMATTER_REV`.
- Actions are pinned by commit with their version in the step name. `conventions_tests.rs` checks
  that every `uses:` is a full commit with its version in the step name, that no workflow holds a
  value of `versions.env`, and that the fuzz matrix, `fuzz/Cargo.toml` and `fuzz/fuzz_targets` list the
  same targets, each with seeds and a dictionary that exist.
- `.github/bump-pins.sh` raises every pin to its latest release; review the diff and let CI pass
  before committing it.
- `deny.toml` covers licences, sources, advisories and bans, each exception with its reason.
  Duplicate versions are denied except upstream-caused ones; C-backed crates with a Rust alternative
  are banned. The fuzz workspace is checked with the same file (`--config deny.toml`). Licence
  exceptions take no `reason` key, so theirs is here: `libfuzzer-sys` may be NCSA, since it is LLVM's
  libFuzzer, linked only into the fuzz workspace and never into a release.

## Fuzzing

- `fuzz/` is its own cargo workspace for `cargo fuzz` on nightly, one target per file in
  `fuzz/fuzz_targets`. CI runs them with `--target x86_64-unknown-linux-gnu`: the released cargo-fuzz
  binary is built for musl and would otherwise build for musl, which AddressSanitizer refuses.
- Byte targets feed parsers and loaders (expression, DXF, SVG, model files, journal, zstd, STEP,
  meshes, preferences, recent files, parameter files, configuration files, environment pictures, and `mcp`: JSON-RPC lines through the MCP server's reader and
  tool argument decoding, `caditor::fuzzing::mcp_lines`); `model_sealed` and the resealed journal target recompute chunk
  checksums so the decoders behind them are reached, while `journal_torn` leaves a damaged tail torn.
- Structured targets (`profile_sweep`, `boolean`, `sketch_solve`, `sketch_edit`, `document`, `save_load`) decode bytes
  with `arbitrary::Unstructured` through the generators in `crates/caditor-checks` (`checks.md`), biased to near-coincident and
  aligned geometry. They hold the invariants of their subject: an `Ok` solid validates, each
  transaction is undone exactly by its inverse, every saved version loads back.
- The document generators (`crates/caditor-checks/src/document/`) make every `Edit` variant and every feature kind:
  `addition` (parameters, owned ones, a feature of each `Family`), `parameter_edit`, `tree_edit`
  (`SetFeatureKind` with a new kind of the feature's own family, groups, appearance), `sketch_edit`
  (projections, labels, active flags, placements with attachments, the builder's `add_projection`
  and `remove_sketch_items`) and `model_edit` (properties, saved views, selection sets,
  configurations raw and through the document's configuration builders). `Generator::of` matches
  `Edit` and `Family::of` matches `FeatureKind` exhaustively, so a new variant fails to compile
  until a generator covers it. Face, edge and vertex references are synthetic names with every
  `FaceOrigin`; `document` runs twice, the second time with references captured from the first
  recompute's bodies (`References::capture`), so they resolve. Imports are one block, written to
  STEP once.
- `save_load` saves after every change, checking each loads back as it was, and logs each change as
  the storage worker does (`caditor_file::fuzzing::Journal`, through the same `Mirror`); at the end
  the journal is written, read and replayed (`replay_journal`), and must replay every entry with no
  issue to the same document. A document state that saves or journals differently from what the
  document holds is a finding: the document settles what loading would (`document.md`).
- `expression` checks each text that parses, typed or stored (`caditor_checks::expression`,
  with `width` a length, `count` plain and `turn` an angle): its stored and its display text
  parse back to the same tree (a refusal only for length or depth), it evaluates without
  panicking, and `with_lengths_scaled` by 2 at plain powers 0 to 2 stores back and, when the text
  has no comparison or stepless rounding and no subnormal value inside it, evaluates to the old
  value times 2 to its length power within `AGREEMENT` of its largest inner value. Scaling by a
  power of two is exact in floating point, so any other difference is the rewrite's.
- `sketch_edit` (`caditor_checks::sketch_edit`) draws a random sketch, then trims, extends, splits,
  breaks, offsets (one side and both), fillets, chamfers, joins at crossings and places points on
  curves at random picks. A refused edit must leave the sketch equal to what it was (edits work
  on a copy, `sketch.md`); an applied one must keep use counts equal to a count from scratch,
  read through `caditor_sketch::fuzzing` (behind the crate's `fuzzing` feature, which
  `caditor-checks` turns on), with every entity in use still in the sketch. It offsets no spline
  of more than `MOST_OFFSET_SPLINE_POINTS` points, since each such offset takes seconds
  (`PERFORMANCE.md`) and would starve the target.
- Kernel and solver work runs under `WORK_BUDGET`, so slow inputs cancel instead of tripping
  libFuzzer's timeout: the structured targets but `sketch_edit` (sketch edits take no interrupt)
  and both STEP targets (`step`, `step_import`). Every
  panic counts, including ones recompute would contain.
- The byte entry points are `caditor_file::fuzzing` and `caditor::fuzzing`, behind their `fuzzing`
  features.
- A byte input a target once timed out or crashed on is kept as a file in
  `crates/caditor-file/regressions` and read by a test of the subject (not in `fuzz/seeds`, which
  tests require to load) that asserts the refusal or the bounded result:
  `dxf-spline-array.dxf` is a block of non-native splines repeated thousands of times, read in
  well under a second since the copies share one fit; `dxf-bolt-array.dxf` the same with a spline
  no fit brings within the drawing's tolerance, whose copies share its closest fit.
- `picture` feeds bytes to `caditor_file::fuzzing::decode_picture` (every format a reference image
  reads), seeded with a small picture of each format (`write_picture_fuzz_seeds`, checked by
  `picture_seeds_decode`); the document generators make reference images with a few bytes behind a
  PNG signature, since the document never decodes them.
- A crash in a structured target (`document` first) is copied to `crates/caditor-checks/regressions/<suite>/`
  and replayed by `cargo test` (`checks.md`); byte targets keep theirs in `crates/caditor-file/regressions`.
- `parameters` and `configurations` feed bytes (as lossy text) to `caditor_file::fuzzing::parameters`
  and `configurations`, then plan and apply the import twice into a document, the second time over
  what the first made; `environment` feeds `decode_environment` and holds the decoded size to the
  pixel limit. Their seeds are the exported files, spreadsheet variants and Radiance pictures
  (`write_table_fuzz_seeds`, `write_environment_fuzz_seeds`, checked by
  `parameter_and_configuration_seeds_read` and `environment_seeds_decode`), the first two sharing
  `csv.dict` and the third using `hdr.dict`.
- Every matrix entry names a `max_len`, passed to libFuzzer as `-max_len`: 4096 for the structured
  targets, 16384 for expression and recent files, 65536 for the other byte formats and 262144 for the
  two STEP targets. The conventions test holds it at or above the largest seed of the entry, since
  libFuzzer truncates a seed longer than `max_len`.
- Seeds are committed in `fuzz/seeds/<kind>` and dictionaries in `fuzz/dictionaries/<kind>.dict`,
  shared by targets reading the same kind of input. The model, journal and zstd seeds are written by
  `CADITOR_WRITE_FUZZ_SEEDS=1 cargo test -p caditor-file write_fuzz_seeds -- --ignored`; tests keep
  every seed loading and the model and journal seeds in format 1
  (`model_and_journal_seeds_are_written_in_format_one`), so they are rewritten with any format
  change. The seed writer refuses any path inside the write-once format corpus
  (`crates/caditor-file/corpus`, `file-format.md`), which proves shipped formats still read.
  `fuzz/corpus` is not committed; CI caches it.
- The readers' deep and wide seeds are built by code, small on disk and large in reach, in the
  shapes of the crafted files that once hung or exhausted memory: STEP styles listing each other
  fifty times a level, a 30-layer diamond assembly, a class of joined representations mapped
  hundreds of times, placements nested to `MAX_DEPTH`, a 200-sided prism's long lists, a list
  nested 60 deep and point and curve replicas chained to their depth limit (`caditor-step`'s
  `read/seeds.rs`); 3MF components nested to `MAX_COMPONENT_DEPTH`, components placing billions
  of objects, a hundred and twenty objects, objects nested in objects and a box of seven thousand
  triangles; SVG groups nested past `MAX_NESTING`, uses fanned four ways seven and ten levels
  deep, `textPath`s and uses of one long path and hundreds of transformed paths
  (`caditor-file`'s `import/seed_tests.rs`). They are written by
  `CADITOR_WRITE_FUZZ_SEEDS=1 cargo test -p caditor-step -p caditor-file fuzz_seeds -- --ignored`
  (with the model, journal and other seeds). A seed named `refused-…` must be refused in words, every
  other one must import (the import tests skip the refused ones); `step_seeds_are_read_within_their_work_bound`,
  `mesh_seeds_are_read_within_their_work_bound` and `svg_seeds_are_read_within_their_work_bound`
  hold every seed of the kind, generated or not, to its reader's `tally`: work linear in the
  (unpacked) bytes and what the read yields (curves, placed vertices and triangles), plus the
  reader's fixed caps only when it says it stopped short (a refusal, or STEP's note that copies were
  cut short: `MAX_WALKED_PLACEMENTS` and `MAX_INSTANCES`; SVG's `MAX_EXPANDED_OBJECTS`; 3MF's placed
  objects, vertices and triangles), so a walk whose work grows with the reach rather than the file
  fails in CI.
- `fuzz/Cargo.lock` is committed and CI fails when it is stale: after any change to the root
  dependencies, run `cargo update -w` in `fuzz/`. A Stop hook in `.claude/settings.json`
  (`.claude/hooks/fuzz-lock-fresh.sh`) runs `cargo metadata --locked` on the fuzz workspace and
  refuses to finish while the lock is stale.
