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
and resolved decisions are removed from this file, and a section disappears once it is empty.
Git history is the record of what was done.

Categories run from the most to the least important. Each entry is tagged
`[importance · ease]`, and within a category the entries that can start now come first, from the
most to the least important and from the easiest to the hardest. An entry that cannot start yet is
tagged `blocked by: <blocker>` with the specific item, decision or external fact, and follows
the unblocked ones; the entry that does the unblocking comes before it.

## Kernel correctness

- [high · hard] About 0.7% of booleans between the fixture solids in random placements still fail
  (`boolean::tests::random_placements_of_every_fixture`, ignored, best run in release): mostly
  `Open` between extruded splines, frustums and tori (once between a holed block and a cuboid), then
  `Intersection` failures between nearly coincident tori and cones that are too intricate to
  intersect, and one placement of the holed block and an extruded spline that is `Ambiguous`.
  Intersection curves crossing at a tangent point (tori touching along their equators, a face
  touching a torus's inner equator) cannot be split, and a result whose pcurves stray past the
  resolution (a cylinder against a tilted torus) or with a lump too thin for the validation mesh is
  refused as invalid; the first two have a test pinning their error
  (`tori_touching_along_their_equators_cannot_be_split`,
  `a_result_straying_past_the_resolution_is_refused_as_invalid`), the thin lump has none.
- [high · hard] Faces or axes apart by more than `LINEAR_RESOLUTION` but by less than a few
  micrometres are neither coincident nor separate: coaxial cylinders of radii 5 and 5 + 2e-6, a plug
  offset 1e-5 in its bore, or blocks of heights differing by 5e-6 fail as `Open`, `Ambiguous`,
  `NonManifold` or `Invalid(LoopOrientation)`, which sloppy STEP imports will hit; 246 of 900
  booleans of blocks and cylinders 1e-6 to 1e-4 mm off an aligned contact with a long plate fail
  (`boolean::tests::aligned_contacts_a_micrometre_or_so_apart`, ignored). `SAME_EDGE`,
  `NEAR_BOUNDARY`, `PCURVE_TOLERANCE` and the coaxial offset are unrelated absolute values; derive
  them from one tolerance model and snap or refuse within a documented band.
- [medium · medium] `same_surface` samples a 7×7 grid, so a spline patch with a bump narrower than a
  seventh of it is declared coincident with a plane and booleans treat it so.
- [medium · medium] Up to next casts about 256 rays over the profile (spread by area, at most
  1,024), so a feature covering a small fraction of it (below roughly 1%) can fall between the rays
  and the extrusion passes through it to the far plane without a word.
- [medium · medium] Filleting both rims of a cylinder of radius 5 and height 10 at a fillet radius
  of 4 fails as `Boolean(Ambiguous)` although its feet do not cross (3.5 works); the tests stay at
  3.
- [medium · medium] Near-duplicate lines in a profile make phantom sliver regions, because a face
  counts as real when its area exceeds tolerance² though a sliver thinner than the tolerance can be
  far larger; judge it by its width.
- [medium · medium] Pattern copies that touch only along a line or at a point fail the union as
  `NonManifold`, so round parts spaced one diameter apart cannot be patterned; keep such copies as
  separate shells of one body, and name the copies in `PatternError::Union`.
- [medium · hard] Shell cannot split a corner whose offsets do not meet when its convex and concave
  edges alternate (two ridges of different slopes crossing) or one convex edge meets concave ones (a
  cavity whose ridge runs over its inside corner): the offset there joins faces the body keeps
  apart, or runs an edge between another pair of faces, which splitting the corner into several
  cannot give. Corners of more than eight faces are not split either. All are refused as `Corner`
  (`shell::tests::a_corner_where_ridges_and_valleys_alternate_is_named`,
  `a_cavity_whose_ridge_runs_over_its_inside_corner_is_named`).
- [medium · hard] `select::classify` lets inside or outside samples win over coincident ones in a
  partly coincident fragment (only coincident samples of opposite senses make it `Ambiguous`);
  treating every partly coincident fragment as `Ambiguous` fails `stress_cylinders_on_a_grid` on
  noise near tolerance boundaries, so split fragments exactly at coincident boundaries instead.
- [low · easy] The spline-surface projection seed grid is capped at 48 samples per direction, so on
  very dense or rough imported nets the foot of an on-surface point is occasionally missed (a few in
  a thousand on 60×60 control points with heights far above their spacing; smooth nets are
  unaffected).
- [low · medium] A face the shell's thickness closes up is dropped only when it has one loop and
  keeps two single edges apart from each other, or none; a band whose side is a chain of edges (a
  rim split by another face's seam) is refused as `EdgeCollapses`.
- [low · medium] A torus whose tube is far thinner than its ring (20 and 0.5) still meshes at 2.7 to
  4.4 times the requested chord, since the Delaunay triangulation of the stretched parameter grid
  picks long triangles; mesh it finer or triangulate by cell.
- [low · hard] Meshes fold where two faces meet at a very small dihedral (lens tips, a plane nearly
  tangent to a torus), giving self-overlapping triangles that `validate` does not see.

## Kernel feedback

- [medium · medium] `BooleanError::Split`, `Open`, `Ambiguous`, `NonManifold` and `Intersection`
  carry no data about where they arose, and the document reduces `Intersection`, `Split`,
  `Ambiguous`, `Open` and `Invalid` to one message blaming "faces or edges that exactly touch",
  wrong for the near-coincident cases. Carry the face or edge names, or a model point, so the error
  can name and highlight them.
- [low · medium] `SweepError::Invalid` cannot say which region failed, since all regions build in
  one `Plan`.
- [low · medium] `ShellError::Walls` still names nothing when the offset solid fails to build
  (`inner_solid` and `settled_layout` raise it with neither face nor edge), so the error cannot say
  where the walls collide.

## Kernel performance

- [medium · medium] The face grid is uniform in uv and sized by the worst curvature anywhere, so one
  small bump multiplies a whole face's triangles, and straight directions are capped at
  `FLAT_ASPECT` times the curved one, so a 1×1000 cylinder gets 121k triangles in the smooth display
  mesh where a few hundred would do.
- [medium · medium] A boolean with an extruded spline of thousands of control points runs for
  seconds (600 control points) to many minutes (6,000) without polling `interrupt::check`: clipping
  the branch along its cap (`surface_surface::clip`) projects each probe onto the extrusion from the
  patch centre, a search over the whole profile, where the previous probe's foot would do. Each
  `Curve::length` of a 6,000-point spline also takes about 70 ms unpolled, and the builder,
  validation and boolean tracing each measure it again.
- [medium · hard] Every boolean rebuilds and revalidates every face of the body through `assemble`
  and `Plan::build` even when the tool touches two (an untouched face only skips tracing and
  classification), so a sequence of hole features is quadratic: in release the 144th hole of a block
  takes about 70 ms against 1 ms for the first. Carry untouched faces through by id, and return
  disjoint operands without the pipeline: a few hundred separated unions take seconds.
- [low · medium] Blending scales worse than linearly: `crosses_boundary` tests every boundary edge
  with no box filter and recounts uses inside the loop, and tools are unioned pairwise even when
  disjoint.
- [low · medium] Marched curves' `closest_parameter` and `length` reseed over all nodes on every
  call, from loops over nearby vertices in `imprint.rs`.

## Document and recompute

- [medium · medium] A recompute reports once more after its feature loop only; a slow late feature
  still hides the bodies before it, and each mesh is not reported as it finishes. Send an update per
  feature and per mesh.
- [medium · hard] The cache keeps one result per feature, so changing a depth and undoing recomputes
  everything after it; it also has no byte budget, holding every intermediate `Solid`. Keep a small,
  size-bounded history per feature.
- [medium · hard] Recompute is single-threaded: independent bodies and the final meshing of each
  body could run in parallel over the dependency data the document already has.
- [low · medium] `SetFeatureKind` refuses an `Import` (`set_feature_kind` pairs no import with an
  import), so an imported body keeps its source solid for good; allowing it would also give
  re-import.

## Sketch solver and expressions

- [medium · medium] When conflict diagnosis finds that a part which failed from its drawn shape
  holds after all (a chain whose line must fold back, reached from a solution of all but one
  constraint), the solve still fails; the solution found could be offered instead.
- [medium · medium] A point on a line segment or arc is held to the infinite line or full circle
  (`Form::OnLine` and the circle form behind `Coincident`), so it can solve beyond the segment's
  ends or outside the sweep, and the line rotates to meet it; bound it or say so.
- [medium · hard] The rank and null-space analysis (`analyze_sparse`, `Echelon::spans_unit` once per
  column) is near cubic on closed chains and never checks `cancelled`: solving the sketch left by
  offsetting a closed, fully dimensioned chain takes 0.2 s at 100 lines, 1.8 s at 200 and 15 s at
  400 in release (the geometry alone solves in milliseconds), and Cancel does nothing meanwhile. Use
  a sparse factorisation with a fill-reducing order, and poll inside `analyze_component`.
- [medium · hard] A drag frame solves geometry only (`solve_geometry_from`, no rank or
  degrees-of-freedom analysis), but the dragged part is still never memoised and the solve itself is
  the cost: dragging an end of a fully dimensioned chain of 2,000 lines to a point it cannot reach
  takes seconds a frame in a release build, where a chain joined only by `Coincident` takes tens of
  milliseconds.
- [medium · hard] Conflict diagnosis confirms each constraint of a conflict with a damped
  Gauss–Newton descent over the whole part, so a conflict running through a part of a few hundred
  lines (a chain of 300 with its far end fixed out of reach) still runs out of `DIAGNOSIS_WORK` and
  is reported as not solving; one factorisation of the Jacobian, updated per constraint left out,
  would make each confirmation cheap.
- [medium · hard] A sketch solved from a degenerate start can fail to solve again from its own
  result: a spline with four coincident control points, tangent to a zero-size arc on one of them,
  with a zero distance from that arc to the spline's first point. `sketch_solve` finds such cases
  within minutes once it requires `solve_from` of a solved geometry to succeed; it does not yet (it
  discards that result), so the property is unchecked.
- [low · medium] `components()` is rebuilt with `BTreeMap`s three times per solve (`Recall::new`,
  `Solver::solve` and the analysis in `solve_from`), and a warm re-solve recalling every part costs
  nearly as much as a cold one: 6,400 dimensioned rectangles take about 0.4 s either way
  (`thousands_of_independent_rectangles_solve_in_a_fraction_of_a_second`, ignored, in release).
- [low · medium] `10 mm^2` means (10 mm)², since a power binds to the measure before it; stored text
  relies on that reading, so changing it needs a new spelling or a format change.

## Sketching

- [high · medium] Every dimension drives: there are no reference (driven) dimensions and no way to
  disable a constraint, so dimensioning determined geometry adds a redundant constraint instead of a
  measurement.
- [high · hard] No projection of model edges or other sketches into a sketch, and bodies and other
  sketches are unpickable while editing.
- [medium · medium] Typed lengths and angles (`@40, 20`, `25 < 30`, `width / 2, 10`) are evaluated
  once and place free points, keeping neither a dimension nor the parameter link; offer to create
  the dimensions.
- [medium · medium] A constraint that fails only once solved (one contradicting the sketch through
  other constraints) is still accepted and reported afterwards; trial-solve it off the UI thread
  before committing.
- [medium · medium] Constraint kinds missing: distance between circles, line and circle, or to a
  spline; arc length and sweep; angle or perpendicular to an arc; arc midpoint; equal splines;
  spline–spline tangency; curvature continuity; symmetric curves. Coincident, perpendicular and
  tangent take exactly two items where parallel and equal chain.
- [medium · medium] No size readout while drawing arcs, slots, polygons or splines, nor in the
  three-point ways of drawing a rectangle or circle; no closed-region or open-end feedback while
  sketching (only a failed extrusion names a sketch's open ends); and no smart-dimension tool that
  takes the entities after the command.
- [medium · medium] Snapping has no midpoints, intersections (only an arc's end snaps to crossings),
  spline targets, grid or inference lines to other points, cannot be turned off for good (Ctrl
  suppresses it while drawing only), and dragged geometry does not snap at all.
- [medium · hard] Tools missing: ellipse (a new entity kind across the solver, the kernel's 2D
  profile curves, which have no ellipse although its 3D curves do, and the file format), sketch
  chamfer, rectangular and circular patterns, rotate, scale and copy of a selection, split at a
  point, text, and fit-point, closed or periodic splines (`BSpline::through` serves only DXF import,
  `BSpline::interpolate` only its own tests, and the control polygon is not drawn).
- [medium · hard] Splines cannot be trimmed or extended (`TrimError::Spline`), trim ignores
  collinear and co-circular overlaps as cutters, and the sketch axes are not cutters. Offset takes
  one chain at a time, leaves the free ends of an open chain sliding along their curves and cannot
  offset splines; a sketch fillet cannot round a spline and drops equal lengths and midpoints of the
  lines it shortens, as trim does.
- [low · easy] A tangent arc cannot continue a line chain without switching tools; the polygon side
  count changes only by one per key.
- [low · medium] In a conflicting sketch every drag is blocked and finishes by blaming the move; the
  cue names no constraint of the conflict.
- [low · medium] Spline intersections sample sign changes, so a near-tangent crossing between
  samples is missed.
- [low · medium] Dimensions all sit at one fixed offset, so collinear chains overlap, and labels
  cannot be dragged. Glyphs stack uncapped (a 64-gon puts 63 `=` glyphs on its first side) and
  cannot be hidden.
- [low · hard] No reference image: a photo or scan cannot be placed on a sketch plane, scaled by two
  points and traced, as a part copied from an existing object or a drawing needs.

## Modelling features

- [high · medium] No feature combines two existing bodies, and a cut affects only one body.
- [high · medium] Hole feature: no hole feature exists. Plain, counterbored and countersunk holes
  placed on a face or a sketch point, parametric and named like the other features.
- [high · medium] Extrusions always start on the sketch plane, with no start offset or start face,
  though `LinearExtent::between` accepts any bounds; revolve has the same gap.
- [high · hard] Bodies cannot be edited directly: no moving, offsetting, deleting or replacing a
  face (push and pull) and no deleting a fillet or chamfer by its faces. An imported STEP body has
  no feature history, so today it can only be cut, joined, filleted or shelled; a wall too thick, a
  hole in the wrong place or a fillet to remove means remodelling it from scratch. Direct edits
  become features of their own, named from the faces they move, so they stay parametric and
  undoable.
- [high · hard] Patterns repeat a whole body: no pattern of chosen features (a row of holes cut into
  a plate), no instances left out, and no linear "total length" mode.
- [medium · medium] Datums cannot be built from points: no datum point, plane through three points,
  mid-plane, plane through an axis and a point, plane normal to an edge at a point, or axis through
  two points. Model vertices are named and pickable but only the measure tool uses them, and datums
  and pattern axes cannot take sketch geometry.
- [medium · medium] Bodies have no colour, material or density, so a multi-body model is one grey
  until picked and mass cannot be shown; this blocks coloured STEP and 3MF export. It also gates
  coloured STEP (writer) and 3MF export and the mass of the mass properties.
- [medium · medium] No interference check: nothing finds where two bodies overlap or touch, or
  reports the overlapping volume, though booleans already compute it.
- [medium · medium] Split a body by a plane or a face, and move or copy one body (`RigidTransform`
  and booleans exist).
- [medium · medium] The kernel has only rigid transforms (`RigidTransform`; `Solid::transformed`
  takes nothing else): add a reflecting and a uniform-scaling transform that keep face and edge
  names stable.
- [medium · hard] The whole model cannot be scaled: no command or feature resizes every body, sketch
  and datum by a factor (uniform, about the origin or a chosen point) as one undoable change.
  Scaling must keep references and names stable, and say what happens to dimensions and parameters
  (scale the stored values, or the parameters they use, or leave expressions alone and scale only
  plain values), so a part drawn at the wrong size or an import in the wrong unit can be fixed
  without redrawing it. Imported bodies also need the kernel's uniform-scaling transform (see the
  transform item above).
- [medium · hard] Extrusions end only on flat faces and planes: up to face and up to next refuse a
  curved face, and up to next needs one flat face that the whole profile meets first.
- [medium · hard] Mass properties are volume, area, centroid and bounding size, exact only for
  bodies of flat faces and straight edges and otherwise taken from the display mesh, with no mass,
  inertia or total. Integrate exactly over the trimmed faces, as `planar_area` already does for
  planes. Mass and inertia wait on density from the colour, material and density item.
- [medium · hard] Blends: only line and circle edges along planes, parallel cylinders and coaxial
  surfaces; no ellipse, spline or intersection edges, not even a straight edge beside a spline
  extrusion face; ends at steps and T-junctions refused; no variable radius, two-distance or
  distance-angle chamfer; a round corner only for three convex straight edges meeting at three
  planes (other corners mitre).
- [medium · hard] Shell: no spline, extrusion or revolution faces, only flat faces open, one
  thickness for the whole body.
- [medium · hard] No live preview of a fillet, chamfer or shell while its panel is open, and no
  viewport handles for extents.
- [medium · hard] No configurations: a model holds one set of parameter values, so sizes of one part
  (a bracket in M4, M6 and M8) are separate copies of the file. Named parameter sets, chosen as a
  whole and kept in the model like versions, with export of each.
- [medium · hard] Sweep along a path and loft between profiles: the kernel has only extrusion and
  revolution, so both need new kernel operations first.
- [medium · hard] Extrusions and revolves have no taper angle or thin wall (an open profile given a
  thickness), which needs a tapered sweep and a wall of an open profile in the kernel.
- [low · medium] Revolve cannot keep the part of a region on one side of the axis.
- [low · medium] Parameters cannot be reordered or given a note, show what uses them only in the
  value's tooltip and mark no unused one in the table, cannot be deleted by inlining their value,
  and expressions cannot refer to measured values or sketch dimensions.
- [low · medium] The measure tool cannot take planes, axes, datums or sketch curves, so a hole axis
  to a datum or a circle's radius cannot be measured.
- [medium · medium · blocked by: kernel reflecting and scaling transforms] Mirror a body, and scale
  one body.
- [medium · hard · blocked by: the sweep feature, and the hole feature for thread sizes and cosmetic
  threads] No helix or spiral curve and no threads: springs, coils and threaded holes and shafts
  cannot be modelled, and the hole feature (listed under the missing feature kinds) has no ISO
  metric thread sizes or cosmetic thread to show a thread without modelling it. The sweep feature
  (same list) needs the helix for modelled threads.
- [medium · hard · blocked by: direct face edits ("Bodies cannot be edited directly")] Draft angle
  on existing faces.
- [medium · hard · blocked by: thin-wall extrusion ("Extrusions and revolves have no taper angle or thin wall")]
  Rib from an open profile.
- [medium · hard · blocked by: datums and pattern axes taking sketch geometry ("Datums cannot be
  built from points")] No pattern along a curve or driven by sketch points.
- [low · hard · blocked by: sketch text ("Tools missing" under Sketching)] Emboss or deboss sketch
  text onto a face.

## STEP import and export

- [high · hard] Healing moves vertices onto their faces and rebuilds an edge only between exactly
  two distinct faces, so real Fusion 360 exports are still refused whole over a vertex a few
  micrometres off an edge that has one face (a cylinder seam, used twice) or whose two faces meet
  along a line `IntersectionCurve::through` cannot rebuild (two half-cylinders), though the file
  declares 0.01 mm. Project the vertices onto lines, circles and ellipses within the declared
  precision instead. Faces that meet only within a coarse declared precision are refused rather than
  refitted to each other.
- [medium · medium] Import canonicalises each placement by writing and re-reading it, and stores
  every placement of a product as its own STEP text. Build each representation once and store each
  product once with placements.
- [medium · medium] Imports cannot be positioned (`Import` has no placement) or refreshed from their
  source file: the path is not kept, so a changed STEP file means deleting the feature and breaking
  what references it.
- [medium · hard] One unsupported surface or curve loses the whole body: an `OFFSET_SURFACE` of a
  spline, extrusion or revolution (it would need a surface fitted within tolerance), `PARABOLA`,
  `HYPERBOLA` and the `*_REPLICA` forms. Fit a spline within the declared precision, or keep the
  other faces and say which were lost. Colours and layers are not read.
- [low · medium] The parse tree still holds several times the file size (a boxed slice per record
  and per list); a flat arena of values would bring it near the file size.
- [low · medium] The writer puts all bodies in one product with no colours, holding the output twice
  in memory.
- [low · hard] No IGES import or export, though older CAM software and many suppliers still exchange
  it.
- [low · medium · blocked by: kernel reflecting and scaling transforms ("The kernel has only rigid
  transforms" under Modelling features)] Placements that scale or mirror are left out with a note; a
  uniform scale could be applied, and a mirror once the kernel can reflect.

## Drawing import and export

- [medium · medium] Sketches cannot be exported: there is no DXF or SVG output of a sketch or flat
  face, though laser and CNC work need it and `DrawingCurve` already models what it would write.
- [medium · medium] DXF import has no options: units come only from `$INSUNITS` and `$MEASUREMENT`
  with no override or scale (templates commonly default to inches), coordinates are not recentred,
  and a new sketch always lands on the XY plane although `SketchTarget::New` takes a plane.
- [low · medium] Curves carry no layer, so the import cannot offer a layer choice, and curves past
  the first 20,000 are left out in drawing order rather than by any choice of the user.

## Mesh import and export

- [medium · medium] No OBJ or glTF export, so models cannot go to renderers, game engines or web
  viewers without another tool.
- [medium · hard] No mesh import (STL, 3MF, OBJ), though the STEP reader already builds
  `FACETED_BREP`s from
  polygons. An imported solid is stored as STEP text like any import (see the STEP storage item
  above), which suits thousands of faceted faces but not a scan of millions of triangles, and
  keeping the source mesh in the model takes a storage of its own. Importing a mesh should give a
  solid that edits like one drawn in caditor, where Fusion and FreeCAD leave thousands of triangle
  faces that fillets, sketches, holes and booleans choke on. The aim, to be researched before
  design:
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
- [low · medium] STL uses absolute f32 coordinates, which resolve only about 0.06 mm at 10^6 mm, and
  merges all bodies into one surface.
- [low · medium] 3MF has no colours, materials or thumbnail, builds everything in memory and cannot
  exceed 4 GiB without ZIP64.

## Accessibility

- [medium · medium] The model drawn in the viewport is invisible to screen readers: only the text
  painted over it has nodes, and nothing says what the view shows (bodies, sketch geometry, their
  count or places). The cancelled and stopped recompute pills are not live regions.
- [medium · medium] Constraints and dimensions are not scene pickables, so N never reaches them and
  a dimension can be re-edited only by double-click or from the tree; with a drawing tool active,
  Space toggles the selection instead of placing at the highlight, so keyboard drawing cannot start
  from existing geometry.
- [medium · medium] High contrast reaches neither the scene colours nor the colour-only sketch
  states.

## Viewer

- [medium · easy] A selection filter, so a click takes only faces, edges, vertices or sketch
  geometry.
- [medium · medium] Display styles: wireframe, hidden line and shaded without edges, plus isolate or
  hide others and look normal to a face.
- [medium · medium] Transparent or X-ray bodies.
- [medium · medium] No box or lasso selection in the 3D view (only inside a sketch), no select all,
  and no selecting an edge's tangent chain or a face's loop outside the fillet panel, so choosing
  many faces or edges for a pattern, shell or export means clicking each one.
- [medium · medium] Edge lines can be eaten by faces at grazing angles, since depth bias is a
  constant factor with no slope term, and the grid and reference fills share the mesh's bias, so a
  face on the XY plane can speckle with the grid. Neither has a test.
- [medium · medium] Adapter choice is only the `WGPU_POWER_PREF` environment variable, so users of
  hybrid laptops or broken drivers cannot pick another adapter from Preferences.
- [medium · hard] Section planes.
- [low · medium] Silhouette edges on curved bodies.
- [low · medium] Line caps, joins and anti-aliasing without MSAA.
- [low · medium] Lighting and the MSAA resolve happen in gamma space; the model keeps no saved view.

## Interface performance

- [medium · medium] Every frame `Marks::collect` formats every constraint's description, evaluates
  every dimension and registers an `interact` per glyph, and an expanded sketch in the tree does the
  same per constraint row, searching the list of involved constraints linearly for each; the
  `large_sketch` benchmark has no constraints. Cache per generation, cull off-screen marks and
  virtualise the tree.
- [medium · medium] Meshes are uploaded whole on the UI thread in the frame that first shows them,
  and never culled in the main or pick pass although each keeps its bounds; the render crate's
  `frame_costs_of_drawing_a_large_scene` benchmark has no meshes, picking or hover.
- [medium · medium] Image export submits every tile at once and holds every tile's readback buffer
  until the image is assembled beside them, about twice the image's size (some 540 MB at the 8192²
  limit), so memory is not yet bounded as `render.md` intends; reuse a few buffers and stream bands
  to the encoder.
- [medium · medium] The feature tree lays out every row each frame, which a STEP import of hundreds
  of bodies makes long.
- [medium · hard] The cached scene is one batch: any change to its content (each drag solution, an
  edit, an evaluation, a new faceting level) facets every drawn sketch again, and a hover or
  selection change restyles and uploads all of it, over a millisecond to rebuild and about half of
  that to upload for a sketch of 24,000 curves in a release build. A batch per feature, with pick
  ids of its own, would limit both to what changed. Face styles are likewise rewritten whole on
  every highlight change.
- [low · easy] With a large sketch selected, every constraint tool rebuilds its candidates each
  frame.
- [low · easy] Zooming in five times reanchors and re-uploads every batch, since the anchor reach is
  four view distances; derive it from the f32 error budget.
- [low · medium] A pick or image readback in flight redraws full frames until polled complete;
  vertex records repeat per-layer data and both ends of shared segments; invisible vertex markers go
  through the colour pass; each mesh's placement uniform is written every frame; resizing recreates
  the MSAA targets per pixel; and an MSAA change rebuilds all ten pipelines with no pipeline cache.
- [low · hard] Snapping projects every point and curve of the sketch on every hover frame
  (`snap.rs`), a cost linear in the sketch that is most of the frame for tens of thousands of lines,
  mostly walking the entities, and a line or slot end walks every line again to find the nearest for
  parallel and perpendicular inference (`drawing.rs` `guides`); a screen-space index would need the
  preimage of the snap radius on the sketch plane, unbounded near the horizon.

## Application

- [medium · easy] Angles display only in degrees though `ux.md` allows radians.
- [medium · medium] One files worker runs everything and Import cannot be cancelled (cancelling Open
  only drops its result while the worker reads on), so a slow STEP import blocks Open behind a
  modal, and the opening modal is drawn before the unsaved-changes prompt, so closing the window
  during a load hides the prompt until the load ends. Give imports their own cancellable job. When a
  worker thread cannot be spawned the job runs on the UI thread.
- [medium · medium] Bodies have no list of their own: a body appears only as the features that build
  it, so showing, hiding, naming or (once they exist) colouring a body means finding the feature
  that made it.
- [medium · hard] Version history shows when a version was saved and after which change, but no
  preview of what it holds, and no way to keep a version from being thinned out.
- [medium · hard] No user guide: Help has only the welcome, the command search, the keyboard
  shortcuts, the notice log and About, besides the tips shown in the view. Nothing explains
  features, the parameter and expression syntax, or the file workflow, and no panel links to help on
  itself. A guide shipped with the app (and readable offline) with a page per tool, opened by F1 for
  the current tool or panel.
- [medium · hard] No clipboard for sketch geometry or features, no parameter import or export.
- [low · easy] Nothing shows where a drop will go while files are dragged over the window (X11).
- [low · medium] Several features chosen in the tree cannot be dragged together; each moves on its
  own.
- [low · medium] The modelling tools borrow Phosphor glyphs that mean something else (`icons.rs`):
  fillet is the full-screen corners, chamfer a generic polygon, revolve the refresh arrows, circular
  pattern a loading spinner, shell a see-through cube, and the sketch fillet shares the fillet's.
  Draw caditor's own icons for fillet, chamfer, shell, extrude, revolve and both patterns, on
  Phosphor's grid and stroke weight so they sit beside it; undecided whether they ship as glyphs
  added to the `icons` font family or as painted shapes.
- [low · medium] Text outside Latin, Greek and Cyrillic shows as missing glyphs in feature and file
  names, since only Inter and egui's defaults are loaded.
- [low · medium] The undo history lists only each step's label, with no summary of what it changed
  (the entities, features or parameters touched).
- [low · medium] No model properties: a model has no title, description, part number, revision or
  notes, so exports carry only the output file's name (the STEP description and its author and
  organisation fields stay empty, and the 3MF metadata holds only the application) and nothing
  identifies a part beyond its path. Empty unless the user fills them in.
- [low · medium] The feature tree has no filter or groups.
- [low · hard] Themes are four fixed `Tokens` sets in `appearance.rs` (dark, light and their
  high-contrast variants) and the 3D view is dark in all of them. Add themes as data: a few shipped
  ones beyond dark and light, a choice of accent colour, a light 3D view (background, grid, edges
  and the `canvas.rs` chrome) chosen with the theme or on its own, and user themes loaded from the
  config directory and picked in Preferences with a live preview. Every theme, shipped or loaded,
  goes through the contrast checks `appearance.rs` runs today (4.5:1, 7:1 for body text in high
  contrast), and a loaded theme that fails them or cannot be read is refused in words, naming the
  colour pair, with the previous theme kept.
- [low · hard] One document per process.
- [low · hard] No localisation.
- [low · hard · blocked by: winit 0.30 has no drag and drop on Wayland (0.31 is only a beta)]
  Dropping files on the window works only under X11.

## Technical drawings

- [medium · hard · blocked by: hidden-line removal ("Display styles" under Viewer)] No 2D drawings
  at all: no sheet with a title block, no projected front, top, side and isometric views of the
  bodies, no section or detail views, no dimensions or notes taken from the model, and no PDF, SVG
  or DXF output of a sheet, though parts made for a workshop need one. Views update with the model
  and their dimensions refer to edges by name, so they survive edits as features do. Hidden-line
  removal (also wanted for the Viewer's display styles) comes first.

## Code health

- [medium · medium] Error enums keep catch-all variants that wrap a formatted message, against the
  specific-variant rule: `ReadError::Unreadable` (the STEP reader's first failed body or unplaced
  note), `SaveError::Failed` and `StorageEvent`'s reasons (strings built from `WriteFailure`) and
  `ValueError::Refused` (serde's `custom`, which only a type's own `Deserialize` raises). Give each
  failure its own variant carrying typed data.

## Checks and CI

- [medium · medium] `tests/crash_flush.rs` runs the crash protection with a real storage worker in a
  child process, not the app itself. `check-install.sh` only runs `--version`; start the packaged
  binary to a first frame under Xvfb and lavapipe, kill it there and recover its journal, check its
  linked libraries and highest glibc symbol against `docs/RELEASING.md`, and run the offscreen tests
  once more on the GL backend that `packaging/INSTALL.md` promises.
- [low · medium] Slow tests to keep an eye on: about a third of the UI tests (75 of 219) take over a
  second each in a debug build, and the UI suite takes about 3.5 minutes on one thread.

## Scope decisions

These are open: each is a large direction the project has not committed to, and each needs a
decision recorded in `docs/` before work starts.

- [high · hard · blocked by: a scope decision recorded in `docs/`] Assemblies: a model is one part
  of several bodies, with no components, instances of another model file, joints or mates, exploded
  views or bill of materials, so a product of several parts cannot be put together or checked for
  fit. Decide whether caditor stays a part modeller, or how assemblies reference part files while
  keeping references stable across edits.
- [medium · hard · blocked by: a scope decision recorded in `docs/`] Surface modelling: no surface
  bodies, so no thicken, offset surface, trim, extend, patch or knit to a solid, which shaped
  consumer parts and repairing open STEP imports need.
- [low · hard · blocked by: a scope decision recorded in `docs/`, and DXF output of sketches
  ("Sketches cannot be exported")] Sheet metal: no flanges, bends with a bend allowance, or flat
  patterns, though laser-cut and bent parts are a common use; flat patterns would go out through the
  DXF export of a flat face or sketch, which does not exist yet either (Drawing import and export).

## Platforms

Linux is the primary platform and Windows the only other one planned; macOS is not a goal.

- [medium · hard] Linux only: there is no Windows build. Supporting Windows needs a Windows target
  in `ci.yml`, `release.yml` and `deny.toml`, and a release archive or installer. `caditor-file` is
  written against Unix: `os::unix` paths and file APIs in `journal.rs`, `recent.rs`, `recovery.rs`,
  `paths.rs`, `lock.rs`, `logs.rs`, `storage.rs` and `save.rs` (`fchown`,
  `rustix::fs::copy_file_range` and `access`), XDG config and state directories, and the atomic save
  that fsyncs the directory after the rename, which Windows cannot do (use `ReplaceFileW` semantics
  instead). The app shows its file dialogs through the XDG desktop portal over `zbus` with a
  `zenity` fallback (`portal.rs`, also on `os::unix` paths), reports a failed start through
  `zenity`, `kdialog` or `notify-send` (`logging.rs`), and flushes the journal on termination with
  `signal-hook` (`crash.rs`); each needs a Windows counterpart (native dialogs, a console control
  handler). The own title bar needs Windows snap, resize borders and DPI handling checked, the
  desktop entry, icons and MIME type need a Windows equivalent (file association, `.ico`), and the
  packaging scripts, `INSTALL.md` and `RELEASING.md` need a Windows section.
- [low · medium] Linux has only the `.tar.zst` with its installer: no AppImage, `.deb` or `.rpm`, so
  caditor is not in software centres and installs never update themselves.
- [low · easy · blocked by: the Windows build, and the user guide for the repeat link] On Windows,
  Microsoft Defender's real-time scanning slows the atomic saves, the recovery journal's frequent
  syncs and version history writes in the folders models live in. Remind the user, once and
  dismissibly (a callout on first save to a folder, repeatable from Preferences and the user guide),
  that they can exclude their models' working folder from Defender, saying what that trades away and
  how to do it; never change Defender settings ourselves.
- [low · medium · blocked by: the project's decision to publish no maintainer identity or repository
  URL] No Flatpak or AUR package: both need a maintainer identity and repository URL in their
  metadata, which the project does not publish (`docs/RELEASING.md`); `packaging/arch/PKGBUILD` only
  builds locally.
