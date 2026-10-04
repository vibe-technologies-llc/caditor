# caditor roadmap

## Direction

caditor is a parametric CAD application. The project is judged on two things before
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

- `tests/crash_flush.rs` runs the crash protection with a real storage worker in a child process,
  not the app itself. `check-install.sh` only runs `--version`; start the packaged binary to a
  first frame under Xvfb and lavapipe, kill it there and recover its journal, check its linked
  libraries and highest glibc symbol against `docs/RELEASING.md`, and run the offscreen tests once
  more on the GL backend that `packaging/INSTALL.md` promises.
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
- About 0.7% of booleans between the fixture solids in random placements still fail
  (`boolean::tests::random_placements_of_every_fixture`, ignored, best run in release): mostly
  `Open` between extruded splines, frustums and tori, then nearly coincident tori and cones that
  are too intricate to intersect, and one `Ambiguous` between the holed block and an extruded
  spline. Intersection curves crossing at a tangent point (tori touching along their equators, a
  face touching a torus's inner equator) cannot be split, and a result whose pcurves stray past
  the resolution (a cylinder against a tilted torus) or with a lump too thin for the validation
  mesh is refused as invalid; each of the three has a test pinning its error.
- Faces or axes apart by more than `LINEAR_RESOLUTION` but by less than a few micrometres are
  neither coincident nor separate: coaxial cylinders of radii 5 and 5 + 2e-6, a plug offset 1e-5
  in its bore, or blocks of heights differing by 5e-6 fail as `Open`, `Ambiguous` or
  `Invalid(LoopOrientation)`, which sloppy STEP imports will hit; 246 of 900 booleans of blocks
  and cylinders 1e-6 to 1e-4 mm off an aligned contact with a long plate fail
  (`boolean::tests::aligned_contacts_a_micrometre_or_so_apart`, ignored). `SAME_EDGE`,
  `NEAR_BOUNDARY`, `PCURVE_TOLERANCE` and the coaxial offset are unrelated absolute values; derive
  them from one tolerance model and snap or refuse within a documented band.
- `same_surface` samples a 7×7 grid, so a spline patch with a bump narrower than a seventh of
  it is declared coincident with a plane and booleans treat it so.
- A torus whose tube is far thinner than its ring (20 and 0.5) still meshes at 2.7 to 4.4 times
  the requested chord, since the Delaunay triangulation of the stretched parameter grid picks long
  triangles; mesh it finer or triangulate by cell.
- Up to next samples at most about 256 rays over the profile, so a feature covering under about
  1% of it is never seen and the extrusion passes through it to the far plane without a word.
- Filleting both rims of a cylinder 10 tall at 4 fails as `Boolean(Ambiguous)` although its
  feet do not cross; the tests use 2 instead.
- Near-duplicate lines in a profile make phantom sliver regions, because a face counts as real
  when its area exceeds tolerance² though a sliver thinner than the tolerance can be far larger;
  judge it by its width.
- Pattern copies that touch only along a line or at a point fail the union as `NonManifold`, so
  round parts spaced one diameter apart cannot be patterned; keep such copies as separate shells
  of one body, and name the copies in `PatternError::Union`.
- The spline-surface projection seed grid is capped at 48 samples per direction, so on dense
  imported nets an on-surface point's foot is missed (28 in 600 at 60×60 control points).
- `select::classify` lets inside or outside samples win over coincident ones in a partly
  coincident fragment (only coincident samples of opposite senses make it `Ambiguous`); treating
  every partly coincident fragment as `Ambiguous` fails `stress_cylinders_on_a_grid` on noise near
  tolerance boundaries, so split fragments exactly at coincident boundaries instead.
- Meshes fold where two faces meet at a very small dihedral (lens tips, a plane nearly tangent to
  a torus), giving self-overlapping triangles that `validate` does not see.

## Kernel feedback

- `BooleanError::Split`, `Open`, `Ambiguous`, `NonManifold` and `Intersection` carry no data, and
  the document reduces five of them to one message blaming "faces or edges that exactly touch",
  wrong for the near-coincident cases. Carry the face or edge names, or a model point, so the
  error can name and highlight them.
- `SweepError::Invalid` cannot say which region failed, since all regions build in one `Plan`.
- `ShellError::Walls` still names nothing when the offset solid fails to build, and a blend's
  failing pairwise union of tools or corner names no edge.

## Kernel performance

- Every boolean clones, splits, reselects and revalidates every face of the body even when the
  tool touches two, so a sequence of hole features is quadratic (43 ms for the 144th hole). Carry
  untouched faces through by id, and return disjoint operands without the pipeline: a pattern of
  1,600 separated copies takes 2.9 s.
- The face grid is uniform in uv and sized by the worst curvature anywhere, so one small bump
  multiplies a whole face's triangles, and straight directions are capped at `FLAT_ASPECT` times
  the curved one, so a 1×1000 cylinder gets 121k triangles where 120 would do.
- Blending scales worse than linearly: `crosses_boundary` tests every boundary edge with no box
  filter and recounts uses inside the loop, and tools are unioned pairwise even when disjoint.
- A boolean with an extruded spline of 6,000 control points runs 0.8 s without polling
  `interrupt::check`: clipping the branch along its cap (`surface_surface::clip`) projects each
  probe onto the extrusion from the patch centre, a search over the whole profile, where the
  previous probe's foot would do. Each `Curve::length` of that spline also takes 70 ms unpolled,
  and the builder, validation and boolean tracing each measure it again.
- Marched curves' `closest_parameter` and `length` reseed over all nodes on every call, from
  loops over nearby vertices in `imprint.rs`.

## Document and recompute

- A wedged recompute cannot be recovered: `cancel` only flips an atomic, the worker's
  `JoinHandle` is not kept, `Action::Recompute` submits to the same thread, and dropping a
  `Recomputer` does not cancel its job. Orphan a stuck worker after a grace period and start a
  fresh one, and report which feature is running and for how long, since `Progress` is only a
  count.
- A recompute reports once more after its feature loop only; a slow late feature still hides the
  bodies before it, and each mesh is not reported as it finishes. Send an update per feature and
  per mesh.
- The cache keeps one result per feature, so changing a depth and undoing recomputes everything
  after it; it also has no byte budget, holding every intermediate `Solid`. Keep a small,
  size-bounded history per feature.
- Recompute is single-threaded: independent bodies and the final meshing of each body could run
  in parallel over the dependency data the document already has.
- `SetFeatureKind` refuses an import although `document.md` says an import stays an import;
  allowing it would also give re-import.

## Sketch solver and expressions

- The rank and null-space analysis (`analyze_sparse`, `Echelon::spans_unit` once per column) is
  near cubic on closed chains and never checks `cancelled`: solving the sketch left by offsetting
  a closed 3000-line chain takes 218 s, and Cancel does nothing meanwhile. Use a sparse
  factorisation with a fill-reducing order, and poll inside `analyze_component`.
- A drag frame solves geometry only (`solve_geometry_from`, no rank or degrees-of-freedom analysis),
  but the dragged part is still never memoised and the solve itself is the cost: dragging an end of
  a fully dimensioned chain of 2,000 lines to a point it cannot reach takes about 7 s a frame in a
  release build (the analysis was about 1 s of it), where a chain joined only by `Coincident`
  takes 15 ms.
- `components()` is rebuilt with `BTreeMap`s several times per solve, so solving stays heavier than
  linear in independent parts: 6,400 dimensioned rectangles take 0.42 s (1.4 s before each part
  kept its own spans), and a warm re-solve recalling every part costs about the same
  (`thousands_of_independent_rectangles_solve_in_a_fraction_of_a_second`, ignored, in release).
- Conflict diagnosis confirms each constraint of a conflict with a Gauss–Newton step over the
  whole part, so a conflict running through a part of more than about five hundred entities still
  runs out of budget and is reported as not solving; one factorisation of the Jacobian, updated
  per constraint left out, would make each confirmation cheap.
- When conflict diagnosis finds that a part which failed from its drawn shape holds after all (a
  chain whose line must fold back, reached from a solution of all but one constraint), the solve
  still fails; the solution found could be offered instead.
- A sketch solved from a degenerate start can fail to solve again from its own result: a spline
  with four coincident control points, tangent to a zero-size arc on one of them, with a zero
  distance from that arc to the spline's first point. `sketch_solve` finds such cases within
  minutes once it requires `solve_from` of a solved geometry to succeed; it does not yet, so the
  property is unchecked.
- A point on a line segment or arc is held to the infinite line or full circle, so it can solve
  beyond the segment's ends or outside the sweep, and the line rotates to meet it; bound it or
  say so.
- `10 mm^2` means (10 mm)², since a power binds to the measure before it; stored text relies on
  that reading, so changing it needs a new spelling or a format change.

## STEP import and export

- Healing covers only edges with exactly two distinct faces, so real Fusion 360 exports are
  refused whole over a vertex a few micrometres off its edge though the file declares 0.01 mm: a
  cylinder seam (one face, used twice) or the line where two half-cylinders meet cannot be
  rebuilt by `IntersectionCurve::through`. Project the vertices onto lines, circles and ellipses
  within the declared precision instead. Faces that meet only within a coarse declared precision
  are refused rather than refitted to each other.
- One unsupported surface or curve loses the whole body: an `OFFSET_SURFACE` of a spline,
  extrusion or revolution (it would need a surface fitted within tolerance), `PARABOLA`,
  `HYPERBOLA` and the `*_REPLICA` forms. Fit a spline within the declared precision, or keep the
  other faces and say which were lost. Colours and layers are not read.
- Import canonicalises each placement by writing and re-reading it, and stores every placement of
  a product as its own STEP text. Build each representation once and store each product once with
  placements.
- A file with no closed solids always reads "holds no solid bodies", whether it is IFC,
  tessellated AP242, a surface model or a wireframe; report the schema and what it holds.
- The parse tree still holds about three times the file size (a boxed slice per record and per
  list); a flat arena of values would bring it near the file size.
- The writer puts all bodies in one product with no colours, holding the output twice in memory.
- Placements that scale or mirror are left out with a note; a uniform scale could be applied, and
  a mirror once the kernel can reflect.
- Imports cannot be positioned (`Import` has no placement) or refreshed from their source file:
  the path is not kept, so a changed STEP file means deleting the feature and breaking what
  references it.
- No IGES import or export, though older CAM software and many suppliers still exchange it.

## Drawing import and export

- Sketches cannot be exported: there is no DXF or SVG output of a sketch or flat face, though
  laser and CNC work need it and `DrawingCurve` already models what it would write.
- DXF import has no options: units come only from `$INSUNITS` and `$MEASUREMENT` with no override
  or scale (templates commonly default to inches), coordinates are not recentred, and a new
  sketch always lands on the XY plane although `SketchTarget::New` takes a plane.
- Curves carry no layer, so the import cannot offer a layer choice, and curves past the first
  20,000 are left out in drawing order rather than by any choice of the user.

## Mesh import and export

- No mesh import (STL, 3MF, OBJ), though the STEP reader already builds `FACETED_BREP`s from
  polygons; it needs the STEP storage item above first. Importing a mesh should give a solid that
  edits like one drawn in caditor, where Fusion and FreeCAD leave thousands of triangle faces that
  fillets, sketches, holes and booleans choke on. The aim, to be researched before design:
  - Repair on import without asking: weld duplicate vertices, close small holes, fix flipped and
    inconsistent normals, split or drop non-manifold and degenerate pieces, and separate shells
    into bodies, saying in the import report what was fixed and what could not be.
  - Rebuild real faces: group triangles into regions and recognise planes, cylinders, cones,
    spheres and tori within a tolerance the import chooses from the mesh's own noise (adjustable
    with a live preview), fit splines to what remains, and join them into a B-rep whose faces and
    edges are named, so a flat side takes a sketch, a hole edge a fillet and a bore its axis.
  - Fall back gracefully: what cannot be fitted stays faceted but still part of a valid solid that
    booleans, shells and direct edits (Modelling features) accept, so the import never fails as a
    whole over a bad region.
  - Stay quick on scans and printer files of millions of triangles, in the background with
    progress and Cancel, and keep the source mesh in the model so the conversion can be redone at
    another tolerance later without breaking what references its faces.
  - Units: STL has none, so the import guesses from the size (a part 0.05 mm across is likely in
    metres) and offers a scale before committing, rather than leaving it to the scale item.
- STL uses absolute f32 coordinates, which lose about 0.06 mm at 10^6 mm, and merges all bodies
  into one surface.
- 3MF has no colours, materials or thumbnail, builds everything in memory and cannot exceed
  4 GiB without ZIP64.
- No OBJ or glTF export, so models cannot go to renderers, game engines or web viewers without
  another tool.

## Interface performance

- With a large sketch selected, every constraint tool rebuilds its candidates each frame.
- Snapping projects every point and curve of the sketch on every hover frame (`snap.rs`), about
  1 ms for 20,000 lines in a release build, most of it walking the entities, and a line or slot
  end walks every line again to find the nearest for parallel and perpendicular inference
  (`drawing.rs` `guides`); a screen-space index would need the preimage of the snap radius on the
  sketch plane, unbounded near the horizon.
- Every frame `Marks::collect` formats every constraint's description, evaluates every dimension
  and registers an `interact` per glyph, and an expanded sketch in the tree does the same with an
  O(n²) `involved` check; the `large_sketch` benchmark has no constraints. Cache per generation,
  cull off-screen marks and virtualise the tree.
- The cached scene is one batch: any change to its content (each drag solution, an edit, an
  evaluation, a new faceting level) facets every drawn sketch again, and a hover or selection
  change restyles and uploads all of it, about 1.2 ms to rebuild and 0.5 ms to upload for a
  sketch of 24,000 curves in a release build. A batch per feature, with pick ids of its own,
  would limit both to what changed. Face styles are likewise rewritten whole on every highlight
  change.
- Zooming in five times reanchors and re-uploads every batch, since the anchor reach is four view
  distances; derive it from the f32 error budget.
- Meshes are uploaded whole on the UI thread in the frame that first shows them, and never
  culled in the main or pick pass although each keeps its bounds; the frame-cost benchmark has no
  meshes, picking or hover.
- Image export submits every tile's readback buffer at once and assembles the image beside them,
  about 540 MB at 8192², contrary to `render.md`'s bounded memory; reuse a few buffers and stream
  bands to the encoder.
- A pick or image readback in flight redraws full frames until polled complete; vertex records
  repeat per-layer data and both ends of shared segments; invisible vertex markers go through the
  colour pass; each mesh's placement uniform is written every frame; resizing recreates the MSAA
  targets per pixel; and an MSAA change rebuilds all ten pipelines with no pipeline cache.
- The feature tree lays out every row each frame, which a STEP import of hundreds of bodies
  makes long.

## Sketching

- No projection of model edges or other sketches into a sketch, and bodies and other sketches
  are unpickable while editing.
- No reference image: a photo or scan cannot be placed on a sketch plane, scaled by two points and
  traced, as a part copied from an existing object or a drawing needs.
- Every dimension drives: there are no reference (driven) dimensions and no way to disable a
  constraint, so dimensioning determined geometry adds a redundant constraint instead of a
  measurement.
- Typed lengths and angles (`@40, 20`, `25 < 30`, `width / 2, 10`) are evaluated once and place
  free points, keeping neither a dimension nor the parameter link; offer to create the
  dimensions.
- A constraint that fails only once solved (one contradicting the sketch through other
  constraints) is still accepted and reported afterwards; trial-solve it off the UI thread before
  committing.
- A drag to a position with no solution freezes the geometry without a cue, and in a conflicting
  sketch every drag does nothing and then blames the move.
- Press-drag-release with a drawing tool places only the release point; place the press point as
  the start too, so one gesture draws a line, rectangle or circle.
- Tools missing: ellipse (a new entity kind across the solver, kernel and file format), sketch
  chamfer, rectangular and circular patterns, rotate, scale and copy of a selection, split at a
  point, text, and fit-point, closed or periodic splines (`BSpline::interpolate` and `through`
  serve only DXF import, and the control polygon is not drawn).
- Splines cannot be trimmed or extended (`TrimError::Spline`), trim ignores collinear and
  co-circular overlaps as cutters, and the sketch axes are not cutters. Offset takes one chain at
  a time, leaves the free ends of an open chain sliding along their curves and cannot offset
  splines; a sketch fillet cannot round a spline and drops equal lengths and midpoints of the
  lines it shortens, as trim does.
- Constraint kinds missing: distance between circles, line and circle, or to a spline; arc length
  and sweep; angle or perpendicular to an arc; arc midpoint; equal splines; spline–spline
  tangency; curvature continuity; symmetric curves. Coincident, perpendicular and tangent take
  exactly two items where parallel and equal chain.
- Spline intersections sample sign changes, so a near-tangent crossing between samples is missed.
- No live length, size or radius readout while drawing, no closed-region or open-end feedback
  while sketching, and no smart-dimension tool that takes the entities after the command.
- Snapping has no midpoints, intersections, spline targets, grid or inference lines to other
  points, cannot be suppressed by a modifier or toggle, and dragged geometry does not snap at
  all.
- Dimensions all sit at one fixed offset, so collinear chains overlap, and labels cannot be
  dragged. Glyphs stack uncapped (a 64-gon puts 63 `=` glyphs on its first side) and cannot be
  hidden.
- A tangent arc cannot continue a line chain without switching tools; the polygon side count
  changes only by one per key.

## Modelling features

- Feature kinds missing: mirror (the kernel has no reflecting transform), hole, draft, sweep,
  loft, split, rib, emboss or deboss of sketch text onto a face, and move, copy or scale of one
  body.
- The whole model cannot be scaled: no command or feature resizes every body, sketch and datum by
  a factor (uniform, about the origin or a chosen point) as one undoable change. Scaling must
  keep references and names stable, and say what happens to dimensions and parameters (scale the
  stored values, or the parameters they use, or leave expressions alone and scale only plain
  values), so a part drawn at the wrong size or an import in the wrong unit can be fixed without
  redrawing it.
- Bodies cannot be edited directly: no moving, offsetting, deleting or replacing a face (push and
  pull) and no deleting a fillet or chamfer by its faces. An imported STEP body has no feature
  history, so today it can only be cut, joined, filleted or shelled; a wall too thick, a hole in
  the wrong place or a fillet to remove means remodelling it from scratch. Direct edits become
  features of their own, named from the faces they move, so they stay parametric and undoable.
- Datums cannot be built from points: no datum point, plane through three points, mid-plane,
  plane through an axis and a point, plane normal to an edge at a point, or axis through two
  points. Model vertices are named and pickable but only the measure tool uses them, and datums
  and pattern axes cannot take sketch geometry.
- No helix or spiral curve and no threads: springs, coils and threaded holes and shafts cannot be
  modelled, and the hole feature (above) has no ISO metric thread sizes or cosmetic thread to show
  a thread without modelling it. Sweep (above) needs the helix for modelled threads.
- Extrusions always start on the sketch plane, with no start offset or face, taper angle or thin
  wall, though `LinearExtent::between` accepts any bounds; revolve has the same gaps.
- Patterns repeat a whole body: no pattern of chosen features (a row of holes cut into a plate),
  no instances left out, no pattern along a curve or driven by sketch points, and no linear
  "total length" mode.
- Extrusions end only on flat faces and planes: up to face and up to next refuse a curved face,
  and up to next needs one flat face that the whole profile meets first.
- No feature combines two existing bodies, and a cut affects only one body.
- Bodies have no colour, material or density, so a multi-body model is one grey until picked and
  mass cannot be shown; this blocks coloured STEP and 3MF export.
- Mass properties are volume, area and centroid from the display mesh, biased low on curved
  bodies, with no mass, inertia, bounding size or total. Integrate exactly over the trimmed
  faces, as `planar_area` already does for planes.
- Blends: only line and circle edges along planes, parallel cylinders and coaxial surfaces; no
  ellipse, spline or intersection edges, not even a straight edge beside a spline extrusion face;
  ends at steps and T-junctions refused; no variable radius, two-distance or distance-angle
  chamfer; corners only for three convex straight edges.
- Shell: no spline, extrusion or revolution faces, only line and circle edges, only flat faces
  open, one thickness for the whole body.
- Revolve cannot keep the part of a region on one side of the axis.
- Several features chosen in the tree cannot be dragged together; each moves on its own.
- No live preview of a fillet, chamfer or shell while its panel is open, and no viewport handles
  for extents.
- Parameters cannot be reordered or given a note, show what uses them only in the value's tooltip
  and mark no unused one in the table, cannot be deleted by inlining their value, and expressions
  cannot refer to measured values or sketch dimensions.
- No configurations: a model holds one set of parameter values, so sizes of one part (a bracket in
  M4, M6 and M8) are separate copies of the file. Named parameter sets, chosen as a whole and kept
  in the model like versions, with export of each.
- The measure tool cannot take planes, axes, datums or sketch curves, so a hole axis to a datum
  or a circle's radius cannot be measured.
- No interference check: nothing finds where two bodies overlap or touch, or reports the
  overlapping volume, though booleans already compute it.

## Technical drawings

- No 2D drawings at all: no sheet with a title block, no projected front, top, side and isometric
  views of the bodies, no section or detail views, no dimensions or notes taken from the model, and
  no PDF, SVG or DXF output of a sheet, though parts made for a workshop need one. Views update
  with the model and their dimensions refer to edges by name, so they survive edits as features
  do. Hidden-line removal (also wanted for the Viewer's display styles) comes first.

## Viewer

- Display styles: wireframe, hidden line and shaded without edges, plus isolate or hide others
  and look normal to a face.
- Silhouette edges on curved bodies.
- Section planes.
- Transparent or X-ray bodies.
- Line caps, joins and anti-aliasing without MSAA.
- A selection filter, so a click takes only faces, edges, vertices or sketch geometry.
- No box or lasso selection in the 3D view (only inside a sketch), no select all, and no selecting
  an edge's tangent chain or a face's loop outside the fillet panel, so choosing many faces or
  edges for a pattern, shell or export means clicking each one.
- Edge lines can be eaten by faces at grazing angles, since depth bias is a constant factor with
  no slope term, and the grid and reference fills share the mesh's bias, so a face on the XY
  plane can speckle with the grid. Neither has a test.
- No touchpad navigation: orbit is right-drag, pan needs a middle button or Shift, and two-finger
  scroll always zooms.
- Adapter choice is only the `WGPU_POWER_PREF` environment variable, so users of hybrid laptops
  or broken drivers cannot pick another adapter from Preferences.
- Lighting and the MSAA resolve happen in gamma space; the model keeps no saved view.

## Accessibility

- Everything drawn in the viewport is invisible to screen readers: the keyboard highlight
  description, tool prompts, snap labels and measure labels are painter text, and the viewport is
  an unnamed `interact`. The cancelled and stopped recompute pills are not live regions.
- Constraints and dimensions are not scene pickables, so N never reaches them and a dimension can
  be re-edited only by double-click or from the tree; with a drawing tool active, Space toggles
  the selection instead of placing at the highlight, so keyboard drawing cannot start from
  existing geometry.
- High contrast reaches neither the scene colours nor the colour-only sketch states.

## Application

- The modelling tools borrow Phosphor glyphs that mean something else (`icons.rs`): fillet is the
  full-screen corners, chamfer a generic polygon, revolve the refresh arrows, circular pattern a
  loading spinner, shell a see-through cube, and the sketch fillet shares the fillet's. Draw
  caditor's own icons for fillet, chamfer, shell, extrude, revolve and both patterns, on
  Phosphor's grid and stroke weight so they sit beside it;
  undecided whether they ship as glyphs added to the `icons` font family or as painted shapes.
- Themes are four fixed `Tokens` sets in `appearance.rs` (dark, light and their high-contrast
  variants) and the 3D view is dark in all of them. Add themes as data: a few shipped ones
  beyond dark and light, a choice of accent colour, a light 3D view (background, grid, edges and
  the `canvas.rs` chrome) chosen with the theme or on its own, and user themes loaded from the
  config directory and picked in Preferences with a live preview. Every theme, shipped or loaded,
  goes through the contrast checks `appearance.rs` runs today (4.5:1, 7:1 for body text in high
  contrast), and a loaded theme that fails them or cannot be read is refused in words, naming the
  colour pair, with the previous theme kept.
- One files worker runs everything and Import cannot be cancelled (cancelling Open only drops its
  result while the worker reads on), so a slow STEP import blocks Open behind a modal, and the
  opening modal is drawn before the unsaved-changes prompt,
  so closing the window during a load hides the prompt until the load ends. Give imports their
  own cancellable job. When a worker thread cannot be spawned the job runs on the UI thread.
- The desktop entry registers only `application/x-caditor`, and the headless `--export` takes only
  caditor models: no DXF or STEP file as input, no PNG output, and it exits 0 though features failed.
- Text outside Latin, Greek and Cyrillic shows as missing glyphs in feature and file names, since
  only Inter and egui's defaults are loaded.
- Version history shows when a version was saved and after which change, but no preview of what it
  holds, and no way to keep a version from being thinned out.
- The undo history lists only each step's label, with no summary of what it changed (the
  entities, features or parameters touched).
- No user guide: Help has only the welcome, the tips and About. Nothing explains features, the
  parameter and expression syntax, or the file workflow, and no panel links to help on itself.
  A guide shipped with the app (and readable offline) with a page per tool, opened by F1 for the
  current tool or panel.
- No model properties: a model has no title, description, part number, revision or notes, so
  exports carry only the file name (the STEP header and 3MF metadata stay empty) and nothing
  identifies a part beyond its path. Empty unless the user fills them in.
- Bodies have no list of their own: a body appears only as the features that build it, so showing,
  hiding, naming or (once they exist) colouring a body means finding the feature that made it.
- The feature tree has no filter or groups.
- Angles display only in degrees though `ux.md` allows radians; core modelling commands have no
  default shortcuts.
- Dropping files on the window works only under X11, since winit 0.30 has no drag and drop on
  Wayland, and nothing shows where a drop will go while files are dragged over the window.
- One document per process.
- No clipboard for sketch geometry or features, no parameter import or export.
- No localisation.

## Scope decisions

These are open: each is a large direction the project has not committed to, and each needs a
decision recorded in `docs/` before work starts.

- Assemblies: a model is one part of several bodies, with no components, instances of another
  model file, joints or mates, exploded views or bill of materials, so a product of several parts
  cannot be put together or checked for fit. Decide whether caditor stays a part modeller, or how
  assemblies reference part files while keeping references stable across edits.
- Surface modelling: no surface bodies, so no thicken, offset surface, trim, extend, patch or
  knit to a solid, which shaped consumer parts and repairing open STEP imports need.
- Sheet metal: no flanges, bends with a bend allowance, or flat patterns, though laser-cut and
  bent parts are a common use; flat patterns would go out through the sketch DXF export.

## Platforms

Linux is the primary platform and Windows the only other one planned; macOS is not a goal.

- Linux only: there is no Windows build. Supporting Windows needs a Windows target in `ci.yml`,
  `release.yml` and `deny.toml`, and a release archive or installer. `caditor-file` is written
  against Unix: `os::unix` paths and file APIs in `journal.rs`, `recent.rs`, `recovery.rs`,
  `paths.rs`, `lock.rs`, `storage.rs` and `save.rs` (`fchown`, `rustix::fs::copy_file_range` and
  `access`), XDG config and state directories, and the atomic save that fsyncs the directory
  after the rename, which Windows cannot do (use `ReplaceFileW` semantics instead). The app uses
  `rfd`'s xdg-portal dialogs and `signal-hook` for the crash flush, both of which need Windows
  counterparts (native dialogs, a console control handler). The own title bar needs Windows snap,
  resize borders and DPI handling checked, the desktop entry, icons and MIME type need a Windows
  equivalent (file association, `.ico`), and the packaging scripts, `INSTALL.md` and
  `RELEASING.md` need a Windows section.
- On Windows, Microsoft Defender's real-time scanning slows the atomic saves, the recovery journal's
  frequent syncs and version history writes in the folders models live in. Remind the user, once
  and dismissibly (a callout on first save to a folder, repeatable from Preferences and the user
  guide), that they can exclude their models' working folder from Defender, saying what that
  trades away and how to do it; never change Defender settings ourselves.
- Linux has only the `.tar.zst` with its installer: no Flatpak, AppImage, `.deb` or `.rpm`, so
  caditor is not in software centres and installs never update themselves.
