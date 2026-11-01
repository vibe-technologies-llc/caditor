# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · easy] Fuzz jobs (`ci.yml`, `fuzz`) overrun their 20-minute limit when the fuzz
  build cache misses ("Cache not found"): in master runs 41298079767, 41258481550 and 41253555198
  journal_torn, journal, mesh, preferences, expression, save_load and model_sealed were cancelled,
  a cold `cargo fuzz` build plus the fuzzing time exceeding the limit. The cache key holds the
  target, so every target builds cold after a lock change; share one build across targets, or
  shorten the fuzzing time when the build ran cold.
- [high · medium · blocked by: the Autodesk app (`DECISIONS.md`)] The Fusion 360 calls follow
  Autodesk's published documentation and are tested against JSON of the shapes it describes
  (`caditor-cloud`'s `fusion/tests.rs`), never against Autodesk's servers with a registered app.
  With one, run once a sign-in, a team hub with several projects, a nested folder, a conversion
  and a download of a large design, and keep any shape that differs as a test.
