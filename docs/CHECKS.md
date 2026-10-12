# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [high · medium] Nothing proves a shipped format still reads (`reliability.md`): format 1 is
  the first shipped one (models, journals, parameter files), but the only files kept are the fuzz
  seeds, which `write_fuzz_seeds` rewrites in whatever the current format is, and tests build their
  files with today's encoder. Start a write-once corpus at format 1, covering each record kind,
  journal entry kind and parameter file of its day, loaded against recorded digests on every run;
  add to it whenever a format version changes, and have the seed writer refuse to touch it.
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
- [low · easy] `caditor-file` depends on `ttf-parser` directly for SVG text (`import/svg/font.rs`),
  which is why `deny.toml` ignores RUSTSEC-2026-0192 (unmaintained) beside winit's sctk-adwaita,
  while the maintained `skrifa` is already built through epaint: port the outlines, `wght` axis and
  GPOS kerning to `skrifa` and `read-fonts` and the direct dependency goes.
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
