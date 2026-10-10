# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [high · easy] Nothing tests that a failed save leaves the file as it was: `replace_checked`
  removes its temporary and returns the error when filling, syncing, the read-back or the rename
  fails, but every test of it takes the success path, and the failing-save tests fail before
  anything is written (a missing folder, unreadable versions). Fail `fill` halfway and the check
  with `NotReadBack` over an existing file and assert its bytes, the folder's contents and the
  reason.
- [high · medium] No old-format files are kept to prove every shipped format still reads
  (`reliability.md`): the "saved before format N" tests build their files with today's encoder
  under an old version number (`model_from_json(1, …)`), so record shapes older builds wrote are
  never read; the only real old bytes are the three version-1 model seeds in `fuzz/seeds/model`, no
  version-2 file exists, and running `write_fuzz_seeds` as `ci.md` says rewrites them in version 3
  (breaking the test that asserts their header). Keep a write-once corpus per format version
  covering each record kind of its day, loaded against recorded digests on every run, and have the
  seed writer refuse to touch it.
- [medium · easy] The nightly stress job runs three ignored tests by `--exact` name, while
  ignored surveys asserting kernel and sketch invariants run nowhere
  (`boolean::tests::aligned_contacts_a_micrometre_or_so_apart`, `blend/survey.rs`,
  `carry_tests.rs`, the solver's memo and fully-constrained surveys), against `ci.md` saying the
  job runs "the ignored kernel and sketch stress tests"; a renamed test also leaves the job green
  running nothing, since libtest passes a filter matching none. Add them and fail a step that ran
  no test.
- [medium · easy] Rules files name identifiers that are gone, and 45 of 566 source files match no
  scoped rule, so editing them never loads the rule that describes them: `canvas::MUTED`,
  `WARNING`, `SNAP`, `HOVERED`, `MEASURE` and `PANEL`, `PROJECTED`, `CUT_PREVIEW` and
  `DRAWING_FACE` became `Chrome` and `ScenePalette` fields in the themes change (`app-look.md`,
  `app-sketching.md`, `app-modelling.md`, `app.md`, `app-input.md`); `render.md` names
  `pick_reference_fills` and a `BEHIND_FACES` factor, `app-modelling.md` `scene::previewed_body`;
  `value_gauges.rs`, `value_handles.rs`, `place_handles.rs`, `turn_handles.rs`,
  `scene_description.rs`, `upload_badge.rs` and all of `caditor-geometry` are among the unmatched;
  and CLAUDE.md's crate table and rules table list DXF/SVG/STEP import and STL/3MF/STEP/PNG export,
  missing mesh import and OBJ, glTF and drawing export. Fix the names and paths, and add a
  conventions test that every source file matches some rule's `paths:`, with an explicit list of
  the files meant to have none.
- [medium · medium] The document fuzzers cover a part of what they claim (`ci.md`): `tree_edit`
  and `sketch_edit` produce 18 of the 34 `Edit` variants (never `SetFeatureKind`, the
  configurations edits, projections, labels, appearance, groups, owners, notes, saved views or
  selection sets) and 3 of the 20 feature kinds, and no target journals generated edits and
  replays them. Generate the rest, replay the journal in `save_load`, and add an exhaustive
  `match` on `Edit` so a new variant fails to compile until a generator covers it.
- [medium · medium] Wall-clock asserts in the default suite, which flake on a loaded runner and
  miss complexity regressions on a fast one: `profile/tests.rs` (2 s), `binary/tests.rs` (2 s),
  `caditor-expression` (2 s), `read/structure.rs` (5 s), `fit.rs` and `import/tests.rs` (10 s)
  and about seven more in the kernel and file crates; the nightly
  `diagnosis_tests` asserts under 5 s, against `sketch-solver.md`'s "never wall-clock time". Bound
  them by work units or iteration counts, and refuse `elapsed()` inside an assert in a
  conventions test.
- [low · easy] `ui_tests::a_hole_is_drilled_at_the_points_of_a_sketch_and_its_panel_changes_the_style_and_sizes`
  fails now and then under load, twice on 2026-10-10 and 11 while other builds ran, and passes on
  its own every time: after `open_combo(&mut harness, "Depth")` and
  `click_lowest("Through all")` it asserts one row named Depth, and finds two, so the pick
  landed elsewhere and the hole stayed blind. Wait for the list to settle before clicking (as
  `open_combo` does before pressing), or choose the depth through the field's id.
- [low · easy] Fixed sleeps before asserting that nothing arrived: `ui_tests/import_jobs.rs` sleeps
  100 to 200 ms after a reader returns, then asserts a cancelled import's result was not applied,
  and `presenting_tests.rs` sleeps 50 ms before asserting no early report, so on a slow runner they
  pass before the message could arrive. Wait on `Files::wait_for_jobs` or a seam signalling the
  result was delivered or dropped.
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
  `caditor-file` also enables serde_json's `raw_value`, which no code uses since the binary format
  (`dependencies.md`: features only when used).
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
