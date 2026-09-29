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

## Checks and CI

- `rust-formatter --check` is required by `docs/RELEASING.md` but no CI job runs it.
- Slow tests to keep an eye on: STEP `every_fixture_survives_a_round_trip` (7 s),
  `blend::every_edge_of_assorted_prisms` (6 s) and about 40 UI tests at over a second each.

## File format and storage

- Version history is never pruned and every save rewrites and fsyncs all of it
  (`binary/model.rs`). Add a retention policy and write history append-only.
- A model whose records total more than 256 MiB saves once and never again, since the previous
  snapshot and the journal snapshot are each one chunk (`MAX_CONTENT`). Chunk per record.

## Kernel correctness

- `SolidClassifier::classify` never returns `PointClass::Undecided`: `cast` reports every
  ambiguous ray with a guess, and the first guess is returned after all twelve rays, so the
  boolean's documented retry at the fragment's other points never runs and a grazing guess is
  taken as a confident class. Return `Undecided` when no ray was clean, and test it.
- `Solid::find_crossing` skips face and edge pairs whose intersection errors (`crossing.rs`
  `.ok()?`, `let Ok(..) else { continue }`), so a self-intersecting STEP import with hard
  spline or near-tangent faces is accepted. Report the check as inconclusive instead.
- Cancelling during a boolean's final validation surfaces as `BooleanError::Invalid`
  (`Tessellation(Cancelled)` wrapped by `PlanError::Build`), so blends retry tool by tool or
  report a false invalid result instead of stopping.
- Blends check that they fit on both faces only at a quarter, half and three quarters of the
  edge (`FIT_FRACTIONS`), so a hole or notch between samples lets the tool bite into the next
  face.
- A blend on a nearly full circular edge whose ends need extensions exceeds a turn and fails as
  a generic `BlendError::Sweep` ("choose fewer edges"); clamp the extension or name the edge.
- Sphere projection has no pole snapping and the analytic surfaces treat a point as on the axis
  only within 1e-14 relative (`surface/projection.rs` `AXIS_EPSILON`), so a pole vertex of a
  model far from the origin gets an arbitrary u, unlike revolutions and splines, which reuse the
  hint's u through `pole_at`.
- Radii are only required to be positive (`curve/conic.rs`, `curve2/primitives.rs`,
  `surface/elementary.rs`), and sweep extents only finite (`LinearExtent::new`), so values far
  below `LINEAR_RESOLUTION` or far above `MODEL_EXTENT` reach the kernel and fail later with
  generic errors.
- Extrusions of a line profile nearly parallel to the direction pass the `parallel` check
  (1e-10) and then project by dividing by `1 - tilt²` (`surface/swept.rs`).
- `EdgeReference::resolve` reports `Ambiguous` when several candidates all score zero on their
  recorded ends, where `Missing` is the truth.
- `EdgeName::between_at` orders its faces only when the end vertex names differ, so an edge
  whose ends have equal names (a closed edge) is named differently depending on its direction.
- `ProfileError::Unresolved`, produced by about thirty internal paths in `profile/`, names no
  curves, so the sketch error can only say to simplify where curves meet.
- Revolve refuses a profile whose regions lie on both sides of the axis with the wording for a
  curve crossing it (`BothSidesOfAxis` and `CrossesAxis` print the same sentence) and lists
  every curve of the sketch.
- Shell cannot drop collapsing cones or faces whose edges run neither around nor along their
  axis, and cannot split a vertex whose edges are partly convex and partly concave (both
  reported).
- Tests missing for: pinched regions (a hole tangent to its outline, squares sharing a corner)
  swept by extrude and revolve; a circle tangent to the revolve axis; shelling or blending bodies
  with voids or several lumps; the `Ambiguous`, `Open`, `Split`, `Intersection` and `Invalid`
  boolean errors and `TooComplex`/`Unfollowable` messages; pinned digests of a tie-broken
  `RegionKey` and of `PieceId::digest`.

## Kernel cancellation and limits

- Nothing in `intersect/` polls the interrupt, though one `intersect_surfaces` call can seed
  32768 pairs and march 32768 steps per direction; `imprint::split_edges`, `clip_branch` and
  healing are unchecked too, and the only cancellation test cancels at the first poll.
- Profile building, sweeps, shell offsets and blend tool construction never poll the interrupt:
  `profile/arrangement.rs` `events` runs up to 100k box tests per curve pair and `within` is
  O(S²) in spline segments, all uncancellable.
- Tessellation polls once per face, and nothing caps a solid's total size: each face may take
  2^19 grid points and 65536 points per edge into one spade triangulation. Poll inside the grid
  and insertion loops and fail beyond a solid-wide budget.
- A face whose boundary crosses itself re-tessellates the whole solid up to five times
  (`tessellation/mod.rs`), redoing every other face and edge; `density()` also runs twice for
  every face with a pole (`pole_edge_segments` and `triangulate`).

## Kernel performance

- The boolean vertex pool buckets by exact x bits (`boolean/imprint.rs` `Pool`), so axis-aligned
  models put every vertex on a plane in one bucket and `near` returns whole x-slabs: O(n²)
  inserts and O(edges × points) splits. Index by a 3D grid of the tolerance.
- Profile keys and faces cost O(R²) and O(F·P): `shared_keys` and `with_keys` count equal keys
  with nested scans, `base_keys` clones every piece, and `lumps` scans every half-edge per face.
  The largest test has 200 regions; a big DXF freezes the worker.
- `Plan::merge_coincident_vertices` refilters every edge per shell, O(shells × edges) on every
  build, and it and `arrangement::cluster` degrade to O(n²) when many points share an x.
- Every `Solid::build` computes `bounding_box`, which builds a full classifier and runs 169
  `point_in_face` queries per curved face only to size the sampling tolerance.
- `face_data` counts seam uses quadratically per face (`classify.rs`), `point_in_face` rebuilds
  it on every call, and `SolidClassifier::corner` searches every face linearly.
- Validation computes mass properties once per shell and probes voids over the whole mesh per
  shell, O(lumps × triangles); `Mesh::contains` copies every triangle on each call.
- `EdgeReference::resolve` and `capture` rebuild the vertex-name map over the whole solid per
  reference, O(references × edges) per blend.
- `prune_dangling` rebuilds its incidence map each round (`boolean/faces.rs`).
- B-spline surface projection of a point off the surface refines four seeds every call, which is
  most of its cost.

## Document and recompute

- Blend and shell features accept every candidate of an `Ambiguous` reference
  (`blend.rs`, `shell.rs` `found.extend(pieces)`) without checking they are pieces of one edge or
  face, so an upstream edit that ties unrelated edges fillets or opens all of them silently.
  Datums and attachments already check (`one_line`, equal planes).
- Meshing requested for an open blend or shell (`Recomputer::mesh`) runs before the queued
  recompute and outside `interruptible` (`worker.rs`), so a heavy body delays every edit and
  ignores Cancel.
- The worker runs `Recompute::run` without `catch_unwind`; a panic in parameter evaluation,
  `Names::of` or stale-body bookkeeping kills the thread and the model never recomputes again.
  Contain each job or respawn the worker.
- Undo and redo stacks are unbounded (`editor.rs`), and entries keep removed features alive,
  imports with their solids and STEP text included.
- Cached failure messages refresh only when the names of `features()` change, but blend, shell
  and datum errors name the features that made each face ("Cut 2 end face"), so renaming those
  keeps the old name in the message.
- Solid results count as unchanged only when they are the same `Arc`, so any edit to a solid
  feature reruns every downstream boolean, blend and shell even when the shape is identical.
- The cache compares sketch definitions with derived equality, which includes the ID counter and
  use counts, so an add-then-undo re-solves the sketch; use `Sketch::same_content`.
- Edits do not check that a revolve's `RevolveAxis::Sketch` line exists in its sketch, so
  `SetFeatureKind` can point the revolve at another sketch with a dangling axis, and the "axis
  cannot be deleted" guarantee does not hold for the new sketch.
- `insert_feature` stores names untrimmed while `rename_feature` trims, so " Extrude 1" and
  "Extrude 1" can coexist.
- Axis-in-plane tolerances disagree: a revolve accepts 1e-6, a datum rotation axis 1e-9, and
  face attachments compare planes exactly, so a slightly noisy imported edge works for one and
  is refused by another.
- Tests missing for `transaction_to` with datums, attached sketches and imports, moves to an
  out-of-range index, and `Ambiguous` resolution in blends and shells.

## Sketch solver and expressions

- The lexer accepts `1e999` as infinity (`f64::from_str`); inside an untaken `if` branch it
  evaluates, then `exact_number` stores it as `inf`, which `parse_stored` cannot read, so the
  expression is lost on load. Reject non-finite literals.
- A solve that fails with no identifiable conflict reports "could not be solved from its current
  shape" with no entity and no next step (`diagnose_failure` returning `Unsolvable`).
- Conflict diagnosis re-solves each probe over the whole failed scope with no probe budget, and
  treats a subset that merely fails to converge from the fixed start as inconsistent, so large
  sketches diagnose slowly and a reported conflict can be wrong or not minimal.
- The solve memo key includes the global scale (the largest coordinate or dimension), so editing
  the outermost dimension or dragging the outermost point misses the memo for every part.
- Each tangent constraint rebuilds the coincidence union-find and scans all constraints twice
  (`system.rs` `joint`, `points_on`), quadratic in tangents per solve.
- A zero-length line (two distinct points at one position) makes its part inadmissible and is
  reported as a conflict of its own constraints; say that the line has no length.
- `10 mm^2` means (10 mm)², and `mm2` or `width²` fail with "no parameter named"; hint at `mm²`.
- Parameters may be named `mm²` or `cm³`, which `check_name` accepts but expressions always read
  as units.
- `floor`, `ceil` and `round` with a step use exact division, so `floor(0.3, 0.1)` is 0.2;
  snap within the comparison tolerance.
- An integral exponent outside the i8 range reports "can only be raised to a whole power".
- A chained comparison (`1 < x < 5`) and a decimal comma (`0,5 mm`) both say "Expected an
  operator"; give each its own message.
- Dimension values are unbounded above, so 1e200 mm overflows the scale and fails as the generic
  unsolvable error, and labels print hundreds of digits.
- Missing functions: `cbrt` (a length from a volume is otherwise impossible), `log10`, `log2`,
  `trunc`, and logical `and`, `or` and `not`.
- `BSpline::through` returns `None` on consecutive equal points (singular collocation); only
  tests use it today.
- Tests missing for: dense and sparse paths giving the same rank and fixed sets across the
  48-variable boundary; minimal conflict sets and two independent conflicts in a large part;
  arcs with tangent, equal and angle constraints; a scale-changing edit; expressions at
  `MAX_TREE_DEPTH` evaluated, printed and dropped on a worker's default stack.

## STEP import and export

- `COMPOSITE_CURVE` pieces are rebuilt for every reference with only a depth limit of 8
  (`read/geometry.rs`), so composites whose segments point at the next composite cost about
  segments^8 evaluations from a few hundred bytes. Memoise curves by entity and charge a work
  budget.
- Surfaces and curves are rebuilt per face and failed solids per referencing entity with no
  budget (`read/mod.rs` decrements it only on success), so many faces sharing one large spline
  surface, or many breps pointing at one bad shell, repeat the work.
- A body with several closed shells (a `SHELL_BASED_SURFACE_MODEL` read as one multi-lump solid)
  round-trips to several solids, and `import/model.rs` `canonical` requires exactly one, so the
  body is dropped as "could not be stored".
- Assembly parts whose placement is a `CARTESIAN_TRANSFORMATION_OPERATOR_3D` or anything but an
  `AXIS2_PLACEMENT_3D` pair are silently left at the origin with no note
  (`read/structure.rs` `relationship`, `mapped_items`).
- One unrecognised byte or an integer beyond i64 rejects the whole file, a repeated entity id
  silently overwrites the earlier one, and `\P` and `\X\` followed by a multi-byte character
  drop the rest of the string (`part21.rs`).
- The writer encodes characters beyond the BMP as `\X2\` surrogate pairs rather than `\X4\`.
- Unsupported entities: `OFFSET_SURFACE`, horn tori (a `DEGENERATE_TOROIDAL_SURFACE` whose tube
  just touches its axis), and colours and layers.
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
- The writer puts all bodies in one product with no colours, holding the output twice in memory.
- Imports cannot be positioned: `Import` has no placement.

## DXF import

- Block content is cloned per instance and only curves are counted (`dxf/mod.rs` `add`), so one
  heavy SPLINE inserted as a large array can reach tens of gigabytes. Cap total points and share
  block shapes between instances.
- A closed LWPOLYLINE or POLYLINE of two vertices with bulges (a circle or lens drawn as a
  polyline) gets only one arc, since `polyline_segments` closes only above two vertices.
- A SPLINE whose weight count differs from its control points silently becomes non-rational
  (`Nurbs::new`).
- HATCH boundaries, SOLID, TRACE, 3DFACE and MLINE are dropped; HATCH boundaries are often the
  only closed profile.
- An INSERT array above 20,000 cells refuses the whole import before checking whether its block
  draws anything.
- `$INSUNITS` codes above 20 read as millimetres with a "does not say" note.
- Fit-point splines ignore their end tangents, and a SPLINE with bad control points is refused
  even when it has fit points.
- `$DWGCODEPAGE` is ignored, so non-UTF-8 names come out garbled.
- `Nurbs::point` finds the knot span linearly for each of up to 16,384 samples, and each visited
  entity rescans its record for codes 67, 60 and 8.
- Parsing keeps an owned `String` per value and clones every record, two to three times the
  file size, with no size limit.

## Mesh export

- STL uses absolute f32 coordinates, which lose about 0.06 mm at 10^6 mm, and merges all bodies
  into one surface.
- 3MF object names escape only control characters, so noncharacters such as U+FFFE make invalid
  XML 1.0.
- 3MF has no colours, materials or thumbnail, builds everything in memory and cannot exceed
  4 GiB without ZIP64.

## Rendering robustness

- Faces without a pick id are discarded in `fs_pick` and write no depth, so while a sketch is
  edited, or a fillet or shell panel chooses edges or faces, edges and curves behind the solid
  are hovered and clicked through it, contrary to "hidden in the view and in picking alike".
  Write id 0 with depth instead of discarding.
- A failed pick readback returns `None` (`picking.rs`) and the app's `last_pick` stays set, so
  hover is stuck and a click waiting for a fresh result waits until the cursor or view moves.
- Orbit has no elevation limit, so dragging past the pole turns the model upside down and
  reverses horizontal drag, and entering a sketch on a rotated face starts with a rolled
  horizon that yaw about Z never removes (`camera.rs`).
- `View::fitted` floors the radius at 1 mm (`MIN_FIT_RADIUS`), so small parts and fit to a small
  selection cannot fill the view.
- Line widths, marker sizes, pick tolerances and the pick window are in physical pixels, so at
  200% edges are 0.75 logical px and picks half as forgiving, while snapping scales.
- GPU device loss is never handled: no device-lost callback, and the UI shares the dead encoder.
- Frame failures redraw in a tight loop, and a hidden window blocks the UI thread for the 1 s
  acquire timeout per frame. Back off and stop drawing while occluded.
- Surface recovery reconfigures with the stale size, which can spin on `Outdated`.
- The device uses default limits with no retry, so windows wider than 8192 px fail and GL or
  downlevel adapters (the mesh shader needs vertex storage) fail at startup; the high-performance
  preference also wakes discrete GPUs.
- One buffer over the device limits breaks every frame including the UI; check sizes and split
  meshes.
- A pick encoded in `begin_frame` is started only by `submit`, so a dropped `Frame` leaves
  picking stuck in `Encoded` forever.
- A zero-size viewport drops every GPU mesh, re-uploaded when it returns.
- The 4x MSAA colour target is stored every frame although it is only resolved; `GrowableBuffer`
  never shrinks from its peak.
- Offscreen tests draw only at a zero-offset viewport and never cover markers, the grid,
  near-plane clipping of lines, unpickable faces or a failed readback.

## Interface correctness

- A new or opened document inherits the selection and the selected tree row: IDs restart in
  every document, `Selection::retain_available` keeps any whose ids exist, and only the camera
  reacts to a session change (`viewport.rs`), so Extrude can pick a sketch the user never
  selected and Delete can hit a stale row. Export exclusions (`Exporter.left_out`) carry over
  the same way.
- A two-sided extrude's Forward and Backward fields take `Rule::Any` (`solid_panel.rs`), so zero
  or negative distances are committed and the feature fails at recompute.
- An import that adds nothing (a DXF of only hatches or text into the edited sketch) returns
  before its notes are shown, so the user sees nothing (`import.rs`).
- Restoring a version always reports success, overwriting an error notice from the apply, and a
  result for another path is dropped without a word (`files.rs`).
- Every successful edit clears the notice (`model.rs`), including save and import failures.
- Fillet creation drops selected edges that can no longer be found and fillets the rest, while
  Shell refuses when any face is missing.
- Zero-size lines, rectangles, circles and arcs are refused silently, including from the
  typed-point field.
- Constraint rows in the sketch tree are styled as links but clicking or hovering them does
  nothing.
- A rejected `commit_field` draft keeps showing its old text and error after the stored value
  changes through undo.
- A shortcut whose stored text does not parse leaves its command unbound rather than on its
  default.
- Rows in the blend and shell panels are non-wrapping, so long edge descriptions can push the
  remove button off the panel.

## Interface performance

- The whole scene is rebuilt every frame and cloned into `PickKey` for comparison, every line and
  fill vertex is rewritten, and fills are re-sorted (`app.rs` `build_scene`, `viewport.rs`).
  Cache geometry per result and key picks on a generation.
- Body mesh conversion and GPU upload run on the UI thread (`bodies.rs` `BodyMesh::build`),
  against `ux.md`. Build `ShadedMesh` and edge polylines on the worker.
- The Export dialog calls `Solid::bounding_box` for every body on every repaint to show the
  deviation (`export.rs`, `MeshResolution::tolerance`).
- A DXF drawing of up to 20,000 curves is turned into a transaction and applied on the UI thread
  (`import.rs`, `drawing_transaction`); only parsing is on the worker.
- Snapping collects every point and curve of the sketch into new vectors and projects onto each
  on every hover frame (`snap.rs`).
- `displayed_sketch` clones and compares whole sketches five or more times per frame, and
  `model_bounds` polylines every sketch twice.
- `find_face` rebuilds a map over every face on each call, and the toolbar, status bar,
  selection and panels call it and capture references for each selected face every frame.
  Cache a `FaceKey` index per result and compute offers when the selection changes.
- Every mesh's face style table is uploaded every frame although only the eye offset changes.
- An open fillet panel reruns `blend_chain` and edge descriptions every frame.
- An expanded sketch in the tree formats and evaluates every constraint every frame with an
  O(n²) `involved` check; virtualise and cache.
- Sketch curves are faceted at a fixed 3° regardless of size or zoom.

## Keyboard and accessibility

- Commands are missing for editing a sketch or opening a feature and finishing it, Detach and
  Place on selected plane or face, Use selected axis and Use selected in the datum panel,
  deleting a parameter, focusing the first failed feature, and dismissing tips.
- Icon-only buttons (`widgets::icon_button`, the tree row's chevron, edit and "⋯" buttons, the
  size buttons) expose their private-use glyph as their accessible name, property captions are
  not tied to their fields, and the shortcut editor's many "Add…" and "Reset" buttons differ
  only by hover text.
- Ctrl and Alt shortcuts are ignored while a text field has focus, so Save after typing a value
  does nothing; commit the field and let global commands through.
- The menu bar and status bar are single non-wrapping rows and the parameter grid is wider than
  the side panel's minimum; the 200% test checks four labels on an empty model.
- The shortcut editor's filter matches titles and categories but not bindings, so a key cannot
  be looked up.
- Micrometres are not a length unit choice, and the cursor readout has two decimals in the
  preferred unit (10 mm steps in metres).
- Typed arc ends always take the shorter sweep, so an arc over 180° cannot be typed.
- UI tests do not cover the parameter table's add, rename and delete, the extent, result, body,
  sketch and axis combos, the datum panel, or Move up and down.

## Sketching

- The arc end snaps to a target not on its circle and keeps the `Coincident`, so the solver
  moves it away from the preview; the circle rim shows "On Line N" but adds nothing.
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
- Typed coordinates are not range-checked, so points far beyond `MODEL_EXTENT` enter the model.
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
- Revolve cannot keep the part of a region on one side of the axis.
- No live preview of a fillet, chamfer or shell while its panel is open, and no viewport handles
  for extents.
- Parameters cannot be reordered or given a note, and a disabled delete does not say what uses
  the parameter.

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
  blocks Open behind a modal, and the opening modal is drawn before the unsaved-changes prompt,
  so closing the window during a load hides the prompt until the load ends. Give imports their
  own cancellable job. When a worker thread cannot be spawned the job runs on the UI thread.
- Dropping a file on the window does nothing (`app.rs` handles no `DroppedFile`).
- Window size, position and panel state are not remembered.
- One document per process.
- No clipboard for sketch geometry or features, no parameter import or export.
- Version history shows only relative times.
- No localisation.
