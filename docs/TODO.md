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
the unblocked ones; the entry that does the unblocking comes before it. An entry tagged `later` instead of
an importance is deliberately not a priority: it waits until the rest is done, and its category
comes last. An entry tagged `next` is taken before every other item, whatever its tag, and carries
a note saying why; it loses the tag when its change lands, like any implemented item.

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
- [low · hard] The face grid is graded per direction but still a tensor product, so a bump divides
  the whole rows and columns through it, and curvature is sampled only on the lattice, so a feature
  narrower than a lattice span is refined only if a checked cell lands on it. Cells split where
  they bow past the chord (a quadtree) would keep the division local and find narrow features.

## Document and recompute

- [medium · hard] Recompute evaluates features on one thread: independent bodies could run in
  parallel over the dependency data the document already has. A body is meshed beside the feature
  loop once no later feature changes it, but one at a time, and those settling only at the last
  features are meshed one after another once the loop ends.

## Sketch solver and expressions

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
- [low · medium] `10 mm^2` means (10 mm)², since a power binds to the measure before it; stored text
  relies on that reading, so changing it needs a new spelling or a format change.

## Sketching

- [medium · medium] No Intersect: Project brings an existing edge, face outline or curve into the
  sketch, but not the curves where a body, a face or a datum plane cuts the sketch plane, so the
  outline of a cross-section (a wall seen through a bore, a casting at a given height) cannot be
  drawn from the model. Intersect would add that outline as projected geometry, named from the
  faces it cuts and following the model through recompute like any projection.
- [medium · hard] Tools missing: ellipse and elliptical arc (a new entity kind across the solver,
  the kernel's 2D profile curves, which have no ellipse although its 3D curves do, and the file
  format), a conic curve (two ends and a point it passes, shaped by a rho value), rectangular and
  circular patterns of sketch geometry and a pattern along a path, text (a font, a height, bold and
  italic, set along a curve, its letters becoming closed regions that extrude), and fit-point,
  closed or periodic splines (`BSpline::through` serves only DXF import, `BSpline::interpolate`
  only its own tests, and the control polygon is not drawn).
- [medium · hard] A spline is only the control points it was drawn with: a point has no tangent or
  curvature handle to set the direction and pull of the curve there, which would be stored as
  constraints on the point rather than as positions so the solver and dimensions keep reading
  them; the degree cannot be chosen; and no point can be inserted or removed while keeping the
  shape.
- [medium · hard] No spur gears: a gear tool in the sketch should draw the outline of an involute
  spur gear from its module (or diametral pitch), tooth count, pressure angle, and optionally
  profile shift, root fillet and bore, as one closed profile ready to extrude. Teeth are involute
  flanks, so it needs either a spline fitted within tolerance per flank or an involute curve in the
  sketch and the kernel's 2D profile curves, and the profile must stay parametric: module, teeth
  and pressure angle are named parameters taking expressions, the pitch, base, root and tip
  circles are shown as construction geometry, and a pair of gears at a centre distance follows from
  the same parameters. Refuse in words a tooth count that undercuts at the chosen shift.
- [medium · hard] The centre of an outline of odd sides and a slanted track place a point without
  a constraint keeping it there, as the sketch has no centroid or point-on-a-direction constraint;
  an odd outline with arcs has no centre at all. A drag snaps only its handle (the moving point
  nearest the press), not whichever moving point comes near a target.
- [medium · hard] Splines cannot be trimmed or extended (`TrimError::Spline`), trim ignores
  collinear and co-circular overlaps as cutters, and the sketch axes are not cutters. Offset takes
  one chain at a time, leaves the free ends of an open chain sliding along their curves and cannot
  offset splines; a sketch fillet cannot round a spline and drops equal lengths and midpoints of the
  lines it shortens, as trim does.
- [low · easy] The sketch chamfer takes one typed distance for both sides: a different distance on
  each curve, or one distance and the angle of the cut, cannot be typed when it is made, and the
  angle cannot be held as a dimension afterwards (the two distances can).
- [low · medium] No blend curve: a curve joining the ends of two sketch curves or edges, tangent
  (G1) or curvature-continuous (G2), as a spline held by `Tangent` and `Curvature` constraints so
  it follows when either end moves, for a transition where a fillet's circular arc is not smooth
  enough.
- [low · medium] Circles are drawn from a centre, two points or three points only: a circle
  tangent to two or three chosen lines, circles or arcs is missing, though the tangency
  constraint it would hold exists.
- [low · medium] No break: Split curve cuts one curve at one chosen point; breaking a curve at
  every crossing with the curves around it in one click, to get the pieces a profile is made of,
  is missing.
- [low · medium] A revolved profile cannot be dimensioned as a lathe drawing shows it: the
  distance from a point to the revolve axis (a construction line) reads as a radius, with no
  option to show and hold it as the diameter across the axis (twice the distance).
- [low · medium] Dimension labels cannot be dragged; only linear dimensions sharing a line stack
  clear of each other.
- [low · hard] No reference image: a photo or scan cannot be placed on a sketch plane, scaled by two
  points (or calibrated by a known distance), given an opacity, locked and traced, as a part
  copied from an existing object or a drawing needs.
- [medium · hard · blocked by: the sweep feature ("Sweep along a path" under Modelling features)]
  Sketches lie on one plane, so a sweep path or rail that bends in space (a cable run, a handle)
  cannot be drawn. A 3D sketch of lines, arcs and splines, with points placed by typed coordinates
  in space or moved off the sketch plane, would be the path; with it a sketch curve projected onto
  a curved face along a direction or to the nearest point, and the curve where two faces meet. The
  solver then holds 3D points and directions and the constraints that mean something there
  (coincident, horizontal and vertical in space, parallel, perpendicular, distance, fix).
- [low · medium · blocked by: section planes ("Viewer")] No slice while sketching: the part of the
  bodies in front of the sketch plane cannot be cut away for the time of the edit, so a sketch
  inside a body is seen through its surfaces rather than on a section of it.

## Modelling features

- [high · hard] No offset face (Fusion 360's Offset Face, reached by Press Pull on a face): choosing
  one or more faces of a body and dragging the arrow on them, or typing a distance or expression,
  moves each along its normal, the neighbouring faces extending or trimming to meet it, so a wall
  thickens, a boss grows taller or a bore widens without a sketch. Planes move parallel,
  cylinders, cones, spheres and tori change radius about the same axis, other faces become their
  offset surface; tangent faces are offered as one chain. It is a feature of its own, named from
  the faces it moves and referring to them by name, with a live preview, and refuses in words a
  distance that would make a face vanish or the body cross itself.
- [high · hard] Bodies cannot be edited directly beyond offsetting faces: no moving, deleting or
  replacing a face and no deleting a fillet or chamfer by its faces. An imported STEP body has
  no feature history, so today it can only be cut, joined, filleted or shelled; a wall too thick, a
  hole in the wrong place or a fillet to remove means remodelling it from scratch. Direct edits
  become features of their own, named from the faces they move, so they stay parametric and
  undoable.
- [medium · medium] A body splits only along a plane (`Split`), not along a curved face or a sketch
  curve swept through it, and cannot be placed by mating faces (a face onto another, flush or at a
  distance, an axis onto another).
- [medium · medium] No solid primitives: a box, cylinder, sphere or torus always starts as a sketch
  and an Extrude or Revolve. Primitive features placed on a plane or face by a click and sizes
  (typed expressions allowed), starting at a corner, a centre or a chosen point, joining, cutting,
  intersecting or making a new body, named from their faces (a box's six sides, a cylinder's wall
  and caps) and previewed live, would start most simple parts in one step.
- [medium · medium] Mirror reflects a whole body: mirroring only chosen features (a hole, a boss, a
  rib) or faces across a plane, kept linked so editing the original updates the mirror as a
  pattern of features does its copies, is missing; today the feature is drawn again or the body is
  mirrored and combined.
- [medium · medium] Datums lack the planes and points that sweeps, lofts and mating need: a plane
  tangent to a cylindrical or conical face, a plane square to a face or curve at a point along it
  (the section plane of a path), a plane through two edges or axes lying in one plane; and points
  at the crossing of two edges, of an edge or axis and a plane, or of three planes, at a distance
  along an edge or curve, and at the centre of a sphere or torus (only corners and round-edge
  centres are offered).
- [medium · medium] No thread feature: a tapped hole names its ISO thread only in its panel, and a
  shaft or boss takes none. A cosmetic thread on a cylindrical face (a bore, a shaft, a boss),
  chosen by designation (ISO metric coarse and fine, M3 to M64, internal or external, with the
  tolerance class and a length that may run to the end or a depth), is a feature of its own named
  from the face it threads and referring to it by name. It draws as the minor or major circle and a
  dashed thread line in the view, carries its designation into the exports that can hold it, and the hole feature creates one for a tapped hole. It is right- or left-handed, and beyond
  the metric series it offers the SI pipe and trapezoidal threads (ISO 228 and ISO 7 pipe, ISO 2901
  trapezoidal). Modelled threads are the item below.
- [medium · hard] The whole model cannot be scaled: no command or feature resizes every body, sketch
  and datum by a factor (uniform, about the origin or a chosen point) as one undoable change.
  Scaling must keep references and names stable, and say what happens to dimensions and parameters
  (scale the stored values, or the parameters they use, or leave expressions alone and scale only
  plain values), so a part drawn at the wrong size or an import in the wrong unit can be fixed
  without redrawing it.
- [medium · hard] Extrusions end only on flat faces and planes: up to face and up to next refuse a
  curved face, and up to next needs one flat face that the whole profile meets first.
- [medium · hard] Mass properties (volume, area, centroid, size, mass and inertia, per body and
  in total) are exact only for bodies of flat faces and straight edges and otherwise taken from
  the display mesh. Integrate exactly over the trimmed faces, as `planar_area` already does for
  planes.
- [medium · hard] Blends: only line and circle edges along planes, parallel cylinders and coaxial
  surfaces; no ellipse, spline or intersection edges, not even a straight edge beside a spline
  extrusion face; ends at steps and T-junctions refused; no variable radius, two-distance or
  distance-angle chamfer; a round corner only for three convex straight edges meeting at three
  planes (other corners mitre). Missing as shapes of their own: a full-round fillet across a
  narrow face between two others, a fillet sized by chord length, a fillet that runs by a rule
  over every edge of a kind, setback corners where three fillets meet, a tangency weight, and a
  curvature-continuous (G2) fillet.
- [medium · hard] Shell: no spline, extrusion or revolution faces, only flat faces open, one
  thickness for the whole body and always inward: no thickness per face, and no wall growing
  outward or to both sides of the faces.
- [medium · hard] No live preview of a shell while its panel is open (it shows the body before it
  for choosing faces), and no viewport handles for extents.
- [medium · hard] No configurations: a model holds one set of parameter values, so sizes of one part
  (a bracket in M4, M6 and M8) are separate copies of the file. Named parameter sets, chosen as a
  whole and kept in the model like versions, with export of each. A configuration could also
  suppress features and set body colours, as Fusion's configuration table does, so one file holds
  a part with and without a chamfer or a boss.
- [medium · hard] Sweep along a path and loft between profiles: the kernel has only extrusion and
  revolution, so both need new kernel operations first. A sweep takes a profile and a path (a
  chain of edges or sketch curves), kept square to the path or parallel to the profile, with an
  optional guide rail, taper and twist; a loft takes two or more profiles or faces (a point may
  end it), open or closed back to the first, with optional rails or a centreline and tangent or
  curvature-continuous conditions at its ends.
- [medium · hard] Extrusions and revolves have no taper angle or thin wall (an open profile given a
  thickness, placed inside, outside or centred on the profile), which needs a tapered sweep and a
  wall of an open profile in the kernel.
- [medium · hard] No pattern along a curve or driven by sketch points: a pattern repeats along one
  or two axes (sketch lines included) or about one, never along a spline or arc (the copies kept
  as they are or turned to follow the curve), nor at the points of a sketch, and it repeats
  features or whole bodies but never chosen faces.
- [medium · hard] No split face: dividing a face along a sketch curve, a plane or another body,
  without cutting the body, so a part line, a stripe of another colour or a face to draft or delete
  in part can be had. It is a feature of its own, naming the faces it splits.
- [low · easy] Combine takes one target and one tool and uses the tool up: a Keep tool switch, and
  several tool bodies in one combine (one cutter for several targets, or several bodies joined),
  are missing, so a cutter used for several cuts must be copied first.
- [low · easy] A hole ends flat: a drilled hole whose bottom is the cone of a drill point (118°
  unless set) with the depth counted to the full diameter, which is what a drill leaves, is
  missing.
- [low · medium] Expressions cannot refer to measured values or sketch dimensions, and a dimension
  or a feature's value cannot be named and listed with the parameters (Fusion's model parameters)
  for other expressions to use.
- [low · medium] No user coordinate system: Measure, Move and the patterns read from the origin and
  the principal axes. A coordinate system placed at a point, its axes taken from edges or faces,
  would be a second origin to measure and place from.
- [low · hard] Scale is uniform: a body cannot be stretched by different factors along the three
  axes (a plane stays a plane, but a cylinder becomes an elliptical one, which the kernel's
  surfaces do not have).
- [medium · hard · blocked by: the sweep feature] No helix or spiral curve and no modelled threads:
  springs, coils and threaded holes and shafts cannot be modelled with real thread geometry (the
  cosmetic thread feature above covers drawing and exchange). The sweep feature (same list) needs
  the helix, and the thread feature then offers a modelled form beside the cosmetic one. A coil
  feature would be the ready tool for springs: revolutions or height and pitch, a round or square
  section, inside or outside the axis, and a taper angle.
- [medium · hard · blocked by: the sweep feature] No pipe: a round, square or triangular section
  swept along a path sketch, solid or with a wall thickness, with sharp or rounded corners, as the
  ready tool for tubing, handrails and cable runs.
- [medium · hard · blocked by: direct face edits ("Bodies cannot be edited directly")] Draft angle
  on existing faces: a fixed angle from a plane, a split at a parting line with an angle on each
  side, and an angle per face, following tangent faces as one chain.
- [medium · hard · blocked by: thin-wall extrusion ("Extrusions and revolves have no taper angle or thin wall")]
  Rib and web from an open profile: a rib extrudes parallel to the sketch plane and a web square
  to it, each thickened and run on to the nearest faces of the body.
- [low · hard · blocked by: sketch text ("Tools missing" under Sketching)] Emboss or deboss sketch
  text, or any sketch profile, onto a face, flat or curved (wrapped around it), raised or
  recessed by a depth.
- [low · hard · blocked by: split face (above)] No silhouette split: dividing a body along its
  outline seen from a chosen direction, so the parting line of a moulded or cast part can be a
  face boundary for a draft to start from.
- [low · hard · blocked by: draft angle (above), rib and web (above)] No plastic-part features:
  the screw boss with its ribs, a lip and groove along a seam, snap fits (hook, loop, groove) and a
  rest (a flat seat on a curved face), which moulded parts need.

## Inspection and analysis

- [medium · medium] No curvature comb: a comb of lines whose length follows the curvature along a
  chosen edge or sketch curve (density and scale adjustable), also across the joint of two edges to
  see whether the curvature continues, as splines and blends are judged by eye.
- [medium · medium] No draft analysis: with a pull direction chosen from an axis, edge or face,
  faces coloured by their draft angle against it, banded by a limit the user sets, showing
  undercuts, parting lines and faces with too little draft before a part is moulded or cast.
- [low · easy] No centre of mass in the view: Measure gives the centroid as numbers, but not as a
  marker in the viewport that can be picked and measured from.
- [low · medium] No minimum radius analysis: concave faces tighter than a radius the user types
  coloured, to show where a cutter or a nozzle cannot reach.
- [low · medium] No surface quality analyses: zebra stripes, a curvature map (Gaussian and the
  principal curvatures), isocurves with combs, and a chrome reflection of the surroundings, all
  shown on the bodies without changing the model.
- [low · medium] No accessibility analysis: faces reachable from a chosen plane or direction
  coloured against those that are not, to judge undercuts for a three-axis machine.

## STEP import and export

- [high · hard] A body whose faces meet only within the file's declared precision (CATIA and
  Autodesk exports whose tangent fillet splines sit micrometres apart) is bent where it can be
  (`step-read.md`, "Bent faces") and otherwise imports as flat facets. On a CATIA V5 export of 11
  bodies at 0.01 mm, bending makes one of the seven loose bodies exact. Two bend every side but
  leave one traced edge off its face at validation (3.6e-6 and 8.2e-5 mm); one fails tracing an
  end-to-end join of two fillets slanted on both (2.6e-3 rad apart); one has a side that is a pole
  or seam; one has an iso-line edge whose neighbours' feet fall on both sides of it within the
  file's noise (the bound test needs a tolerance from the precision rather than the domain); one
  bends a face whose boundary then crosses itself in uv. The bent path has no STEP test yet (a
  fixture of a spline tangent to its neighbour a few micrometres off), and that file now reads in
  8.7 s against 7.6 s. Healing still traces an edge only between exactly two distinct faces, so an
  edge used twice by one face (a cylinder seam) with a vertex a few micrometres off is refused
  outright when the file declares no precision.
- [medium · hard] One unsupported surface or curve loses the whole body: an `OFFSET_SURFACE` of a
  spline, extrusion or revolution (it would need a surface fitted within tolerance) and the
  `*_REPLICA` forms. Fit a spline within the declared precision, or keep the
  other faces and say which were lost. A body of faces in mixed colours imports in the default
  look, since face colours are not mapped onto the imported faces.
- [low · hard] No IGES import or export, though older CAM software and many suppliers still exchange
  it.

## Drawing import and export

- [medium · medium] Drawing export takes one sketch at a time (several flat faces go side by side
  into one file, but are not nested to save material), and the files hold no text or dimensions.

## Mesh import and export

- [medium · hard] Mesh import (STL, OBJ, 3MF) repairs simple defects and joins flat areas into
  planar faces, but curved areas stay faceted, a sealed hollow is filled, non-manifold edges and
  gaps of more than `MAX_FILLED_HOLE_EDGES` edges lose their whole shell, and a mesh of more than
  `MAX_FACETED_FACES` faces after joining (most scans and organic prints) is refused. An imported
  solid is stored as STEP text like any import, and the source mesh is not kept. What remains:
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

## Viewer

- [medium · medium] No named views: the model keeps no saved view, so an angle and zoom worth
  coming back to (a view of a hidden bore, the way a drawing was taken) is lost. Named views
  stored in the model, restored from a list or the palette, with the Isometric home view
  redefinable.
- [medium · medium] A face, edge or body hidden behind another can be reached only by hiding the
  one in front or by stepping the keyboard highlight: a click held on a spot, or a key, should list
  everything under the pointer to choose from.
- [medium · hard] Section planes: a cut through all bodies at a plane or flat face, moved by a
  distance and turned by angles, with a flip, the cut faces drawn hatched or filled, and several
  at once, which only looks into the model and changes nothing. Measure works on what it shows.
- [low · easy] The selection filter has no bodies, so a click cannot pick a whole body: a Bodies
  filter, and a priority switch (body, face, edge) that sets the filter in one step.
- [low · easy] A box or lasso selects only what is seen: a Select through switch to take the faces
  and bodies hidden behind others as well.
- [low · easy] The shaded styles cannot show hidden edges: shaded with hidden edges dashed.
- [low · easy] The camera is perspective or orthographic: a third mode that is perspective in
  general and turns orthographic on looking square at a standard view.
- [low · medium] Silhouette edges on curved bodies.
- [low · medium] Line caps, joins and anti-aliasing without MSAA.
- [low · medium] Lighting and the MSAA resolve happen in gamma space.
- [low · medium] Bodies are opaque or all see-through (X-ray): a body cannot be made translucent on
  its own, to see a part inside another while modelling it.
- [low · medium] Dragging a brush over faces to select them (paint selection), and named selection
  sets: a group of faces, edges or bodies saved under a name, picked again later as the input of a
  fillet, a hide or a pattern, and healed like other references when the model changes.

## Interface performance

- [low · medium] Every frame `Marks::collect` still measures and lays out every dimension and
  groups every glyph of the edited sketch, on screen or not, and an expanded sketch card formats
  the description of each constraint row near view; the `large_sketch` benchmark has no
  constraints.
- [medium · medium] Meshes are uploaded whole on the UI thread in the frame that first shows them;
  the render crate's `frame_costs_of_drawing_a_large_scene` benchmark has no meshes, picking or
  hover.
- [medium · hard] The cached scene is one batch: any change to its content (each drag solution, an
  edit, an evaluation, a new faceting level) facets every drawn sketch again, and a hover or
  selection change restyles and uploads all of it, over a millisecond to rebuild and about half of
  that to upload for a sketch of 24,000 curves in a release build. A batch per feature, with pick
  ids of its own, would limit both to what changed. Face styles are likewise rewritten whole on
  every highlight change.
- [low · medium] Vertex records repeat per-layer data and both ends of shared segments, and
  resizing recreates the MSAA targets per pixel: they must match the surface they resolve into, so
  keeping larger ones would need a resolve pass of their own.
- [low · hard] Snapping projects every point and curve of the sketch on every hover frame
  (`snap.rs`), a cost linear in the sketch that is most of the frame for tens of thousands of lines,
  mostly walking the entities, and a line or slot end walks every line again to find the nearest for
  parallel and perpendicular inference (`drawing.rs` `guides`); a screen-space index would need the
  preimage of the snap radius on the sketch plane, unbounded near the horizon.

## Application

- [medium · medium] One files worker runs everything and Import cannot be cancelled (cancelling Open
  only drops its result while the worker reads on), so a slow STEP import blocks Open behind a
  modal, and the opening modal is drawn before the unsaved-changes prompt, so closing the window
  during a load hides the prompt until the load ends. Give imports their own cancellable job. When a
  worker thread cannot be spawned the job runs on the UI thread.
- [medium · medium] On Wayland the portal file dialog request passes an empty parent window, so the
  dialog is not tied to caditor's and can open behind it (Stop waiting in the status bar recovers
  the window); it needs an exported xdg-foreign handle, which winit does not offer and the raw
  Wayland connection would need `unsafe` to reach (a third `unsafe` crate, as `windows.md` keeps
  for Win32). X11 names the window already.
- [medium · hard] Version history shows when a version was saved and after which change, but no
  preview of what it holds, and no way to keep a version from being thinned out.
- [medium · hard] No user guide: Help has only the welcome, the command search, the keyboard
  shortcuts, the notice log and About, besides the tips shown in the view. Nothing explains
  features, the parameter and expression syntax, or the file workflow, and no panel links to help on
  itself. A guide shipped with the app (and readable offline) with a page per tool, opened by F1 for
  the current tool or panel.
- [medium · hard] No clipboard for features, and sketch geometry copies only within one caditor
  (the system clipboard gets a line of text, not the geometry); no parameter import or export.
- [low · medium] The modelling tools borrow Phosphor glyphs that mean something else (`icons.rs`):
  fillet is the full-screen corners, chamfer a generic polygon, revolve the refresh arrows, circular
  pattern a loading spinner, shell a see-through cube, and the sketch fillet shares the fillet's.
  Draw caditor's own icons for fillet, chamfer, shell, extrude, revolve and both patterns, on
  Phosphor's grid and stroke weight so they sit beside it; undecided whether they ship as glyphs
  added to the `icons` font family or as painted shapes.
- [low · medium] Text outside Latin, Greek and Cyrillic shows as missing glyphs in feature and file
  names, since only Inter and egui's defaults are loaded.
- [low · hard] Themes are four fixed `Tokens` sets in `appearance.rs` (dark, light and their
  high-contrast variants) and the 3D view is dark in all of them. Add themes as data: a few shipped
  ones beyond dark and light, a choice of accent colour, a light 3D view (background, grid, edges
  and the `canvas.rs` chrome) chosen with the theme or on its own, and user themes loaded from the
  config directory and picked in Preferences with a live preview. Every theme, shipped or loaded,
  goes through the contrast checks `appearance.rs` runs today (4.5:1, 7:1 for body text in high
  contrast), and a loaded theme that fails them or cannot be read is refused in words, naming the
  colour pair, with the previous theme kept.
- [low · hard] No automation: nothing can be driven by a script or macro, as Fusion's scripts and
  add-ins do, to make repetitive geometry, run a batch over files or add a tool. An interface
  would go through `Action`s and `Transaction`s like the UI, so scripts cannot break the model's
  rules, and would need a decision on the language and on safety (a script cannot reach files
  or the network unasked).
- [low · hard] One document per process.
- [low · hard] No localisation.
- [low · hard · blocked by: winit 0.30 has no drag and drop on Wayland (0.31 is only a beta)]
  Dropping files on the window works only under X11.

## Technical drawings

- [medium · hard] Vector hidden-line removal: the hidden-lines-removed display style hides edges
  by the depth buffer, which gives pixels, not curves. A drawing view needs each edge of a body
  split where faces cover it, as 2D curves in the view's plane with their visible and hidden
  pieces (silhouettes of curved faces included), computed off the UI thread and cancellable.
- [medium · hard · blocked by: vector hidden-line removal (above)] No 2D drawings
  at all: no sheet with a title block, no projected front, top, side and isometric views of the
  bodies, no section or detail views, no dimensions or notes taken from the model, and no PDF, SVG
  or DXF output of a sheet, though parts made for a workshop need one. Views update with the model
  and their dimensions refer to edges by name, so they survive edits as features do.

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
  consumer parts and repairing open STEP imports need. Also stitch and unstitch, boundary fill
  (a solid from the cell several surfaces and bodies enclose), ruled and sweep or loft surfaces,
  and freeform (T-spline) shaping, which Fusion keeps in its own environment.
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
- [low · medium] On Windows, hovering caditor's own maximize button does not offer Snap Layouts:
  that needs the button to answer `WM_NCHITTEST` with `HTMAXBUTTON`, which winit does not expose,
  so it would be another `caditor-windows` subclass hook.
- [low · medium] On Windows, journal markers and fallback journals hash the path as spelled
  (`paths::path_hash`), so the same file reached as `C:\A\m.caditor` and `c:\a\M.caditor` gets
  two fallback journals; changing the hash must still find journals written under the old one.
  Normalising needs the final path (`GetFinalPathNameByHandleW`) without showing users `\\?\`
  names.
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

## Accessibility

- [later · medium] In the 3D view's description, single bodies, faces or sketch curves have no
  nodes of their own to step through, and datums are named without where they lie.
