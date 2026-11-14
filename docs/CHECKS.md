# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [high · medium · blocked by: the Autodesk app (`DECISIONS.md`)] The Fusion 360 calls follow
  Autodesk's published documentation and are tested against JSON of the shapes it describes
  (`caditor-cloud`'s `fusion/tests.rs`), never against Autodesk's servers with a registered app.
  With one, run once a sign-in, a team hub with several projects, a nested folder, a conversion
  and a download of a large design, and keep any shape that differs as a test.
- [low · small] The ignored `screenshots` test stops at the 200% looks (`dark-200`, `light-200`):
  the cloud scenes wait for Home's Sign in button (`ui_tests/home.rs`, `sign_in`), which the page
  has scrolled out of view at that scale, so those looks render only the scenes before them. Bring
  the button into view before waiting for it, or scroll the page in the scene.
