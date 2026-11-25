# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · easy] The UI harness's `click_button` takes the first of several same-named buttons
  in AccessKit's node order, which follows egui's `Id` hashes, so an `ahash` update can move a
  test onto another button (ahash 0.8.14 did so for three). Tests still clicking a name two
  enabled buttons carry: `constraint_remedies` (both "Delete Horizontal Line 2" remedies),
  `notices` (the notice's "Undo" and the toolbar's), `holes` ("Stop measuring hole 1 from this
  axis/edge"), `model_properties_and_views` and `selection_sets` ("Rename it", "Delete it"),
  `sketch_modify_status` (the status pills) and the `button_sweep` quarters. Name the buttons
  apart where a screen reader user could not tell them apart either, have the tests say which
  where they are the same action twice, then make the lookup refuse a name two enabled buttons
  carry.
- [medium · medium] Fuzzing misses the shapes that break the readers: the crafted STEP, 3MF and
  SVG files in `BUGS.md` need depth and fan-out that no committed seed has, so they need seeds
  built by code and bounded by `tally`; no target edits sketches (trim, split, break, offset,
  fillet) checking use counts against `counted_from_scratch`; and `expression` only round-trips
  stored text, not display text through `parse`, evaluation or `with_lengths_scaled`.
- [high · medium · blocked by: the Autodesk app (`DECISIONS.md`)] The Fusion 360 calls follow
  Autodesk's published documentation and are tested against JSON of the shapes it describes
  (`caditor-cloud`'s `fusion/tests.rs`), never against Autodesk's servers with a registered app.
  With one, run once a sign-in, a team hub with several projects, a nested folder, a conversion
  and a download of a large design, and keep any shape that differs as a test.
