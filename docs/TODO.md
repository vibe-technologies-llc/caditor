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
- [low · hard] Two faces side by side that the shell's thickness both closes up (a narrow chamfer
  cone below a narrow lid cone) are refused as `ClosesBesideClosing`: their joints would need the
  offset of the next face that survives, found across a run of dropped bands whose ridges meet
  each other, so the bands would have to be merged into one band spanning several faces, with
  their seams and joints ordered across all of them.
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
- [medium · medium] A large face triangulated in pieces still leaves about a fifth of its
  triangles to one serial remainder (those straddling a cut, and fans like those along a plate's
  long straight sides whose circumcircles leave every strip: 2 of the 9 ms of the top of a plate
  with 113 holes), and strips are cut at point quantiles rather than where the face is narrow
  (through a row of holes), so past about six strips they keep little. Spade's exact in-circle
  predicates on the cocircular samples of round holes are a quarter of all tessellation time.
- [low · hard] `SolidResult::cuts`/`joins` keep every tool solid with every history entry,
  though only patterns, mirrors and an open feature read them; a removal's tool shares little
  with the result, since a difference reverses the tool's kept faces and their pcurves (an
  addition's tool shares its kept faces' geometry, which the history counts once). Dropping them
  needs the feature's key to say whether anything wants its tools: a later pattern or mirror
  repeating it, and the feature open in the app, whose tools `Recomputer::mesh` meshes on request
  without recomputing. Adding a pattern or opening a hole would then evaluate the feature again
  and, as `same_shapes` compares tools, everything below it; the tools are usually a few faces
  next to the body, so this waits for a model where they weigh.
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
- [medium · hard] Before naming a conflict, diagnosis descends on it from the drawn shape and from
  the closest witness, and damped Gauss–Newton crawls on a set that cannot hold: near its
  least-squares point the Jacobian is nearly singular, so the line search cuts each step back and
  it gains about 1%, too much to count as stalled. On a chain of 300 lines with its far end fixed
  out of reach those two descents (three attempts each, most running all 100 iterations) take about
  370,000 of the 500,000 units of `DIAGNOSIS_WORK`, against about 90,000 for the search and 26,000
  for trimming, leaving about 14,000, so a longer chain would run out there and be reported as not
  solving. A step regularised along the near-null direction (Levenberg–Marquardt, or the
  factorisation trimming already makes) would reach the minimum in a few steps.
- [medium · hard] A sketch solved from a degenerate start can fail to solve again from its own
  result: a spline with four coincident control points, tangent to a zero-size arc on one of them,
  with a zero distance from that arc to the spline's first point. `sketch_solve` finds such cases
  within minutes once it requires `solve_from` of a solved geometry to succeed; it does not yet (it
  discards that result), so the property is unchecked.

## Sketching

- [high · medium] A sketch reaches model geometry only through Project or Intersect first: drawing a
  line from a body's corner, centring a circle on a round edge's centre or dimensioning a point
  from a body's edge means projecting each item, then going back to the tool. Snapping (`snap.rs`,
  `tracking.rs`) could take the corners, edge middles, round edges' centres and edges of the shown
  bodies (as background while a sketch is edited, read from the body's state at the sketch through
  `body_result_seen_by`), and a point landing on one would project that item in the shape's own
  transaction and join the point to the projection, as Onshape and Fusion infer from model edges;
  Smart dimension and the constraint tools would take a body edge or corner the same way. The body
  vertices and edges then need projecting to the screen on the UI thread within the bounds the
  sketch's own snapping keeps to (Interface performance).
- [medium · medium] The Line tool cannot turn into an arc mid-chain: carrying on with a tangent arc
  means the Tangent arc key, then the Line key again. A press on the chain's last point dragged
  away (`drawing.rs`; a press-drag now places the press only for a first point,
  `viewport::DRAG_DRAWS_FROM_PRESS`) could draw a tangent arc from it, the release placing its end and the
  chain carrying on with lines afterwards, as Fusion's line tool does; the chain's anchors
  (`ChainStep`) already let the two tools share one chain.
- [medium · hard] Tools missing: a pattern of sketch geometry along a path (copies tied to the
  path would need a vector-equality or along-the-curve spacing the solver lacks), and text (a
  font, a height, bold and italic, set along a curve, its letters becoming closed regions that
  extrude).
- [low · hard] An ellipse's offset is a free fit-point spline that does not follow the ellipse:
  holding it would need a constraint keeping each fit point on the ellipse's normal at its own
  parameter (a `Distance` from the ellipse lets every fit point slide along the offset and would
  put a dimension on each), a new constraint kind with its solver form, file record and glyph.
  An ellipse also cannot be offset within a chain of lines and arcs, whose joints would need the
  spline to meet them.
- [medium · hard] A spline is only the control points it was drawn with: a point has no tangent or
  curvature handle to set the direction and pull of the curve there, which would be stored as
  constraints on the point rather than as positions so the solver and dimensions keep reading
  them; the degree cannot be chosen; and no point can be inserted or removed while keeping the
  shape.
- [medium · hard] A spur gear (`sketch.md`, Spur gears) is drawn once: its curves are free sketch
  geometry, not tied to the values in the Spur gear panel (expressions using parameters are read
  when it is drawn), so changing the module, teeth or pressure angle, or a parameter they use,
  means deleting it and drawing it again, and nothing holds its shape when one of its points is
  dragged. Keeping it parametric needs a gear stored in the sketch (its expressions and the curves
  it made, saved in the file) that recompute draws again from the evaluated values, holding its
  curves fixed like projected geometry; the teeth changing the number of curves is the hard part.
  A pair of meshing gears at a centre distance following from the same parameters is missing, and
  so is a sprocket for roller chain (ISO 606 pitch and roller diameter, tooth count) in the same
  tool.
- [medium · hard] The centre of an outline of odd sides and a slanted track place a point without
  a constraint keeping it there, as the sketch has no centroid or point-on-a-direction constraint;
  an odd outline with arcs has no centre at all. A drag snaps only its handle (the moving point
  nearest the press), not whichever moving point comes near a target.
- [medium · hard] Splines cannot be trimmed or extended (`TrimError::Spline`). Offset takes
  one chain at a time, leaves the free ends of an open chain sliding along their curves and cannot
  offset splines; a sketch fillet cannot round a spline and drops equal lengths and midpoints of the
  lines it shortens, as trim does.
- [low · easy] A typed value locks only a direction (`Drawing::lock_heading`): a line's length
  cannot be held while the pointer chooses its direction, nor a circle's or arc's radius while the
  pointer chooses where it goes round, nor one side of a rectangle while the pointer sets the other.
  A lone length typed as a lock (a marker after it, with its own prompt beside `HEADING_PROMPT`)
  could hold the distance from the last point, the pointer's direction landing on the circle of
  that radius with snaps joining where they cross it, and keep the length as a dimension as a
  typed one is (`TypedDimension`).
- [low · medium] Offset copies a chain to one side only: an outline round a centreline (a slot
  following a path, a wall drawn by its middle) needs two offsets and the closing lines or arcs
  drawn by hand. A Both sides way of Offset (`offsetting.rs`, the distance on each side) closing an
  open chain's ends with arcs or lines (the cap ends of Fusion and SolidWorks), each held to the
  original as the one-sided offset is, would make the closed profile in one step; a thin wall
  extrusion covers only an outline extruded straight.
- [low · medium] No symmetric drawing: while half of a symmetric outline is drawn the other half
  cannot be made with it. Choosing a sketch line or axis to draw about (a Sketch menu toggle, as
  SolidWorks's dynamic mirror) could add each finished shape's mirror image with its `Symmetric`
  constraints in the shape's own transaction, as Mirror does afterwards (`mirroring.rs`).
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
- [medium · easy] New features always start from the `DEFAULT_*` constants of their `*_tools.rs`
  (a 1 mm fillet, a 6 by 10 mm hole, a 1 mm shell, a 10 mm extrusion), so filleting a part with 3 mm
  fillets one set of edges at a time means typing 3 each time. The last value committed for each
  (a fillet's radius, a chamfer's form and distances, a shell's thickness, a hole's size, fit,
  style and depth kind, a pattern's counts) could be kept while caditor runs, as Sketch fillet
  keeps `LastSizes` and Spur gear its `GearSettings`, and start the next one of its kind, as typed
  (a parameter only while the model still has it).
- [medium · easy] Fillet and Chamfer take only edges (`blend_tools::selected_edges` refuses faces
  with "Select the edges of a body first"), so rounding every edge of a face or of a whole body
  goes through Select the edges around the selected faces first. Selected faces could give their
  boundary edges and a body chosen in the tree every edge of it (`body_selection::face_boundary`,
  as that command uses), so filleting a face's edges is one step as in Fusion, Onshape and
  SolidWorks; the edges are captured as they are, as a selection of them is.
- [medium · easy] An extrusion's or revolve's panel names its sketch in a combo but cannot open it
  (`solid_panel::sketch_row`), so changing the profile of an extrusion made from a face, whose
  sketch is hidden, means finding that sketch in the tree. The row could offer Edit the sketch as
  the hole panel does (`hole_panel::EDIT_SKETCH`), and Edit the sketch of the selected face
  (palette, the view's context menu) could enter the sketch of the feature that made the face
  (`viewport::feature_of`) directly, as SolidWorks's Edit Sketch on a face does.
- [medium · medium] Only extrusion and revolve ends, a hole's depth, an offset face's distance, a
  datum plane's offset and the move and placement handles drag in the view
  (`manipulator::Manipulator`, `length_handles::Measured`): a fillet's radius, a chamfer's
  distance, a shell's or thin wall's thickness, an extrusion's end offset and taper, a hole's
  diameter and a datum plane's angle are only typed. Each could be a `Handle::Length` or a turn
  handle on the open feature (a fillet's at the middle of its first chosen edge along the bisector
  of its faces, a shell's on the rim of its first opened face), with its value beside the pointer
  and committed through `manipulator::Held` like the others.
- [medium · medium] Handle drags move only in steps along their line or plane (`reach_handles.rs`,
  `place_handles.rs`, `move_manipulator.rs`): an extrusion's arrow dropped on a face or corner
  does not stop there, a hole's square does not snap to a round edge's centre or an edge's middle,
  and the dragged value cannot be typed mid-drag. Snapping a drag to the shown bodies' corners,
  edge middles, round edges' centres and faces (the distance to them along the handle, ahead of
  the steps, Ctrl still dragging freely) and opening the typed-point field on a digit while a
  handle is held or hovered, committing to the dragged field, would give the drag-to-geometry and
  typed values of Fusion's handles; a snap sets a measured value rather than a reference, and the
  readout would say what it snapped to.
- [medium · medium] A hole placed on a face moves only by Position X and Y along its hidden
  sketch's own axes or by dragging, so a hole 8 mm from two edges, or concentric with a round edge,
  means editing the sketch, projecting the edges and dimensioning. While `hole_tools::lone_point`
  holds, the hole panel (`hole_panel.rs`) could have From an edge rows (a straight edge of the face
  picked with Use selected or Choose in the view, and a distance) that project the edge into the
  hidden sketch and add a `Distance` from it in one change, and Concentric with a picked round edge
  (its projection's centre `Coincident` with the point), so the hole follows those edges when the
  body changes; Add another hole could place a further point on the face by a click, as Fusion's
  hole places several.
- [low · easy] Revolve takes only a sketch: a flat face of a body selected with an axis or straight
  edge is refused (`NOTHING_TO_REVOLVE`), though Extrude turns one selected face into a hidden
  sketch of its boundary (`solid_tools::create_on_face`). Revolve could do the same with the face
  and the axis picked with it, and both could take several coplanar faces of one body as one
  profile rather than one face (`sketch_placement::selected_face`).
- [low · medium] Primitives and patterns have no size handles: a box's width, depth and height, a
  cylinder's radius and height and the other primitives' sizes are only typed, while only their
  position drags (`place_handles.rs`), and a linear pattern's spacing and count or a circular
  pattern's count and angle do not drag at all. Arrows on a box's faces and a cylinder's rim and
  top, and for a pattern one at its last copy dragging the spacing or angle and one past it adding
  copies, would follow `length_handles.rs` and commit through `manipulator::Held`.
- [low · medium] Mirror faces closes an opening only in one plane or on one elementary face beside
  it: an opening on an extrusion, revolution or spline face, one running all the way around a
  round face (a collar or a groove), or one spanning several curved faces is refused, since
  closing it needs a freeform patch, a ring face or a surface fitted across it.
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
- [medium · hard] Sweep along a path and loft between profiles: the kernel has only extrusion and
  revolution, so both need new kernel operations first. A sweep takes a profile and a path (a
  chain of edges or sketch curves), kept square to the path or parallel to the profile, with an
  optional guide rail, taper and twist; a loft takes two or more profiles or faces (a point may
  end it), open or closed back to the first, with optional rails or a centreline and tangent or
  curvature-continuous conditions at its ends.
- [medium · hard] Patterns stop short of what other modellers repeat: a curve pattern follows only
  the curves of one sketch, never a chain of model edges (a 3D path, the copies then turning with
  its tangent and normal), and a pattern repeats features or whole bodies but never chosen faces.
  Every copy is the original's tool placed again, never recomputed where it lands (a copy of an
  extrusion up to next stops where the original did, not on the face it meets), and one pattern
  cannot chain a shift, a turn and a mirror.
- [low · hard] Split face wraps closed outlines only within once round a cylinder or cone, and a
  wrapped open chain may not run right round and back to the edge of the faces it started from:
  a helical stripe drawn as one outline running round more than once, a band whose ends meet
  after one turn and a chain turning back after going round all need a tool that crosses the seam
  of the unrolled window in one piece (its two edges glued into one solid, the caps then whole
  rings), which the window's separate regions cannot build; a sketch of two or more open chains
  (a stripe between two helices) is refused too. Spheres stay refused in words: a sphere has no
  flat unrolling, so wrapping onto one first needs a chosen projection (stereographic, equal area
  or along its axis) and the distortion it brings, a design decision before any kernel work.
- [low · medium] Open decision on kept measurements: a failed or suppressed measurement fails the
  features using its value, as a failing feature's dependents do; whether they should instead keep
  its last reading (the measured parameter's stored value already holds it) is undecided.
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
- [low · hard] No silhouette split: dividing a body's faces along its outline seen from a chosen
  direction (a split face whose tool is that outline), so the parting line of a moulded or cast
  part can be a face boundary for a draft to start from.
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
  edge lines, 28 ms of meshes, 20 ms of silhouettes; time it again, and consider a coarser mesh for
  bodies small on screen (batches are culled by their bounds, which only pays once bodies are
  batches of their own). Each body's edges and vertices could be a
  batch of its own, cached while its `BodyMesh`, style and highlight are unchanged, with pick ids
  that do not shift when other bodies come and go; hovering over a large model rebuilds the same
  way. Reading is now bound by single parts: the VZ330's lead screw (406 faces, helical splines of
  7 by 1441 control points) takes most of the 71 s alone, in healing and `find_crossing`; opening a
  saved model reads every import text with `read_step` again, crossing check included
  (`step_cache.rs`).
- [low · hard] No IGES import or export, though older CAM software and many suppliers still exchange
  it.

## Drawing import and export

- [low · medium] SVG text comes in as outlines in Inter (upright and italic): `textPath` is left
  out, vertical writing modes, `textLength`, `baseline-shift` and shaping beyond pair kerning
  (ligatures, marks, right-to-left scripts) are not applied, and letters of different glyphs that
  overlap are not merged into one outline.

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

- [low · easy] Look straight at the selected face takes only a flat face and always looks along its
  outward normal (`sketch_placement::face_to_look_at`): pressed again it changes nothing, and a
  principal or datum plane, a sketch not being edited, a round face or a straight edge cannot be
  looked at. Pressed while the view already faces it, it (and Look at sketch) could turn the view a
  quarter turn about the normal, as SolidWorks's Normal To and Onshape's View normal to do, and it
  could take a plane or a sketch (its solved plane), a round face (along its axis) or a straight
  edge (along it).
- [low · medium · blocked by: wgpu's GL backend] On GL and other devices without texture view
  formats the multisample resolve still averages in gamma space. A resolve of its own (a pass
  reading the samples through a `texture_multisampled_2d` and averaging them in linear light) was
  tried: it matches the view-format resolve on Vulkan, but wgpu 30's GL backend binds a
  multisampled texture as `TEXTURE_2D` (`gles::Texture::get_info_from_desc` never chooses
  `TEXTURE_2D_MULTISAMPLE`), so every sample reads as zero there and the frame comes out black. It
  needs that fixed in wgpu, or a GL-only blit resolve into an sRGB texture.

## Interface performance

- [low · medium] A camera move zoomed out over `large_sketch` (5,000 dimensions, 3,000 glyph
  constraints, 20,000 unjoined lines) still takes 7 to 9 ms a frame in a release build, now
  little of it dimensions (about 1 ms: tiny ones collapse and crowded ones are left out before
  layout): placing the glyphs of the circles and the lines long enough to keep theirs takes about
  5 ms, walking each group's placements against the obstacles, and the open-end rings take much
  of the rest, every ring in view projected, merged through a `BTreeSet` and painted (tens of
  thousands, also most of the 0.9 ms of a still frame). Rings could merge on a coarser grid when
  dense, and glyph groups could test a cheaper neighbourhood before walking placements.
- [medium · hard] The cached scene is one batch: any change to its content (each drag solution, an
  edit, an evaluation, a new faceting level) facets every drawn sketch again, and a hover or
  selection change restyles and uploads all of it, over a millisecond to rebuild and about half of
  that to upload for a sketch of 24,000 curves in a release build. A batch per feature, with pick
  ids of its own, would limit both to what changed.
- [low · hard] Snapping projects every point and curve of the sketch on every hover frame
  (`snap.rs`), a cost linear in the sketch that is most of the frame for tens of thousands of lines,
  mostly walking the entities, and a line or slot end walks every line again to find the nearest for
  parallel and perpendicular inference (`drawing.rs` `guides`); a screen-space index would need the
  preimage of the snap radius on the sketch plane, unbounded near the horizon.

## Application

- [high · medium] Outside sketch editing no dimension is shown on the model (`viewport.rs` hands
  `annotations::Annotations` only the edited sketch), so changing a size means opening the
  feature's panel or entering its sketch. Selecting a face or opening a feature could show, on the
  model, the dimensions of the sketch it was swept from (laid out by `annotation_layout` on that
  sketch's solved plane, as for the edited sketch) with the feature's own values as dimensions (an
  extrusion's distances along its reach, a revolve's angles, a fillet's radius, a hole's diameter
  and depth), each double-clicked to an inline `commit_field` committing through
  `field::dimension_transaction` or the feature's `SetFeatureKind`, as SolidWorks shows a
  feature's dimensions on double-click. Which dimensions show for a hidden sketch or a sketch
  several features use needs a rule.
- [medium · easy] No Repeat the last command: filleting several sets of edges, adding a run of
  datum planes or drilling holes on several faces reaches for the same button each time. A command
  (palette, the view's context menu as "Repeat <title>", the Edit menu) running again the last
  modelling or sketch command triggered, kept in the `Workspace` beside `deferred_commands` and
  offered with that command's own availability, so it takes the new selection as the command
  would.
- [medium · medium] Expression fields complete nothing: a parameter's name is typed exactly from
  memory or looked up in the Parameters panel. Typing in a `field::commit_field` could list the
  parameters whose names start with what is typed, with their values, Tab taking one; and while a
  field has focus, a click on a dimension label in the view or a value in the Parameters panel could
  insert its parameter's name (naming the value first when it has none, as `field::NamedField`
  does), as SolidWorks inserts a dimension clicked while an equation is typed.
- [medium · medium] No Select similar: every hole of one size, every fillet face of one radius or
  every face of the same shape is selected by clicking each one. Select similar (Edit menu,
  palette, the context menu's Select submenu) could add from the shown bodies the faces of the same
  surface kind and size as the selected ones (a cylinder of the same radius, a plane parallel to
  it, a torus of the same radii), from a hole's wall every hole of the same diameter and depth
  (`body_selection::hole_of` per hole), and for edges those of the same kind and length or radius,
  beside the other growers in `body_selection.rs` with the same cheap availability check.
- [medium · hard] Pasting features cannot carry a feature that picks faces or edges of another
  copied feature (a fillet copied with its extrusion): face and edge names are digests over the
  feature id, so the copy is left out with the reason. Renaming them needs each picked face or edge
  found again in the copy's recomputed result (by matching it in the original's) before the paste
  is applied. Pasted features also take no group, and a copy from another model keeps none of its
  references outside the copied set.
- [low · easy] Numeric fields do not step: Up and Down in a `field::commit_field` holding a plain
  number could add or take away one unit of its last digit (Shift for ten), previewed through
  `Action::Preview` as typing is, so a size is tried without retyping it; an expression or a named
  value would be left alone.
- [low · easy] Selection growers still missing beside those in `body_selection.rs`: Invert the
  selection (every face, edge or vertex of the shown bodies, of the kind selected, not selected
  now), Select the faces of the feature that made this face (every face whose `FaceOrigin` names
  that feature, to offset or colour a boss as a whole) and Select the loop of the selected edge
  (with the edge and one of its faces selected, that face's loop holding it, as Fusion's
  double-click on an edge selects a loop).
- [low · medium] The status bar reads the size of one selected item (`Offers::size`) but nothing for
  two: the distance between two points, edges or faces, or the angle between two flat faces or
  straight edges, still needs Measure open. While two items are selected and Measure is closed,
  the status bar could show the Between them distance or angle beside the selection, measured by
  `Measurements` on its worker as Measure does, as SolidWorks's status bar shows it.
- [low · hard] No automation: nothing can be driven by a script or macro, as Fusion's scripts and
  add-ins do, to make repetitive geometry, run a batch over files or add a tool. An interface
  would go through `Action`s and `Transaction`s like the UI, so scripts cannot break the model's
  rules, and would need a decision on the language and on safety (a script cannot reach files
  or the network unasked).
- [low · hard] One document per process.
- [low · hard] No localisation.

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

- [low · medium] Slow tests to keep an eye on: with egui, its glyph stack and the PNG encoder built
  optimised in the dev profile (`dependencies.md`) a UI test takes a few tenths of a second, and
  none of the ten that took over a second (the screen-reader test, the shortcut reset tests, the
  hole, extrusion, chamfer and section panel tests, the PNG export test) takes over 0.55 s. What is
  left is the app's own unoptimised frames and the recompute and meshing of the bodies the tests
  build; re-measure the whole UI suite and list the tests still over half a second.

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
  title bar (dragging, Aero Snap, Snap Layouts on the maximize button, resize strips,
  double-click, the corner close, mixed-DPI monitors), the rfd dialogs owned by the window, sign-out flushing the journal, the MSI from
  SmartScreen to uninstall, and a model and its journal on a USB stick (FAT32/exFAT, no POSIX
  rename) and on a network share.
- [low · medium · blocked by: the project's decision to publish no maintainer identity] The MSI
  and `caditor.exe` are not code-signed, so SmartScreen warns on first run; signing needs a
  certificate tied to an identity.
- [low · medium · blocked by: the project's decision to publish no maintainer identity or repository
  URL] The Linux `.deb`, `.rpm` and AppImage never update themselves: AppImage self-update needs
  update information and a `.zsync` file naming the repository the release is published in, and a
  `.deb` or `.rpm` update needs a package repository to add to the package manager, neither of
  which the project publishes (`docs/RELEASING.md`), so caditor is also not in a software centre.
- [low · medium · blocked by: the project's decision to publish no maintainer identity or repository
  URL] No Flatpak or AUR package: both need a maintainer identity and repository URL in their
  metadata, which the project does not publish (`docs/RELEASING.md`); `packaging/arch/PKGBUILD` only
  builds locally.
