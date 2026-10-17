# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [high · easy] `cargo deny check advisories` fails: RUSTSEC-2026-0370, 0371 and 0372 mark
  `zvariant`, `zbus_names` and `zvariant_derive` unmaintained (merged into `zbus` 6), and they come
  in only through `egui-winit` → `accesskit_winit` → `accesskit_unix` 0.24 → `atspi` → `zbus` 5,
  with no newer `accesskit_unix` published. Ignore them in `deny.toml` with that reason, as
  `ttf-parser` is, and drop the ignores once `accesskit_unix` moves to `zbus` 6.
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
