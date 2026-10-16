# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · medium] Errors carried as constant text: no app function returns `Result<_, String>`
  any more and `errors_are_typed_rather_than_text` (`conventions_tests.rs`) keeps it so, but about
  35 still return `Result<_, &'static str>` (`drawing.rs`, the `Offers` fields in `offers.rs`
  (whose datum fields convert `DatumFromSelectionError::text` at the boundary),
  `sketch_pattern_tools.rs`, the tools' `steppable` and `highlight_needed` shared with
  `commands::Reason`, and a few more), so the variants wrapping them hold that text
  (`PatternSketchError::Sketch`, `SymmetryError::ChosenLine`).
  `modifying::Outcome::Refused` and `queued.rs` (whose `Awaited::of` matches a refusal's words)
  still pass a sentence along. Type each, then extend the check to `&'static str` errors. The
  document's feature `Failure` (`reason` and `remedy` text built by each feature's `error`) is the
  same shape and follows.
- [low · medium] Slow tests to keep an eye on: with egui, its glyph stack and the PNG encoder built
  optimised in the dev profile (`dependencies.md`) a UI test takes a few tenths of a second, and
  none of the ten that took over a second (the screen-reader test, the shortcut reset tests, the
  hole, extrusion, chamfer and section panel tests, the PNG export test) takes over 0.55 s. What is
  left is the app's own unoptimised frames and the recompute and meshing of the bodies the tests
  build; re-measure the whole UI suite and list the tests still over half a second.
