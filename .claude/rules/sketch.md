---
paths:
  - "crates/caditor-sketch/**"
---

# Sketches

## Entities

- A sketch lies on a `Plane`; curves refer to points by `EntityId`. The origin and two axes have
  reserved IDs (`EntityId::REFERENCES`) the counter never reaches; stored IDs stay below
  `FIRST_UNSTORABLE_ID`. Reference geometry never moves and cannot be removed or changed.
- Construction (`Sketch::set_construction`): any curve, never a point; a set beside the entities,
  part of `same_geometry`. It solves, snaps and takes constraints like any curve, but
  `solid::profile_curves` leaves it out, so it neither splits regions nor changes their keys. It can
  be a revolve axis.
- Inactive constraints (`Sketch::set_active`): a set of `ConstraintId`s beside the constraints, part
  of `same_content`. The solver, `Sketch::evaluate`, the joint classes, `restating` and
  `contradicting` read `active_constraints()` only, so an inactive one holds nothing, never
  conflicts or counts as redundant, and a new one is not refused for restating it. Mirror, the
  patterns, extend and the sketch fillet ask what holds through `active_constraints_using` (an
  inactive `Symmetric`, `Coincident`, `Midpoint` or `Fix` counts for nothing there; the fillet's
  `merge_point` drops a moved constraint only as a duplicate of an active one). An inactive
  dimension is a reference: its displayed value is `Sketch::measured`.
  Removing a constraint forgets the flag and label offset; whatever takes a kept constraint out and
  puts it back carries both (`detach_constraint`, `attach_constraint`, `add_detached_constraint`,
  `DetachedConstraint`): trim, split and break via `restructure` and `move_constraints`, and the
  fillet and chamfer when merging the corner's second point into the kept one.
- Label offset (`Sketch::set_label_offset`): a map beside the constraints, part of `same_content`
  so moving a label is undoable. Millimetres along and across a frame the app derives from the
  measured geometry (`annotation_layout::label_frame`), so the label follows the geometry. Only
  dimensions take one (`NotADimension`), finite only; removing the constraint forgets it.
- `annotation.rs` (public `caditor_sketch::annotation`) is the unit-free dimension layout shared by
  the canvas (`app-sketching.md`, screen points) and drawing export (`file-import-export.md`,
  drawing units): `measured` (`Measured` over `LineSpan`s), `lanes` (linear dimensions along one
  line on one side whose spans overlap, shortest nearest, sides from `away_from` the sketch's
  middle) and `Obstacles` (label `Footprint`s in a grid of caller-chosen cell size, each overlap
  counted once, plus their `bounds`). Thinning thresholds stay with the canvas.
- Projected geometry (`Sketch::set_projected`, a set beside the entities, part of `same_geometry`)
  is a curve and its points, or a lone point, whose position the document supplies. The solver
  holds it fixed like the origin (`PointHandle::Fixed`, a circle's `RadiusHandle::Fixed`): no
  freedom, reads as fully constrained, and a `SolveMemo` is recalled only under the fixed values it
  was made with. A projected edge cut into pieces upstream stays this one curve while the pieces
  join end to end (`document.md`); it never becomes several curves, so a constraint on it is kept
  or the sketch fails in words, never losing one silently. A constraint on projected or reference
  geometry alone is `OnlyReference` when added (`add_constraint`, `check_constraint`); one inserted
  under its ID (`insert_constraint`: undo, journal replay, loading) is refused only on reference
  geometry alone, since projecting a curve after constraining its points leaves a kept constraint on
  projected geometry and undoing its removal must restore it. Trim, extend and the sketch fillet refuse it (`Projected`); it
  still cuts, offsets, mirrors and takes constraints. It is profile geometry unless made construction.
- `Sketch::open_ends`: end points of profile curves joined to nothing (their class of points,
  shared or `Coincident`, holds no other curve end and lies on no other profile curve by
  `Coincident` or `Midpoint`). Recompute keeps them in `SketchResult::open_ends`, computed on the
  worker with each solve.
- `Sketch::points_beyond_curves`: points an active `Coincident` holds on a line or arc (not
  construction, not an axis) past its drawn ends by more than `BEYOND_TOLERANCE`. The solver
  deliberately holds a point to the whole line or circle (`Form::OnLine`, the circle form), since
  snapping to an extension relies on it. Recompute keeps them in `SketchResult::beyond` so the app
  says so rather than bounding the solve.
- `Sketch::free_points`: points no curve uses (a constraint using one does not count), in one pass.
- `Sketch::layout_points`: the free points plus every point whose only users are layout rays ending
  at it. A ray is a construction line, not projected, tied by an active `Equal` or `Angle` to another
  construction line from the same start (`layout_rays`), which is how a circular pattern holds its
  copies; the hub where rays start is used as a start, so it is never a layout point. Holes drill and
  point patterns copy at these, so a circular pattern of points is a bolt circle. A generic
  "points only construction curves use" rule was rejected: a hole's From an edge and Concentric
  placements project construction lines and circles whose loose points must not drill.
- `Ellipse`: centre point, major-axis end point and a minor radius (finite, above zero,
  `InvalidMinorRadius`); `EllipticalArc` adds start and end points, counter-clockwise
  (`Role::Elliptic`). `Sketch::ellipse` gives the `EllipseGeometry`, parameter the angle of
  `centre + major cos t + minor sin t`, minor axis a quarter turn counter-clockwise from the major.
  Nothing keeps minor below major; exports take the longer as major.
- `Spline` is its points and a `SplineKind`: `Control`, `Fit` (passes through them, with a
  `FitSpacing`) or `Conic` (start, apex, end and a rho); control and fit are open or `closed`. New
  fit splines are `FitSpacing::Centripetal` (`SplineKind::fit`); `Even` (even parameter steps) is
  kept so stored sketches read back unchanged, and `Sketch::respace_fit_spline` converts one
  keeping its points. `Sketch::spline` gives any kind as a clamped `BSpline`
  (`sketch::spline_through`), so drawing, intersections, profiles and exports ignore the kind.
  - A closed spline is periodic (`Entity::spline_ends` is none: no open ends, joins nothing at an
    end) and needs three points (`TooFewClosedPoints`).
  - A piece (`SplineKind::Piece { lead, trail }`, named "Spline") is an open cubic control spline
    whose inner spans are uniform and whose first and last spans are `lead` and `trail` times as
    long (`curve::piece_knots`): what trim and extend leave of a control spline, so the curve keeps
    its shape exactly. At least `MIN_PIECE_POINTS` (`TooFewPiecePoints`), spans finite and above
    zero (`InvalidEndSpans`); `same_form` compares the spans, which nothing solves.
  - Knotted (`SplineKind::Knotted { spans }`, named "Spline", saved as `spline_knots`) is an open
    cubic control spline whose spans between knots are the given widths (relative, summing to one
    when made by the sketch): what knot insertion leaves, since no other kind holds a knot where
    the old ones are not uniform. The spans are at least two, finite and above zero
    (`InvalidKnotSpans`) and three fewer than the points (`KnotSpanCount`); `curve::span_knots`
    builds the knots, `same_form` compares the spans, nothing solves them. `SplineKind` is not
    `Copy` because of it, so `Entity::spline_kind` returns a clone. Trim and extend read the result
    as a `Piece` when its inner spans are all equal (always with up to six points), else as a
    `Knotted` of the spans of the part's knots (`Part::kind`).
  - A conic has exactly three points (`ConicPoints`) and rho within `MIN_RHO..=MAX_RHO`
    (`InvalidRho`): a rational quadratic with middle weight `rho / (1 - rho)`, passing the point
    rho of the way from the chord's middle to the apex (below 0.5 ellipse, 0.5 parabola, above
    hyperbola). `same_structure` compares the form, not rho, which changes like a radius.
  - Control, conic and even fit splines are affine-invariant, centripetal fit splines
    similarity-invariant: mirror, patterns, copies and uniform scaling keep the kind; only a
    non-uniform map (oblique projection, skewed DXF block) bends a centripetal spline between its
    mapped points.
- Text (`text.rs`, `Sketch::texts`): a map of `Text` beside the entities, keyed by `EntityId`s from
  the same counter (`check_new_id` refuses one an entity, text or constraint holds), part of
  `same_geometry` and `heap_size`. A `Text` is its anchor (a point entity: `NotAPoint`,
  `MissingEntity`), its content (lines separated by `LINE_BREAK`, not blank, at most
  `MAX_TEXT_CHARS` counting the breaks, no other control characters: `InvalidText`), a height and
  an angle as `Expression`s (length, and angle in degrees from the sketch's x axis), bold and
  italic, and a `TextAlign` (`Left`, `Centre` or `Right`, left by default). The anchor is the
  start, middle or end of the first line's baseline by that alignment, and each following line is
  set below it, aligned about the same vertical through the anchor (`lettering.md`). The anchor
  counts as a use, so it cannot be removed while the text stands (`InUse`), and it is neither a free point nor a layout point (holes never
  drill at it). `add_text`, `insert_text` (explicit id), `remove_text` and `replace_text` (value
  only; a changed anchor is `ChangesStructure`). The text holds no constraint and adds no freedom:
  the anchor is an ordinary point, constrained and dimensioned like any other.
  - `Sketch::evaluate` works out each text's `TextSize` (`Text::size`: height above zero and at most
    `MAX_LENGTH`, angle finite; else `SketchError::Text` naming it), the solve writes them into the
    solved geometry (`set_text_sizes`, like a conic's rho), and `Sketch::text_size` gives them, or
    the literal value for an unsolved definition. Parameters a text uses count in `parameters` and
    `uses_parameter`.
  - The letters are not entities: the document lays them out (`document.md`) and names each
    letter curve `letter_curve(text, letter, segment)`, an id at or above `FIRST_UNSTORABLE_ID`
    holding 40 bits of the text's id, 11 of the letter's position in the string and 12 of the
    segment, so letters before an edit keep their curve names. `Sketch::text_of_letter_curve` and
    `entity_label` ("Text 5") read one back for error messages.
  - Copy and paste carry a selected text with a copy of its anchor (`SketchClip::text_count`; the
    clipboard writes it as a `text` entity record and carries its parameters like a dimension's).
    Trim, mirror and the patterns leave texts alone; one whose anchor a trim would remove makes the
    trim refuse it as in use.
- `insert_entity` and `insert_constraint` take explicit IDs and check references, for loading.
- Uses of each entity are counted incrementally, so refusing to remove a used one never scans the
  sketch. The sketch never cascades a removal; the document's `remove_sketch_items` expands a
  deletion into its users first (`document.md`).

## Constraints

- Stable `ConstraintId`s; dimensions hold `Expression`s. A dimension is a magnitude: never negative,
  the side coming from the drawn geometry. `add_constraint` refuses a literal that could never hold
  (`SketchError::DimensionValue`); loading and undo go through `insert_constraint` and stay lenient;
  a parameter-driven value turning negative fails that dimension at solve with its own message.
  `Sketch::measured` gives the drawn value: new dimensions start from it and unreadable stored ones
  fall back to it.
- `check_constraint` refuses constraints that do not fit the entity kinds; the UI asks before
  offering one. `restating` and `contradicting` (`relation.rs`) find the existing constraint a new
  one repeats (same kind on the same items; a level line equals `HorizontalPoints` on its ends; a
  radius and a diameter of one circle) or cannot hold with. The sketch still accepts both, since
  stored files may hold them. `Sketch::relations` indexes active constraints by subject (and the
  level/upright, parallel/perpendicular pairs) in ordered maps, so a candidate check is a lookup.
- `Distance`: between two lines, both ends of the second at the distance from the first (implies
  parallel). Between circles or arcs, the gap between their full circles, apart or one within the
  other as drawn (`Form::CircleGap`, tangency offset by the value, zero is tangent); line to
  circle or arc, the gap on the side the centre lies (`Form::LineGap`). A gap of crossing curves
  measures zero. Point to spline: square to the spline from its closest point
  (`Sketch::closest_on_curve`, Newton on the parameter). Spline to line, circle or arc: the gap
  where the spline bulges toward it on its drawn side, the spline touching the curve offset by
  the value (`Sketch::spline_gap` gives the two points, the distance's stationary point as the
  solver's start), so a crossing spline keeps a bulge rather than measuring zero.
- `Angle` measures from the first line's direction (reversed when `reversed`, which the UI sets so
  a chain's corner is measured inside it) to the second's. It also takes a line and an arc or
  elliptical arc sharing an end (directly, by `Coincident`, or the arc's end on the line; else
  `NotJoined`), the arc's direction being its tangent leaving the joint (`Sketch::angle_direction`,
  `angle_vertex`); trim, extend, fillet and split drop such an angle on an arc they reshape
  (`keeps_sweep`).
- `HorizontalDistance` and `VerticalDistance` take two points, a point or circle and a line, or two
  circles or arcs (never two lines: `NotApplicable`). A circle or arc counts as its centre, a line
  as where it crosses the axis-parallel through the other item (`Sketch::axis_offset_ends` gives
  the two measured points for `measured`, canvas and export). The solver holds the other item,
  moved by the value along the axis on its drawn side, on the line (`Form::LineOffset`, a signed
  distance, finite when the line turns level). A line along the measuring axis is `RunsAlong`.
- `AxisDiameter { point, axis }`: a point's distance from a line held as the diameter across it, as
  a lathe drawing dimensions a revolved profile. Value and `Sketch::measured` are twice the
  distance; the solver holds half on the drawn side (`Form::LineDistance`); it restates a
  `Distance` between the same point and line. The point must not be the line's own; the reference
  axes are lines like any other.
- `Perpendicular` between a line and a circle or arc puts the centre on the line (`Form::OnLine`);
  between lines it is the right angle.
- `ArcLength` and `Sweep` take an arc, never a circle: sweep is the angle from the radius to the
  start to the radius to the end (`Form::Angle`, so any sweep below a full turn holds without
  flipping), length the start radius times that counter-clockwise sweep (`Form::ArcLength`). Sweep
  strictly between 0° and 360°, length above zero (`DimensionError`). The app offers them from the
  Distance and Angle tools with one arc selected.
- `Tangent` between splines sharing an end (joined directly or by `Coincident`) holds the first
  legs at the joint parallel; sharing none, they touch somewhere along both at a parameter on each
  (`sketch-solver.md`).
- `Curvature(a, b)`: a spline's curvature at one end equals that of the line, arc, circle or spline
  (end to end) its end lies on. Curvature only; the app adds `Tangent`. A two-control-point spline
  is straight (`WrongKind`), as is a pair sharing no end.
- `Equal`: lines and splines to one length (any mix with at least one spline; spline length is a
  five-point Gauss–Legendre sum per knot span, `BSpline::length`, the rule the solver
  differentiates), circles and arcs to one radius, ellipses and elliptical arcs to both radii (only
  the minor when they share centre and axis point).
- Ellipses take: `Coincident` with a point (on the whole ellipse, never one of its own points);
  `Concentric` with circles, arcs, ellipses, points; `Horizontal`/`Vertical` (major axis, same as
  `HorizontalPoints` on centre and axis point, so `restating` finds either); `Tangent` with a line,
  circle, arc, spline or ellipse (at a shared point if they have one, a spline at one of its ends,
  anywhere along both otherwise, `sketch-solver.md`); `Equal` with an ellipse; `Angle` with a line
  (an elliptical arc sharing an end); `Midpoint` (elliptical arc only); `OnMinorAxis` with a point
  (on the line through the centre square to the major axis; never its own points); `Distance` from
  a point (square from the closest point, `Sketch::closest_on_ellipse`), a line (gap where the
  ellipse reaches toward it on the centre's side; zero when crossing), a circle or arc (gap at the
  stationary point of the distance from the circle's centre whose gap is smallest), a spline or
  ellipse (gap where they come closest, zero when crossing; `Sketch::ellipse_gap` gives the two
  points on the whole ellipse); `MajorRadius`/`MinorRadius` (above zero; the major restates a
  `Distance` between centre and axis point). Every other constraint, `Radius` included, refuses
  them.
- `Rho { conic, value }`: dimension of a conic's rho (`Dimension::NONE`, a plain number within
  `MIN_RHO..=MAX_RHO`, `DimensionError::RhoOutOfRange`). Adds no equation and no degree of freedom;
  the solver shapes the conic with the evaluated value (`System::rhos`) and writes it into the
  solved geometry, so a parameter drives the shape. `Sketch::measured` gives it. Projected conics
  refuse it (`OnlyReference`).
- Point targets (`target.rs`, `PointTarget`): a sketch point, the middle of a line, arc or
  elliptical arc, the crossing of two curves (lines, axes, circles, arcs, ellipses, splines), the
  centre of corners or sides (`Centroid`, the mean of its members, a point at itself and a line or
  arc at its middle), a quadrant of a circle or of an arc's whole circle
  (`Quadrant`) and an end of an ellipse's axes (`EllipseEnd`). `HorizontalPoints` and
  `VerticalPoints` hold a point level or upright with one (a sketch point is the two-point form
  every older use writes); `OnDirection { point, through, line, square }` holds it on the line
  through one along a line or square to it (the perpendicular bisector when `through` is that
  line's middle); `Centroid { point, corners }` holds it at that centre (`corners` lists points, lines and
  arcs; an odd outline with an arc lists its pieces). `check_target` refuses a wrong kind, a
  repeated entity or the held point itself (`SameEntity`) and fewer than two members
  (`TooFewCorners`); `Constraint::entities` lists each entity once. `target_places`
  gives where a target lies (both crossings of a line and a circle), `target_position` the one a
  constraint means: the place it misses least, the nearest the held point among ties.
  `describe_target` names it in words ("the crossing of Line 1 and Circle 2"), so `describe`
  reads "Horizontal Point 5 and the midpoint of Line 1", "Perpendicular bisector of Line 1 at
  Point 5", "Centre of Point 1, Point 2 and Point 3 at Point 7". `restating` compares targets with
  a crossing's curves and the corners in order, and a target that is a sketch point keeps the
  two-point subject. Trim, extend, split and the sketch fillet drop a constraint tracking a middle
  of a curve they reshape (`follows_a_middle` in `keeps_length` and `keeps_sweep`); a `Midpoint` is
  relaxed instead (`Editing operations`).
- `Midpoint { point, curve }` takes a line, arc or elliptical arc, never a circle or whole ellipse.
  On an arc it is two single-branch equations (`Form::OnBisector`, on the chord's perpendicular
  bisector; `Form::ArcBulge`, signed distance from the centre across the chord equals the radius on
  the side a counter-clockwise arc bulges), so a solve never lands on the opposite side. On an
  elliptical arc the middle is halfway by parameter (angle of `Sketch::ellipse`), affine-natural.

## Editing operations

Trim, extend, offset, mirror, patterns, fillet, chamfer, split and break work on a copy and replace
the sketch only if every step succeeded. A changed curve is removed and inserted again under the
same ID (`restructure`), with every constraint still true of it. Joints are judged by a `TOLERANCE`
relative to the extent.

- Trim and extend (`trim.rs`): cutters are every other curve (construction and splines included)
  and the two reference axes as infinite lines (`Cutter::Axis`; the origin and lone points are not
  cutters); targets are every curve but those, axes never trimmed or extended (`Reference`). A
  crossing at a curve's own end is a joint, not a cut.
  - Splines (`trim/spline.rs`) are cut where `Cutter::spline_cut_positions` finds crossings, by the
    parameter of the drawn `BSpline` (`Course::Spline`, period 1 when closed). A kept stretch is cut
    out exactly by blossoming (`Knotted::between`: the knots inside it, clamped at its ends, each
    control point the blossom of the polynomial piece under it at its knot window); a control
    point whose window and position are unchanged keeps its point and ID, every other is a new
    point and the old ones no curve uses go with their constraints. An open control spline or piece
    becomes a plain control spline when the stretch lies within one span or both its end spans
    are whole, else a `Piece` (a `Knotted` when its inner spans are uneven); a closed
    one is unrolled over two turns (`Knotted::periodic`) and opens into one; a conic stays a conic
    (normalised middle weight gives the new rho; its `Rho` dimension is dropped). A fit-point
    spline cannot hold an exact piece, so its piece is a fit spline of the same spacing through the
    cut and the fit points inside the stretch (a point within the tolerance of a cut gives way to
    it), its shape near the cut shifting slightly. `NoPiece` refuses a stretch with no such spline.
  - On a trimmed spline `Equal` and `Rho` are dropped; a `Coincident` point or `Distance` (where it
    measures on the spline) stays on the piece it lies on, moves to the far piece or is dropped
    with a removed stretch; `Tangent` and `Curvature` joined at a removed end are dropped, joined
    at the far end move to the far piece. The far piece of a split gets no constraint to the near
    one (no constraint holds two splines on one curve).
  - Extend carries a spline's end along its own continuation (`reach_along_spline`): the last
    polynomial piece run on past the end (a conic's own conic), searched for crossings in windows
    doubling from the end span's width (`MAX_DOUBLINGS`), stopping where the continuation turns a
    quarter turn from the end's direction, leaves `REACH_MARGIN` times the sketch's extent or
    passes infinity. A control spline or piece takes the longer end span (`Piece`, exact), a conic
    a new apex and rho, a fit spline keeps every point, its old end becoming a fit point. A closed
    spline is `Closed`.
  - A line cut by a collinear line, or a circle or arc by an arc on the same circle, is cut at the
    ends of the overlap: the cutter's ends inside the target (`overlap_ends`, same tolerance); the
    overlap's own crossings are not looked for. A whole circle on a circle, or a cutter whose ends
    lie outside the target, cuts nothing. Extend does not look for overlaps.
  - Dropped: `Equal` on a shortened line (kept only by the fillet and chamfer, below); `ArcLength`
    and `Sweep` on a shortened or extended arc (`keeps_sweep`; the sketch fillet does the same); any
    distance dimension between the two old ends or points joined to them by `Coincident`.
  - A `Midpoint` on a line, arc or elliptical arc a trim, extend, split, break, fillet or chamfer
    reshapes is relaxed, not dropped (`keeping.rs`): `restructure_leaving` hands the midpoints
    `keeps_length`/`keeps_sweep` refuse to `settle_midpoints`, which turns each, under its ID, flag
    and label, into `Coincident(point, curve)` when the point still lies within `SLACK` of the
    curve's drawn shape (`shape_of`, `Shape::closest`: the segment, arc or elliptical arc as it now
    is), else drops it. The middle of the shorter curve is not where the point is, so the point is
    never moved and never held at the new middle; a point on the part cut away has no curve left to
    stay on. A split and a trim of the middle offer the leftovers to the far piece
    (`settle_midpoints(&[piece], …)`), so the point stays on the line that now holds it. The relaxed
    constraint is a subset of the equations the midpoint held, so it adds no conflict and the
    sketch gains one degree of freedom per midpoint, which the user pins again if wanted. A
    constraint already holding the same point on the curve, or one `check_constraint` refuses (the
    point is an end of the curve), leaves nothing in its place.
  - An `Equal` between the two lines of a fillet or chamfer corner survives (`shorten_to`:
    `detach_equal_lines` before the reshaping, `restore_equal_lines` after) when both are lines and
    their new lengths agree within `SLACK`, which is how the corner's own geometry decides: a
    fillet takes the same tangent length from both lines of a corner, so equal lines stay equal and
    the constraint is equivalent to the one it replaces, adding no redundancy; a chamfer by one
    distance does the same, by two different distances the lengths differ and the `Equal` is
    dropped. An `Equal` with a line the operation does not reshape never survives, nor does one
    when only one of its lines is reshaped (trim, extend, split, fillet against an arc).
  - A split-off piece gets fresh IDs, the construction flag and `split_from`: the curve it came
    from (that curve's own original when it is a piece itself), so every piece of one curve names
    the same original (`Sketch::pieces_of`) and blends follow an edge onto them
    (`kernel-naming.md`). It is a map beside the entities, part of `same_geometry`, forgotten with
    the entity, kept by `restructure`, saved as the entity record's `split_from` key; split and
    break record it the same way (`add_piece`). A piece split from itself is `SplitFromItself`.
  - A line piece is `Collinear` with the kept part (or keeps horizontal or vertical and a new end
    on the kept line); an arc piece gets its own centre, `Concentric` and `Equal`. Tangent,
    parallel, perpendicular and angle constraints to a curve joined only at the far end move to the
    piece holding that end.
  - A new end joins its cutter: `Coincident` with the cutter's end point when the cut lies there,
    else a point on the cutter (an axis included). End points no curve uses are removed.
  - `extend` moves an end to the nearest crossing beyond it and joins it the same way; an end
    shared with another curve, coincident with another point or fixed is refused.
- Offset (`offset.rs`): `offset_chains` splits the chosen curves into chains and takes any number
  of them, ordering each run of lines and arcs joined end to end into one open or closed chain
  (walked from a free end so `Side` means the same on the working copy), a circle into a chain of
  its own, and an ellipse, elliptical arc or spline into a chain of its own (below); branches are
  refused, and so is an ellipse or spline with an end touching another chosen curve
  (`NotInChain`). `offset_chain` is the same for exactly one chain (`SeveralChains` otherwise).
  `offset_each` offsets every chain in one pass on a working copy that replaces the sketch only
  when all succeed, each chain to the side its closure names (`offset` gives them all one `Side`,
  `offset_both_sides` runs every chain both ways), so one undoable step and one refusal naming the
  first chain that fails. `Chain::outline` joins offset curves: smooth joints meet, line corners
  meet sharp, a convex corner with an arc gets a round arc about the original corner, a concave one
  is trimmed where the carriers cross.
  - A closing notch or tab is dropped (`Chain::settle`, repeated until nothing more drops): an arc
    whose offset shrinks to nothing (`Vanished`) and, each round, the stretch the joints trim back
    the furthest past nothing (`UsedUp`, its `run_along` the untrimmed offset at or below the
    tolerance). Neighbours across a dropped run re-meet without the original corner (`bridge`):
    sharp where their carriers cross; two lines running on in one direction along one line merge
    into one line (`Junction::Merge`, a `Stretch` made as the first's offset, so no joint is left
    free to slide); two lines running on in one direction side by side get a step (`Made::Step`,
    `Chain::step`), a line square to both at the distance beyond the corner where the outer one's
    original leaves the dropped run (its end, or the inner one's start when the outer comes after),
    so the step is the sharp corner's mitre a line corner gets; it is held `Perpendicular` to that
    original and at the `Distance` from that corner point, joined sharp, adding no freedom; two
    lines running back against each other whose offsets have passed or reached each other (a
    notch's walls) close too. A dropped curve gets no offset and no constraint, the outline adding
    no freedom. Refused, naming the first curve dropped (`Collapses` for an arc, `UsedUp`
    otherwise): a dropped curve at an open chain's end, fewer than two curves left, or neighbours
    that cannot re-meet (arcs whose carriers miss). A notch dropped stays closed when later edits
    widen it, as a vanished arc does.
  - `offset` holds the distance with the typed expression on the first curve and after every sharp
    corner; after a smooth joint or round corner it follows from the `Tangent` there, so a line only
    gets `Parallel` and an arc nothing. A closed chain's closing `Coincident` is added last, so the
    one relation a loop repeats is never a whole redundant constraint.
  - The free ends of an open chain are held (`hold_free_ends`): the offset's first and last end
    point lies on the normal of the original end, a `Perpendicular through` (`OnDirection` with
    `square`) the original end for a line and a `Parallel through` the arc's centre along a
    construction radius to the original end for an arc, so the distance alone fixes the copy and
    the sketch gains no degree of freedom from the copy. A closed chain and the both-sides outline
    have no free end, a circle needs none.
  - `offset_both_sides` (`Chain::both_sides`, `Caps` of `Round` or `Flat`): a closed chain gets the
    two one-sided outlines; an open chain becomes one closed outline (`Chain::capped`): left
    outline, cap round the end point, right outline reversed, cap round the start point, the start
    cap's joint closing the loop. A round cap is a half circle centred on the end point (`Made::Cap`)
    joined by `Tangent` to both sides; a flat cap is a line through the end point (`Coincident`)
    `Perpendicular` to the end curve (square to a line, through an arc's centre), joined sharp.
    Either adds no freedom or redundancy. Caps crossing the outline are `CrossesItself`; an open
    elliptical arc or open spline (offsets are free splines) is `CapsOnSpline`. The outline is profile geometry
    even round a construction chain.
- Mirror (`mirror.rs`): points on the mirror line are shared, others copied with `Symmetric` to the
  original; arcs and elliptical arcs swap ends to stay counter-clockwise; circles and ellipses add
  `Equal` (`Sketch::copy_needs_equal`: an ellipse's minor radius is a variable its points do not
  hold, an elliptical arc's ends fix it unless both lie on its major axis); a curve that is its own
  image is left out (an ellipse centred on the line with axis along or square to it). No other
  constraint is copied, since symmetry holds the copy. A point already `Symmetric` about the same
  line, or joined by `Coincident` to one that is (`symmetric_partner`), takes that image, so a
  curve mirrored after its neighbour joins the image already there.
- Patterns (`pattern.rs`): `rectangular_pattern` repeats curves and lone points along one or two
  directions (`PatternRow`: count including the original, spacing, angle from the x axis; the
  second defaults to square to the first); `circular_pattern` about an existing point, the origin
  included (`CircularPattern`: count including the original, over a full turn or a total angle).
  Copies get fresh IDs and the construction flag; circles and ellipses an `Equal` to the original
  (`copy_needs_equal`), added after the ties so only the ellipse's already-held major radius is the
  dependent row and no tie is reported redundant. Instances capped at `MAX_PATTERN_INSTANCES`,
  within `MAX_LENGTH`.
  - Copies stay parametric, each tied to the instance before it (the first to the original), so
    editing the original or any spacing moves everything after it. Rectangular: every point tied to
    its predecessor by `HorizontalDistance` and `VerticalDistance` holding the typed spacing (a row
    along an axis with a literal angle uses the spacing itself and `HorizontalPoints` or
    `VerticalPoints` for the other component; any other angle, a parameter included, uses
    `abs(spacing * cos(angle))` and `abs(spacing * sin(angle))`). Circular: every point tied by two
    construction lines from the centre (`Equal` and an `Angle` holding the step: total over count
    minus one, or 360° over the count), created once per point and instance; the origin as centre
    gets a point `Coincident` with it to start the lines from, since a line cannot end on a
    reference.
  - A point a curve already holds on its own circle or ellipse (an arc's end; an elliptical arc's
    end not setting the minor radius, the one farther from the major axis, both when both lie on
    it) is tied by one row, not two, or none when two curves hold it (`implied_gradients`,
    `kept_rows`): the row whose direction is least along the curve's gradient there, so no tie is
    reported redundant.
  - No constraint among the copied items is copied (a copy tied point by point is already as
    constrained as its original; repeating them would make a whole copy redundant, as with mirror).
    A `Fix` on an original does not reach the copies, so fixing the original moves them.
  - Selection points on the centre are shared by the copies and held by a `Coincident` when not
    already; a circle centred on it is its own image and left out. `PatternError` refuses nothing
    selected, count below 2, over the cap, zero spacing, parallel directions, a centre that is not a
    point, nothing off the centre, angle of 0° or a full turn or more, geometry past `MAX_LENGTH`.
    `rectangular_image` and `circular_image` give faceted copies for a preview without touching the
    sketch.
- Ellipses as cutters (`intersect::Shape::Ellipse`, crossings by sampled roots along the ellipse as
  for splines; two ellipses by the roots of one's level along the other,
  `intersect::ellipse_ellipse`) in trim, extend and break. Trim takes a `Course::Ellipse` cut by
  the crossings of the whole ellipse with each cutter (`Cutter::ellipse_cut_positions`; one lying on
  the same ellipse cuts at its ends): a whole ellipse cut twice opens into an elliptical arc keeping
  its constraints; an elliptical arc is shortened (`keeps_sweep`) or split into two arcs sharing
  centre and axis point, the far piece held to the kept one by `Equal` (then holding only the minor
  radius). Split and break take elliptical arcs: pieces share centre and axis point, and the shared
  point fixes one minor radius for both except on the major axis (`lies_on_major_axis`), where an
  `Equal` holds them. Extend carries an elliptical arc's end round its ellipse to the nearest
  crossing beyond it (`reach_around_ellipse`, `Reach::AroundEllipse`); a whole ellipse is `Closed`
  and cannot be split or broken.
- An ellipse, elliptical arc or spline offsets as a fit-point spline (`offset/around.rs`, `Free`;
  `Chain::follows` is false), its fit points the exact offset at even parameter steps (the
  curve's parameter for an ellipse, the spline's parameter span for a spline), doubled from
  `FIRST_FIT_POINTS` (a share for an arc; four per control point for a spline, at most
  `MAX_FIT_POINTS`) until the centripetal spline strays from the offset by at most `FIT_TOLERANCE`
  of the curve's reach (at most `MAX_FIT_POINTS`); closed for a whole ellipse or a closed spline,
  open end to end otherwise. Inward past the tightest bend (sampled signed curvature times the
  distance reaching one, `collapses_under`) is `Collapses`; a spline whose fitted offset crosses
  itself (sampled segments properly crossing, `crosses_itself`) is `CrossesItself`. The spline has
  no constraint to the original (none could hold along its length), so it adds its fit points'
  degrees of freedom and the app says it will not follow; an open spline's free ends are not held.
  A spline of any kind (fit or control point) is read as its curve. Such a curve chosen with a
  chain it touches is `NotInChain`; beside chains it does not touch it is offset with them.
  `Sketch::curve_crossings` gives where a spline or ellipse crosses another curve.
- Fillet (`fillet.rs`): a `Corner` is where exactly two lines, arcs or elliptical arcs end, kept by
  one of its points. `rounding` refuses a radius whose touching point would not lie on a curve
  short of its far end (`TooLarge`). Between lines and arcs the centre is where their offset
  carriers cross; with an elliptical arc the centre rolls along the ellipse at the radius on the
  inside (`rolled_center`: first root from the corner along the arc of the other side's signed
  distance minus the radius, over `ELLIPSE_SAMPLES`, bisected; the other side's distance being the
  line's, the circle's or the closest point's on its whole ellipse). `fillet` adds the arc
  `Tangent` to both with a `Radius` dimension and keeps the corner point as a sharp held on both
  carriers by `Coincident`, so dimensions, fixes and symmetry on the corner still hold.
- Joining non-meeting curves first (`fillet/crossing.rs`): `join_at_crossing` takes a `Pick` on
  each (curve and a point on it), the corner at the crossing of their carriers nearest the picks
  (`NeverMeet`, `SameCurve`), and replaces the end on the far side of the corner from the pick with
  one shared point there, as trim does (`keeps_length`/`keeps_sweep`, distance dimensions between
  old ends dropped, unused old ends removed; midpoints relaxed likewise). An end that must move outward is checked as extend
  checks it (`Held`); an arc or elliptical arc whose crossing lies off it extends its nearer free
  end round its circle or ellipse (by angle or ellipse parameter, as extend's `reach_around_ellipse`
  does), keeping its minor radius. An elliptical arc's carrier is its whole ellipse
  (`fillet::whole`): crossings by `intersect::crossings` against a line or circle carrier and
  `intersect::ellipse_ellipse` between two, none for two arcs of one ellipse (`same_ellipse`). A
  circle becomes the arc from the corner to the next crossing round it on the picked side (up to
  four against an ellipse; a pick past both neighbours takes the angularly nearer side), that end
  held on the other curve by `Coincident`, so a later join on the same circle extends it round.
  Curves already meeting are left alone. `crossing_picks` gives picks for two selected curves: the
  crossing nearest both drawn curves, a line's, arc's or elliptical arc's longer part, a circle's
  side making the blunter corner (two circles: each its side away from the other's centre).
- Chamfer (`fillet.rs`, same corners): a `ChamferSize` is one distance for both curves (`Equal`), a
  distance on each (`Distances`) or a distance on the first and the cut's angle
  (`DistanceAndAngle`), each a `Dimensioned` (typed expression and value). `bevel` finds the points
  at the distances from the corner on each curve (along a line, by chord on an arc or elliptical
  arc), refusing one past a curve's far end (`TooFar`) or a distance not above zero. With an angle
  the first point is at the distance and the second where a ray from it, turned by the angle from
  the direction back to the corner toward the second curve, meets that curve (`AngleMisses`,
  `AngleOutOfRange` outside 0° to 180°). `chamfer` shortens both curves (`shorten_to`, shared with
  the fillet), joins them with a line and keeps the sharp the same way. It holds a `Distance` from
  the sharp to each new end with that side's typed expression, so either side can change alone;
  with an angle, the first end's `Distance` and an `Angle` between the cut and the first curve
  measured inside the cut-off triangle (`from`, `to`, `reversed` giving a positive drawn value are
  found by measuring, `AngleNotHeld` if none does; an elliptical arc's measured from its tangent at
  the new end).
- Insert a control point (`insertion.rs`): `insert_control_point(spline, near)` adds one control
  point to an open control spline, piece or knotted spline of at least four points, keeping the
  curve exactly by Boehm's knot insertion (`Knotted::inserted`, in homogeneous coordinates like the
  blossoms): a knot goes into the span at the parameter closest to `near` (clamped `SPAN_MARGIN`
  of the span from its knots so no span collapses), else into the span with the longest curve
  that leaves its neighbours alone. With degree three the knot replaces the two control points
  before the span's end by three, so those two keep their IDs and move along the polygon, the
  third is new and gets the next place in the list, and the result is `Part::kind` (a `Piece` or
  `Knotted`). Moving them may not break a constraint, so they must be free: no constraint, no
  other user, not projected, listed once (`InsertError::HeldPoint` names the first that is not;
  with no `near` the next longest span is tried). Constraints on the spline itself hold, since
  its curve does not change. Refused with `FitSpline`, `ClosedSpline` (a periodic spline would
  need a closed knotted kind), `Conic`, `TooFewPoints` (degrees one and two would change degree),
  `Projected`, `NotASpline`. It works on a clone, so a refusal changes nothing.
- Split (`split.rs`): `split_at` cuts a line or arc at an existing point on it between its ends
  (`check_split`), which becomes the end both pieces share, its `Coincident` on the curve dropped.
  A line's pieces are `Collinear` or each keeps horizontal or vertical; a point that was the line's
  `Midpoint` makes the pieces `Equal`. An arc's pieces share the centre, so one radius with no
  constraint added. Tangent, parallel, perpendicular and angle constraints joined at the far end
  move to the far piece (`far_constraints`, as trim). A spline (`split_spline` in `trim/spline.rs`)
  is cut at the parameter of the drawn `BSpline` closest to the point, both pieces exact by the
  blossoming trim uses (`spline_halves`): the point is the last control point of the near piece
  and the first of the far one, the constraints fall as in a trim of the stretch (`spline_fates`),
  and the pieces get no constraint to each other. Control splines, pieces and conics split; a
  fit-point spline (`FitSpline`: no fit spline holds a piece exactly) and a closed one
  (`ClosedSpline`: one point only opens it) are refused in words, as are circles, whole ellipses
  and projected curves.
- Break (`breaking.rs`): `break_curve` splits a line, arc, elliptical arc or open control spline at
  every crossing with the other curves and the two axes, found as trim finds cuts (`open_cuts`).
  Each cut goes through `split_at` from the curve's start toward its end, the next cut lying on the
  piece just made, so `Collinear` constraints chain without repeating. The cut point is the end
  point of a cutter lying there when one does, else a new point held on the cutter by `Coincident`
  (an axis included); a crossing shared by several cutters is joined to one. Pieces keep the
  curve's constraints as a split does and are fully determined by the crossings, so the degrees of
  freedom of lines and arcs do not change (a spline's pieces add control points).
  `break_curves` breaks each of several curves in turn on a working copy, skipping those that
  cannot break (circles, whole ellipses, fit-point and closed splines, reference or projected
  curves, curves crossing nothing); `BreakError` names the curve for a single refusal, else
  `NothingToBreak` or `NothingSelected`.

## Relations the geometry shows (`inference.rs`)

- `Sketch::shown_relations` finds relations a drawing already shows within a `Tolerance` (distance
  and angle; `Tolerance::of` is `RELATIVE_DISTANCE` of `Sketch::extent` and `ANGLE_DEGREES`), for
  the `RelationKind`s asked, in this order: coincident ends (curve ends, lone points, the origin,
  never two ends of one curve), concentric circles and arcs, points mirrored about an axis (the
  reference axes and construction lines; arc centres left out), horizontal and vertical lines,
  tangents (a line or arc leaving a joint along another, a line or circle grazing a circle within
  both drawn curves), perpendicular directions (one per pair of slanted direction groups, a joined
  pair preferred), parallels and equal lengths and radii (each group chained to its first). Points
  match through a grid of tolerance-sized cells and direction/size groups anchor at their smallest,
  so a large import costs no pairwise scan. A candidate the sketch refuses (`check_constraint`),
  restates or contradicts (`Relations`, which also records earlier candidates) is left out.
- `Sketch::inferred_relations` keeps those that hold (`keep_holding`): solve first
  (`InferenceError::Unsolved`), then add each kind as a stage on the solved geometry of the one
  before, so a relation earlier ones imply is named redundant by the rank analysis and dropped. A
  conflicting stage drops the newest candidate of each conflict and solves again; one whose solve
  moves any point or radius more than `MOVE_LIMIT` tolerances is tried one candidate at a time,
  each kept only when it solves, adds rank and stays within the limit. Returns what is kept with
  the degrees of freedom left.
- `Sketch::datum_dimensions` (`datum.rs`) dimensions what is still free from a datum point (origin
  or any point; else `InferenceError::NotAPoint`) at measured values through `keep_holding` in one
  stage, so only rank-adding dimensions are kept: the datum's horizontal and vertical distances
  from the origin (unless it is the origin or fixed), each circle's diameter, arc's radius and
  ellipse's minor radius, then every other free point's horizontal and vertical distance from the
  datum, nearest first so a rectangle cornered on it gets width and height. An offset within the
  tolerance gets `VerticalPoints` or `HorizontalPoints` instead. Fixed and reference geometry is
  left out.

## Checking a sketch (`check.rs`)

- `Sketch::flaws` names what the eye misses within a `Tolerance`: curves of no length
  (`Flaw::NoLength`: line, arc length or radius, or every spline control point within the
  distance), ends of different curves within the distance and not joined (`NearlyJoined`, with the
  gap; a no-length curve's two ends count as joined), and a line, circle or arc lying on another
  (`OVERLAP_SAMPLES` points along it all within the distance: `LiesOn`, the newer on the older when
  each lies on the other) or sharing a stretch (two or more samples: `Overlaps`). Projected and
  reference geometry left out; pairs found by a sweep over boxes.
- `Sketch::fix` gives each flaw's repair as entities to remove and constraints to add (`Fix`): a
  `Coincident` for nearly joined ends; a no-length curve removed (its points no other curve or
  constraint uses go with it, the two kept ends joined); a curve lying on another removed the same
  way, each kept end not already joined to an end of the other held on it by `Coincident`. A
  partial overlap has no fix (`Fix::is_empty`): which piece to keep is the user's choice.

## Tangent circles (`tangent_circle.rs`)

- `tangent_circle` draws a circle tangent to three of the sketch's lines, circles and arcs (an arc
  counts as its full circle, an axis as the infinite line), or to two at a given radius, held by a
  `Tangent` to each and, for two, a `Radius` holding the typed expression (`Dimensioned`).
  `TangentError` refuses any other count, a radius with three curves, none with two, a curve chosen
  twice, a point or spline, a radius not above zero, no touching circle, and one past `MAX_LENGTH`.
  Curves whose offsets coincide (parallel lines at twice the radius, three parallel lines) have no
  discrete solution and are refused as no circle.
- `tangent_circle_near` finds every circle (`TangentCircle`) over each side choice (outside or
  inside a circle, either side of a line), keeps those touching every curve within a tolerance
  relative to their reach, and takes the one whose centre is nearest the given point, the smallest
  when none. Two curves offset both by the radius and intersect the offsets (`fillet::centers`).

## Blend curves (`blend.rs`)

- `blend` joins two curve ends (`BlendEnd`: a line, arc or spline and one of its end points,
  `Sketch::ends_of_curve`) with a new spline held by `Coincident` from each end to the picked
  point, `Tangent` to each curve and, for `Continuity::Curvature`, `Curvature` to each. It is
  ordinary geometry; projected curves can be blended.
- `blend_curve` gives control points for the preview without touching the sketch, and `blend`
  starts the spline from exactly them, so the first solve moves nothing. Tangent: a cubic of four
  points. Curvature: six, the third offset square to the direction so the spline bends as the curve
  does (arc `1/r`, line zero, spline its own end curvature; `unit_bend`).
- `BlendError` refuses: a curve with no ends, a point not an end of its curve, both ends on one
  curve, ends already at one place, an end with no direction, control points past `MAX_LENGTH`, and
  curves already joined so the solver would read the spline's other end as the joint (`joined_ends`,
  the joint search `Tangent` and `Curvature` use). A curvature blend to a two-point spline is
  refused by `Curvature` itself (`Edit`).

## Spur gears, pairs and sprockets (`gear.rs`, `gear/pair.rs`, `gear/sprocket.rs`)

- `SpurGear`: module, tooth count (`MIN_TEETH..=MAX_TEETH`), pressure angle in degrees
  (`MIN_PRESSURE_ANGLE..=MAX_PRESSURE_ANGLE`), profile shift in modules, root fillet radius and bore
  diameter (0 for none). `GearCircles` are the standard ones (pitch `m z / 2`, base pitch times
  `cos α`, tip `ADDENDUM` and root `DEDENDUM` modules either side, moved out by the shift).
- `SpurGear::outline` refuses in words (`GearError`): a tooth count undercut at the shift (rack rule
  `1 - x - z sin²α / 2` with `UNDERCUT_SLACK` modules of grace, so 17 teeth pass at 20° with no
  shift; names the fewest teeth and least shift), teeth coming to a point (`MIN_TIP_LAND`), a root
  circle through the centre, teeth meeting at the root, a root fillet too large (names the largest,
  `largest_root_fillet`), a bore leaving under `MIN_RIM` modules, anything past `MAX_LENGTH`.
- One tooth's upper half is worked out in its own frame, then mirrored and turned to every tooth.
  The flank is a clamped control spline of 4 to 6 points fitted to the involute with the roll angle
  as parameter (arc length has no closed form and the involute leaves the base circle at a cusp)
  within `FLANK_TOLERANCE` modules (`fit::least_squares`, end points pinned). The involute starts
  `START_ROLL` above the base circle; a radial line runs to the root circle when the root lies
  below it. The outline (`GearPiece`s in order, each starting where the last ended): tip arc, flank,
  line, fillet, root arc, then the next tooth.
- `Sketch::add_gear` draws it about a `GearCentre`: a free position gets a new point, an existing
  point is shared, the origin gets a point held on it by `Coincident`. Pieces share end points, tip
  and root arcs share the centre, and the pitch, base, root and tip circles are construction
  circles on it, so the outline is one closed profile and the bore a circle inside it. No other
  constraint: the gear does not follow later changes to its values. The drawing is shared
  (`Sketch::draw_toothed`: circles, then the closed outline, then the bore), as is the placing of
  the centre (`wheel_centre_position`, `wheel_centre_point`).
- `SpurGear::outline_turned` turns the whole outline about its centre; the tooth centred on the +x
  axis is the unturned one.
- `GearPair` (`pair.rs`): a driver, the partner's tooth count and bore. The partner shares module,
  pressure angle and root fillet and takes the opposite profile shift (`GearPair::partner`), so the
  standard centre distance `m (z1 + z2) / 2` (`centre_distance`) holds with a tip clearance of
  `DEDENDUM - ADDENDUM` modules and no backlash. It sits that far along +x from the driver
  (`partner_centre`), turned by `PI + PI / z2` so a gap of its own lies on the line of centres
  where the driver's tooth points (`partner_turn`, which also takes the driver's turn, the partner
  counter-turning by the tooth ratio). A partner that cannot be cut is refused as
  `GearError::Partner` naming the meshing gear. `Sketch::add_gear_pair` works both outlines out
  before drawing either; the partner gets a new centre point, joined to the driver's by no
  constraint, and its own construction circles and bore.
- `Sprocket` (`sprocket.rs`): a `ChainSize` (designation, pitch, roller diameter; `ChainSize::ALL`
  holds the ISO 606 A and B series from 05B to 48B, `DEFAULT` is 08B), a tooth count
  (`MIN_SPROCKET_TEETH..=MAX_SPROCKET_TEETH`) and a bore. The pitch circle is `p / sin(180° / z)`
  across, through the roller centres. Each gap is made of the ISO 606 annex values, in the middle
  of their limits: a seat arc about the roller centre of radius `0.505 d1` (the least ISO allows)
  over `130° - 90° / z`, a flank arc tangent to it, curving the same way, of radius the mean of
  `0.12 d1 (z + 2)` and `0.008 d1 (z² + 180)`, running to the tip circle of diameter
  `d + p (1.125 - 0.8 / z) - d1` (between the ISO least `d + p (1 - 1.6 / z) - d1` and greatest
  `d + 1.25 p - d1`), and a tip arc on that circle to the next gap. Lines are not part of this
  form. Teeth coming to a point (`MIN_TIP_LAND` of the pitch) are refused as
  `GearError::SprocketPointed`; the bore leaves `MIN_RIM_PITCHES` pitches below the root.
  `SprocketOutline` pieces are `GearPiece`s in the same order as a gear's (gap: lower flank, seat,
  upper flank, tip arc); the pitch, tip and root circles are construction circles
  (`Sketch::add_sprocket`). The first gap lies on the +x axis.

## Faceting and splines (`curve.rs`, `fit.rs`)

- Curves are drawn as polylines within a chord tolerance (`Faceting`, bounded counts per turn and
  `MAX_SEGMENTS`); `Faceting::within` accepts any value (NaN or below zero as the finest). Trim
  pieces and extensions are faceted the same way.
- Sketch splines are clamped, degree `min(MAX_SPLINE_DEGREE, points - 1)`, exact derivatives,
  banded elimination (`banded.rs`); control splines have uniform knots. `BSpline::fit` approximates
  a dense polyline within a tolerance, `interpolate` passes through points at evenly spaced
  parameters, `through` follows unevenly spaced points without loops.
- Centripetal fit splines (`spacing.rs`, `FitLayout`) use parameters stepping by the square root of
  the distance (`centripetal_parameters`, each step at least `SHORTEST_GAP_SHARE` of the mean, so
  repeated points stay solvable): chord length bulges the long gap beside a tight cluster and even
  steps loop and backtrack. Open: `interpolate_centripetal` (`averaged_knots`, banded collocation).
  Closed: `interpolate_closed_centripetal`, a periodic cubic with knots at the parameters round the
  loop (`looped`) solved as a cyclic system (`periodic_through_knots`), clamped at the seam by
  `BSpline::periodic_on`.
- `BSpline::periodic` is the uniform cubic over the points round a loop (`periodic_knots`), clamped
  at its seam by knot insertion so ends meet with equal first and second derivatives;
  `interpolate_closed` passes through the points at knots `i / n` (`periodic_through`).
  `BSpline::conic` is a rational quadratic; `weights` is the one rational case, and the rational
  basis and derivatives (`rational_basis`) serve curve and solver alike.
- A spline meets a line or circle where its signed distance changes sign between samples
  (`intersect::spline_roots`, bisected); a dip toward zero without a sign change is searched by
  golden section (two crossings, or one touching point within the tolerance). Two splines cross
  where their sampled polylines do, refined by Newton on both parameters
  (`intersect::spline_spline`) and kept when both curves meet within the tolerance.
