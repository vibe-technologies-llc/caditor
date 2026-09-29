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

- Every save rewrites and fsyncs the whole version history (`binary/model.rs`); where the file
  system can share extents (`copy_file_range` on Btrfs or XFS), clone the unchanged versions from
  the previous file instead of writing them again.

## Kernel correctness

- Shell cannot drop collapsing cones or faces whose edges run neither around nor along their
  axis, and cannot split a vertex whose edges are partly convex and partly concave (both
  reported).
- Opening a face of a void moves it into the void, so the void's side walls stop the thickness
  short of the opening instead of reaching it (`shell/mod.rs` `extendable`).
- Tests missing for a boolean that fails with `Split`, `Intersection` or `Invalid` on real
  solids; only their conversions and messages are tested.
- Even-depth selection keeps the region inside a hole that touches its outline, since both are in
  one connected component, so such a sketch sweeps as if it had no hole until the region is
  chosen.

## Kernel performance

- B-spline surface projection of a point off the surface refines four seeds every call, which is
  most of its cost.

## Document and recompute

- Undo entries keep removed features alive, imports with their solids and STEP text included;
  `MAX_UNDO_STEPS` bounds how many there are, not how large they are.

## Sketch solver and expressions

- A solve that fails with no identifiable conflict reports "could not be solved from its current
  shape" with no entity and no next step (`diagnose_failure` returning `Unsolvable`).
- Conflict diagnosis re-solves each probe over the whole failed scope with no probe budget, and
  treats a subset that merely fails to converge from the fixed start as inconsistent, so large
  sketches diagnose slowly and a reported conflict can be wrong or not minimal.
- The solve memo key includes the global scale (the largest coordinate or dimension), so editing
  the outermost dimension or dragging the outermost point misses the memo for every part.
- Each tangent constraint rebuilds the coincidence union-find and scans all constraints twice
  (`system.rs` `joint`, `points_on`), quadratic in tangents per solve.
- `10 mm^2` means (10 mm)², since a power binds to the measure before it; stored text relies on
  that reading, so changing it needs a new spelling or a format change.
- `BSpline::through` returns `None` on consecutive equal points (singular collocation); only
  tests use it today.
- Tests missing for: dense and sparse paths giving the same rank and fixed sets across the
  48-variable boundary; minimal conflict sets and two independent conflicts in a large part;
  arcs with tangent, equal and angle constraints; a scale-changing edit; expressions at
  `MAX_TREE_DEPTH` evaluated, printed and dropped on a worker's default stack.

## STEP import and export

- One unrecognised byte or an integer beyond i64 rejects the whole file, a repeated entity id
  silently overwrites the earlier one, and `\P` and `\X\` followed by a multi-byte character
  drop the rest of the string (`part21.rs`).
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
- HATCH boundaries, SOLID, TRACE, 3DFACE and MLINE are dropped; HATCH boundaries are often the
  only closed profile.
- An INSERT array above 20,000 cells refuses the whole import before checking whether its block
  draws anything.
- Fit-point splines ignore their end tangents.
- `$DWGCODEPAGE` is ignored, so non-UTF-8 names come out garbled.
- `Nurbs::point` finds the knot span linearly for each of up to 16,384 samples, and each visited
  entity rescans its record for codes 67, 60 and 8.
- Parsing keeps an owned `String` per value and clones every record, two to three times the
  file size, with no size limit.

## Mesh export

- STL uses absolute f32 coordinates, which lose about 0.06 mm at 10^6 mm, and merges all bodies
  into one surface.
- 3MF has no colours, materials or thumbnail, builds everything in memory and cannot exceed
  4 GiB without ZIP64.

## Rendering robustness

- Faces without a pick id are discarded in `fs_pick` and write no depth, so while a sketch is
  edited, or a fillet or shell panel chooses edges or faces, edges and curves behind the solid
  are hovered and clicked through it, contrary to "hidden in the view and in picking alike".
  Write id 0 with depth instead of discarding.
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
- Constraint rows in the sketch tree are styled as links but clicking or hovering them does
  nothing.
- A rejected `commit_field` draft keeps showing its old text and error after the stored value
  changes through undo.
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
