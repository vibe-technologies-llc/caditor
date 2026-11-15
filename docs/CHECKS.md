# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [high · small] CI's `check` job fails at "Run the offscreen render tests on the OpenGL backend":
  `a_lost_device_is_reported_and_a_new_one_draws` and
  `every_offered_anti_aliasing_level_smooths_edges_and_keeps_front_geometry_and_picks_exact`
  (`caditor-render`'s `offscreen_tests.rs`) panic with "the pick should be read back" there, on
  the runs of 2026-11-14 from "Compile less for tests…" on, while the Vulkan run of the same tests
  in the Test step passes. The steps after it (the clean-checkout check, Lint, cargo-deny) are
  skipped with it, so a push is linted only on the machine that made it. The cause is not looked
  into yet; a lead (unconfirmed): the pick readback thread waits with `PollType::Wait`, which on
  GL holds the context (`PERFORMANCE.md`, Interface), while the tests poll on their own thread.
- [medium · small] The fuzz jobs run into their 20 minute limit (`ci.yml`): `journal` and
  `journal_torn` on one run of 2026-11-14 and `svg` on the next were cancelled at 20m, a failed
  run though no input failed.
- [medium · small] `long_parameter_chains_evaluate_and_restore_in_linear_time`
  (`caditor-document`'s `lib.rs`) bounds cost by a ratio of wall-clock times, against
  `rust-style.md`'s rule that tests bound cost by work done, and passes `conventions_tests.rs`'s
  `no_assertion_reads_the_clock` only because `elapsed` is read outside the assert; it also leaves
  out measurements and features, the inputs that make loading quadratic (`PERFORMANCE.md`,
  Document). Count work with a `tally` instead, over inserts of features reading measurements, and
  have the conventions check refuse a `Duration` compared in a test.
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
