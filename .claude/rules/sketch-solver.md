---
paths:
  - "crates/caditor-sketch/src/solve/**"
  - "crates/caditor-sketch/src/sketch.rs"
  - "crates/caditor/src/drag_solver.rs"
---

# Sketch solver

## Solving

- `solve` evaluates dimensions first; lengths are at most `MAX_LENGTH`, a huge value refused in
  words rather than overflowing the solver's scale.
- Then damped Gauss-Newton with minimal-norm steps on each independent part: SVD up to
  `DENSE_LIMIT` variables, above that CGLS on the sparse Jacobian with a sparse factorisation for
  the analysis (`sparse.rs`). Every equation has an analytic gradient.
- Convergence is checked after every step and cancellation before it, so the diagnosis budget
  counts steps taken. The analysis polls cancellation before each part, each constraint and each
  null-space solve. Tests bound cost with `solve/tally.rs` work units, never wall-clock time.
- An attempt ends converged, at a least-squares minimum, pressed against a collapse (such a minimum
  with a moving line or arc span, or a radius, near the collapse length) or unfinished. Retries
  perturb each part by a fraction of its own extent.
- The parts are worked out once per solve (`numeric::Parts`, from the starting values) and shared
  by the memo, the solver and the analysis. Only a sketch with spline parameters works them out
  again from the current values, since a parameter moving along its spline changes which control
  points its equations reach.
- A `Component` lists the spans (a line's ends, an arc's centre and start) with an end among its
  variables (`System::spans_at_variable`), so collapse checks look at a part's own spans, not the
  whole sketch's.
- A solve that would collapse a line or an arc's radius to nothing is not converged, reported as a
  conflict; if it already had no length, `SketchError::NoLength` naming it.
- Each part solves at its own scale (largest coordinate or length dimension, at least 1, rounded up
  to a power of two; `system.rs`), which sets its tolerance, step limit and degenerate lengths, so
  no part depends on another.

## Dragging

- `solve_dragging` takes `Drag`s (a point or a circle's radius with its target). It starts from the
  targets and first holds them there (`FROZEN`); when that cannot work they are only `STIFF` times
  as willing to leave their targets as free geometry is to move. Geometry that already satisfies
  its constraints does not move; under-constrained geometry moves as little as possible.
- The app's `drag_solver.rs` runs it on its own worker thread through `solve_geometry_from`: the
  same solve without the rank and degrees-of-freedom analysis, returning a memo whose parts are
  marked not analysed, so they warm-start a later solve but never stand in for its analysis.

## Equation forms

- Two-branch equations take their branch from the starting geometry, so a solve never flips:
  tangent side, signed and horizontal/vertical distance, the side of a circle or line a point keeps
  its distance on. Internal circle tangency instead follows whichever circle is currently larger.
- A tangent whose curves share a point (directly or through coincidences) is the radius there
  perpendicular to the line (or both radii along one line), keeping full rank where the distance
  form has none. A zero distance between points is solved as a coincidence.

## Spline handles (`solve/spline.rs`)

- `System::add_splines` builds every spline's `SplineHandle` once, before the constraints: its
  control point handles, degree, knots and weights, and the forms read only the handle, so each
  constraint works on every kind. An open control spline's handles are its points; a closed one's
  are its points taken round again (`points + 3` handles on `periodic_knots`, a point's gradient
  arriving at each copy); a conic's its three points with the rational weights.
- A fit-point spline's control points are variables of their own (`System::hidden`, named
  `Variable::Control` in the memo, counted with the spline in `entity_variables`), started from
  the interpolation of the fit points, and the fit points are held on the curve at their
  parameters by implicit `Form::Through` equations (the point minus the basis-weighted control
  points): an open one shares its end points with its fit points and holds the inner ones, a
  closed one holds every fit point. Its knots and parameters are the `FitLayout` of the fit points
  where the solve starts (`i / n` and uniform knots for an evenly spaced one). The equations
  determine the hidden points, so the spline counts two degrees of freedom per fit point, and its
  fit points take constraints and dimensions like any point.
- A centripetal spline's knots depend on where its fit points are, which no equation
  differentiates, so `solve_with` solves in rounds: when the solved fit points give knots more
  than `KNOT_SETTLING` from those the round started with (`respaced`), it solves again from the
  solved sketch, with the fit points of centripetal splines held stiff (`spaced_fit_variables`,
  frozen first like dragged points), so the rest takes up the change and the knots settle the
  next round; at most `SPACING_ROUNDS` rounds, the last one's system finishing the solve. A point
  on the spline then lies on the curve the sketch draws (a test moves the fit points under it).
- A spline's ends (`SplineEnd`, its end point and which end) take their legs from the handle
  (`end_legs`), so a tangent or curvature at a fit-point spline's end uses its hidden control
  points and a conic's end curvature its weights (`end_factor` of the handle's knots at that end,
  times `w_end w_after / w_next²`). Closed splines have no ends; their handles are `periodic`, the
  basis taking the parameter round one turn (`rem_euclid`).
- A conic's weights come from its active `Rho` dimension when it has one, else from its stored
  rho, and a memo key holds every weighted spline's weights among its entities (`Key::weights`),
  since rho is no variable and would not otherwise reach the key.

## Spline parameters (`solve/spline.rs`)

- A point on a spline, a point at a distance from one (`Form::SplineFoot`, square to it there, and
  `Form::SplineDistance`, the signed distance along its normal on the side the point started), a
  tangent between a spline and a curve not sharing one of its end points, and a distance from a
  spline to a line or circle get a parameter of their own along the spline: an extra variable after
  the geometry's, counted in the degrees of freedom, never perturbed, clamped to the spline's range
  (a point held beyond the end of a spline that cannot move is a conflict), kept in the `SolveMemo`
  (`System::parameters` lists each constraint's, named by constraint and position). On a closed
  spline it wraps instead (`System::wrapping_parameters`: never pushed against a bound, taken
  modulo one turn after each step), so a point slides across the seam.
- A distance from a spline to a line or circle is the tangency forms offset by the value
  (`Form::SplineOnLine` and `SplineOnCircle` carry a side, taken from the start, and the value;
  a tangency is side 1, value 0), with `SplineAlongLine` or `SplineAcrossRadius` keeping the
  spline square to the gap there.
- A tangent between two splines sharing no end takes a parameter on each (`pair::closest_pair`:
  the nearest pair of samples, refined by Newton on the squared distance): `Form::CurvesMeet`
  along x and y and `Form::CurvesAlong`, their tangents parallel, three equations for two
  parameters. The curve forms read a `CurveHandle` (a spline's handle or an ellipse's) evaluated
  at its parameter (`CurveAt`: point, tangent and second derivative, pushing gradients into
  whichever handle it is), so the same forms serve ellipses (below).
- Equal lengths with a spline (`Form::SameLength` of `LengthOf::Line` or `Spline`) differentiate
  the Gauss–Legendre sum of the speed node by node, five nodes in every knot span of the handle
  (`length_nodes`), as `BSpline::length` sums it.
- An angle to an arc is `Form::Angle` on the radius from the centre to the joint, turned by a
  right angle (counter-clockwise at the arc's start, clockwise at its end) through the target value.
- It starts at the closest point (or the stationary point of the distance to the other curve),
  refined by Newton so geometry that already holds does not move; among starts equally near (within
  `TIE`) one with a non-vanishing first derivative wins, and where it vanishes a tangency uses the
  second derivative, never a fixed fallback that could line up by chance.
- A step pushing a parameter already at an end of its range further out is retaken with it held, so
  the rest moves instead of the step being spent on the clamp.

## Ellipses

- An ellipse's minor radius is a variable beside the circles' radii (`System::radii`, fixed when
  projected) and its centre to axis point a span, so it counts five degrees of freedom and an
  elliptical arc seven: its start and end are held on it by two implicit `Form::OnEllipse`.
- `Form::OnEllipse` is `b/2 (x²/a² + y²/b² − 1)` in the ellipse's frame, a length that is the
  distance for a circle. `Form::EllipseTangent` holds the signed distance from the centre to the
  line at `sqrt(a²(n·u)² + b²(n·v)²)` (the ellipse's reach toward the line) plus its `value` on
  the side the centre started, so a `Distance` from a line to an ellipse is the same form with
  the gap as value (`System::ellipse_line_gap`) and needs no parameter; when the line and ellipse
  share a point a tangency is `Form::EllipseTouch`, the line along the ellipse's tangent there,
  for the same reason a joined circle tangency is a right angle. `MajorRadius` is a
  `PointDistance` and `MinorRadius` a `Radius` on the minor radius.
- A circle or arc tangent to an ellipse at a shared point is `Form::EllipseTouchCircle`: the same
  touch with the circle's radius there turned a right angle as the tangent, so the radius runs
  along the ellipse's normal, one equation for either side.
- Elsewhere an ellipse gets a parameter of its own, as a spline does (`solve/ellipse.rs`,
  `System::ellipse_parameter_start`): a circle or arc tangent to it sharing no point, a point at a
  distance from it and a circle or arc at a distance from it. The parameter is the ellipse's angle
  over a full turn, always wrapping (`System::wrapping_parameters`), on the whole ellipse as a
  point on an arc lies on its whole circle; the forms evaluate the ellipse there through
  `EllipseHandle::at` (point, tangent and second derivative by the parameter, gradients through
  `push_point` and `push_tangent` into the centre, axis point and minor radius). A point's
  distance is `Form::EllipseFoot` (square to the ellipse there) and `Form::EllipseDistance` (the
  signed distance along its normal on the side the point started), even at zero; a circle's is
  `Form::EllipseOnCircle` (the point there at the radius plus the value from the centre, the side
  taken from the start, side 1 and value 0 for a tangency) and `Form::EllipseAcrossRadius` (the
  ellipse square to the radius there). A point's parameter starts at its closest point
  (`EllipseGeometry::closest_parameter`), a circle's at the stationary point of the distance from
  its centre whose gap to the circle is smallest (`touching_parameter`, sign changes of the slope
  over `STATIONARY_SAMPLES`, bisected), so geometry that already holds does not move.
- Between an ellipse and a spline or another ellipse (`solve/pair.rs`), a tangent or distance
  takes a parameter on each curve, in the constraint's order (`System::pair_parameter_start`, both
  started at `closest_pair` over `pair::Course`, the ellipse's samples taken round a full turn and
  its parameter wrapping, the spline's clamped). A tangent sharing no point is `Form::CurvesMeet`
  along x and y and `Form::CurvesAlong`; a distance is `Form::CurvesFoot` (the second curve's
  point square to the first's tangent), `Form::CurvesGap` (along the first's normal at the value,
  on the side it started) and `Form::CurvesAlong`, so the gap is where they come closest, three
  equations for two parameters either way. Where they share a point a tangent needs no parameter,
  since the meeting equations would only repeat the coincidences: two ellipses through one point
  are `Form::EllipsesTouch` (their normals there parallel, each normal's gradient through
  `ellipse_touch_along`), and a spline ending on an ellipse is `Form::EllipseTouch` with its first
  leg (`Joints::pair_joint`). `Sketch::ellipse_gap` and `spline_gap` give the measured points of
  such a pair from the same `closest_pair` (`curve_pair_gap`), so geometry that already holds
  does not move.
- An angle between a line and an elliptical arc sharing an end is `Form::NormalAngle`: the angle
  from the line to the ellipse's outward normal at the joint (`ellipse_touch_along`'s normal),
  turned a right angle as an arc's radius is (`Joints::arc_joint` takes elliptical arcs), so the
  arc's direction is its tangent leaving the joint and a chamfer by a distance and an angle can
  hold its angle on an elliptical arc.
- `OnMinorAxis { point, ellipse }` is `Form::OnMinorAxis`: the point's offset from the centre
  along the major axis is zero, so with a `Coincident` on the ellipse it holds the point at a
  minor axis end however the axis is turned.
- `Equal` between ellipses is `EqualLength` of the centre-to-axis spans and `EqualRadius` of the
  minor radii (`EllipseHandle::minor_circle`), only the latter when both handles share the centre
  and axis point, so a split elliptical arc's pieces get no zero row.
- `Midpoint` on an elliptical arc is `Form::OnEllipse` and `Form::EllipseMiddle`: the major radius
  times the wrapped difference between the point's parameter (`atan2(y/b, x/a)` in the ellipse's
  frame) and the start's plus half the counter-clockwise sweep to the end, a single branch, since
  the opposite point is half a turn away.

## Curvature at a spline's end

- A clamped spline's curvature at its end is `end_factor` (from its knots at that end) times the cross
  product of its first two legs over the first leg's length cubed, at either end with the legs
  taken from that end; a test checks it against the spline's own derivatives.
- `Curvature` with a line holds the first two legs parallel; with a circle or arc,
  `Form::EndCurvature` holds the curvature times the radius at the side the centre lay on at the
  start; between splines, `Form::MatchedCurvature` holds the two curvatures, each measured going
  into its spline, opposite.

## Constraint state

- Degrees of freedom and each entity's state come from the rank and null space of the Jacobian at
  the solution. A constraint whose equations add no rank over older ones is redundant, naming what
  it duplicates.
- Above `DENSE_LIMIT` the analysis (`sparse::Triangular`) merges the normalised rows, in
  constraint order, into an upper-triangular basis by Givens rotations, its columns in a
  minimum-degree order of the column graph, so fill stays within the symbolic Cholesky structure
  and a closed chain analyses in near-linear work (a test bounds it against the chain's length). A
  row adds rank when an entry above `RANK_TOLERANCE` reaches a column with no basis row. A variable
  is fixed when its row of the null space (each free column set to one, back-substituted) stays
  within `sqrt(NULL_SPACE_TOLERANCE)`: a part with no free column is fixed throughout, a column
  that reaches no free column through the basis is fixed outright, and the rest take one
  back-substitution per free column or one transposed solve per column, whichever is fewer. Dense
  and sparse analysis agree on the same parts, closed chains included.

## Conflict diagnosis (`diagnosis.rs`)

- A conflict is named only on evidence that its constraints cannot hold together, never because one
  solve from one start failed: constraints the solver merely cannot reach together from the drawn
  shape (a line that would have to fold back) are solved from where diagnosis reached, or
  `SketchError::Unsolvable` when it reached nowhere. The evidence is numerical, not a proof.
- The first candidate is the support of the failed solve's least-squares point (skipped when it is
  pressed against a collapse); when it holds, or there is none, QuickXplain-style divide and conquer
  over the part's constraints holding the newest one looks for one. Probes solve the sub-parts
  their constraints form from the closest known state, remember each outcome by its equations and
  share one budget, `DIAGNOSIS_WORK`.
- A probe's failure only steers the search. The set found is trimmed, oldest first, of every
  constraint without which it still fails; each kept constraint comes with a witness, a solution of
  the set without it. A descent is conclusive when its unperturbed attempt ends at a least-squares
  minimum clearly above tolerance, or pressed against a collapse, and no retry converges. The set
  cannot hold when its own probe and every descent from the drawn shape (also with spline
  parameters mid-range and held) and from the closest witnesses are conclusive; any descent that
  converges means it holds (remembered, the search runs again); otherwise it is inconclusive.
- Trimming tries a step before a descent (`witness.rs`): the set's failing part is linearised and
  factored once at its probe's least-squares end (`sparse::Triangular` over the normalised rows,
  solved through the semi-normal equations, `Triangular::normal_solution`). A constraint's witness
  is that point moved through the pseudo-inverse columns of its own rows, weighted so the
  linearisation of the rest holds (the misfit along the left null space lands on the removed rows)
  and, where that leaves a choice, the step is shortest; chord steps through the same factor with
  fresh residuals follow (at most `CHORD_STEPS`, each halving what the rest leaves) until every
  part of the rest holds within its own tolerance. Only a constraint whose step does not get there
  falls back to a descent, and dropping one factors the smaller set again. The factorisation is
  charged as one step on the set and each solve through it `1/SOLVES_PER_STEP` of one, about its
  time in a debug build: the 600 confirmations of a 300-line chain cost about 26,000 units where
  descents needed about 540,000 (an ignored test in `diagnosis_tests.rs`, run by the nightly stress
  job; a quick one checks that the step confirms exactly the constraints a 40-line chain needs).
- Only a set judged unable to hold is reported, minimal by its witnesses; recompute reports it as
  the feature's error with `FeatureError.constraints` and `FixTarget::Constraint`.
- Every failed part is diagnosed on its own budget, newest first, up to `DIAGNOSED_PARTS` (the rest
  are `Unsolvable`). Two or more reports are `SketchError::Several`, so the user meets every
  problem at once; recompute merges them into one `FeatureError`.
- When the search ends with no such set (or its budget is spent) and the part holds after all (a
  probe of all its equations converged, else one more descent of the whole part from the closest
  known state, on what is left of the budget, converges), that solution is the part's
  (`diagnosis::Found`): the solve succeeds with it when every failed part found one, and the memo
  keeps it under the drawn start, so the next solve starts there without diagnosing again. A part
  that found one is never reported.
- A failure with neither (budget spent, inconclusive evidence, no descent of the whole part
  converging) is `SketchError::Unsolvable` with the part's curves, free points and newest
  constraint; recompute words it as that geometry not solving from its current shape.

## SolveMemo (`memo.rs`)

- `Sketch::solve_from` takes the previous solve's `SolveMemo` (recompute passes the feature's last
  good `SketchResult::memo`). Each part is keyed by its entities, constraints, dimension values,
  starting values and own scale, and remembered under both starting and solved values, so an
  untouched part starts from its old solution and reuses its rank analysis once its solved values
  match exactly, however the rest of the sketch grew or moved. A memo also holds the sketch's fixed
  values (projected geometry) once, and is recalled only under the same ones.
- An outcome lines up with its key's variables in order, so recalling it maps nothing by name:
  its solved values and fixed columns follow the part's variables, and a part whose equations or
  variables differ from the one its key was made from is analysed afresh.
