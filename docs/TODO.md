# caditor roadmap

## Direction

caditor is a parametric CAD application for Linux. The project is judged on two things before
anything else:

1. **User experience.** Modelling should feel direct and predictable, without the failure modes
   that FreeCAD is known for: broken references after an upstream edit, workbench and mode
   juggling, opaque errors and a UI that freezes. The concrete requirements are in
   `.claude/rules/ux.md`.
2. **Never losing work.** Crashes and data loss are catastrophic failures. The concrete
   requirements are in `.claude/rules/reliability.md`.

Features are added only once they meet both bars; an unpolished feature is not shipped. An item
is implemented when it works end to end in the app, not when the APIs exist. Implemented items
and resolved decisions are removed from this file, and a milestone disappears once it is empty.
Git history is the record of what was done.

Categories below are ordered so that earlier ones unblock or protect later ones, and entries
within a category run from most to least important.

## Data safety

- Release builds set `panic = "abort"` (root `Cargo.toml`), so the panic containment around
  evaluators, meshing and export (`recompute.rs` `evaluate_contained`, `export/mod.rs`) does
  nothing in shipped binaries and one bad feature ends the process. Unwind in release, and add a
  check that fails if the release profile aborts.
- The recovery scan deletes a journal as unchanged when none of its entries could be read
  (`recovery.rs`: `replayed.entries.is_empty()`), so a journal whose first entry comes from a
  newer version or is damaged is destroyed. Delete only when nothing was left unread.
- `storage.rs` `Worker::start` removes the journal it replaces even when creating the new one
  failed (`None != Some(replaced)`), so restoring on a full disk deletes the only copy of the
  recovered work. Remove the old journal only after the new one is written.
- Save As checks neither the journal lock nor an existing target: `with_extension` silently adds
  `.caditor` and can replace another model, and saving onto a file open in another window renames
  a new journal over that window's locked one (`files.rs`, `storage.rs` `create_journal`). Check
  `journal_for` and the lock before saving and confirm replacing a file the dialog did not ask
  about.
- `write_atomically` renames over a symlink instead of writing through it (`save.rs`), which
  forks symlinked models and `preferences.json`. Resolve the final link before choosing the
  temporary sibling.
- After one journal write error the journal is dropped for the session and its lock released
  (`storage.rs`), so a transient ENOSPC leaves the session unprotected, lets another instance
  offer the stale journal, and brings back discarded changes after "Don't save". The app's
  warning also disappears on the next edit (`model.rs` clears `notice`). Retry by rewriting the
  journal from the app's entries, keep the path locked, and show a persistent status pill until
  protection is back.
- Recovering a file that loaded with problems loses the `.damaged` backup: the journal header
  has no "loaded with problems" flag and `Model::switch_to` resets `keep_original`. Store the
  flag in the header.
- A failed read of the previous file (EIO, EACCES) makes `save_with` write a file with no
  version history (`save.rs` `read_previous`). Fail the save or ask instead.
- The startup scan locks a journal by path and later removes it by path, so it can unlink the
  new journal an owner renamed into place between the two (`recovery.rs`). Compare the locked
  inode with the path's before deciding.
- The files worker runs STEP import, version history and model loads with no `catch_unwind`
  (`files.rs` `spawn_worker`), so a panic there never reports and leaves the status bar busy.
  Contain each job and send a failure event.
- No SIGTERM or SIGHUP handling: logout or `kill` drops queued journal entries (`main.rs` only
  installs a panic hook). Flush through the existing `Flusher` on those signals.
- Quitting after "Close Without Saving" waits at most 5 s for the storage worker and quits
  anyway (`files.rs` `CLOSE_TIMEOUT`), so a busy worker leaves the discarded changes to be
  offered for recovery next start.
- Journals are created with default permissions while saves copy the target's
  (`storage.rs` `create_journal`), so a 0600 model gets a world-readable journal of its content.
- Two instances overwrite each other's preferences because `Settings::save` writes the map read
  at startup without re-reading the file, and an unreadable `preferences.json` is replaced by
  defaults (`settings.rs`).
- Orphaned `.<name>.<pid>-<n>.tmp` files from a crash during save are never cleaned up, a failed
  directory fsync after a successful rename is reported as a failed save, and any
  `create_new` error in `keep_backup` is misreported as too many backups (`save.rs`).
- Non-UTF-8 paths are dropped from the journal header and recent files (`journal.rs`,
  `recent.rs` use `to_str`), so a crashed file with such a name recovers as untitled.

## Checks and CI

- No workflow runs tests or clippy on push or pull request; `.github/workflows/release.yml` only
  runs on `v*` tags. Add one on the Ubuntu 22.04 container with `--locked`.
- The render offscreen tests pass silently without an adapter (`offscreen_tests.rs` prints
  "skipping" and returns), and CI installs no Vulkan driver. Install lavapipe and fail when a
  `CADITOR_REQUIRE_GPU`-style variable is set.
- No fuzz targets for the Part 21 parser, DXF reader, binary container and value decoder,
  expression parser, or `read_step` and model `load` end to end. Every crash in "Hostile input"
  below was found by reading code.
- Wire `cargo deny` or `cargo audit` into CI; `ttf-parser` (through winit and sctk-adwaita) is
  already flagged unmaintained (RUSTSEC-2026-0192).
- The `preserve_order` feature on the dev-dependency `serde_json` changes the production
  `serde_json::Value` map type under test (`caditor-file/Cargo.toml`), so tests do not exercise
  what users run.
- The release workflow installs whatever stable is current with no pinned toolchain and actions
  pinned by tag, not commit, which undercuts the `SOURCE_DATE_EPOCH` reproducibility.
- Slow tests to keep an eye on: STEP `every_fixture_survives_a_round_trip` (7 s),
  `blend::every_edge_of_assorted_prisms` (6 s) and about 40 UI tests at over a second each.

## Hostile input

- STEP knot multiplicities are expanded before they are checked (`read/spline.rs`
  `expand_knots`), so one `B_SPLINE_CURVE_WITH_KNOTS` with a multiplicity of 4e18 aborts on
  allocation. Check the sum against points plus degree plus one with checked adds first.
- STEP spline degree is not bounded until `BSpline::new`, after `uniform_knots` and `clamp` have
  allocated and looped by it (`read/spline.rs`, `read/geometry.rs`). Refuse degree above 9 on
  reading.
- STEP assembly placement can be made exponential: `structure.rs` `walk` returns at
  `MAX_DEPTH` without counting, and `MAX_INSTANCES` is per representation, not per file. A
  20 KB layered graph runs about 2^32 walks, and 1000 solids placed 1000 times make 10^6 solid
  copies. Memoise placements and apply one budget per file.
- A crafted `saved_at` panics Version History: `binary/model.rs` computes
  `UNIX_EPOCH + Duration::from_secs(self.saved_at)`. Use `checked_add`.
- Nested DXF blocks can hang the import: only depth and cycles are limited and the cap counts
  emitted shapes, so blocks fanning out into TEXT do exponential work (`dxf/mod.rs`). Count
  visited cells and entities against a budget.
- DXF SPLINE degree is unbounded (`dxf/mod.rs`, `geometry.rs`), so degree 50 000 costs about
  10^11 operations. Refuse degree above 9.
- A small model file can declare many 256 MiB chunks, all decompressed into one `Vec` at once
  (`binary/model.rs` `record_contents`), and fake chunk headers make the resync scan quadratic
  because the payload is hashed before the header is rejected. Cap total decompressed bytes,
  decode records one at a time, and checksum the header separately.
- A stored expression of 64 000 characters builds a left-deep tree of about 32 000 levels before
  the depth check, and dropping it recurses on the files worker's 2 MiB stack
  (`caditor-expression/src/parse.rs`). Stop at `MAX_STORED_TREE_DEPTH` while parsing.

## Document and recompute

- Add then undo leaves the model Unsaved: `Document` and `Sketch` derive `PartialEq` over their
  ID counters, which never go back, and the app compares documents for `dirty` (`model.rs`).
  Compare content only.
- A body disappears from the view and from export when the feature that makes it fails, because
  `bodies` records only up-to-date states (`recompute.rs`). Carry the last good state as a stale
  body, drawn tinted, as `ux.md` promises.
- Cancelling during meshing reports Up to date: `is_complete` looks only at feature states, and
  `BodyMeshes` keeps the old mesh on screen until the next edit. Count unmeshed bodies as
  incomplete.
- Restoring an earlier version lowers sketch `next_id` counters, because `transaction_to`
  re-inserts the old sketches unchanged; later entities reuse IDs that face names were derived
  from. Reserve IDs below the current sketch's counter.
- Kernel operations and tessellation take no cancel token (`boolean`, `blend`, `shell`,
  `extrude`, `revolve`, `tessellate`), so a superseded job runs a multi-second boolean to the
  end. Pass `CancelToken` into their bounded loops.
- No early cutoff: dependents are compared by `Arc` pointer, so any edit that leaves geometry
  unchanged (`20 mm` to `2 cm`, a satisfied constraint, the settle before every sketch edit)
  rebuilds and re-meshes everything downstream. Keep the old `Arc` when a sketch or datum result
  is equal.
- Renaming a feature or parameter invalidates the cache, because the key compares the whole
  `Feature` including its name and the fingerprint includes parameter names. Key on content and
  refresh only message text.
- Feature names may repeat (`rename_feature`, `insert_feature` check only emptiness), while
  every error message names features by name. Refuse or suffix duplicates.
- A revolve's model axis is drawn from the body's final state (`datum.rs` `displayed_axis`), not
  the state the feature used, so a later fillet or split edge hides or misplaces it.
- A datum plane's rotation axis is not checked against its base plane: XY turned about Z is XY
  again and an oblique axis gives a plane that does not contain it (`datum.rs`). Require the
  axis to lie in the plane and say so otherwise.
- Two-sided extrude skips the above-zero rule the other extents apply (`solid.rs`), so negative
  distances silently extrude the other way.
- Some edit errors show raw IDs and positions ("entity 42 no longer exists", "Position 3 is
  outside the list") through undo and stale transactions. Map them to sentences with labels.
- Each sketch's profile arrangement is rebuilt once per solid feature and once more for display.
  Cache the `Profile` in `SketchResult`.
- Every blend's and shell's input state is tessellated on every run, although only the open one
  is shown. Mesh the before-state on demand.
- Parameter evaluation and cycle checks are O(P²), and restoring a version O(P³), on the UI
  thread (`parameter_dependencies`, `EvaluationOrder::of`, `cycle_through`). Build the
  dependency map once per apply.
- `Document::check` clones the document twice and the parameter table calls it for every row
  every frame. Add cheap `can_remove_*` queries.
- Untested: the `Import` feature's evaluation, `TwoSides` and `Symmetric` extents, revolve
  `OneSide` and `Symmetric`, `BodyOperation::Intersect`, cache reuse through solid chains, and
  cancel during meshing.

## Sketch solver

- A tangent at a line–arc joint, or on a point snapped onto a circle, is reported redundant and
  the sketch keeps one degree of freedom too many, since the distance-equals-radius form
  (`equation.rs` `LineTangent`) adds no rank there. Such sketches can never show as fully
  constrained, and the tangent direction converges only to about 1e-4 rad. Use a joint form
  (direction perpendicular to the radius) when the curves share a point.
- Internal circle tangency fixes which circle is larger when the system is built
  (`system.rs` `larger_first`), so changing a radius so the inner circle becomes the larger
  one makes the sketch unsolvable.
- `Distance = 0` between points is accepted but gives rank 1 instead of 2, so freedoms and
  entity states are wrong. Turn it into `Coincident` or refuse it.
- Angle dimensions depend on the direction each line was drawn, so a 60° corner in a chain of
  lines is dimensioned as 120°, and typing 60 turns the line instead. Record the measured
  quadrant when the dimension is made.
- Horizontal or Vertical on a nearly perpendicular line shrinks it towards zero length and
  still succeeds. Refuse collapsed lines like zero radii.
- Retry perturbations move every point by a fraction of the whole sketch's size
  (`numeric.rs` `PERTURBATIONS` times `context.scale`), which can flip small features' branches
  in large sketches. Scale per component.
- Every Gauss–Newton iteration builds a dense Jacobian and runs a full SVD
  (`numeric.rs`), so a connected 100-line sketch costs about 1e9 flops per iteration and a
  1000-segment DXF loop about 1e11. Use a sparse factorisation and share one rank-revealing
  decomposition between solving and analysis.
- Explaining a conflict re-solves the failed part once per constraint from scratch
  (`solve/mod.rs` `solves_with`), up to 300 SVDs each. Narrow suspects from the residuals first
  and warm-start.
- No drag or incremental solve: every edit re-solves and re-analyses the whole sketch, and there
  is no way to solve towards a dragged point. Needed for dragging geometry in the viewport.
- Removing an entity recounts all uses at every level of the cascade (`sketch.rs`
  `remove_entity`).
- `BSpline::interpolate` uses evenly spaced parameters, so unevenly spaced DXF fit points
  overshoot and loop, and fitting searches knot spans linearly and solves dense systems
  (`fit.rs`, `curve.rs`). Use chord-length parameters, binary search and banded solvers.
- Untested: tangent joints, internal-tangency swaps, `Distance = 0`, branch keeping under
  perturbation, and a time budget for a large chained sketch.

## Expressions and units

- A plain number in an angle field is degrees, while trigonometry reads plain numbers as
  radians, so `pi/2` in a revolve angle is 1.57°. Read plain angle results as radians, or refuse
  non-literal plain results with a hint to add `deg` or `rad`.
- Units bind only to the literal before them, so `10/2 mm` fails as mm⁻¹ and `(2+3) mm` fails
  to parse (`parse.rs` `unit_after`). Allow a unit after any primary.
- `round`, `floor` and `ceil` work in base units, so `round(1.26 cm)` is 13 mm. Add a step
  argument.
- Non-finite intermediate values are checked only at the end, so `max((-8)^(1/3), 5 mm)` hides
  a NaN and `(-4)^0.5` reports "too large" instead of a negative root.
- `in` and `ft` are still accepted and reserved (`quantity.rs`), against the SI-only rule. Keep
  them readable in stored text but stop accepting them in fields.
- Unit spelling is exact: `10 MM` and `10 degrees` fail with "Expected an operator", and the
  displayed `mm²` cannot be typed back. Say which units exist.
- Missing: conditionals and comparisons, `mod`, `hypot`, `exp`, `ln`, `sign`, `clamp`, and the
  constants `e` and `tau`.

## Kernel correctness

- Mixed convex and concave blends silently drop convex edges whose references are lost in the
  concave pass (`blend/mod.rs`: `ReferenceError::Missing => Vec::new()`), and the feature
  succeeds unrounded. Fail naming the edge, and assert blend face counts in
  `every_edge_of_assorted_prisms`.
- Region keys change with what else is selected: `region.rs` tie-breaks only against regions in
  the same batch, and `select` re-keys the selected lumps alone, so a cap face is renamed when
  a neighbouring region joins the selection. Compute tie-breaks once over the whole arrangement.
- A reference whose name is unique is accepted without checking neighbours
  (`naming/reference.rs`), but cut occurrences are numbered along the curve, so removing two
  crossings silently hands an old face's reference to a different side face. Require neighbour
  overlap even for unique names.
- Twisted faces tessellate as two triangles at any tolerance, because density looks only at
  `duu` and `dvv` (`tessellation/density.rs`); curvature is also sampled on one 9×9 lattice.
  Include twist, sample per knot span and refine by measured error.
- Blend errors can name the wrong edge or a vertex of an intermediate solid
  (`blend/mod.rs` `remapped` falls back to the first convex edge).
- `fits` checks the blend only at the middle of the edge, so a face that narrows elsewhere lets
  the tool cut through.
- Shell blames every failure on the thickness (`plan.build`, profile and extrude errors map to
  `TooThick`), and fails on convex faces with a radius below the thickness and at vertices of
  four or more faces whose offsets do not meet. Add distinct errors, drop collapsed faces, and
  split such vertices into edges.
- The marcher treats step collapse and the step cap as a tangent end (`march.rs`), and
  `prune_dangling` then discards the incomplete cut, so failures surface far away as
  `Ambiguous` or a wrong keep. Report them as errors.
- `split_edges` skips tiny pieces without merging their vertices (`imprint.rs`), leaving a gap in
  the chain.
- The loop tracer orders edges at a cone apex by the normal of one ruling (`trace.rs`
  `vertex_normal`), so cuts through the apex can take the wrong next edge. Order by uv direction
  at poles.
- Spline surfaces that coincide over only part of their extent are not seen as coincident
  (`coincidence.rs` samples the whole domain), so booleans march between identical surfaces.
- Point classification returns `Outside` when every ray was ambiguous (`classify.rs`), and pole
  probing uses an absolute `1e-3` in v, which lies outside small knot ranges.
- Same-name edges are ordered by exact float comparison of midpoints (`build/plan.rs`,
  `topology/mod.rs`), so last-bit noise can swap occurrences after an upstream change.
- Long wiggly splines fail with "runs back over itself": the per-pair budget in
  `profile/intersect.rs` is charged before the box test, so about 450 monotone segments always
  exhaust it. Prune first and give exhaustion its own error.
- Two real profile crossings closer than 0.1% of the sketch size merge into one
  (`profile/intersect.rs` deduplicates by leaf size).
- STEP export of a solid that cannot be meshed writes each void as its own inverted solid and
  still succeeds (`write/shape.rs`), and an unclassified void is attached to the first lump.
- Validation does not check faces or loops against each other, so self-intersecting imports pass
  and fail later in booleans.
- `Solid::bounding_box` uses only edges and vertices, so a sphere's box misses half of it.
- Untested: spline surfaces in intersections, booleans, blends and shells; import edge naming;
  every blend and shell refusal branch; cuts through a cone apex.

## Kernel performance

- Booleans re-trace, re-fit and re-classify every face of both solids (`boolean/faces.rs`),
  including untouched ones that can still fail. Pass untouched faces through.
- No spatial index: vertex pooling, edge–face and face–face candidate pairs, face bounds lookup
  and `merge_coincident_vertices` are all quadratic (`imprint.rs`, `boolean/mod.rs`,
  `build/plan.rs`). Add a grid hash for points and a BVH for face boxes.
- Blends run one full boolean per edge on a growing solid, and mixed selections analyse twice.
  Union the tools first.
- `heal_edges` rebuilds use and incidence maps for every join.
- A projection hint never saves work: `numeric.rs` runs the global search before using it, twice
  per `Revolution` projection. Try the hint locally first.
- B-spline curve evaluation allocates about ten vectors per call (`bspline.rs`), unlike the
  surface's stack arrays.
- B-spline surface projection sorts the whole 49×49 grid and refines four seeds every call, and
  recomputes poles.
- Intersection-curve `bounds`, `seeds` and `trimmed` walk every node; use `partition_point`.
- A failing face re-meshes the whole solid up to five times, and validation retries that three
  times. Retry only the failing face.
- The profile arrangement is quadratic in curves (`arrangement.rs` `split` scans every event per
  source), about 1e9 comparisons at `MAX_DRAWING_CURVES`.
- Classifier lookups are linear per face and quadratic per loop in `corner`.

## STEP import and export

- Solids vanish without a note when a placement cannot be transformed or its search hits the
  depth limit or a cycle (`read/mod.rs`, `structure.rs`); if all do, the message is "holds no
  solid bodies".
- Units given as a complex `MEASURE_WITH_UNIT` are read as millimetres (`read/units.rs` uses
  `fields()`, which refuses complex instances), so such inch files import 25.4 times too small.
- Plain `BEZIER_CURVE`, `UNIFORM_CURVE` and `QUASI_UNIFORM_CURVE` are refused because the name
  match covers only `B_SPLINE_CURVE` (`read/geometry.rs`), and multi-segment Bézier curves and
  surfaces get quasi-uniform knots instead of the standard's.
- Nested sub-assemblies can be placed upside down, since parent and child are guessed from
  `children` and `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` is never read.
- Product names lose to body names such as "Body1", and `NEXT_ASSEMBLY_USAGE_OCCURRENCE`
  instance names are ignored.
- Unsupported entities: `OFFSET_SURFACE`, `DEGENERATE_TOROIDAL_SURFACE` (the kernel refuses
  spindle tori), `COMPOSITE_CURVE`, `OFFSET_CURVE_3D`, `POLY_LOOP` (so `FACETED_BREP`, which
  is also misreported as an open surface body), `CLOSED_SHELL`s inside
  `SHELL_BASED_SURFACE_MODEL`, Part 21 edition 3 sections, and colours and layers.
- The declared `UNCERTAINTY_MEASURE_WITH_UNIT` is ignored, and healing covers only edges with
  exactly two faces.
- Import canonicalises each placement by writing and re-reading it, parses every import again on
  each model load, journal replay and recovery scan, and stores every placement of a product as
  its own STEP text. Build each representation once, store each product once with placements,
  and cache solids by text digest.
- The healing check projects every edge sample without hints (`read/topology.rs`), up to about
  2400 surface evaluations per sample on spline faces.
- The parse tree costs several times the file size with a `String` per token and a `BTreeMap`
  of instances, and the reader walks all entities about six times.
- The writer meshes every body just to sort its shells, even single-shell ones, and puts all
  bodies in one product with no colours, holding the output twice in memory.
- Imports cannot be positioned: `Import` has no placement.

## File format and storage

- Version history is never pruned and every save rewrites and fsyncs all of it
  (`binary/model.rs`). Add a retention policy and write history append-only.
- A model whose records total more than 256 MiB saves once and never again, since the previous
  snapshot and the journal snapshot are each one chunk (`MAX_CONTENT`). Chunk per record.
- Opening Version History decompresses and holds every version at once
  (`version_snapshots(usize::MAX)`), and `load_version` always starts from the head. Keep only
  the rolling snapshot and start from the nearest keyframe.
- Deltas set no zstd window or long-distance matching (`caditor-zstd` `compress_after`), so
  versions above the default window are close to full copies. Size the window to prefix plus
  data, raise the decoder's `windowLogMax`, and test with a prefix of several MiB.
- Every record is recompressed at level 9 on each save, including unchanged STEP text.
- Fields and chunk kinds an older reader does not know are dropped on its next save without a
  report, and the reserved header bytes are never read. Keep unknown content and define a
  must-understand flag.

## DXF import

- `$MEASUREMENT = 0` (imperial) is ignored when `$INSUNITS` is absent or 0, so such drawings
  import 25.4 times too small.
- HATCH boundaries, SOLID, TRACE, 3DFACE and MLINE are dropped; HATCH boundaries are often the
  only closed profile.
- Fit-point splines ignore their end tangents and use uniform parameters; a SPLINE with bad
  control points is refused even when it has fit points.
- `$DWGCODEPAGE` is ignored, so non-UTF-8 names come out garbled.
- Parsing keeps an owned `String` per value and clones every record, two to three times the
  file size, with no size limit.

## Mesh export

- STL uses absolute f32 coordinates, which lose about 0.06 mm at 10^6 mm, and merges all bodies
  into one surface.
- 3MF has no colours, materials or thumbnail, builds everything in memory and cannot exceed
  4 GiB without ZIP64.
- STEP export and each body's tessellation cannot be cancelled while running.

## Rendering robustness

- Picking lets translucent fills win: pick fills write no depth, so a principal or datum plane
  in front of a face or region owns the pixel, and choosing a sketch plane can put the sketch on
  XZ instead of the clicked face (`viewport.rs` pick pass, `scene.rs`).
- Line widths, marker sizes, pick tolerances and the pick window are in physical pixels, so at
  200% edges are 0.75 logical px and picks half as forgiving, while snapping scales.
- A click can act on a stale hover from an earlier cursor position or view, since `apply_pick`
  never compares the result's key with the current one.
- GPU device loss is never handled: no device-lost callback, and the UI shares the dead encoder.
- Frame failures redraw in a tight loop, and a hidden window blocks the UI thread for the 1 s
  acquire timeout per frame. Back off and stop drawing while occluded.
- Surface recovery reconfigures with the stale size, which can spin on `Outdated`.
- The surface format is the first non-sRGB one the driver lists, which can be snorm or float on
  HDR setups. Prefer `Bgra8Unorm` or `Rgba8Unorm`.
- The device uses default limits with no retry, so windows wider than 8192 px fail and GL or
  downlevel adapters (the mesh shader needs vertex storage) fail at startup; the high-performance
  preference also wakes discrete GPUs.
- One buffer over the device limits breaks every frame including the UI; check sizes and split
  meshes.
- A zero-size viewport drops every GPU mesh, re-uploaded when it returns.

## Interface performance

- The whole scene is rebuilt every frame and cloned into `PickKey` for comparison, every line and
  fill vertex is rewritten, and fills are re-sorted (`app.rs` `build_scene`, `viewport.rs`).
  Cache geometry per result and key picks on a generation.
- Body mesh conversion and GPU upload run on the UI thread (`bodies.rs` `BodyMesh::build`),
  against `ux.md`. Build `ShadedMesh` and edge polylines on the worker.
- `displayed_sketch` clones and compares whole sketches five or more times per frame, and
  `model_bounds` polylines every sketch twice.
- `find_face` rebuilds a map over every face on each call, and the toolbar, status bar,
  selection and panels call it and capture references for each selected face every frame.
  Cache a `FaceKey` index per result and compute offers when the selection changes.
- An open fillet panel reruns `blend_chain` and edge descriptions every frame.
- An expanded sketch in the tree formats and evaluates every constraint every frame with an
  O(n²) `involved` check; virtualise and cache.
- Dragging a navigation slider writes and fsyncs `preferences.json` every frame.
- Sketch curves are faceted at a fixed 3° regardless of size or zoom.

## Keyboard, accessibility and look

- Ctrl and Alt shortcuts stop when any widget has focus, not only a text field, because
  `app.rs` uses `egui_wants_keyboard_input`. Use `text_edit_focused`.
- N gets stuck on a datum plane, which is registered twice (curve and fill), so
  `step_highlight` alternates between its copies.
- Typed points split on every comma, so `max(w, 10), 5` is refused (`typed_point.rs`), and the
  field opens only on a digit, sign, point or `@`.
- Key auto-repeat re-fires commands, so holding Space toggles the highlight in and out of the
  selection.
- Not commands, so neither in the palette nor bindable: recompute and cancel, rename, move and
  delete feature, add parameter, open recent, recover unsaved work, cancel export, dismiss
  notice. Delete does nothing outside sketches, as the tree has no selection.
- Red and amber dimension labels on the dark canvas are about 3:1 in light and 1.9:1 in light
  high contrast (`annotations.rs` uses the theme's `error_fg_color`). Give the canvas its own
  tested colours.
- The hover label, prompt and coordinate readout have no backdrop and fall to about 2:1 over
  bodies.
- Fixed-size dialogs and lists clip at 200% on 1080p (`widgets.rs`, `shortcut_editor.rs`,
  `palette.rs`, `history.rs`). Clamp to the content rect.
- The Fit button's tooltip hard-codes "(F)" instead of the current binding.
- Several actions fail without a word: New sketch on an unusable face, a datum that fails its
  check, fillet or shell with nothing usable, clicking a curved face while choosing a plane.
- The Plane tool silently falls back to XY when a selected face cannot be captured, and a face
  that is only outdated is described as made after the sketch.
- The contrast test requires only 3:1 for pill text on status fills; require 4.5:1, and 7:1 in
  high contrast.
- No screen-reader support: the egui AccessKit integration is off.

## Sketching

- The arc end snaps to a target not on its circle and keeps the `Coincident`, so the solver
  moves it away from the preview; the circle rim shows "On Line N" but adds nothing.
- The line chain keeps going after it closes back on its start.
- No dragging of sketch geometry and no window selection: primary drag is never handled.
- No construction geometry: every curve becomes profile curves, so a centreline splits regions
  and changes their keys.
- No projection of model edges or other sketches into a sketch, and bodies and other sketches
  are unpickable while editing.
- Constraints missing: symmetric, midpoint, fix, collinear, concentric, horizontal and vertical
  distance, diameter, horizontal and vertical between two points, point on spline, tangent to a
  spline, line–line spacing and point–circle distance; Equal and Parallel take only two items.
- Tools missing: trim, extend, offset, mirror, sketch fillet, three-point and tangent arcs, slot,
  polygon, ellipse, and polar or length input in the typed-point field.
- Snapping has no midpoints, intersections, spline targets, grid or inference lines to other
  points.
- Dimensions all sit at one fixed offset, so collinear chains overlap, and labels cannot be
  dragged.
- The fillet panel calls a split edge "no longer there" although recompute fillets all its
  pieces.

## Modelling features

- Feature kinds missing: linear and circular patterns, mirror (the kernel has no reflecting
  transform), hole, draft, sweep, loft, split, and move or copy body.
- Extents missing: through all, up to next and up to face for extrude, and two angles for
  revolve.
- No feature combines two existing bodies, and a cut affects only one body.
- No suppress, rollback bar, insert-here, drag reorder, or delete with a preview of its
  dependents.
- Blends: only line and circle edges along planes, parallel cylinders and coaxial surfaces; no
  ellipse, spline or intersection edges; ends at steps and T-junctions refused; no variable
  radius, two-distance or distance-angle chamfer; corners only for three convex straight edges.
- Shell: no spline, extrusion or revolution faces, only line and circle edges, only flat faces
  open, one thickness for the whole body.
- No live preview of a fillet, chamfer or shell while its panel is open, and no viewport handles
  for extents.

## Viewer

- Hide and show for bodies, sketches and planes; consumed sketches currently cover the faces
  they lie on and win picks.
- Orthographic projection for standard views.
- Silhouette edges on curved bodies.
- Section planes.
- Transparent or X-ray bodies.
- Line caps, joins and anti-aliasing without MSAA.
- Measure tool with distances and angles, vertex selection, a selection filter, and mass
  properties (volume, area and centroid are already computed by `Mesh`).
- Screenshot and image export.

## Application

- One files worker runs everything and Open and Import cannot be cancelled, so a slow STEP import
  blocks Open behind a modal. Give imports their own cancellable job. When a worker thread cannot
  be spawned the job runs on the UI thread.
- Save As keeps any extension the name already has, so "Bracket v1.2" is saved without
  `.caditor` and disappears from the Open dialog's filter; check for the model extension as
  export does.
- Dropping a file on the window does nothing.
- The welcome dialog's "Start with an empty model" only closes the dialog when a file was opened
  from the command line.
- A plain number typed as a parameter value ignores the preferred length unit, so `10` becomes
  10 mm where dimensions would read 10 cm.
- Window size, position and panel state are not remembered.
- One document per process.
- No clipboard for sketch geometry or features, no parameter import or export.
- Version history shows only relative times.
- No localisation.
