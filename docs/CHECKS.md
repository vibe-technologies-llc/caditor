# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · small] Two `caditor-file` tests fail on every Windows run
  (`journal_during_saves::a_crash_flush_during_a_save_writes_the_changes_made_while_it_runs` and
  `a_save_that_panics_fails_alone_and_the_journal_and_later_saves_carry_on`): they `fs::copy` the
  recovery journal while the storage worker holds its byte-range lock, which Windows refuses (OS
  error 33, "another process has locked a portion of the file"). Copy it through the reader the
  recovery path uses, or release the gate first, so the Windows job can pass.
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
