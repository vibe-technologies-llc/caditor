# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · medium] Fuzzing misses the shapes that break the readers: the crafted STEP, 3MF and
  SVG files in `BUGS.md` need depth and fan-out that no committed seed has, so they need seeds
  built by code and bounded by `tally`.
- [low · small] `ui_tests::assistant::api_assistant_calls_live_mcp_tools_updates_model_and_lands_in_undo_history`
  failed once on the Linux runner (CI run 45621358670) with "key save did not arrive": the test
  waits five seconds of wall time for the background key save, which a loaded runner can exceed;
  it passed three times locally. Wait on the save's own completion (or the harness's settle)
  rather than a fixed deadline.
- [high · medium · blocked by: the Autodesk app (`DECISIONS.md`)] The Fusion 360 calls follow
  Autodesk's published documentation and are tested against JSON of the shapes it describes
  (`caditor-cloud`'s `fusion/tests.rs`), never against Autodesk's servers with a registered app.
  With one, run once a sign-in, a team hub with several projects, a nested folder, a conversion
  and a download of a large design, and keep any shape that differs as a test.
