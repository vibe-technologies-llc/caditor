# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · medium] Errors carried as constant text: the app has no function returning
  `Result<_, String>` or `Result<_, &'static str>` any more and `errors_are_typed_rather_than_text`
  (`conventions_tests.rs`) keeps it so. The document's feature `Failure` (`reason` and `remedy`
  text built by each feature's `error`) is the same shape and follows.
- [low · medium] Slow tests to keep an eye on: with egui, its glyph stack and the PNG encoder built
  optimised in the dev profile (`dependencies.md`) a UI test takes a few tenths of a second, and
  none of the ten that took over a second (the screen-reader test, the shortcut reset tests, the
  hole, extrusion, chamfer and section panel tests, the PNG export test) takes over 0.55 s. What is
  left is the app's own unoptimised frames and the recompute and meshing of the bodies the tests
  build; re-measure the whole UI suite and list the tests still over half a second.
