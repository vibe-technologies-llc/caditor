# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · small] The fuzz jobs run into their 20 minute limit (`ci.yml`): `journal` and
  `journal_torn` on one run of 2026-11-14 and `svg` on the next were cancelled at 20m, a failed
  run though no input failed.
- [medium · medium] Fuzzing misses whole readers and the shapes that break them: no target reads
  parameter files (`parse_parameters`), configuration files (`parse_configurations`) or
  environment pictures (`decode_environment`); `ci.yml` runs libFuzzer with no `-max_len`, so
  inputs stay near 4 KiB and never reach the depth and fan-out of the crafted STEP, 3MF and SVG
  files in `BUGS.md` (those need seeds built by code, bounded by `tally`); no target edits sketches
  (trim, split, break, offset, fillet) checking use counts against `counted_from_scratch`; and
  `expression` only round-trips stored text, not display text through `parse`, evaluation or
  `with_lengths_scaled`.
- [high · medium · blocked by: the Autodesk app (`DECISIONS.md`)] The Fusion 360 calls follow
  Autodesk's published documentation and are tested against JSON of the shapes it describes
  (`caditor-cloud`'s `fusion/tests.rs`), never against Autodesk's servers with a registered app.
  With one, run once a sign-in, a team hub with several projects, a nested folder, a conversion
  and a download of a large design, and keep any shape that differs as a test.
- [low · small] The ignored `screenshots` test stops at the 200% looks (`dark-200`, `light-200`):
  the cloud scenes wait for Home's Sign in button (`ui_tests/home.rs`, `sign_in`), which the page
  has scrolled out of view at that scale, so those looks render only the scenes before them. Bring
  the button into view before waiting for it, or scroll the page in the scene.
