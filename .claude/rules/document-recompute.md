---
paths:
  - "crates/caditor-document/src/recompute.rs"
  - "crates/caditor-document/src/presenting.rs"
  - "crates/caditor-document/src/worker.rs"
  - "crates/caditor-document/src/values.rs"
  - "crates/caditor/src/model.rs"
---

# Recompute

## Evaluation

- `ParameterValues` (`values.rs`) evaluates parameters in dependency order and reports cycles.
- `Recompute` walks the features in tree order and reuses a cached result when the feature's
  content (by `same_content`, so a sketch's ID counter and a name do not count), the values of the
  parameters it uses and its upstream results are unchanged. Upstream results count as unchanged
  when they are the same `Arc` or equal in what dependents read (a sketch's plane and entities, a
  datum's result, a solid's body and geometry), so an edit that leaves geometry alone stops there.
- The cache keeps a short history per feature (`history.rs`, `ResultHistory`): up to
  `RESULTS_KEPT_PER_FEATURE` entries, the one last used first, so undoing a change or switching a
  value back finds the earlier entry and its result `Arc`, and everything below reuses its own
  earlier entries in turn. Earlier entries (all but the latest of each feature) are held within
  `EARLIER_RESULTS_BUDGET`, estimated from `Solid::approximate_size` with room for its mesh; the
  least recently used go first, and the latest entry of a feature is never dropped for size.
- A failed result is also recomputed when a name its message could hold changed (its own, those of
  the parameters and features it uses, those of every feature before it, which name the faces it
  works on); which used features are suppressed is part of its key too.
- A failing feature is `Failed` with a `FeatureError` (reason, remedy, `FixTarget`, and a model
  point `place` where the failure arose when the kernel gives one) and keeps its last good result.
  `Failure::Error` boxes it, keeping every `Result` small. Dependents fail with a pointer back to it; everything else is unaffected. A
  panic inside an `Evaluator` is caught and becomes that feature's error.
- Features below the rollback bar are `RolledBack` and suppressed ones `Suppressed`, never
  evaluated, with cache entries kept so rolling forward or unsuppressing reuses them. A feature
  using a suppressed one fails naming it (`FixTarget::Unsuppress`); one using a feature that no
  longer exists fails saying so.
- A feature that computes is then checked for healed references (`healing.rs`, panics caught): a
  reference that resolved to topology of another name (fragments of a split face or edge keep its
  name and do not count) or a chosen region `resolve_regions` healed or left out.
  `FeatureStatus::healing` then holds a `Healing`, cached with the result and, like a failure's
  message, recomputed when the names it could hold change. `Healing::update` is the undoable
  "Update references", refused once the feature differs from the one checked so an evaluation
  lagging an edit never reverts it.

## Bodies

- Each body's latest good state is carried through the tree and is part of the upstream of every
  feature using it (`bodies_used`). A feature changing a body gets its current solid through
  `Inputs::body`; a failing one is skipped, so later features build on the state before it.
- A body whose creating feature failed or was not reached keeps its last good state as a stale body
  (`is_stale`), drawn tinted and still exported; one whose creating feature is suppressed or rolled
  back has no state, so the model shows as of the bar.
- `body_seen_by` gives the state a given feature used, where a revolve's model axis is drawn.

## Display data

- Computed on the worker and cached inside the shared results (`OnceLock`), so the UI only reads
  it: each body's final state is meshed at the recompute's `MeshQuality` (intermediate states are
  not), and every sketch a solid feature sweeps, or of at most `MAX_UNSWEPT_REGION_ENTITIES`
  entities, gets its regions with a triangulation each (the app tints them while it is edited).
- A body settles once the walk passes the last active feature that changes or consumes it
  (`settling`, from `Feature::body` and `consumed_bodies`). Settled bodies are meshed during the
  feature loop by a display thread scoped to the run (`presenting.rs`), beside the features still
  computing; the rest are meshed after the loop. A run without display data
  (`run_without_display`) starts no thread.
- A sketch's profile arrangement is built once per result, shared by every feature sweeping it and
  the display, under the run's cancel token (a build cancelled midway is not kept). A sketch that
  solves again to the same geometry (`Sketch::same_geometry`) shares the last result's arrangement
  and regions, so display data already found is not rebuilt.
- A sketch result's regions can also be asked for (`Recomputer::regions`, queued with the
  requested meshes), which the app does for the edited sketch past `MAX_UNSWEPT_REGION_ENTITIES`.
- The state before an open blend or shell, and the tools of an open cut (`SolidResult::cuts`: the
  swept tool of an extrusion or revolve removing from a body, each drill of a hole, the placed
  copies of a pattern's repeated cuts, kept with the result), are meshed only when the app asks
  (`Recomputer::mesh`). An addition keeps its swept tool as `SolidResult::joins`, which only a
  pattern repeating it reads.
- A panic or failure while meshing leaves the body without a mesh (`mesh_failed`) but keeps its
  shape for later features. A run cancelled before every shown body was meshed is not complete.
- A mesh is kept with its result, so a different `set_mesh_quality` clears the cache and the next
  run rebuilds every result as new `Arc`s (which the app's body meshing keys on). The worker applies
  it from the next submission; a panic's cache reset keeps it.
- `SolidResult::names` is a `NameIndex` built on first use, so the app finds a face, edge or vertex
  by name without scanning the solid.

## Recomputer worker (`worker.rs`)

- `Recomputer` runs recompute on a worker thread and calls a wake callback after each report. A
  newer submission or `cancel` stops the running job between features (evaluators also get a
  `CancelToken`); features not reached are `Outdated`.
- A run reports as it goes once it has taken `FEATURES_DONE_AFTER` (`Outcome::FeaturesDone`, an
  evaluation that is never `is_complete`), so a slow feature or mesh does not hide what came before
  it. Before each feature it evaluates, the walk hands the display thread a glimpse of the
  evaluation so far, when a feature was computed or a body settled since the last one; the thread
  reports the latest glimpse once the time is up, even while that feature is still running, and
  again after each body it meshes. After the loop the run reports once more before the regions
  and meshes, and after each mesh. A run that recomputes nothing before a slow feature reports
  nothing early, since nothing shown would change.
- In a glimpse the features not reached yet are pending (`Evaluation::is_pending`) and keep the
  status and result of their last computation (none if never computed), so the bodies they make or
  change show as they last were rather than stale or missing; the tree shows them waiting, and a
  dragged sketch stays shown until the evaluation is no longer pending at it.
- An early report does not end the submission in `Recomputer`, so a worker replaced after one still
  reports the cancel, with the pending features `Outdated` (`outdating_pending`).
- Each run is contained: a panic outside any evaluator runs it again without the cache, and a second
  reports `Outcome::Failed` with the last good evaluation, so the worker lives on.
- `cancel` cancels only the work running or queued when it is called: each job records the cancel
  count at submission, so a `cancel` after a job finished (or while idle) trips nothing later.
  Requested meshes run after the queued recompute, newest first (asking again moves a result to the
  front), so the state on screen is meshed before stale ones.
- A draft (`Recomputer::submit_draft`, a document with a change the user has not committed, under
  a serial of the app's) runs on the same worker against a copy of the cache
  (`Recompute::draft_copy`, cheap since entries hold `Arc`s), so upstream results are reused while
  the cache, the last reported evaluation and the model's reports are never touched. Its finished,
  complete evaluation is kept apart (`Recomputer::take_draft`); an unfinished or cancelled one is
  dropped. A draft never pre-empts the model: submitted while a model submission is unanswered it
  waits until that is reported, and any model submission drops a waiting draft and stops a running
  one.
- `Progress` carries the feature running and when it began; the status bar shows it, and the log
  line of a replaced worker names it.
- A worker that does not stop is replaced: once a `cancel` or newer submission has gone unanswered
  for `STOP_GRACE`, `poll` or the next submission starts a fresh worker (same evaluator, wake and
  mesh quality, empty cache) and leaves the old one to finish on its own. The newest submission is
  then reported as `Outcome::Cancelled` (when a cancel followed it) or sent again to the new worker.
- `Recomputer::forget`, called by `Model::switch_to` before a new document's first submission,
  drops the last reported evaluation on both sides along with the worker's cache and queued
  meshes, since feature ids restart per document: a cancel answered by a replaced worker, or a run
  that panics twice, then reports nothing of the previous model under the new ids.
- `Recomputer::submit_retrying_failures` (the Recompute command, F5) reruns every failed feature
  though its key is unchanged, which a plain submission reuses, as it does a caught panic's error;
  a retrying submission superseded by a plain one still retries.
