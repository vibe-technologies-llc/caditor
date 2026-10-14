# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · hard · next] Errors carried as text break the error rule (errors are enums of
  specific variants with typed data, never a formatted message): the app crate returns
  `Result<_, String>` at about 370 sites outside tests (`feature_tree.rs`, `field.rs`,
  `feature_fields.rs`, `gearing.rs`, `hole_placement.rs`, `sketch_tools.rs`, `sketch_drag.rs` and
  some forty more), so callers can only pass the words on: the MCP server's
  `SketchEditError::Unmovable { reason: String }` (`mcp/sketch_edits.rs`, from `sketch_drag`'s
  `Moving::of` and `Transforming`), `ToolError::Value`'s `reason`, `Conflicting { reason }`
  (`mcp/writing.rs`) and `NotAPlane`'s `reason` (`mcp/places.rs`) wrap them, as do
  `queued.rs`'s and `filleting.rs`'s `reason`, `history.rs`' and `image_textures.rs`'
  `Failed(String)`, `drop_target.rs`' `detail`, caditor-file's `reference_images::Read(String)`
  and caditor-step's `NotRebuilt { reason }` and `graph.rs` `reason`. Replace each with an enum
  per module whose variants carry the typed data, worded by its `Display` (thiserror), and wrap
  them in the callers' errors with `#[from]`/`source`; strings that are a user's own names or
  typed text stay. Then add a check (`conventions_tests.rs`) refusing `Result<_, String>` and an
  error variant whose only payload is a `reason`, `message` or `detail` string outside tests, and
  record it in `rust-style.md`. The document's feature `Failure` (`reason` and `remedy` text built
  by each feature's `error`) is the same shape and follows once the app is done. Next because
  every new error written against these functions copies the pattern (the MCP sketch tools just
  did).
- [low · medium] Slow tests to keep an eye on: with egui, its glyph stack and the PNG encoder built
  optimised in the dev profile (`dependencies.md`) a UI test takes a few tenths of a second, and
  none of the ten that took over a second (the screen-reader test, the shortcut reset tests, the
  hole, extrusion, chamfer and section panel tests, the PNG export test) takes over 0.55 s. What is
  left is the app's own unoptimised frames and the recompute and meshing of the bodies the tests
  build; re-measure the whole UI suite and list the tests still over half a second.
