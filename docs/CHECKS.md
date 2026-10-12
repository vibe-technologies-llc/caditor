# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · medium] The document fuzzers cover a part of what they claim (`ci.md`): `tree_edit`
  and `sketch_edit` produce 18 of the 34 `Edit` variants (never `SetFeatureKind`, the
  configurations edits, projections, labels, appearance, groups, owners, notes, saved views or
  selection sets) and 3 of the 20 feature kinds, and no target journals generated edits and
  replays them. Generate the rest, replay the journal in `save_load`, and add an exhaustive
  `match` on `Edit` so a new variant fails to compile until a generator covers it.
- [low · easy] The XML test of a hundred thousand distinct namespaces asserts nothing about cost
  since its wall-clock bound went: give `Parser::intern` a `tally` unit per entry it compares and
  bound the total by the declarations made.
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
