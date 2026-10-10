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

- [medium · hard] A boolean still validates the volume of every shell it touches by meshing the
  whole shell, and passes every face of the body through `Plan::build` and the cheap checks, so a
  sequence of hole features stays quadratic with a smaller constant: in release the 144th hole of
  a block takes about 21 ms (14 of them the volume check's mesh) against 1 ms for the first
  (`carry_tests::holes_drilled_one_by_one_and_blocks_joined_one_by_one_take_bounded_time`).
  Shells whose volume and placement could be known without meshing, and lumps of a many-lump body
  that the tool does not reach (a union touching one of 300 separated blocks takes 18 ms), are not
  carried yet.
- [medium · medium] Each display mesh is held twice on the CPU: the kernel `Mesh` (f64, with a
  32-byte vertex) and the app's `ShadedMesh`. A display mesh also reuses no face once the body's
  box changes (a longer extrusion), since the display tolerance follows the solid's extent and is
  part of every face's key (`tessellation/reuse.rs`); a tolerance stepped by extent would let such
  edits keep the faces they leave alone. The largest face bounds a mesh's time on many threads
  (the 14 ms top of a plate with 113 holes, against 18 ms for the whole body).
- [medium · medium] Every intermediate solid is a deep copy: a `Pcurve` is a `Vec` of samples and
  B-spline curves own their data, so carried coedges clone their pcurves (`build/plan.rs`
  `PlanPcurve::Settled`, `boolean/faces.rs`), and the result history holds each feature's body in
  full. Pcurve samples and curve data shared through `Arc`, as spline surfaces already are, would
  let successive bodies share what did not change. `SolidResult::cuts`/`joins` also keep every
  tool solid forever, though only patterns, mirrors and an open feature read them.
- [low · easy] The recompute pool is scoped to a run (`pool.rs`), so every run and draft starts up
  to `available_parallelism()` threads again, and `draft_copy` deep-copies every cache entry for
  each draft.
- [low · hard] The face grid is graded per direction but still a tensor product, so a bump divides
  the whole rows and columns through it, and curvature is sampled only on the lattice, so a feature
  narrower than a lattice span is refined only if a checked cell lands on it. Cells split where
  they bow past the chord (a quadtree) would keep the division local and find narrow features.

## Sketch solver and expressions

- [medium · hard] A drag frame solves geometry only (`solve_geometry_from`, no rank or
  degrees-of-freedom analysis), but the dragged part is still never memoised and the solve itself is
  the cost: dragging an end of a fully dimensioned chain of 2,000 lines to a point it cannot reach
  takes seconds a frame in a release build, where a chain joined only by `Coincident` takes tens of
  milliseconds.
- [medium · hard] Conflict diagnosis confirms each constraint of a conflict with a damped
  Gauss–Newton descent over the whole part, so a conflict running through a part of a few hundred
  lines (a chain of 300 with its far end fixed out of reach) still runs out of `DIAGNOSIS_WORK` and
  is reported as not solving. Each confirmation there already converges in one step, but the set
  found holds all 600 distances and coincidences and each step is charged its ~900 equations, so
  trimming alone needs ~540,000 units against the ~430,000 left: the cost is quadratic in the
  conflict's size whatever the descent does. One factorisation of the Jacobian at the set's
  least-squares point (its left null vector gives every witness as a step through one
  pseudo-inverse column), checked against the residuals and charged by that work, would make each
  confirmation cheap; a confirmation that does not converge falls back to the descent.
- [medium · hard] A sketch solved from a degenerate start can fail to solve again from its own
  result: a spline with four coincident control points, tangent to a zero-size arc on one of them,
  with a zero distance from that arc to the spline's first point. `sketch_solve` finds such cases
  within minutes once it requires `solve_from` of a solved geometry to succeed; it does not yet (it
  discards that result), so the property is unchecked.
- [low · medium] `10 mm^2` means (10 mm)², since a power binds to the measure before it; stored text
  relies on that reading, so changing it needs a new spelling or a format change.

## Sketching

- [medium · hard] Tools missing: a pattern of sketch geometry along a path (copies tied to the
  path would need a vector-equality or along-the-curve spacing the solver lacks), and text (a
  font, a height, bold and italic, set along a curve, its letters becoming closed regions that
  extrude).
- [low · medium] Fit-point splines pass their points at evenly spaced parameters
  (`BSpline::interpolate`, `interpolate_closed`), so unevenly spaced fit points overshoot between
  them; chord-length parameters would need the knots, and the solver's spline handles, to follow
  the points. A point held on a closed spline stops at its seam, its parameter clamped to one
  turn, and DXF fit-point splines still import as control splines (`BSpline::through`).
- [low · medium] Ellipses and elliptical arcs are drawn, constrained (point on, concentric, level
  or upright axis, tangent to a line, both radii) and swept, but cannot be trimmed, extended,
  split, broken, offset, filleted, mirrored or patterned (a copy needs its minor radius held equal
  to the original's, which no constraint does), take no tangent with a circle, arc, spline or
  another ellipse, no distance, `Equal` or midpoint, offer snaps only at their own points (not
  the other ends of their axes or an arc's middle), project into other sketches as splines,
  and their radii are left out of exported drawing dimensions.
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
  the same parameters. Refuse in words a tooth count that undercuts at the chosen shift. A
  sprocket for roller chain (ISO 606 pitch and roller diameter, tooth count) belongs in the same
  tool.
- [medium · hard] The centre of an outline of odd sides and a slanted track place a point without
  a constraint keeping it there, as the sketch has no centroid or point-on-a-direction constraint;
  an odd outline with arcs has no centre at all. A drag snaps only its handle (the moving point
  nearest the press), not whichever moving point comes near a target.
- [medium · hard] Splines cannot be trimmed or extended (`TrimError::Spline`). Offset takes
  one chain at a time, leaves the free ends of an open chain sliding along their curves and cannot
  offset splines; a sketch fillet cannot round a spline and drops equal lengths and midpoints of the
  lines it shortens, as trim does.
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
- [low · hard · blocked by: vector hidden-line removal ("Technical drawings")] Project takes edges,
  corners and the boundaries of faces, but not the outline of a body seen square to the sketch
  plane: the silhouette lines of a cylinder lying along the plane or the circle of a sphere cannot
  be brought in, so a sketch following a body's outline is drawn by hand.

## Modelling features

- [medium · hard] Offset face moves planes, cylinders, cones, spheres and tori only: a spline,
  extrusion or revolution face becomes its offset surface (which needs a surface fitted within
  tolerance), and a fillet moved beside a plane that stays is refused rather than re-blended.
- [high · hard] Bodies cannot be edited directly beyond offsetting faces: no moving, deleting or
  replacing a face and no deleting a fillet or chamfer by its faces. An imported STEP body has
  no feature history, so today it can only be cut, joined, filleted or shelled; a wall too thick, a
  hole in the wrong place or a fillet to remove means remodelling it from scratch. Direct edits
  become features of their own, named from the faces they move, so they stay parametric and
  undoable.
- [low · medium] A body splits along a plane, a sketch curve or another body whole, but not along
  one curved face of another body extended past its edges, and a mate places it by one pair of
  faces or axes: a face and an axis together (flush and concentric) take two mates in a row, and
  there are no tangent, angle or point mates.
- [low · medium] Mirror reflects a whole body or chosen features (extrusions, revolves, holes), but
  not chosen faces: reflecting a set of faces (a pocket's walls and floor of an imported body
  with no features) across a plane, kept linked to the faces it copies, is missing.
- [low · medium] 3MF exports carry no cosmetic thread: naming one needs object metadata
  (`metadatagroup`) under a name in a namespace of caditor's own, and that namespace's URI, kept
  for good once files carry it, has not been chosen.
- [low · medium] Scale model refuses a centre off principal geometry a feature uses (a mirror
  across the YZ plane, a revolve or circular pattern about the Z axis, a coordinate system at the
  origin), since that geometry cannot move with the model; scaling about such a point would need
  the feature moved onto a datum standing where the principal geometry lands.
- [medium · hard] An end up to the next face or a curved face follows curved or several faces
  only on one side and without an offset (two sides or an offset need one flat face). An end
  cannot end on a whole body (where the profile last leaves it); an extrusion along a direction
  takes no taper, up to next or curved face; a revolve turns up to a face or plane only when it
  holds the axis, never up to a curved face or the next face it meets; and a hole stops at the
  next face only where that face is flat.
- [medium · hard] A concave fillet running out under a rounded rim whose fill reaches nearly to
  the rim's tangent with the top face (a 3 mm fillet on a notch floor 3.5 mm under a puck's top
  with a 3 mm rim) fails as a face that could not be divided; smaller ones and chamfers work
  (`blend::tests::a_notch_fillet_climbing_onto_a_rounded_rim_stays_inside_the_puck`).
- [medium · hard] Blends: only line and circle edges along planes, parallel cylinders and coaxial
  surfaces; no ellipse, spline or intersection edges, not even a straight edge beside a spline
  extrusion face; ends at steps and T-junctions refused; no variable radius; a round corner only
  for three convex straight edges meeting at three planes (other corners mitre). Missing as shapes
  of their own: a full-round fillet across a narrow face between two others, a fillet sized by
  chord length, a fillet that runs by a rule over every edge of a kind, setback corners where three
  fillets meet, a tangency weight, and a curvature-continuous (G2) fillet.
- [medium · hard] Blend corners still refused where a blend is possible (`blend/survey.rs` chamfers
  every corner of twenty bodies): a convex edge chosen with the concave edges at its foot (a boss's
  corner edge with its base) ends after the fill where two fill faces meet and is `UnsupportedEnd`
  (`a_convex_edge_rising_from_bevelled_concave_edges_is_refused_at_its_foot`), a missing corner
  case that needs the base blend carried round the corner's own blend; and feet meeting exactly
  across a fill are refused, `TooLarge` when a rim chamfer meets a boss's skirt on the face between
  them and `Lost` when the fills use up a pocket's walls
  (`feet_meeting_exactly_across_a_fill_are_refused`), a tolerance question of whether a face
  narrowed to nothing should vanish. Fillets fail more corners than chamfers: a concave edge with
  the convex edge rising from its end (`AfterFill(TooLarge)`) and a notch's floor edge with its
  wall edges (`Boolean(Invalid(PcurveEnds))`).
- [medium · hard] Shell: no spline, extrusion or revolution faces, only flat faces open, one
  thickness for the whole body and always inward: no thickness per face, and no wall growing
  outward or to both sides of the faces.
- [medium · medium] An extrusion starting at a face or plane shows no reach arrows, and a revolve
  starting off its sketch plane shows no angle arrows.
- [medium · hard] Sweep along a path and loft between profiles: the kernel has only extrusion and
  revolution, so both need new kernel operations first. A sweep takes a profile and a path (a
  chain of edges or sketch curves), kept square to the path or parallel to the profile, with an
  optional guide rail, taper and twist; a loft takes two or more profiles or faces (a point may
  end it), open or closed back to the first, with optional rails or a centreline and tangent or
  curvature-continuous conditions at its ends.
- [medium · hard] No pattern along a curve or driven by sketch points: a pattern repeats along one
  or two axes (sketch lines included) or about one, never along a spline or arc (the copies kept
  as they are or turned to follow the curve), nor at the points of a sketch, and it repeats
  features or whole bodies but never chosen faces. Every copy is the original's tool placed again,
  never recomputed where it lands (a copy of an extrusion up to next stops where the original
  did, not on the face it meets), and one pattern cannot chain a shift, a turn and a mirror.
- [medium · hard] No split face: dividing a face along a sketch curve, a plane or another body,
  without cutting the body, so a part line, a stripe of another colour or a face to draft or delete
  in part can be had. It is a feature of its own, naming the faces it splits.
- [low · medium] Expressions cannot read a measured value (a distance or angle taken from the
  geometry): parameters evaluate before and apart from recompute, so a measured one would need
  recompute to evaluate parameters in tree order beside the features, the measured reference
  healed like any other and a cycle through the feature it drives refused. Nor can a measurement
  be kept in the model: a named reading between two references, updated on every recompute and
  drawn in the view, by which a clearance is watched while upstream features change and which
  expressions could then use.
- [low · hard] Scale is uniform: a body cannot be stretched by different factors along the three
  axes (a plane stays a plane, but a cylinder becomes an elliptical one, which the kernel's
  surfaces do not have).
- [medium · hard · blocked by: the sweep feature] No helix or spiral curve and no modelled threads:
  springs, coils and threaded holes and shafts cannot be modelled with real thread geometry (the
  cosmetic Thread feature covers drawing and exchange). The sweep feature (same list) needs
  the helix, and the thread feature then offers a modelled form beside the cosmetic one. A coil
  feature would be the ready tool for springs: revolutions or height and pitch, a round or square
  section, inside or outside the axis, and a taper angle.
- [medium · hard · blocked by: the sweep feature] No pipe: a round, square or triangular section
  swept along a path sketch, solid or with a wall thickness, with sharp or rounded corners, as the
  ready tool for tubing, handrails and cable runs.
- [medium · hard · blocked by: direct face edits ("Bodies cannot be edited directly")] Draft angle
  on existing faces: a fixed angle from a plane, a split at a parting line with an angle on each
  side, and an angle per face, following tangent faces as one chain.
- [medium · hard] Rib and web from an open profile: a rib extrudes parallel to the sketch plane
  and a web square to it, each thickened and run on to the nearest faces of the body.
- [low · hard · blocked by: sketch text ("Tools missing" under Sketching)] Emboss or deboss sketch
  text, or any sketch profile, onto a face, flat or curved (wrapped around it), raised or
  recessed by a depth.
- [low · hard · blocked by: split face (above)] No silhouette split: dividing a body along its
  outline seen from a chosen direction, so the parting line of a moulded or cast part can be a
  face boundary for a draft to start from.
- [low · hard · blocked by: draft angle (above), rib and web (above)] No plastic-part features:
  the screw boss with its ribs, a lip and groove along a seam, snap fits (hook, loop, groove) and a
  rest (a flat seat on a curved face), which moulded parts need.

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
- [low · hard] A face whose surface cannot be read is left out (`step-read.md`, "Unreadable
  faces"), but from the outer shell that turns the whole body into flat facets, since the kernel
  holds only closed solids, and a lost face with holes is not closed at all; an edge whose curve
  cannot be read still loses the whole body. An offset that folds anywhere in its basis's domain
  is refused even where the face's own region is clear: fit over the region the face uses.
- [low · easy] STEP styling is read and written in every form the standard and the tests
  cover (`step-read.md`), but has been checked only against a handful of real files with plain
  colours. Read files with see-through bodies and faces and with copies coloured on their own from
  SolidWorks, CATIA, Creo, NX, Fusion and FreeCAD (`STEP_CORPUS`), and support any styling the
  import report names as not understood.
- [high · hard] A large STEP import still stalls the interface while its bodies arrive: every
  showing rebuilds the base scene, whose one batch holds every body edge as line segments (13.2
  million for the VZ330 assembly before edges were sampled sparingly, 0.25 to 0.4 s a rebuild on
  the UI thread in release, and the renderer uploads the whole batch again unbudgeted). Drawing a
  still frame of the VZ330 then took about 190 ms on the GPU (RX 9070 XT, 1920×1080, 4x): 100 ms of
  edge lines, 28 ms of meshes, 20 ms of silhouettes; time it again, and consider culling batches by
  bounds and a coarser mesh for bodies small on screen. Each body's edges and vertices could be a
  batch of its own, cached while its `BodyMesh`, style and highlight are unchanged, with pick ids
  that do not shift when other bodies come and go; hovering over a large model rebuilds the same
  way. Reading is now bound by single parts: the VZ330's lead screw (406 faces, helical splines of
  7 by 1441 control points) takes most of the 71 s alone, in healing and `find_crossing`; opening a
  saved model reads every import text with `read_step` again, crossing check included
  (`step_cache.rs`).
- [low · hard] No IGES import or export, though older CAM software and many suppliers still exchange
  it.

## Drawing import and export

- [low · medium] SVG import leaves text out with a note rather than importing it as outlines. The
  only font in the workspace, Inter, is bundled by the `caditor` crate (`assets/fonts`), out of
  `caditor-file`'s reach, and turning text into outlines also needs a font parser (`ttf-parser` is
  already in the lock file through egui) plus text layout: `tspan`, per-glyph x and y lists,
  `text-anchor`, font size and family through the cascade. Decide whether the app hands the font
  to the import or `caditor-file` bundles its own.
- [low · medium] Drawing export places dimensions by a fixed offset from the sketch's middle
  without the canvas's lanes or obstacle avoidance, so crowded sketches overlap their labels. The
  canvas's layout lives in `caditor`'s `annotation_layout.rs`, which `caditor-file` cannot call
  without a dependency cycle; it would have to move to a crate both use (or the app hand the
  exporter its placed labels).

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

- [low · medium] Section caps are drawn from back faces, so a sheet that is not a closed solid
  (an imported open shell) shows its far side capped as if cut, and box selection of sketch curves
  still takes curves a section plane cuts away.
- [low · medium] Translucent lines keep square ends, so a translucent polyline still notches where
  its segments meet at an angle; joining them without blending twice needs mitred joins built
  from the neighbouring segments, which instances do not know.
- [low · medium · blocked by: wgpu's GL backend] On GL and other devices without texture view
  formats the multisample resolve still averages in gamma space. A resolve of its own (a pass
  reading the samples through a `texture_multisampled_2d` and averaging them in linear light) was
  tried: it matches the view-format resolve on Vulkan, but wgpu 30's GL backend binds a
  multisampled texture as `TEXTURE_2D` (`gles::Texture::get_info_from_desc` never chooses
  `TEXTURE_2D_MULTISAMPLE`), so every sample reads as zero there and the frame comes out black. It
  needs that fixed in wgpu, or a GL-only blit resolve into an sRGB texture.

## Interface performance

- [medium · medium] An egui-only repaint (a tooltip, hover over a panel, a spinner tick) draws the
  whole 3D pass again. Keeping the resolved view in a surface-sized texture and copying it while
  the scene generation, view, rect, graphics settings and picks are unchanged would save the GPU
  that work.
- [low · easy] Smaller per-frame costs measured or found in the resource survey:
  - `CommandFrame::invoke_detailed` allocates each unavailable reason with `to_string`
    (`commands.rs`).
  - `Highlight::is_hovered` searches a slice for every scene item (`scene.rs`), and `hovered` can
    hold a whole body.
  - Key hints, shortcut lists and the window title are formatted again every frame.
- [low · easy] Fallback system fonts are held twice in RAM (`font_fallbacks.rs` loads them as owned
  data and epaint copies it again), up to tens of MB with CJK fonts. Face analysis also keeps a
  split mesh per body after its panel closes (`analysis.rs`).
- [low · medium] A pick in flight is polled every millisecond (`PICK_CHECK`, `app.rs`) until it is
  answered. A helper thread blocking on the device could wake the app instead.
- [low · medium] GPU records are wider than needed. Mesh vertices are 28 B, where a normal packed
  into 16-bit values would give 20 to 24 B. Silhouette records are 60 B, and two 16-bit values per
  normal would give 48 B. Line, marker and fill records carry an `f32x4` colour that `Unorm8x4`
  would fit. Pickable fills are uploaded twice (`fills` and `pick_fills`), the hidden-edges style
  pushes a second copy of every edge, and batches are never culled, the pick pass included.
- [low · medium] Annotations are laid out only near the view and kept until the view or sketch
  changes (`annotations::Marks`), but a camera move zoomed out over a dense sketch still places
  every glyph in view against crowded `Obstacles`: about 350 ms a frame for the `large_sketch`
  benchmark (5,000 dimensions, 3,000 glyph constraints) in a release build, and a still frame
  there still paints and hit-tests every mark in view (about 10 ms). Glyphs could thin out or
  give up early when their surroundings are full. A sketch card always lays out redundant rows
  and rows wanting focus, whose height varies.
- [medium · hard] The cached scene is one batch: any change to its content (each drag solution, an
  edit, an evaluation, a new faceting level) facets every drawn sketch again, and a hover or
  selection change restyles and uploads all of it, over a millisecond to rebuild and about half of
  that to upload for a sketch of 24,000 curves in a release build. A batch per feature, with pick
  ids of its own, would limit both to what changed.
- [low · medium] Vertex records repeat per-layer data and both ends of shared segments, and
  resizing recreates the MSAA targets per pixel: they must match the surface they resolve into, so
  keeping larger ones would need a resolve pass of their own.
- [low · hard] Snapping projects every point and curve of the sketch on every hover frame
  (`snap.rs`), a cost linear in the sketch that is most of the frame for tens of thousands of lines,
  mostly walking the entities, and a line or slot end walks every line again to find the nearest for
  parallel and perpendicular inference (`drawing.rs` `guides`); a screen-space index would need the
  preimage of the snap radius on the sketch plane, unbounded near the horizon.

## Application

- [medium · medium] On Wayland the portal file dialog request passes an empty parent window, so the
  dialog is not tied to caditor's and can open behind it (Stop waiting in the status bar recovers
  the window); it needs an exported xdg-foreign handle, which winit does not offer and the raw
  Wayland connection would need `unsafe` to reach (a third `unsafe` crate, as `windows.md` keeps
  for Win32). X11 names the window already.
- [medium · hard] Version history shows when a version was saved and after which change, but no
  preview of what it holds.
- [medium · hard] Pasting features cannot carry a feature that picks faces or edges of another
  copied feature (a fillet copied with its extrusion): face and edge names are digests over the
  feature id, so the copy is left out with the reason. Renaming them needs each picked face or edge
  found again in the copy's recomputed result (by matching it in the original's) before the paste
  is applied. Pasted features also take no group, and a copy from another model keeps none of its
  references outside the copied set.
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
- [medium · hard · blocked by: winit 0.30 has no drag and drop on Wayland (0.31 is only a beta)]
  Dropping files on the window, and the outline and card that say what the drop would do
  (`drop_target.rs`), work only under X11 and Windows: on Wayland nothing is reported while files
  are dragged over the window and a drop does nothing, so importing there goes through the file
  dialog alone. Move to a winit that reports `HoveredFile` and `DroppedFile` on Wayland, or take the
  `wl_data_device` events directly (which needs `unsafe` access to the raw connection, as the
  portal parent window above does), and test that a drag over the window shows the card and a drop
  imports on a Wayland session. Dragging a file from the file manager onto the window is how most
  people expect to import, so this is the way in that most needs to work everywhere.

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
- [medium · hard · blocked by: 2D drawings (above)] Annotations beyond plain dimensions and notes:
  centre marks and centrelines on holes and round edges, hole and thread callouts read from the
  hole and thread features, ordinate and baseline dimensions, tolerances (plus and minus, limits,
  ISO 286 fits), geometric tolerances with datum features (ISO 1101), surface texture symbols
  (ISO 21920) and weld symbols (ISO 2553), and a title block filled from the model properties.
- [low · medium · blocked by: 2D drawings (above)] Views and sheets beyond the standard ones:
  auxiliary views square to an inclined face, broken views shortening a long part, broken-out and
  half sections, cropped views, first- or third-angle projection, ISO 5457 sheets from A4 to A0 and
  several sheets per drawing, a revision table, and section hatching chosen per material.

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
  keeping references stable across edits. The same decision covers deriving: bringing the bodies,
  sketches or parameters of another model file into this one, linked so they update when that file
  changes, which a skeleton model driving several parts, or a part fitted to its neighbour, needs.
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
- [low · medium · blocked by: the project's decision to publish no maintainer identity or repository
  URL] No Flatpak or AUR package: both need a maintainer identity and repository URL in their
  metadata, which the project does not publish (`docs/RELEASING.md`); `packaging/arch/PKGBUILD` only
  builds locally.
