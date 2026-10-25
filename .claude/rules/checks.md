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
- `addition` also builds, now and then, the chain a cache key can forget (`derived_chain`): a
  measurement writing a parameter, a parameter adding it to another, and a feature reading that
  one.
- `seeded::for_each_case` feeds `BYTES_PER_CASE` bytes from a SplitMix64 stream per seed into the
  generator and prints the failing seed; `CADITOR_CASES` sets the number of cases of every suite
  (the stress job runs 10,000), the defaults keep the suites to a few seconds each. A property is
  checked after every step, not once at the end, so the failure names the step that broke it.
- Random bytes reach few bodies, so each suite counts what it reached and fails below a floor
  (`FEWEST_BODIES_*`): a generator change that stops producing bodies fails the suite instead of
  letting it pass on empty documents.
- Properties of `document_tests.rs` (after the random steps, a pass doubles the numeric part of up to
  `MOST_PERTURBED` parameters and compares again, since editing a value is what the cache keys must
  follow):
  - an incremental `Recompute` kept across the steps equals a fresh one on every feature's state and,
    for features up to date, result (`same_result`: sketches by solution and, when fully constrained,
    by position within `LARGEST_SHIFT`, since a warm start may settle free degrees of freedom
    differently; solids by body and shape; lazy display caches, reuse counters and id counters are
    not content). A failed feature keeps its last good result, so results of failed features are
    not compared; the words of a failure are compared too
    (`regressions/document/stale-body-name-in-a-failure.bin` is a model where the key forgot a
    renamed body);
  - every body validates, meshes watertight at `MeshQuality::COARSE` and has finite mass properties;
  - undoing every step restores the empty document and redoing them the last one;
  - a document saves and loads to the same content.
- `kernel_tests.rs`: every solid `profile::swept_solid` makes is sound, and for pairs of them placed
  at random every boolean that succeeds is sound and obeys the identities of volume (exact
  quadrature, `mass_properties_tessellating`): union plus intersection is the two volumes,
  intersection plus difference is the first, union commutes. A refusal is not a failure; the
  surveys of success live in `caditor-kernel`. A directed mesh edge must be matched by as many
  opposite ones (`invariants::watertightness`), not exactly one: profile regions touching along an
  edge make a legitimate non-manifold solid.
- `sketch_tests.rs`: solving a sketch twice gives the same geometry, a solved sketch solves again
  from its own result, and the second solve moves no point or radius by more than `LARGEST_SHIFT`
  (a micrometre; centripetal respacing leaves tens of nanometres). Sketches with control points
  within a micrometre of each other are skipped, since a one-point spline has no side or tangent.
  The seeds beyond which these still fail are in `docs/BUGS.md`, so `CADITOR_CASES` stays under
  them.
- `regressions/<suite>/` holds inputs that once broke a fuzz target or a suite, replayed by
  `every_input_kept_in_the_regression_folder_still_holds` through the same checks. A crash libFuzzer
  finds in `document` is copied there (`fuzz/artifacts` is not tracked), next to the fix.
- Evaluation is bounded by a poll count (`POLL_LIMIT`) rather than a clock, so a case never gives
  different answers on a loaded machine; a case that exhausts it is skipped.
- A property that fails on inputs the model should handle is a finding: record it in `docs/BUGS.md`,
  keep it as an ignored test, and leave the passing suites asserting only what holds.

## What the suites were shown to catch

Each past fix was reversed in the source and the suites run; a mutation that survived got a better
property or a pinned input.

- The stale parameter fingerprint of a parameter derived from a measurement (`988545dd`) fails the
  incremental suite at seed 12, the 13th case, once `addition` offers the chain (`derived_chain`: a
  measurement keeping `measured`, `derived = measured + spare`, a datum point offset by `derived`);
  before it, 2,000 and 6,000 random cases passed. `stale-parameter-derived-from-a-measurement.bin`
  replays that case. Seed 395 once looked like a catch; it was a sketch with free degrees of
  freedom solved differently warm and fresh, which is by design, so sketch geometry is compared only
  for fully constrained sketches.
- A failure's words naming a feature that a face reference only mentions (the pattern or maker of
  the face's origin, deleted or renamed since) survived until the key held `dependencies()` names
  rather than `features()` names (seed 95, `stale-origin-name-in-a-failure.bin`).
- A healing kept the whole feature it checked, so a feature hidden since its references were
  healed compared unequal to the fresh pass's (seed 1652, `stale-visibility-in-a-healing.bin`);
  `Healing` now keeps only the feature's id and the kind it checked.
- A failure naming the feature that consumed a body it uses kept that name after the consumer was
  suppressed (seed 9268, `stale-consumer-in-a-failure.bin`), until the key's upstream held every
  feature of the view, the consumers of its bodies included.
- The undo that could not be applied around configurations and moves (`b94cd16b`) fails the undo,
  save, soundness and recompute suites alike, with the generator's `check_inverse` naming the edit.
- The menu bar's gear dropping its command (`49903142`) fails the button sweep and the harness's
  dropped-command guard in `app-tests.md`, with no test written for the gear.
- Layout regressions of a side panel (`73c8123c`, the parameter table's names) are caught only by
  their own tests and `large_interface.rs`: the sweeps lay out windows, not side panels.
