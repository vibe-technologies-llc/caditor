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

- Slow tests to keep an eye on: STEP `every_fixture_survives_a_round_trip` (7 s),
  `blend::every_edge_of_assorted_prisms` (6 s) and about 40 UI tests at over a second each.

## Kernel correctness

- Shell cannot drop collapsing cones or faces whose edges run neither around nor along their
  axis, and cannot split a vertex whose edges are partly convex and partly concave (both
  reported).
- Tests missing for a boolean that fails with `Split`, `Intersection` or `Invalid` on real
  solids; only their conversions and messages are tested.

## Sketch solver and expressions

- Conflict diagnosis treats a subset that fails to converge from the starting shape as
  inconsistent, so a reported conflict can be constraints the solver merely could not reach
  together rather than a true contradiction. Probes also always start from the drawn shape, so a
  conflict spanning a whole large part costs a full solve per probe and runs out of budget.
- `10 mm^2` means (10 mm)², since a power binds to the measure before it; stored text relies on
  that reading, so changing it needs a new spelling or a format change.

## STEP import and export

- Unsupported entities: `OFFSET_SURFACE`, horn tori (a `DEGENERATE_TOROIDAL_SURFACE` whose tube
  just touches its axis), and colours and layers.
- The declared `UNCERTAINTY_MEASURE_WITH_UNIT` is ignored, and healing covers only edges with
  exactly two faces.
- Import canonicalises each placement by writing and re-reading it, parses every import again on
  each model load, journal replay and recovery scan, and stores every placement of a product as
  its own STEP text. Build each representation once, store each product once with placements,
  and cache solids by text digest.
- The parse tree still holds about three times the file size (a boxed slice per record and per
  list); a flat arena of values would bring it near the file size.
- The writer puts all bodies in one product with no colours, holding the output twice in memory.
- Imports cannot be positioned: `Import` has no placement.

## Mesh export

- STL uses absolute f32 coordinates, which lose about 0.06 mm at 10^6 mm, and merges all bodies
  into one surface.
- 3MF has no colours, materials or thumbnail, builds everything in memory and cannot exceed
  4 GiB without ZIP64.

## Rendering robustness

- GPU device loss is never handled: no device-lost callback, and the UI shares the dead encoder.
- The device uses default limits with no retry, so windows wider than 8192 px fail and GL or
  downlevel adapters (the mesh shader needs vertex storage) fail at startup; the high-performance
  preference also wakes discrete GPUs.

## Interface performance

- The whole scene is rebuilt every frame and cloned into `PickKey` for comparison, every line and
  fill vertex is rewritten, and fills are re-sorted (`app.rs` `build_scene`, `viewport.rs`).
  Cache geometry per result and key picks on a generation.
- Body mesh conversion and GPU upload run on the UI thread (`bodies.rs` `BodyMesh::build`),
  against `ux.md`. Build `ShadedMesh` and edge polylines on the worker.
- A DXF drawing of up to 20,000 curves is turned into a transaction and applied on the UI thread
  (`import.rs`, `drawing_transaction`); only parsing is on the worker.
- Snapping collects every point and curve of the sketch into new vectors and projects onto each
  on every hover frame (`snap.rs`).
- `displayed_sketch` clones and compares whole sketches five or more times per frame, and
  `model_bounds` polylines every sketch twice.
- `find_face` rebuilds a map over every face on each call, and the toolbar, status bar,
  selection and panels call it and capture references for each selected face every frame.
  Cache a `FaceKey` index per result and compute offers when the selection changes.
- An expanded sketch in the tree formats and evaluates every constraint every frame with an
  O(n²) `involved` check; virtualise and cache.
- Sketch curves are faceted at a fixed 3° regardless of size or zoom.

## Sketching

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
- Parameters cannot be reordered or given a note.

## Viewer

- The principal planes, axes and origin cannot be hidden.
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
- Dropping files on the window works only under X11, since winit 0.30 has no drag and drop on
  Wayland, and nothing shows where a drop will go while files are dragged over the window.
- Window size, position and panel state are not remembered.
- One document per process.
- No clipboard for sketch geometry or features, no parameter import or export.
- No localisation.
