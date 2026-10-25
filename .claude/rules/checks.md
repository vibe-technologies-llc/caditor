---
paths:
  - "crates/caditor-checks/**"
---

# Checks

`caditor-checks` holds what `cargo test` and the fuzz targets share: the generators that turn bytes into
documents, sketches and profiles (`document/`, `sketch.rs`, `profile.rs`, `arbitrary::Unstructured`,
biased to near-coincident and aligned geometry), the invariant checks (`invariants.rs`, `check_solid`)
and the seeded property suites (`*_tests.rs`). `fuzz/` re-exports it, so a generator is written once
and a new `Edit` or `FeatureKind` fails to compile in one place (`ci.md`).

- Its lint list is the workspace's without `panic`, since an assertion that fails is its job
  (`every_member_inherits_the_workspace_lints` names the exception); indexing, `unwrap` and the rest
  stay denied.
- `seeded::for_each_case` feeds `BYTES_PER_CASE` bytes from a SplitMix64 stream per seed into the
  generator and prints the failing seed; `CADITOR_CASES` sets the number of cases of every suite
  (the stress job runs 20,000), the defaults keep the suites to a few seconds each. A property is
  checked after every step, not once at the end, so the failure names the step that broke it.
- Random bytes reach few bodies, so each suite counts what it reached and fails below a floor
  (`FEWEST_BODIES_*`): a generator change that stops producing bodies fails the suite instead of
  letting it pass on empty documents.
- Properties of `document_tests.rs`:
  - an incremental `Recompute` kept across the steps equals a fresh one on every feature's state and,
    for features up to date, result (`same_result`: sketches by `same_geometry` and solution, solids
    by body and shape; lazy display caches, reuse counters and id counters are not content). A failed
    feature keeps its last good result, so results of failed features are not compared. The words of
    a failure are compared only by the ignored `failures_name_things_as_they_are_named_now`, which
    pins a known staleness (`docs/BUGS.md`);
  - every body validates, meshes watertight at `MeshQuality::COARSE` and has finite mass properties;
  - undoing every step restores the empty document and redoing them the last one;
  - a document saves and loads to the same content.
- Evaluation is bounded by a poll count (`POLL_LIMIT`) rather than a clock, so a case never gives
  different answers on a loaded machine; a case that exhausts it is skipped.
- A property that fails on inputs the model should handle is a finding: record it in `docs/BUGS.md`,
  keep it as an ignored test, and leave the passing suites asserting only what holds.
