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

## Sketch solver

- No incremental solve: every edit re-solves and re-analyses the whole sketch. Solve only the
  parts an edit touches, which dragging in the viewport will need at interactive rates.
- Untested: branch keeping under perturbation.

## Kernel correctness

- A grid point on the same ruling as a cone apex makes a zero-area triangle there, whose normal
  points anywhere (`tessellation/face.rs`); it shows once the cone's grid is refined, which is
  why measured refinement is limited to spline, revolution and extrusion faces. Keep grid points
  off the apex's rulings or drop such triangles without leaving a T-junction.
- Shell fails on convex faces with a radius below the thickness and at vertices of four or more
  faces whose offsets do not meet (both now reported as such). Drop collapsed faces, and split
  such vertices into edges.
- Spline surfaces that coincide over only part of their extent are not seen as coincident
  (`coincidence.rs` samples the whole domain), so booleans march between identical surfaces.
- Validation does not check faces or loops against each other, so self-intersecting imports pass
  and fail later in booleans.
- Untested: spline surfaces in intersections, booleans, blends and shells; import edge naming;
  every blend and shell refusal branch.

## Kernel performance

- Booleans re-trace, re-fit and re-classify every face of both solids (`boolean/faces.rs`),
  including untouched ones that can still fail. Pass untouched faces through.
- Edge–face and face–face candidate pairs are tested all against all (`imprint.rs`). Add a BVH
  for face boxes.
- Blends run one full boolean per edge on a growing solid, and mixed selections analyse twice.
  Union the tools first.
- `heal_edges` rebuilds use and incidence maps for every join.
- B-spline surface projection of a point off the surface refines four seeds every call, and
  recomputes poles.
- A failing face re-meshes the whole solid up to five times, and validation retries that three
  times. Retry only the failing face.
- The profile arrangement is quadratic in curves (`arrangement.rs` `split` scans every event per
  source), about 1e9 comparisons at `MAX_DRAWING_CURVES`.
- Blend corners look up loops quadratically (`blend/corner.rs`).

## STEP import and export

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
- The writer puts all bodies in one product with no colours, holding the output twice in memory.
- Imports cannot be positioned: `Import` has no placement.

## File format and storage

- Version history is never pruned and every save rewrites and fsyncs all of it
  (`binary/model.rs`). Add a retention policy and write history append-only.
- A model whose records total more than 256 MiB saves once and never again, since the previous
  snapshot and the journal snapshot are each one chunk (`MAX_CONTENT`). Chunk per record.
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
- Fit-point splines ignore their end tangents, and a SPLINE with bad control points is refused
  even when it has fit points.
- `$DWGCODEPAGE` is ignored, so non-UTF-8 names come out garbled.
- Parsing keeps an owned `String` per value and clones every record, two to three times the
  file size, with no size limit.

## Mesh export

- STL uses absolute f32 coordinates, which lose about 0.06 mm at 10^6 mm, and merges all bodies
  into one surface.
- 3MF has no colours, materials or thumbnail, builds everything in memory and cannot exceed
  4 GiB without ZIP64.

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
