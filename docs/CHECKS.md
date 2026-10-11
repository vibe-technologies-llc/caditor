# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [high · medium] No old-format files are kept to prove every shipped format still reads
  (`reliability.md`): the "saved before format N" tests build their files with today's encoder
  under an old version number (`model_from_json(1, …)`), so record shapes older builds wrote are
  never read; the only real old bytes are the three version-1 model seeds in `fuzz/seeds/model`, no
  version-2 file exists, and running `write_fuzz_seeds` as `ci.md` says rewrites them in version 3
  (breaking the test that asserts their header). Keep a write-once corpus per format version
  covering each record kind of its day, loaded against recorded digests on every run, and have the
  seed writer refuse to touch it.
- [medium · easy] No fuzz target reaches mesh import: `fuzz/fuzz_targets` covers DXF, SVG, STEP,
  the model and journals, but nothing feeds `parse_mesh` STL, OBJ or 3MF, so the ZIP reader
  (`zip_read.rs`, tested only on archives our own writer made) and the XML reader on the 3MF path
  are never fuzzed, and the unbounded vertex copies of 3MF placement (`BUGS.md`) sit there. Add a
  `mesh` target over every `MeshFormat`, seeded with small STL, OBJ and 3MF files, to the CI matrix
  and `fuzz/Cargo.toml`.
- [medium · medium] The document fuzzers cover a part of what they claim (`ci.md`): `tree_edit`
  and `sketch_edit` produce 18 of the 34 `Edit` variants (never `SetFeatureKind`, the
  configurations edits, projections, labels, appearance, groups, owners, notes, saved views or
  selection sets) and 3 of the 20 feature kinds, and no target journals generated edits and
  replays them. Generate the rest, replay the journal in `save_load`, and add an exhaustive
  `match` on `Edit` so a new variant fails to compile until a generator covers it.
- [medium · medium] Nothing tests the app when the storage worker dies or stops: `Model::send_save`
  failing to send, `restart_storage` after a failed `record` and `poll_storage` returning an error
  (`caditor/src/model.rs`) decide whether pending journal entries are kept, a save reports its
  failure and the journal is taken up again, and no test stops or breaks the worker. Add a test
  seam that stops the storage worker on command and cover a failed record keeping every entry and
  starting a new worker, a failed save reported as failed, and quitting with the worker gone.
- [medium · medium] Wall-clock asserts in the default suite, which flake on a loaded runner and
  miss complexity regressions on a fast one: `profile/tests.rs` (2 s), `binary/tests.rs` (2 s),
  `caditor-expression` (2 s), `read/structure.rs` (5 s), `fit.rs` and `import/tests.rs` (10 s)
  and about seven more in the kernel and file crates; the nightly
  `diagnosis_tests` asserts under 5 s, against `sketch-solver.md`'s "never wall-clock time". Bound
  them by work units or iteration counts, and refuse `elapsed()` inside an assert in a
  conventions test.
- [low · easy] Rules a machine could check that nothing checks: `reliability.md`'s "never
  `#[allow]`" (clippy's `allow_attributes` and `allow_attributes_without_reason` are not denied;
  the unsafe crates would move to `#[expect]`), and `ci.md`'s fuzz matrix, action pins and the
  ban on copying pins into a workflow's `env` (the matrix in `ci.yml`, `fuzz/fuzz_targets` and
  `fuzz/Cargo.toml` agree today by hand). Deny the lints and compare the three target lists and
  the `uses:` pins in `conventions_tests.rs`.
- [low · easy] `deny.toml` ignores RUSTSEC-2026-0192 (ttf-parser unmaintained) "because it comes in
  through winit's sctk-adwaita", but `caditor-file` now depends on it directly for SVG text
  (`import/svg/font.rs`), while the maintained `skrifa` is already built through epaint: port the
  outlines, `wght` axis and GPOS kerning to `skrifa` and `read-fonts`, or correct the reason.
  `caditor-file` also enables serde_json's `raw_value`, which no code uses since the binary format,
  and `caditor-windows` enables `windows-sys`'s `Win32_UI_Shell`, which nothing in it calls
  (`dependencies.md`: features only when used; `cargo xwin clippy` confirms its removal).
- [low · medium] Slow tests to keep an eye on: with egui, its glyph stack and the PNG encoder built
  optimised in the dev profile (`dependencies.md`) a UI test takes a few tenths of a second, and
  none of the ten that took over a second (the screen-reader test, the shortcut reset tests, the
  hole, extrusion, chamfer and section panel tests, the PNG export test) takes over 0.55 s. What is
  left is the app's own unoptimised frames and the recompute and meshing of the bodies the tests
  build; re-measure the whole UI suite and list the tests still over half a second.
- [low · medium] `ui_tests.rs` holds the harness and 432 tests in 21,600 lines, touched by about
  half of all commits, the merge hotspot of parallel worktrees and too large to read whole; 58
  topic modules already exist under `ui_tests/`. Move the root tests into them by topic, keeping
  the harness.
