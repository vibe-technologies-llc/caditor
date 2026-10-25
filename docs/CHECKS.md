# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · medium] The document generators (`caditor-checks`) never build a measurement that feeds
  a parameter which another parameter and a feature read, so the incremental-against-fresh
  recompute suite does not catch a cache key forgetting a derived parameter (reversing `988545dd`
  survives 6,000 cases). Make `addition` offer the chain (a measurement with a `parameter`, a
  parameter whose expression names it and another parameter, a feature using that one) and keep the
  reversal as the check that the suite fails.
- [high · medium · blocked by: the Autodesk app (`DECISIONS.md`)] The Fusion 360 calls follow
  Autodesk's published documentation and are tested against JSON of the shapes it describes
  (`caditor-cloud`'s `fusion/tests.rs`), never against Autodesk's servers with a registered app.
  With one, run once a sign-in, a team hub with several projects, a nested folder, a conversion
  and a download of a large design, and keep any shape that differs as a test.
