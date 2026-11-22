# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

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
- [low · small] The ignored `screenshots` test still stops at the 200% looks (`dark-200`,
  `light-200`), now in `principal_scenes`: `click_beside` for the Principal planes group title finds
  no text, which the feature tree has scrolled or wrapped out of view at that scale, so the scenes
  after the principal ones (`move_scene`, the reference picture, body group and drawings scenes)
  render only at the other looks. Bring the row into view before the click.
