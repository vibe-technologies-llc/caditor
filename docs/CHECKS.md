# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · hard · next] Errors carried as text break the error rule (errors are enums of
  specific variants with typed data, never a formatted message). The shared helpers (`field.rs`,
  `feature_fields.rs`, `feature_values.rs`, `typed_point.rs`, `sketch_placement.rs`,
  `reference_picking.rs`, `dimensioning.rs`, `model_dimensions.rs`), `projecting.rs`, the file
  workflow (import, export, history, pictures, parameter and configuration files) and
  caditor-step's reader (`Fault`) are typed; the rest of the app crate still returns
  `Result<_, String>` at about 300 sites (`feature_tree.rs`, `gearing.rs`, `sketch_tools.rs`,
  `hole_placement.rs`, `sketch_toolbar.rs`, `solid_panel.rs`, `selection_sets.rs`,
  `patterning.rs`, `sketch_drag.rs` and some seventy more), and the MCP server wraps them
  (`SketchEditError::Unmovable { reason }`, `ToolError::Value`'s `reason`, `Conflicting { reason }`,
  `NotAPlane`'s `reason`), as do `queued.rs`'s and `filleting.rs`'s `reason`. Until each caller
  is typed, the typed modules keep bridges to remove with it: `From<…> for String` on every new
  enum (`ExpressionError`, `CheckError`, `NamedEditError`, `DimensionEditError`, `RuleError`,
  `QuantityError`, `ChoiceRefusal`, `TypedValueError`, `TypedPointError`, `ChangeError`,
  `NewSketchError`, `DimensionToolError`, `SideOutOfRange`, `SheetLengthError`, `ImportBlocked`,
  `PlacementError`), the variants holding a callee's text (`reference_picking::ChangeError::
  Unconverted`, `DimensionToolError::Unproposed` from `sketch_tools.rs`,
  `SheetLengthError::Typed` from `modifying::Value::typed`), and the `E = String` defaults of
  `Choice`, `Segment`, `Picker`, `AxisRow` and `plane_selector::Row`. `sketch_placement.rs` and
  `drawing_export.rs` also return `Result<_, &'static str>`, the same shape. Convert each module
  to its own enum worded by `Display` (thiserror), wrapped in callers' errors with
  `#[from]`/`source`; strings that are a user's own names or typed text stay. Then add a check
  (`conventions_tests.rs`) refusing `Result<_, String>`, `From<…> for String` and an error variant
  whose only payload is a `reason`, `message` or `detail` string outside tests, and record it in
  `rust-style.md`. The document's feature `Failure` (`reason` and `remedy` text built by each
  feature's `error`) is the same shape and follows once the app is done. Next because every new
  error written against these functions copies the pattern.
- [low · medium] Slow tests to keep an eye on: with egui, its glyph stack and the PNG encoder built
  optimised in the dev profile (`dependencies.md`) a UI test takes a few tenths of a second, and
  none of the ten that took over a second (the screen-reader test, the shortcut reset tests, the
  hole, extrusion, chamfer and section panel tests, the PNG export test) takes over 0.55 s. What is
  left is the app's own unoptimised frames and the recompute and meshing of the bodies the tests
  build; re-measure the whole UI suite and list the tests still over half a second.
