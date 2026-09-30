---
paths:
  - "crates/caditor-document/src/recompute.rs"
  - "crates/caditor-document/src/worker.rs"
  - "crates/caditor-document/src/values.rs"
  - "crates/caditor/src/model.rs"
---

# Recompute

## Evaluation

- `ParameterValues` (`values.rs`) evaluates parameters in dependency order and reports cycles
  rather than following them.
- `Recompute` walks the features in tree order and reuses a cached result when all of these are
  unchanged:
  - the feature's content: an `Arc`, compared by pointer first, then its ID and kind by
    `same_content`, so a sketch's ID counter does not count, and neither does its name;
  - the values of the parameters it uses;
  - its upstream results.
- A failed result is also recomputed when a name its message could hold changed: its own, those of
  the parameters and features it uses, and those of every feature before it, which name the faces
  it works on.
- Upstream results count as unchanged when they are the same `Arc` or:
  - sketches: the same plane and entities;
  - datums: the same result;
  - solids: the same body and an equal solid.
  - An edit that leaves geometry alone (a satisfied constraint, a settle, a feature remaking its
    body as it was) stops there.
- A failing feature is `Failed` with a `FeatureError` (reason, remedy and a `FixTarget`) and keeps
  its last good result. Its dependents fail with a pointer back to it; everything else is
  unaffected.
- A panic inside an `Evaluator` is caught and becomes that feature's error.

## Bodies

- Each body's latest good state is carried through the tree and is part of the next change's
  upstream of every feature that uses the body (`bodies_used`).
- A feature that changes a body gets its current solid through `Inputs::body`. A failing one is
  skipped, so later features of the body build on the state before it.
- `Evaluation::body` gives each body's final solid and `body_result` the shared result holding it.
- A body whose creating feature failed or was not reached keeps the last good state of its latest
  feature as a stale body (`is_stale`), drawn tinted and still exported.
- `body_seen_by` gives the state of a body a given feature used, which is where a revolve's model
  axis is drawn.

## Display data

- Computed on the worker at the end of each run and cached inside the shared results
  (`OnceLock`), so the UI only reads it.
  - Each body's final state is tessellated (`SolidResult::mesh`; intermediate states are not),
    after its bounding box is found (`SolidResult::bounding_box`, which the export dialog's
    deviation reads), by `Solid::display_mesh` at the recompute's `MeshQuality` (`SMOOTH` unless
    set by `Recompute::with_mesh_quality` or `set_mesh_quality`).
  - Every sketch that a solid feature sweeps gets its regions with a triangulation each
    (`SketchResult::regions`).
- A sketch's profile arrangement is built once per result and shared by every feature that sweeps
  it and by the display. Regions are found under the run's cancel token; a build or triangulation
  cancelled midway is not kept, so the next run builds it again.
- The state before an open blend or shell is meshed only when the app asks (`Recomputer::mesh`,
  sent by `Model::mesh_before` for the open feature).
- A panic or failure while meshing leaves the body without a mesh (`mesh_failed`) but keeps its
  shape for later features. A run cancelled before every shown body was meshed is not complete.
- A mesh is kept with its result, so `set_mesh_quality` with a different quality clears the cache:
  the next run rebuilds every result and meshes it at the new quality (new `Arc`s, which the app's
  body meshing keys on). `Recomputer::set_mesh_quality` passes it to the worker, where it applies
  from the next submission and to requested meshes; a panic's cache reset keeps it.
- `SolidResult::names` is a `NameIndex` built on first use and kept with the result: the faces of
  each name in solid order and the first edge of each name, so the app finds a face or edge it
  holds by name without scanning the solid.

## Recomputer worker

- `Recomputer` runs recompute on a worker thread.
- A newer submission or `cancel` stops the running job between features (evaluators also receive a
  `CancelToken`). Features that were not reached are reported as `Outdated`.
- The worker calls a wake callback after each report (UI redraw).
- Each run is contained: a panic outside any evaluator runs it again without the cache, and a
  second panic reports `Outcome::Failed` with the last good evaluation (shown as stopped, with
  Restart), so the worker lives on.
- Requested meshes run after the queued recompute under a token that a newer submission or
  `cancel` trips, and stay queued until they finish.
