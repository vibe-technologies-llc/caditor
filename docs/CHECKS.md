# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [low · medium] Slow tests to keep an eye on: with egui, its glyph stack and the PNG encoder built
  optimised in the dev profile (`dependencies.md`) a UI test takes a few tenths of a second, and
  none of the ten that took over a second (the screen-reader test, the shortcut reset tests, the
  hole, extrusion, chamfer and section panel tests, the PNG export test) takes over 0.55 s. What is
  left is the app's own unoptimised frames and the recompute and meshing of the bodies the tests
  build; re-measure the whole UI suite and list the tests still over half a second.
- [medium · easy] The sketch session changes (`queued.rs`, Cancel sketch, the context-switch
  notice's Undo) have no UI tests: one each for Cancel sketch, the notice's Undo, and a queued
  action run when the recompute lands (`Model::hold_evaluations`) and one dropped by Escape.
- [low · easy] The screenshot scenes `orbit`, `more-keys`, `waiting-circular-pattern` and
  `panel-choosing-list` have never been rendered, and the interface refresh was compared only in
  the dark and light looks: render all twelve looks and check the high-contrast, 200% and light
  canvas ones.
