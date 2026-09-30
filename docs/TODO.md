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

- Slow tests to keep an eye on: about 40 UI tests at over a second each.

## Kernel correctness

- Shell cannot split a corner whose offsets do not meet when its convex and concave edges
  alternate (two ridges of different slopes crossing) or one convex edge meets concave ones (a
  cavity whose ridge runs over its inside corner): the offset there joins faces the body keeps
  apart, or runs an edge between another pair of faces, which splitting the corner into several
  cannot give. Corners of more than eight faces are not split either. All are refused as
  `Corner` (`shell::tests::a_corner_where_ridges_and_valleys_alternate_is_named`,
  `a_cavity_whose_ridge_runs_over_its_inside_corner_is_named`).
- A face the shell's thickness closes up is dropped only when it has one loop and keeps two
  single edges apart from each other, or none; a band whose side is a chain of edges (a rim split
  by another face's seam) is refused as `EdgeCollapses`.
- About 2% of booleans between the fixture solids in random placements still fail
  (`boolean::tests::random_placements_of_every_fixture`, ignored, best run in release): mostly
  `Open` and `Ambiguous`, then nearly coincident tori and cones that are too intricate to
  intersect. Intersection curves crossing at a tangent point (tori touching along their
  equators, a face touching a torus's inner equator) cannot be split, and a result whose pcurves
  stray past the resolution (a cylinder against a tilted torus) or with a lump too thin for the
  validation mesh is refused as invalid; each of the three has a test pinning its error.

## Sketch solver and expressions

- Conflict diagnosis confirms each constraint of a conflict with a Gauss–Newton step over the
  whole part, so a conflict running through a part of more than about five hundred entities still
  runs out of budget and is reported as not solving; one factorisation of the Jacobian, updated
  per constraint left out, would make each confirmation cheap.
- When conflict diagnosis finds that a part which failed from its drawn shape holds after all (a
  chain whose line must fold back, reached from a solution of all but one constraint), the solve
  still fails; the solution found could be offered instead.
- `10 mm^2` means (10 mm)², since a power binds to the measure before it; stored text relies on
  that reading, so changing it needs a new spelling or a format change.

## STEP import and export

- Unsupported entities: an `OFFSET_SURFACE` of a spline, extrusion or revolution (it would need a
  surface fitted within tolerance), and colours and layers.
- Healing covers only edges with exactly two faces, and faces that meet only within a coarse
  declared precision are refused rather than refitted to each other.
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

## Interface performance

- Snapping projects every point and curve of the sketch on every hover frame (`snap.rs`), about
  1 ms for 20,000 lines in a release build, most of it walking the entities, and a line or slot
  end walks every line again to find the nearest for parallel and perpendicular inference
  (`drawing.rs` `guides`); a screen-space index would need the preimage of the snap radius on the
  sketch plane, unbounded near the horizon.
- An expanded sketch in the tree formats and evaluates every constraint every frame with an
  O(n²) `involved` check; virtualise and cache.
- The cached scene is one batch: any change to its content (each drag solution, an edit, an
  evaluation, a new faceting level) facets every drawn sketch again, and a hover or selection
  change restyles and uploads all of it, about 1.2 ms to rebuild and 0.5 ms to upload for a
  sketch of 24,000 curves in a release build. A batch per feature, with pick ids of its own,
  would limit both to what changed.

## Sketching

- No projection of model edges or other sketches into a sketch, and bodies and other sketches
  are unpickable while editing.
- Tools missing: offset, mirror, sketch fillet and ellipse.
- Snapping has no midpoints, intersections, spline targets, grid or inference lines to other
  points, and dragged geometry does not snap at all.
- Dimensions all sit at one fixed offset, so collinear chains overlap, and labels cannot be
  dragged.

## Modelling features

- Feature kinds missing: mirror (the kernel has no reflecting transform), hole, draft, sweep,
  loft, split, and move or copy body.
- Patterns repeat a whole body: no pattern of chosen features (a row of holes cut into a plate),
  no instances left out, no pattern along a curve or driven by sketch points, and a copy's faces
  are described as the face they copy.
- Extrusions end only on flat faces and planes: up to face and up to next refuse a curved face,
  and up to next needs one flat face that the whole profile meets first.
- No feature combines two existing bodies, and a cut affects only one body.
- Several features chosen in the tree cannot be dragged together; each moves on its own.
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

- Silhouette edges on curved bodies.
- Section planes.
- Transparent or X-ray bodies.
- Line caps, joins and anti-aliasing without MSAA.
- A selection filter, so a click takes only faces, edges, vertices or sketch geometry.

## Application

- The modelling tools borrow Phosphor glyphs that mean something else (`icons.rs`): fillet is the
  full-screen corners, chamfer a generic polygon, revolve the refresh arrows, circular pattern a
  loading spinner, shell a see-through cube. Draw caditor's own icons for fillet, chamfer, shell,
  extrude, revolve and both patterns, on Phosphor's grid and stroke weight so they sit beside it;
  undecided whether they ship as glyphs added to the `icons` font family or as painted shapes.
- One files worker runs everything and Open and Import cannot be cancelled, so a slow STEP import
  blocks Open behind a modal, and the opening modal is drawn before the unsaved-changes prompt,
  so closing the window during a load hides the prompt until the load ends. Give imports their
  own cancellable job. When a worker thread cannot be spawned the job runs on the UI thread.
- Dropping files on the window works only under X11, since winit 0.30 has no drag and drop on
  Wayland, and nothing shows where a drop will go while files are dragged over the window.
- One document per process.
- No clipboard for sketch geometry or features, no parameter import or export.
- No localisation.
