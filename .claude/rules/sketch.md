---
paths:
  - "crates/caditor-sketch/**"
---

# Sketches

## Entities

- A sketch lies on a `Plane`; curves refer to their points by `EntityId`. The origin and two axes
  have reserved IDs (`EntityId::REFERENCES`) the counter never reaches; stored IDs stay below
  `FIRST_UNSTORABLE_ID`. Reference geometry never moves and cannot be removed or changed.
- Any curve, never a point, can be construction geometry (`Sketch::set_construction`): a set beside
  the entities, part of `same_geometry`. It solves, snaps and takes constraints like any curve, but
  `solid::profile_curves` leaves it out, so a centreline neither splits regions nor changes their
  keys. It can be a revolve axis.
- Any constraint can be inactive (`Sketch::set_active`): a set of `ConstraintId`s beside the
  constraints, part of `same_content`. The solver, `Sketch::evaluate`, the joint classes and `restating` and
  `contradicting` read `active_constraints()` only, so an inactive constraint no longer holds, never
  conflicts or counts as redundant, and a new one is not refused for restating it. A dimension
  left inactive is a reference: its displayed value is `Sketch::measured` of the solved geometry.
  Removing a constraint forgets the flag; trim keeps it on the constraints it rebuilds.
- A dimension can carry a label offset (`Sketch::set_label_offset`, a map beside the constraints,
  part of `same_content` so moving a label is an undoable change): where its label was placed, in
  millimetres along and across a frame the app derives from the measured geometry
  (`annotation_layout::label_frame`), so the label follows the geometry. Only dimensions take one
  (`NotADimension`), finite only; removing the constraint forgets it.
- Projected geometry (`Sketch::set_projected`, a set beside the entities, part of
  `same_geometry`) is a curve and its points, or a lone point, whose position the document
  supplies. The solver holds it fixed like the origin: its points are `PointHandle::Fixed` and a
  projected circle's radius `RadiusHandle::Fixed`, so it adds no freedom and reads as fully
  constrained, and a `SolveMemo` is recalled only under the fixed values it was made with. A
  constraint on projected or reference geometry alone is `OnlyReference`. Trim, extend and the
  sketch fillet refuse a projected curve (`Projected`); it still cuts, offsets, mirrors and takes
  constraints. It is ordinary profile geometry unless made construction.
- `Sketch::open_ends` lists the end points of profile curves (not construction) joined to nothing:
  an end whose class of points (shared or joined by `Coincident`) holds no other curve end and
  lies on no other profile curve (`Coincident` or `Midpoint` with it). Recompute keeps them in
  `SketchResult::open_ends`, worked out on the worker with each solve.
- `Sketch::points_beyond_curves` lists the points an active `Coincident` holds on a line or arc
  (not construction, not an axis) that lie past its drawn ends by more than `BEYOND_TOLERANCE`: the
  solver holds a point on a line or arc to the whole line or circle (`Form::OnLine` and the circle
  form), deliberately, since snapping to a line's extension relies on it. Recompute keeps them in
  `SketchResult::beyond`, so the app says so rather than bounding the solve.
- `Sketch::free_points` lists the points no curve uses (a constraint using one does not count), in one
  pass; the hole feature drills at them.
- An `Ellipse` is its centre point, the point at the end of its major axis and a minor radius
  (finite, above zero, `InvalidMinorRadius`); an `EllipticalArc` adds a start and an end point,
  running counter-clockwise like an arc (`Role::Elliptic`, `Sketch::ellipse` gives the
  `EllipseGeometry` of either, its parameter the angle of `centre + major cos t + minor sin t`
  with the minor axis a quarter turn counter-clockwise from the major). Nothing keeps the minor
  radius below the major one; exports take the longer as the major axis.
- `insert_entity` and `insert_constraint` take explicit IDs and check references, for loading.
- Uses of each entity are counted incrementally, so refusing to remove a used one never scans the
  sketch and undoing a large import stays fast. The sketch never cascades a removal; the document's
  `remove_sketch_items` expands a deletion into its users first (`document.md`).

## Constraints

- Stable `ConstraintId`s; dimensions hold `Expression`s. A distance between two lines holds both
  ends of the second at the distance from the first, so it implies parallel. A distance between
  two circles or arcs is the gap between their full circles, apart or one within the other as
  drawn (`Form::CircleGap`, the tangency equation offset by the value, so zero is tangent); from a
  line to a circle or arc it is the gap on the side the centre lies (`Form::LineGap`). Measured,
  a gap of crossing curves is zero. An angle measures from its first line's direction (or its
  reverse when `reversed`, which the UI sets so a chain's corner is measured inside it) to its
  second's.
- `AxisDiameter { point, axis }` is a point's distance from a line held as the diameter across it,
  as a lathe drawing dimensions a revolved profile: its value and `Sketch::measured` are twice the
  distance, the solver holds the point at half the value on its drawn side (`Form::LineDistance`),
  and it restates a `Distance` between the same point and line. The point must not be the line's
  own; the reference axes are lines like any other.
- `Perpendicular` between a line and a circle or arc puts the centre on the line
  (`Form::OnLine`), so the line crosses the curve square; between lines it is the right angle.
- `ArcLength` and `Sweep` take an arc, never a circle: the sweep is the angle from the radius to
  its start to the radius to its end (`Form::Angle`, so any sweep below a full turn holds without
  flipping), the length the start radius times that counter-clockwise sweep (`Form::ArcLength`).
  A sweep must lie strictly between 0° and 360° and a length above zero (`DimensionError`). The
  app offers them from the Distance and Angle tools with one arc selected.
- `check_constraint` refuses constraints that do not fit the entity kinds; the UI asks before
  offering one. `restating` and `contradicting` (`relation.rs`) find the constraint already in the
  sketch that a new one repeats (same kind on the same items; a level line is the same as
  `HorizontalPoints` on its ends; a radius and a diameter of one circle) or cannot hold with. The
  sketch itself still accepts both, since stored files may hold them. `Sketch::relations` indexes
  the active constraints by subject (and the level/upright, parallel/perpendicular pairs) in
  ordered maps, so checking many candidates costs a lookup each rather than a scan.
- A dimension is a magnitude: distances are never negative, the side coming from the drawn
  geometry. `add_constraint` refuses a literal value that could never hold
  (`SketchError::DimensionValue`, so it fails when typed, not at solve); loading and undo go through
  `insert_constraint` and stay lenient, and a parameter-driven value that turns negative fails that
  dimension at solve with its own message.
- `Sketch::measured` gives a dimension's drawn value: new dimensions start from it and unreadable
  stored ones fall back to it.
- `Tangent` between two splines sharing an end (end points joined directly or by `Coincident`)
  holds the two first legs at the joint parallel; between splines sharing no end it makes them
  touch somewhere along both, at a parameter on each (`sketch-solver.md`).
- `Curvature(a, b)` holds a spline's curvature at one of its ends equal to that of the line, arc,
  circle or spline its end lies on (another spline: end to end), so a smooth joint shows no kink in
  its curvature. It holds the curvature only; the app adds `Tangent` with it. A spline of two
  control points is straight and refused (`WrongKind`), and so is a pair not sharing an end.
- `Distance` from a point to a spline is measured square to the spline, from its closest point
  (`Sketch::closest_on_curve`, refined by Newton on the spline's parameter). From a spline to a
  line, circle or arc it is the gap where the spline bulges toward it, on the side it was drawn:
  the spline touches the line or circle offset by the value (`Sketch::spline_gap` gives the two
  points, the stationary point of the distance as the solver starts from it), so a spline
  crossing a line keeps a bulge at the distance rather than measuring zero.
- `Equal` holds lines and splines to one length, any mix with at least one spline (a spline's
  length is a five-point Gauss–Legendre sum per knot span, `BSpline::length`, the same rule the
  solver differentiates), and circles and arcs to one radius.
- `Angle` also takes a line and an arc sharing an end (joined directly, by `Coincident`, or the
  arc's end lying on the line; refused otherwise as `NotJoined`). The arc's direction there is its
  tangent leaving the joint along the arc (`Sketch::angle_direction`, `angle_vertex`), so trim,
  extend, fillet and split drop such an angle on an arc they reshape (`keeps_sweep`).
- Ellipses take `Coincident` with a point (on the whole ellipse, never one of its own points),
  `Concentric` with circles, arcs, ellipses and points, `Horizontal`/`Vertical` (its major axis,
  the same as `HorizontalPoints` on the centre and axis point, so `restating` finds either),
  `Tangent` with a line only, and `MajorRadius`/`MinorRadius` (a dimension above zero; the major
  one restates a `Distance` between the centre and the axis point). Every other constraint,
  `Radius` and `Equal` included, refuses them.
- `Midpoint { point, curve }` takes a line or an arc, never a circle. On an arc it is two
  single-branch equations (`Form::OnBisector`, the point on the chord's perpendicular bisector, and
  `Form::ArcBulge`, its signed distance from the centre across the chord equal to the radius on the
  side a counter-clockwise arc bulges), so a solve never lands on the opposite side of the circle.

## Editing operations

Trim, extend, offset, mirror, patterns, fillet, chamfer, split and break work on a copy and replace the sketch only if every step
succeeded. A changed curve is removed and inserted again under the same ID (`restructure`), with
every constraint still true of it. Joints are judged by a `TOLERANCE` relative to the extent.

- Trim and extend (`trim.rs`): cutters are every other curve, construction curves and splines
  included, and the two reference axes as the infinite lines they are (`Cutter::Axis`, the origin
  and lone points are not cutters); targets are every curve but those, the axes never trimmed or
  extended (`Reference`). A crossing at a curve's own end is a joint, not a cut. Splines cannot
  themselves be trimmed or extended.
  - A line cut by a collinear line, or a circle or arc by an arc on the same circle, is cut at the
    ends of the overlap: the cutter's ends lying inside the target (`overlap_ends`, judged by the
    same tolerance) are the cuts, and the overlap's own crossings are not looked for. A whole
    circle on a circle, or a cutter whose ends lie outside the target, cuts nothing.
    Extend does not look for overlaps.
  - `Midpoint` and `Equal` on a shortened line, and `Midpoint`, `ArcLength` and `Sweep` on a
    shortened or extended arc (`keeps_sweep`; the sketch fillet does the same), are dropped, as is any distance dimension between
    its two old ends or points joined to them by `Coincident` (one end often outlives the trim,
    shared with another curve, and would keep measuring to the far end).
  - A split-off piece gets fresh IDs and the construction flag. A line piece is `Collinear` with
    the kept part (or keeps horizontal or vertical and a new end on the kept line); an arc piece
    gets its own centre, `Concentric` and `Equal`. Tangent, parallel, perpendicular and angle
    constraints to a curve joined only at the far end move to the piece holding that end.
  - A new end joins its cutter: `Coincident` with the cutter's end point when the cut lies there,
    else a point on the cutter (an axis included, which the solver treats as the infinite line).
    End points no curve uses any more are removed.
  - `extend` moves an end to the nearest crossing beyond it and joins it the same way; an end
    shared with another curve, coincident with another point or fixed is refused.
- Offset (`offset.rs`): `offset_chain` orders the chosen lines and arcs into one open or closed
  chain (splines, branches and separate chains refused), walked from a free end so `Side` means the
  same thing on the working copy. `Chain::outline` joins offset curves: smooth joints meet, line
  corners meet sharp, a convex corner with an arc gets a round arc about the original corner, a
  concave one is trimmed where the carriers cross.
  - `offset` holds the distance with the typed expression on the first curve and after every sharp
    corner; after a smooth joint or round corner it follows from the `Tangent` there, so a line only
    gets `Parallel` and an arc nothing. The closing `Coincident` of a closed chain is added last, so
    the one relation a closed loop repeats is never a whole redundant constraint.
- Mirror (`mirror.rs`): points on the mirror line are shared, others copied with `Symmetric` to the
  original; arcs swap ends to stay counter-clockwise, circles add `Equal`, a curve that is its own
  image is left out. No other constraint is copied, since symmetry holds the copy.
- Patterns (`pattern.rs`): `rectangular_pattern` repeats the chosen curves and lone points along one
  or two directions (a `PatternRow` each: count including the original, spacing, angle from the x
  axis, the second defaulting to square to the first), `circular_pattern` about an existing point,
  the origin included (`CircularPattern`: a count including the original, spread over a full turn
  or across a total angle). Copies get fresh IDs and the construction flag, circles an `Equal` to
  the original. Instances are capped at `MAX_PATTERN_INSTANCES` and must stay within `MAX_LENGTH`.
  - Copies stay parametric with existing constraints, each tied to the instance before it (the
    first to the original), so editing the original or any spacing moves everything after it. A
    rectangular copy's every point is tied to its predecessor by `HorizontalDistance` and
    `VerticalDistance` holding the typed spacing expression (a row along an axis, an angle that is
    a literal, uses the spacing itself and `HorizontalPoints` or `VerticalPoints` for the other
    component; any other angle, a parameter included, uses `abs(spacing * cos(angle))` and
    `abs(spacing * sin(angle))`), the side coming from the drawn geometry like any distance. A
    circular copy's every point is tied by two construction lines from the centre (`Equal` and an
    `Angle` holding the step: the total over count minus one, or 360° over the count), created
    once per point and instance; the origin as centre gets a point `Coincident` with it to start
    the lines from, since a line cannot end on a reference.
  - No constraint among the copied items is copied: a copy tied point by point is already as
    constrained as its original (its dimensions, parallels, tangents and the like hold because
    the shape does), so repeating them would make a whole copy redundant, as with mirror. A `Fix`
    on an original does not reach the copies, so fixing the original moves the copies with it.
  - Points of the selection lying on the centre are shared by the copies and held there by a
    `Coincident` when not already; a circle centred on it is its own image and left out. Refused in
    words (`PatternError`): nothing selected, a count below 2, more than the instance cap, a zero
    spacing, two parallel directions, a centre that is not a point, nothing off the centre, an
    angle of 0° or a full turn or more, geometry reaching past `MAX_LENGTH`. `rectangular_image`
    and `circular_image` give the faceted copies for a preview without touching the sketch.
- Ellipses cut other curves (`intersect::Shape::Ellipse`, crossings by sampled roots along the
  ellipse as for splines) and are cutters for trim, extend and break, but are never trimmed,
  extended, split, broken, offset, filleted, mirrored or patterned: each refuses them in words
  (`NotTrimmable`, `NotExtendable` or `Closed`, `NotLineOrArc`, `NotOffsettable`,
  `MirrorError::Ellipse`, `PatternError::Ellipse`), since a copy would need its minor radius held
  equal and no constraint does that. `Sketch::curve_crossings` gives where a spline or an ellipse
  crosses another curve.
- Fillet (`fillet.rs`): a `Corner` is where exactly two lines or arcs end, kept by one of its
  points. `rounding` refuses a radius whose touching point would not lie on a curve short of its far
  end (`TooLarge`). `fillet` adds the arc `Tangent` to both with a `Radius` dimension and keeps the
  corner point as a sharp held on both carriers by `Coincident`, so dimensions, fixes and symmetry
  on the corner still hold.
- Chamfer (`fillet.rs`, same corners): a `ChamferSize` is one distance for both curves (`Equal`),
  a distance on each (`Distances`) or a distance on the first and the angle of the cut
  (`DistanceAndAngle`), each a `Dimensioned` (typed expression and its value). `bevel` finds the
  points at the distances from the corner on each curve (along a line, by the chord on an arc),
  refusing one past a curve's far end (`TooFar`) or a distance not above zero. With an angle the
  first point is at the distance and the second where a ray from it, turned by the angle from the
  direction back to the corner toward the second curve, meets that curve (`AngleMisses` when it
  never does, `AngleOutOfRange` outside 0° to 180°). `chamfer` shortens both curves to the points
  (`shorten_to`, shared with the fillet), joins them with a line and keeps the sharp the same
  way. It holds a `Distance` from the sharp to each new end with the typed expression of that
  side, so either side can be changed alone afterwards; with an angle, the first end's `Distance`
  and an `Angle` between the cut and the first curve measured inside the cut-off triangle (the
  `from`, `to` and `reversed` that make the drawn value positive are found by measuring,
  `AngleNotHeld` if none does), so the angle is a dimension like the distance.
- Split (`split.rs`): `split_at` cuts a line or arc at an existing point lying on it between its
  ends (`check_split`), which becomes the end both pieces share, its `Coincident` on the curve
  dropped. A line's pieces are `Collinear`, or each keeps its horizontal or vertical; a point that
  was the line's `Midpoint` makes the pieces `Equal`. An arc's pieces share the centre, so they
  keep one radius with no constraint added. Tangent, parallel, perpendicular and angle
  constraints joined at the far end move to the far piece (`far_constraints`, as trim does);
  circles, splines and projected curves are refused.
- Break (`breaking.rs`): `break_curve` splits a line or arc at every crossing with the other curves
  and the two axes, found as trim finds its cuts (`open_cuts`: a collinear overlap cuts at its
  ends, a crossing at a curve's own end is a joint). Each cut goes through `split_at` from the
  start of the curve toward its end, the next cut lying on the piece just made, so the pieces'
  `Collinear` constraints chain without repeating one another. The cut point is the end point of
  the cutter lying there when one does (so two curves broken one after the other, or a line
  ending on another, share one point), else a new point held on the cutter by `Coincident`
  (an axis included); a crossing shared by several cutters is joined to one of them. The
  pieces keep the curve's constraints as a split does and are fully determined by the crossings,
  so the sketch's degrees of freedom do not change. `break_curves` breaks each of several curves
  in turn on a working copy, skipping those that cannot break (circles, splines, reference or
  projected curves, curves crossing nothing); `BreakError` names the curve for a single refusal
  and says `NothingToBreak` or `NothingSelected` otherwise. Circles are refused like split, since
  breaking one would replace it with arcs.

## Relations the geometry shows (`inference.rs`)

- `Sketch::shown_relations` finds the relations a drawing already shows within a `Tolerance` (a
  distance and an angle; `Tolerance::of` is `RELATIVE_DISTANCE` of `Sketch::extent` and
  `ANGLE_DEGREES`), for the `RelationKind`s asked, in this order: coincident ends (curve ends, lone
  points and the origin, never two ends of one curve), concentric circles and arcs, points mirrored
  about an axis (the reference axes and construction lines; arc centres are left out, their ends
  and an equal radius mirror the arc), horizontal and vertical lines, tangents (a line or arc
  leaving a joint along another, a line or circle grazing a circle within both drawn curves),
  perpendicular directions (one per pair of slanted direction groups, a joined pair preferred),
  parallels and equal lengths and radii (each group chained to its first). Points are matched
  through a grid of tolerance-sized cells, groups of directions and sizes are anchored at their
  smallest, so a large import costs no pairwise scan of points. A candidate the sketch refuses
  (`check_constraint`), restates or contradicts (`Relations`, which also records the candidates
  found before it) is left out.
- `Sketch::inferred_relations` keeps those that hold (`keep_holding`): the sketch is solved first
  (`InferenceError::Unsolved` when it does not), then each kind is added as a stage on the solved
  geometry of the one before, so a relation the earlier ones imply holds exactly and the rank
  analysis names it redundant, and is dropped. A stage that conflicts drops the newest candidate
  of each conflict and solves again; one whose solve moves any point or radius more than
  `MOVE_LIMIT` tolerances (a relation that holds only nearly can pull a dependent set apart) is
  tried one candidate at a time instead, each kept only when it solves, adds rank and stays within
  the limit. What is kept comes back with the degrees of freedom left.

- `Sketch::datum_dimensions` (`datum.rs`) dimensions what is still free from a datum point (the
  origin or any point; `InferenceError::NotAPoint` otherwise) at the measured values, through the
  same `keep_holding` in one stage, so only dimensions adding rank are kept: the datum's horizontal
  and vertical distances from the origin (unless it is the origin or fixed), each circle's
  diameter, arc's radius and ellipse's minor radius, then every other free point's horizontal and
  vertical distance from the datum, nearest points first so a rectangle cornered on it gets its
  width and height. An offset within the tolerance is held by `VerticalPoints` or
  `HorizontalPoints` instead of a dimension of nothing. Fixed and reference geometry is left out.

## Checking a sketch (`check.rs`)

- `Sketch::flaws` names what the eye misses, within a `Tolerance`: curves of no length
  (`Flaw::NoLength`: a line, arc length or radius, or every spline control point within the
  distance), ends of different curves at most the distance apart and not joined
  (`NearlyJoined`, with the gap; the two ends of a curve of no length count as joined, so its
  fix is not named twice), and a line, circle or arc lying on another (`OVERLAP_SAMPLES` points
  along it all within the distance of the other: `LiesOn`, the newer on the older when each lies
  on the other) or sharing a stretch with it (two or more samples: `Overlaps`). Projected and
  reference geometry is left out; curves are paired through a sweep over their boxes.
- `Sketch::fix` gives each flaw's repair as entities to remove and constraints to add (`Fix`):
  a `Coincident` for ends nearly joined; a curve of no length removed, its points no other curve
  or constraint uses with it, and its two kept ends joined; a curve lying on another removed the
  same way, each kept end not already joined to an end of the other held on it by `Coincident`.
  A partial overlap has no fix (`Fix::is_empty`), since which piece to keep is the user's choice.

## Tangent circles (`tangent_circle.rs`)

- `tangent_circle` draws a circle tangent to three of the sketch's lines, circles and arcs (an
  arc counts as its full circle, an axis as the infinite line it is), or to two of them at a
  given radius, and holds it with a `Tangent` to each and, for two, a `Radius` holding the typed
  expression (`Dimensioned`), so it follows when the curves move. Any other count, a radius with
  three curves, none with two, a curve chosen twice, a point or spline, a radius not above zero,
  no circle touching them and one reaching past `MAX_LENGTH` are refused (`TangentError`).
- `tangent_circle_near` finds every circle first (`TangentCircle`), then takes the one whose
  centre is nearest the given point, the smallest when there is none. For three curves each of
  the eight choices of side (outside or inside a circle, either side of a line) is a system
  whose circle equations differ by linear terms: two linear equations leave a line in
  (centre, radius) space that the first circle's quadratic cuts in at most two points. For two
  curves each choice of side offsets both curves by the radius and the circle centres are where
  the offset curves cross (`fillet::centers`). Candidates are kept only when they touch every
  curve within a tolerance relative to their reach.
- Circles tangent to curves whose offsets coincide (two parallel lines at twice the radius, three
  parallel lines) have no discrete solution and are refused as no circle.

## Blend curves (`blend.rs`)

- `blend` joins two curve ends (`BlendEnd`: a line, arc or spline and one of its end points,
  `Sketch::ends_of_curve`) with a new spline held by `Coincident` from each of its ends to the
  picked point, `Tangent` to each curve and, for `Continuity::Curvature`, `Curvature` to each, so
  it follows when either curve moves. It is ordinary geometry; projected curves can be blended.
- `blend_curve` gives the control points without touching the sketch, for the preview, and `blend`
  starts the spline from exactly them, so the constraints already hold and the first solve moves
  nothing. A tangent blend is a cubic of four points, its inner ones a third of the gap along
  each curve's direction leaving its end. A curvature blend has six: the second point a fifth of
  the gap along that direction (shorter where the curve bends tightly, so the third stays within a
  leg of the tangent), the third offset square to it so the spline bends as the curve does there
  (an arc's `1/r` with the side it turns to, a line's zero, a spline's own end curvature), the
  offset scaled from the spline's curvature per unit offset at its end (`unit_bend`).
- Refused in words (`BlendError`): a curve with no ends (a circle, a point), a point that is not an
  end of its curve, both ends on one curve, ends already at one place, an end with no direction,
  control points past `MAX_LENGTH`, and curves already joined so that the solver would read the
  spline's other end as the joint (checked after the `Coincident`s through `joined_ends`, the same
  joint search `Tangent` and `Curvature` use). A curvature blend to a two-point spline is refused
  by `Curvature` itself (`Edit`).

## Faceting and splines (`curve.rs`, `fit.rs`)

- Curves are drawn as polylines within a chord tolerance (`Faceting`, bounded counts per turn and
  `MAX_SEGMENTS`); `Faceting::within` accepts any value (NaN or below zero as the finest). Trim
  pieces and extensions are faceted the same way.
- Sketch splines are clamped, uniform knots, degree `min(MAX_SPLINE_DEGREE, points - 1)`, exact
  derivatives, banded elimination (`banded.rs`). `BSpline::fit` approximates a dense polyline
  within a tolerance, `interpolate` passes through points at evenly spaced parameters, `through`
  follows unevenly spaced points without loops.
- A spline meets a line or circle where its signed distance changes sign between samples
  (`intersect::spline_roots`, bisected); where the distance dips toward zero between samples
  without changing sign, the deepest point is found by golden-section search, giving two crossings
  when it passes zero and one touching point when it comes within the tolerance. Two splines cross
  where their sampled polylines do, each candidate refined by Newton on both parameters
  (`intersect::spline_spline`) and kept when both curves meet within the tolerance.
