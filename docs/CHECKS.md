# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [high · medium · blocked by: the Autodesk app (`DECISIONS.md`)] The Fusion 360 calls follow
  Autodesk's published documentation and are tested against JSON of the shapes it describes
  (`caditor-cloud`'s `fusion/tests.rs`), never against Autodesk's servers with a registered app.
  With one, run once a sign-in, a team hub with several projects, a nested folder, a conversion
  and a download of a large design, and keep any shape that differs as a test.
- [medium · medium] The `sketch_edit` fuzz job timed out in CI at f3346e24 (libFuzzer's 10 s, ASan
  build) on a 242-byte input. On master with the solver's turning-metric step it still runs 3.6 s
  unsanitised in release, near the CI limit under ASan. Find which edit's solve takes the time,
  bound it by `solve/tally.rs` work, and add the input to the target's seeds once it is quick. The
  input, base64:
  `KjoIbThBrCYaSejbhab3etAENzFfxGMadPKyFSRrhWDnkh8KPjJ7kbfV8HCdV3alpaWlpaWlpaWlpUGsJhpJ6NuFpvd60AQ3MV/EYxp08rIVJGuFYOeSHwo+MnuRt9XwcJ1XdqWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaUEgkvNF2UReYityiMC+yrk9NIs5AMxpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpQSCS80XZRF5iK3KIwL7KuT00izkAzGlpaWlpaWlpaWlP/4QuaU=`
