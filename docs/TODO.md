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

- [high · hard] Booleans between the fixture solids in random placements all succeed on the
  survey's seed (`boolean::tests::random_placements_of_every_fixture`, ignored), but another seed
  still fails an extruded spline against a torus with `Invalid(EdgeOffSurface)` (an edge 1.3e-6 off
  its face). Tori that nearly coincide still fail when turned rather than shifted: a torus and its
  copy turned by 1e-5 to 1e-3 radians are `Ambiguous` or `Split`, each after about 0.7 s of
  seeding. Intersection curves crossing at a tangent point (tori touching along their equators, a
  face touching a torus's inner equator) cannot be split
  (`tori_touching_along_their_equators_cannot_be_split`), and a lump too thin for the validation
  mesh is refused as invalid: the difference of a torus and its copy shifted 1e-5 along each axis
  is `Invalid(VoidOutside)`, with no test pinning it.
- [medium · hard] Offsets within `LINEAR_RESOLUTION` compound past it: a block whose back and
  bottom are each within the resolution of a plate's faces (8.3e-7 and 6.2e-7) has its corner
  1.03e-6 off the plate's edge, so the corner is neither pooled with the edge nor apart from it,
  and about 0.4% of aligned contacts with offsets between 1e-7 and 1e-4 fail as `Open`, `Split` or
  `Invalid(VertexOffCurve)` (offsets of 1e-6 to 1e-4 alone, as in
  `boolean::tests::aligned_contacts_a_micrometre_or_so_apart`, all combine), and an extruded plug
  flush with a plate 1.1e-6 to 2e-6 off its bore's axis is `Open` (the UI test of a failing
  combine's place uses it), though 3e-6 and more combine. Pooling points within the resolution of each other
  transitively, or snapping faces within the resolution onto each other before imprinting, would
  close it; whichever is chosen, the band where offsets are snapped or refused is still to be
  documented.
- [medium · hard] Shell cannot split a corner whose offsets do not meet when its convex and concave
  edges alternate (two ridges of different slopes crossing) or one convex edge meets concave ones (a
  cavity whose ridge runs over its inside corner): the offset there joins faces the body keeps
  apart, or runs an edge between another pair of faces, which splitting the corner into several
  cannot give. Corners of more than eight faces are not split either. All are refused as `Corner`
  (`shell::tests::a_corner_where_ridges_and_valleys_alternate_is_named`,
  `a_cavity_whose_ridge_runs_over_its_inside_corner_is_named`).
- [medium · hard] `select::classify` lets inside or outside samples win over coincident ones in a
  partly coincident fragment (only coincident samples of opposite senses make it `Ambiguous`). A
  sample now counts as coincident only on a face of the same elementary surface, which removed the
  tangent-line noise that made every partly coincident fragment `Ambiguous` fail
  `stress_cylinders_on_a_grid`, and none occurs in the boolean tests or the aligned-contact survey;
  making a mixed fragment `Ambiguous` is unchecked against the random-placement survey, and
  splitting fragments exactly at coincident boundaries would replace it.
- [low · medium] A face the shell's thickness closes up is dropped only when it has one loop and
  keeps two single edges apart from each other, or none; a band whose side is a chain of edges (a
  rim split by another face's seam) is refused as `EdgeCollapses`. Counting chains as sides closes
  faces that survive (a chamfered box), so it needs the offset outline's orientation as well, and
  the joint vertex of a chain then has too few live faces to be placed: a split vertex of two
  offset surfaces only, which `offset_vertex` and `split_vertex` do not handle.
- [low · hard] Meshes fold where two faces meet at a very small dihedral (lens tips, a plane nearly
  tangent to a torus), giving self-overlapping triangles that `validate` does not see: an extruded
  spline intersected with a frustum leaves two tangent edges at one vertex, the end parting bisects
  them down to 1e-7, and the mesh then uses one edge twice in the same direction
  (`assert_watertight` fails on it).

## Kernel performance

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
- [low · medium] Tracing a boolean measures the shortest piece leaving a vertex in full, unpolled:
  half the profile of an extruded spline of 6,000 control points takes about 0.1 s in release, the
  longest stretch without a poll in that boolean. `Curve::length` integrates 96 points per cubic
  span; a bound good enough for the chord probes, or fewer points per span, would do.
- [low · hard] The face grid is graded per direction but still a tensor product, so a bump divides
  the whole rows and columns through it, and curvature is sampled only on the lattice, so a feature
  narrower than a lattice span is refined only if a checked cell lands on it. Cells split where
  they bow past the chord (a quadtree) would keep the division local and find narrow features.

## Document and recompute

- [medium · hard] The cache keeps one result per feature, so changing a depth and undoing recomputes
  everything after it; it also has no byte budget, holding every intermediate `Solid`. Keep a small,
  size-bounded history per feature.
- [medium · hard] Recompute evaluates features on one thread: independent bodies could run in
  parallel over the dependency data the document already has. A body is meshed beside the feature
  loop once no later feature changes it, but one at a time, and those settling only at the last
  features are meshed one after another once the loop ends.
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

- [high · hard] No projection of model edges or other sketches into a sketch, and bodies and other
  sketches are unpickable while editing.
- [medium · easy] Nothing turns the view square-on to the edited sketch again: the camera faces it
  only on entering (`face_edited_sketch`), the standard views are axis-aligned, and Look at face
  needs a selected face, which cannot be picked in a sketch, so a sketch on a datum or slanted face
  is lost after one orbit until it is left and entered again. Add a command (palette, key and the
  sketch bar) that animates to the sketch's facing view.
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
- [medium · medium] No size readout while drawing splines, tangent arcs or arc slots; no
  closed-region or open-end feedback while sketching (only a failed extrusion names a sketch's open
  ends); and no smart-dimension tool that takes the entities after the command.
- [medium · medium] Snapping has no midpoints of arcs, spline targets or crossings with splines,
  grid or inference lines to other points, and dragged geometry does not snap at all.
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
- [low · easy] Dimensions taken from drawn geometry round to three decimals of the shown unit
  (`rounded_for_display` through `LengthUnit::measured` and `AngleUnit::measured`), so in radians a
  right angle becomes 1.571 rad, 0.012° off, and in metres anything over a metre is rounded to the
  millimetre; the dimension then moves the geometry it measured. Round to the model's precision (a
  micrometre, a thousandth of a degree) written in the chosen unit; `units.rs` pins 1.745 rad today.
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

- [medium · medium] A cut affects only one body: no extrusion or revolve cut removes material from
  several bodies at once, only a Combine of two.
- [medium · medium] Holes are drilled only at the free points of a sketch the user draws first, with
  no standard sizes, no tapped or threaded holes, no hole on a face by clicking it, none at circle
  centres, no slot and no hole of several diameters (stepped); each hole of a feature shares the
  feature's sizes.
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
- [medium · medium] Split a body by a plane or a face, and copy one body (`RigidTransform` and
  booleans exist); a body can be moved but only by typed distances and turns about the origin's axes,
  not dragged, turned about its own axis or centre, or placed by mating faces.
- [medium · hard] The whole model cannot be scaled: no command or feature resizes every body, sketch
  and datum by a factor (uniform, about the origin or a chosen point) as one undoable change.
  Scaling must keep references and names stable, and say what happens to dimensions and parameters
  (scale the stored values, or the parameters they use, or leave expressions alone and scale only
  plain values), so a part drawn at the wrong size or an import in the wrong unit can be fixed
  without redrawing it.
- [medium · hard] Extrusions end only on flat faces and planes: up to face and up to next refuse a
  curved face, and up to next needs one flat face that the whole profile meets first.
- [medium · hard] Mass properties are volume, area, centroid, bounding size and mass from the
  body's density, exact only for bodies of flat faces and straight edges and otherwise taken from
  the display mesh, with no inertia or total over several bodies. Integrate exactly over the
  trimmed faces, as `planar_area` already does for planes.
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
- [medium · easy] A STEP file cut off inside its `DATA` section (no `ENDSEC;`, or cut mid-entity,
  as an interrupted download leaves it) is refused whole as damaged: at end of input
  `Parser::recover` fails hard and every entity already read is dropped, though `step-read.md` says
  damage is survived and reported. Return what was read with `trailer_missing` and a note that the
  file is cut short, leaving the refusal to `NoSolids` or `NotRebuilt` when the solids themselves
  are incomplete; `damage_is_located_and_other_files_are_refused` pins the refusal today.
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
- [low · medium] The writer puts all bodies in one product without their colours (bodies have one,
  `BodyAppearance`), holding the output twice in memory.
- [low · hard] No IGES import or export, though older CAM software and many suppliers still exchange
  it.

## Drawing import and export

- [medium · easy] DXF block expansion counts objects (`MAX_EXPANDED_OBJECTS`) but not the shapes
  they decode to, and once `MAX_DRAWING_CURVES` is reached `Interpreter::push` still walks every
  shape of every further instance to count it as left out: a 3 MB file with a block of one
  100,000-vertex polyline inserted as a 700 × 700 array keeps the files worker busy for minutes,
  and an import cannot be cancelled. Count the rest of an item in one step once the limit is
  reached, and charge the visit budget per shape so a hostile file ends in `TooManyObjects`.
- [medium · medium] Drawing export takes one sketch or one flat face at a time: several faces (the
  parts of a nest) cannot go into one file, construction geometry is left out with no option to keep
  it on a layer, and the files hold no text or dimensions. A face's intersection edges are written
  as polylines, which some CAM software joins poorly; fitting them as splines within the chord would
  keep each one a single curve.
- [low · medium] Curves past the first 20,000 are left out in drawing order rather than by any
  choice of the user: the import options' layer choice is made after that cut, so leaving layers
  out cannot bring the curves of the others back in.

## Mesh import and export

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
- [low · medium] 3MF leaves out the bodies' colours and materials (`BodyAppearance`) and has no
  thumbnail, builds everything in memory and cannot exceed 4 GiB without ZIP64.

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
- [medium · medium] The side panels keep fixed minimum widths in points (the model panel 270,
  Measure and Interference 220 each), so at 200% the 3D view is squeezed to a sliver: about 30
  points with all three open on a 1920 px screen, about 50 with the model panel and Measure on a
  1366 px one. Cap the panels' combined share, or dock the inspect panels together, so the view
  keeps a usable width, with a UI test at 200%.

## Viewer

- [medium · medium] Display styles stop at shaded with edges, without edges and wireframe: no hidden
  line style.
- [medium · medium] Transparency is all or nothing: the X-ray style draws every body translucent with
  unpickable faces, and no body can be translucent or hidden on its own except by hiding its
  feature. A body's colour is one for all its faces: no face can be coloured apart.
- [medium · medium] No box or lasso selection in the 3D view (only inside a sketch), no select
  all of a body's faces or edges (Select all takes every shown body's), and no selecting a face's
  tangent neighbours or a hole's wall, so choosing many faces for a pattern, shell or export
  still means clicking most of them.
- [medium · medium] Edge lines can be eaten by faces at grazing angles, since depth bias is a
  constant factor with no slope term, and the grid and reference fills share the mesh's bias, so a
  face on the XY plane can speckle with the grid. Neither has a test.
- [medium · hard] Section planes.
- [low · medium] Silhouette edges on curved bodies.
- [low · medium] Line caps, joins and anti-aliasing without MSAA.
- [low · medium] Lighting and the MSAA resolve happen in gamma space; the model keeps no saved view.

## Interface performance

- [medium · easy] A large selection is handled item by item on the UI thread: after Select all on
  an imported body of tens of thousands of faces, edges or vertices, `Offers::of` describes and
  captures every one (`describe_vertex` scans every edge per vertex, so all vertices cost V×E), the
  status bar's tooltip joins every description, Measure builds and draws a card per item each frame
  though more than two only ever say to select fewer, and `retain_available` and the panels' Use
  selected re-check the whole selection each frame. Describe and list the first few with "and N
  more", and measure only one or two items.
- [medium · easy] `annotation_layout::placements` builds every shift along a line's full on-screen
  length before `place_glyphs` takes the first free one, for every constrained line each frame, on
  screen or not, so zoomed in on a feature of tens of micrometres each line allocates tens of
  megabytes a frame. Clip segment anchors to the view and generate placements lazily from the
  middle, with a cap.
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
- [medium · medium] The constraint buttons' offers (`ConstraintOffers::refresh`) check every tool's
  candidates against every constraint of the sketch (`new_relations`, through `contradicting` and
  `restating`, which rebuilds each constraint's `Subject`), so a selection of a few hundred
  constrained lines costs hundreds of milliseconds, and since the offers key on the displayed
  sketch's generation, dragging that selection pays it every frame. Index the constraints by
  subject once per revision and build a tool's candidates only when its selection shape can match.
- [medium · medium] The Measure and Interference panels redo work per body or pair each frame while
  open: `Interference::refresh` rebuilds and compares every pair and `report` looks each up again,
  and both panels format and lay out a card per contact or shown body with no virtualisation, so a
  STEP import of a thousand bodies means half a million pairs a frame. Cache the pairs and the
  report per basis, and virtualise or cap the cards.
- [medium · hard] The cached scene is one batch: any change to its content (each drag solution, an
  edit, an evaluation, a new faceting level) facets every drawn sketch again, and a hover or
  selection change restyles and uploads all of it, over a millisecond to rebuild and about half of
  that to upload for a sketch of 24,000 curves in a release build. A batch per feature, with pick
  ids of its own, would limit both to what changed. Face styles are likewise rewritten whole on
  every highlight change.
- [low · easy] The parameter table calls `Document::can_remove_parameter` for every row each
  frame, which scans every feature through `parameter_users`, though `app-look.md` says that scan
  runs only for the hovered row; check once per revision.
- [low · easy] Files hovered over the window whose extension is not a model, DXF or STEP one have
  their content sniffed on the UI thread every frame (`drop_target::verdict` through
  `import::starts_like_step`: stat, open and read), as `Files::start` does for the command-line
  path, so a file on a stalled network mount freezes the window while it is dragged. Judge hovered
  files by extension and leave the content check to the files worker on drop.
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

- [medium · easy] Without a desktop portal the file dialogs fall back to zenity, started with
  `--save` but not `--confirm-overwrite`, which zenity 3 leaves off, and `Files::check_output` and
  `check_save_target` ask before replacing only a name caditor completed with an extension, so an
  export, image or drawing saved there over an existing file of the same name replaces it without
  a question. Pass the flag, or ask through the "Replace …?" dialog whatever the dialog did.
- [medium · easy] The Linux file dialogs filter by lowercase globs (`*.step`, `*.dxf` from
  `portal/xdg.rs` `patterns`) with no All files choice, so on portals that match globs
  case-sensitively (GTK's) a `PART.STEP` or `PLAN.DXF` cannot be picked for Import, though
  dropping it works. Emit case-insensitive patterns (`*.[sS][tT][eE][pP]`) and add All files to
  Open and Import, leaving the decision to the content check.
- [medium · medium] One files worker runs everything and Import cannot be cancelled (cancelling Open
  only drops its result while the worker reads on), so a slow STEP import blocks Open behind a
  modal, and the opening modal is drawn before the unsaved-changes prompt, so closing the window
  during a load hides the prompt until the load ends. Give imports their own cancellable job. When a
  worker thread cannot be spawned the job runs on the UI thread.
- [medium · medium] Bodies are listed (Bodies group) but cannot be renamed on their own, since a
  body is named by the feature that made it, nor deleted or selected as a whole in the view.
- [medium · medium] While a native file dialog is open (`Files::picking`) every shortcut and the
  palette are blocked, yet nothing in the window says a dialog is waiting and nothing cancels it;
  the portal request passes an empty parent window, so the dialog is not tied to caditor's and can
  open behind it, and neither the portal wait nor zenity times out. Say that the dialog is waiting,
  with a Cancel that abandons the pick, and pass the window handle to the portal.
- [medium · medium] A model whose records exceed 2 GiB uncompressed (`MAX_DECOMPRESSED`) can be
  neither saved nor journaled: `encode_over` checks only the compressed size, so the read-back
  runs out of its budget and every save fails as "did not read back intact", and the journal
  snapshot exceeds `MAX_JOINED_CONTENT`, so `rewrite` reports `Unconvertible` and the session has
  no crash protection. Importing an assembly with a few hundred placements of a part of a few MB of
  STEP text reaches it, since each placement is stored as its own text (STEP import and export).
  Measure the uncompressed snapshot and say before committing an import that it would make the
  model too large; storing each product once removes the usual cause.
- [medium · hard] Version history shows when a version was saved and after which change, but no
  preview of what it holds, and no way to keep a version from being thinned out.
- [medium · hard] No user guide: Help has only the welcome, the command search, the keyboard
  shortcuts, the notice log and About, besides the tips shown in the view. Nothing explains
  features, the parameter and expression syntax, or the file workflow, and no panel links to help on
  itself. A guide shipped with the app (and readable offline) with a page per tool, opened by F1 for
  the current tool or panel.
- [medium · hard] No clipboard for sketch geometry or features, no parameter import or export.
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
- [low · hard · blocked by: a scope decision recorded in `docs/`] Sheet metal: no flanges, bends
  with a bend allowance, or flat patterns, though laser-cut and bent parts are a common use; flat
  patterns would go out through the DXF export of a flat face.

## Platforms

Linux is the primary platform and Windows the only other one; macOS is not a goal.

- [medium · medium] Windows has been built and tested only in CI (`windows`, `package-windows`)
  and run under wine: no one has used it on a real Windows desktop yet. Check by hand the built-in
  title bar (dragging, Aero Snap, resize strips, double-click, the corner close, mixed-DPI
  monitors), the rfd dialogs owned by the window, sign-out flushing the journal, the MSI from
  SmartScreen to uninstall, and a model and its journal on a USB stick (FAT32/exFAT, no POSIX
  rename) and on a network share.
- [low · easy] A failed install over an existing one removes the old version as well: `install.sh`
  lists each path before copying it and `roll_back` deletes every listed path, including files the
  previous install had there. Copy to temporary names beside the targets and rename them into place
  once every copy succeeded, and add an upgrade that fails midway to `check-install.sh`.
- [low · medium] On Windows, hovering caditor's own maximize button does not offer Snap Layouts:
  that needs the button to answer `WM_NCHITTEST` with `HTMAXBUTTON`, which winit does not expose,
  so it would be another `caditor-windows` subclass hook.
- [low · medium] Windows paths compare case-sensitively: the recent files list can hold one model
  twice under different casing, and journal markers and fallback journals hash the path as spelled
  (`paths::path_hash`), so the same file reached as `C:\A\m.caditor` and `c:\a\M.caditor` gets
  two fallback journals. Normalising needs the final path (`GetFinalPathNameByHandleW`) without
  showing users `\\?\` names.
- [low · medium] On Windows, saving writes every kept version from memory: ReFS block cloning
  (`FSCTL_DUPLICATE_EXTENTS_TO_FILE`) would give `os::clone_range` what `copy_file_range` gives on
  Linux.
- [low · medium · blocked by: the project's decision to publish no maintainer identity] The MSI
  and `caditor.exe` are not code-signed, so SmartScreen warns on first run; signing needs a
  certificate tied to an identity.
- [low · medium] Linux has only the `.tar.zst` with its installer: no AppImage, `.deb` or `.rpm`, so
  caditor is not in software centres and installs never update themselves.
- [low · easy · blocked by: the user guide for the repeat link] On Windows,
  Microsoft Defender's real-time scanning slows the atomic saves, the recovery journal's frequent
  syncs and version history writes in the folders models live in. Remind the user, once and
  dismissibly (a callout on first save to a folder, repeatable from Preferences and the user guide),
  that they can exclude their models' working folder from Defender, saying what that trades away and
  how to do it; never change Defender settings ourselves.
- [low · medium · blocked by: the project's decision to publish no maintainer identity or repository
  URL] No Flatpak or AUR package: both need a maintainer identity and repository URL in their
  metadata, which the project does not publish (`docs/RELEASING.md`); `packaging/arch/PKGBUILD` only
  builds locally.
