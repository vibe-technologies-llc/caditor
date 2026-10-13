---
paths:
  - "crates/caditor-document/src/recompute.rs"
  - "crates/caditor-document/src/history.rs"
  - "crates/caditor-document/src/presenting.rs"
  - "crates/caditor-document/src/pool.rs"
  - "crates/caditor-document/src/lookahead.rs"
  - "crates/caditor-document/src/worker.rs"
  - "crates/caditor-document/src/values.rs"
  - "crates/caditor/src/model.rs"
---

# Recompute

## Evaluation

- `ParameterValues` (`values.rs`) evaluates parameters in dependency order and reports cycles. A
  measured parameter (`document.md`, Measurement) and every parameter reading one is `Unmeasured`
  there; the walk overlays the measurements' results into the parameters of each feature using one
  and re-derives dependent parameters (`Measured::overlay`, `derive`, `compute`). `into_evaluation`
  does the same for every up-to-date reading, and `take_readings` carries them into the app's
  values and drafts. A feature's used features (`Measured::features_read_by`: its `features()` plus
  the measurements its parameters read) make its key's upstream, its view and its lookahead
  dependencies, so a changed reading recomputes its users and a failed or suppressed measurement
  fails them through `missing_upstream`.
- `Recompute` walks the features in tree order and reuses a cached result when the feature's content
  (`same_content` of the kind, and the feature's `pieces`: a sketch's ID counter and a name do not
  count), the values of the parameters it
  uses (`Key::of` fingerprints the overlaid parameters) and its upstream results are unchanged.
  Upstream counts as unchanged when it is the same `Arc` or equal in what dependents read (a
  sketch's plane and entities, a datum's result, a solid's body and geometry), so an edit leaving
  geometry alone stops there.
- Features evaluate in parallel while the walk commits them in tree order, so statuses, cache
  entries and their order, `recomputed`, glimpses and progress are those of a sequential walk. An
  evaluator sees only what it uses (`View`): the results of `features()` and the states of
  `bodies_used` at its place, plus the features that consumed those bodies (`Inputs::missing_body`
  names them); reading anything else would break the cache keys.
  - From the first feature the walk must compute, `Lookahead` (`lookahead.rs`) follows the
    dependency graph: a feature depends on the features it uses and on the last earlier feature
    changing or consuming each body it uses or changes (`Feature::bodies`, `consumed_bodies`). Once
    those have a provisional outcome it builds the view and key, takes a matching cache entry
    without touching recency (`ResultHistory::peek`), and otherwise queues the evaluation on the
    pool.
  - The pool (`pool.rs`) is a `rayon_core::ThreadPool` (`Threads`) a `Recompute` builds on its first
    parallel run (available parallelism) and keeps for its life; `draft_copy` shares it and it ends
    with the last `Recompute` holding it. Each run scopes its queue on it (`in_place_scope`, no
    `unsafe`) and starts serving tasks only as queued work outgrows idle ones; the walk stays on the
    run's own thread. A pool that cannot start, or `with_workers(0)` in tests, is the sequential
    walk. Evaluations run lowest tree index first, before any mesh. The walk takes an outcome only
    when the view it ran on is the walk's own (`View::same_as`, by `Arc`); otherwise (another view,
    a panic on the pool outside the evaluator, no thread started) it evaluates inline. A cancel
    halts the lookahead, queued evaluations end cancelled unrun, uncommitted features are
    `Outdated`, and whatever the lookahead ran past the cancel is dropped.
- `ResultHistory` (`history.rs`) keeps up to `RESULTS_KEPT_PER_FEATURE` entries per feature, the one
  last used first, so undoing a change or switching a value back finds the earlier entry and its
  result `Arc`, and everything below reuses its own earlier entries in turn. Earlier entries (all
  but each feature's latest) are held within `EARLIER_RESULTS_BUDGET`. An entry is measured as it
  stops being its feature's latest (by then meshed if it ever will be): each result `Arc` (its own,
  and the others, cuts and joins inside it) by `Solid::owned_size` and `DisplayMesh::approximate_size`
  (a never-meshed intermediate state counts its solid alone), plus every kernel buffer its solids
  share (`Solid::shared_buffers`, `kernel.md`). `Holdings` counts each buffer address once across
  earlier entries (a holder count per address; the entry keeps the addresses it took), so
  successive states of a body sharing untouched geometry weigh what they hold together. A buffer
  leaves the total when its last earlier holder goes, so an address never stands for two buffers.
  A running total means an insert looks at earlier entries only when over budget; least recently
  used go first, and a feature's latest entry is never dropped for size.
- A measurement of a whole body also keys on that body's appearance (`Key::appearances`, the
  `BodyAppearance` of each `Measurement::appearance_bodies`) and its density's parameters join the
  parameter fingerprint, since the density and the body's name lie outside the features and
  parameters it uses. Mass properties are cached in the result (`SolidResult::exact_mass`,
  `document.md`), so such a recompute integrates nothing again.
- A failed result is also recomputed when a name its message could hold changed (its own, those of
  the parameters and features it uses, those of every feature before it); which used features are
  suppressed is part of its key too.
- A failing feature is `Failed` with a `FeatureError` (reason, remedy, `FixTarget`, and a model
  point `place` where the kernel gives one) and keeps its last good result. `Failure::Error` boxes
  it, keeping every `Result` small. Dependents fail with a pointer back to it; everything else is
  unaffected. A panic inside an `Evaluator` is caught and becomes that feature's error.
- A feature missing an input it is still being set up for is `Waiting` with a `MissingInput` (an
  enum, one variant per missing input carrying what it names: `Axis` of a circular pattern, `Path`
  of a sweep, `Profiles` of a loft with fewer than two profiles (or one and no end point),
  `Regions { sketch }` of an extrusion, revolve or sweep with none chosen, `Edges { body
  }` of a blend, `FacesToMove` of an offset face, `FacesToDelete` of a delete face, `FacesToSplit` of a split face), returned as
  `Failure::Waiting`. `waiting_words` ("Waiting for an axis") and `prompt` (what to choose, naming
  sketch or body from the document) word it where shown, so its cache entry holds no names. It is
  not a failure (`failures`, `failed_count` and F8 leave it out) but otherwise acts as one: it keeps
  its last good result, its body's later features build on the state before it, and dependents
  fail through `missing_upstream` as for any feature without a result.
- Features below the rollback bar are `RolledBack` and suppressed ones `Suppressed`, never
  evaluated, with cache entries kept so rolling forward or unsuppressing reuses them. A feature
  using a suppressed one fails naming it (`FixTarget::Unsuppress`); one using a feature that no
  longer exists fails saying so.
- A computed feature is then checked for healed references (`healing.rs`, panics caught): a
  reference that resolved to topology of another name (fragments of a split face or edge keep the
  name and do not count) or a chosen region `resolve_regions` healed or left out.
  `FeatureStatus::healing` holds a `Healing`, cached with the result and recomputed, like a
  failure's message, when the names it could hold change. `Healing::update` is the undoable "Update
  references", refused once the feature differs from the one checked so a lagging evaluation never
  reverts an edit.

## Bodies

- Each body's latest good state is carried through the tree and is part of the upstream of every
  feature using it (`bodies_used`). A feature changing a body gets its current solid through
  `Inputs::body`; a failing one is skipped, so later features build on the state before it.
- A body whose creating feature failed or was not reached keeps its last good state as a stale body
  (`is_stale`), drawn tinted and still exported; one whose creating feature is suppressed or rolled
  back has no state, so the model shows as of the bar.
- `body_seen_by` gives the state a given feature used, where a revolve's model axis is drawn.
- A feature making a body of each piece stands each piece under its piece's body ID
  (`document.md`, Separate pieces); a piece whose feature failed stays a stale body by the same
  rule, judged by its making feature.

## Display data

- Computed on the worker and cached in the shared results (`OnceLock`), so the UI only reads it:
  each body's final state is meshed at the recompute's `MeshQuality` (intermediate states are not),
  and every sketch a solid feature sweeps, or of at most `MAX_UNSWEPT_REGION_ENTITIES` entities,
  gets its regions with a triangulation each (the app tints them while edited).
- A body settles once the walk passes the last active feature changing or consuming it (`settling`,
  from `Feature::body` and `consumed_bodies`). The walk hands settled bodies to a display thread
  scoped to the run (`presenting.rs`), which queues their meshes on the pool beside the features
  still computing; after the loop every shown body not yet meshed is queued and the walk's thread
  helps mesh them. A body queued twice is meshed once (`Arc` identity). A run without display data
  (`run_without_display`) meshes nothing and starts no display thread.
- A body's new state is meshed from the state last meshed for it (`LastMeshes` in `pool.rs`, through
  `Solid::display_mesh_reusing`), so only changed faces are triangulated again
  (`kernel-tessellation.md`). `Recompute` holds one such result per body, shared with draft copies,
  cleared with the cache and dropped with the body's feature; requested meshes neither use nor
  replace it.
- A sketch's profile arrangement is built once per result, shared by every feature sweeping it and
  the display, under the run's cancel token (a build cancelled midway is not kept). A sketch that
  solves again to the same geometry (`Sketch::same_geometry`) shares the last result's arrangement
  and regions.
- A sketch result's regions can be asked for (`Recomputer::regions`, queued with requested meshes);
  the app does so for the edited sketch past `MAX_UNSWEPT_REGION_ENTITIES`.
- The state before an open blend or shell, and the tools of an open cut (`SolidResult::cuts`: the
  swept tool of an extrusion or revolve removing from a body, each drill of a hole, the placed
  copies of a pattern's repeated cuts or a mirror's reflected ones), are meshed only when the app
  asks (`Recomputer::mesh`). An addition keeps its swept tool as `SolidResult::joins`, which only a
  pattern or mirror of features reads.
- A panic or failure while meshing leaves the body without a mesh (`mesh_failed`) but keeps its
  shape for later features. A run cancelled before every shown body was meshed is not complete.
- A mesh is kept with its result, so `set_mesh_quality` clears the cache and the next run rebuilds
  every result as new `Arc`s (which the app's body meshing keys on). The worker applies it from the
  next submission; a panic's cache reset keeps it.
- `SolidResult::names` is a `NameIndex` built on first use, so the app finds a face, edge or vertex
  by name without scanning the solid.

## Recomputer worker (`worker.rs`)

- `Recomputer` runs recompute on a worker thread and calls a wake callback after each report. A
  newer submission or `cancel` stops the running job between features (evaluators also get a
  `CancelToken`); features not reached are `Outdated`. `cancel` affects only work running or queued
  when called: each job records the cancel count at submission. Requested meshes run after the
  queued recompute, newest first (asking again moves a result to the front).
- Once a run has taken `FEATURES_DONE_AFTER` it reports as it goes (`Outcome::FeaturesDone`, never
  `is_complete`). Before each feature it evaluates, the walk hands the display thread a glimpse of
  the evaluation so far, when a feature was computed or a body settled since the last; building one
  is O(features), so the walk spends at most `GLIMPSE_ALLOWANCE` or a `GLIMPSE_SHARE_OF_RUN`th of the
  run on them (`GlimpseBudget`), and past that a glimpse is only the settled bodies, sent for meshing
  without an evaluation (`Glimpse::evaluation` is `None`) and without replacing the latest; the thread
  reports the latest glimpse once the time is up, even while a feature is still running, and again
  after bodies are meshed for it. After the loop the run reports before the regions and meshes, and
  as meshes finish; a report that only adds meshes waits until `MESHES_REPORTED_EVERY` has passed
  since the last. A run that recomputes nothing before a slow feature reports nothing early.
- In a glimpse the features not reached are pending (`Evaluation::is_pending`) and keep the status
  and result of their last computation (none if never computed), so their bodies show as they last
  were; the tree shows them waiting, and a dragged sketch stays shown until the evaluation is no
  longer pending at it. An early report does not end the submission, so a worker replaced after one
  still reports the cancel with pending features `Outdated` (`outdating_pending`).
- Each run is contained: a panic outside any evaluator runs it again without the cache, and a
  second reports `Outcome::Failed` with the last good evaluation, so the worker lives on.
- A draft (`Recomputer::submit_draft`: a document with an uncommitted change, under a serial of the
  app's) runs on the same worker against a copy of the cache (`Recompute::draft_copy`, sharing the
  pool and every entry as `Arc`s; `retry_failures` copies the one it marks), so upstream results are
  reused while the cache, the last reported evaluation and the model's reports are never touched.
  Only a finished, complete draft is kept (`Recomputer::take_draft`). A draft never pre-empts the
  model: submitted while a model submission is unanswered it waits for that report, and any model
  submission drops a waiting draft and stops a running one.
- A task on the cache (`Recomputer::submit_on_cache`, a `CacheTask`: the app's search for the
  picks of pasted features) runs on the same worker after the queued draft and model submission,
  on a draft copy of the cache, so it evaluates only what the model's results do not cover. Its
  `CancelToken` turns once a later model submission is sent (`Shared::models`, which drafts do not
  count) or the worker is replaced, so a task never delays the model's recompute; the caller plans
  again. A panic in a task is caught and logged and leaves the cache alone. A task still queued on
  a replaced worker runs only when that worker comes free, cancelled.
- `Progress` carries the feature running and when it began; the status bar shows it and the log line
  of a replaced worker names it.
- A worker that does not stop is replaced: once a `cancel` or newer submission has gone unanswered
  for `STOP_GRACE`, `poll` or the next submission starts a fresh worker (same evaluator, wake and
  mesh quality, empty cache) and leaves the old one to finish alone. The newest submission is then
  reported as `Outcome::Cancelled` (when a cancel followed it) or sent again to the new worker.
- `Recomputer::forget`, called by `Model::switch_to` before a new document's first submission,
  drops the last reported evaluation on both sides with the worker's cache and queued meshes, since
  feature ids restart per document.
- `Recomputer::submit_retrying_failures` (the Recompute command, F5) reruns every failed feature
  though its key is unchanged, which a plain submission reuses (a caught panic's error too); a
  retrying submission superseded by a plain one still retries.
