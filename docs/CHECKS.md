# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [high · medium] Two tests of a combine failing at a named place have lost their failing case,
  since snapping now combines what they built: `caditor-document`'s
  `combine_tests::a_combine_failing_where_faces_nearly_touch_names_them_and_where` (red since the
  face snapping) and the UI tests using `combine_nearly_touching_blocks` (a plug at (10, 5.0000015),
  now inside the sideways snap's reach). The one remaining case near the reach, a plug 2.5e-6 to
  3e-5 off its bore's axis away from its seams (`BUGS.md`), fails as `Invalid(LoopOrientation)`
  with no place; giving that failure a place and moving both tests onto it would restore them.

- [low · medium] Slow tests to keep an eye on: with egui, its glyph stack and the PNG encoder built
  optimised in the dev profile (`dependencies.md`) a UI test takes a few tenths of a second, and
  none of the ten that took over a second (the screen-reader test, the shortcut reset tests, the
  hole, extrusion, chamfer and section panel tests, the PNG export test) takes over 0.55 s. What is
  left is the app's own unoptimised frames and the recompute and meshing of the bodies the tests
  build; re-measure the whole UI suite and list the tests still over half a second.
- [high · medium · blocked by: the Autodesk app (`DECISIONS.md`)] The Fusion 360 calls follow
  Autodesk's published documentation and are tested against JSON of the shapes it describes
  (`caditor-cloud`'s `fusion/tests.rs`), never against Autodesk's servers with a registered app.
  With one, run once a sign-in, a team hub with several projects, a nested folder, a conversion
  and a download of a large design, and keep any shape that differs as a test.
