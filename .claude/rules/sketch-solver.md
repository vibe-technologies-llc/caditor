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
- Damped Gauss-Newton with minimal-norm steps per independent part: SVD up to `DENSE_LIMIT`
  variables, above that CGLS on the sparse Jacobian with a sparse factorisation for the analysis
  (`sparse.rs`). Every equation has an analytic gradient.
- When a whole Gauss-Newton step does not lower the cost, the step is Newton's on the cost
  (`numeric/newton.rs`): near a least-squares point that cannot reach zero the Jacobian is nearly
  singular along what only the residuals' curvature resists (a chain bending), so a Gauss-Newton
  step there overshoots and its line search crawls. The Hessian is the Gauss-Newton term plus each
  residual times its second derivatives, by finite differences of the analytic gradients (`PROBE`
  of the part's scale) with columns sharing no equation perturbed together, so a part costs a few
  linearisations. It is factored by sparse Cholesky (`sparse::Cholesky`, in the analysis's
  minimum-degree order) under a Levenberg-Marquardt shift, raised from `LEAST_DAMPING` until it
  factors and the whole step lowers the cost, then lowered as far as the model predicted the fall
  well. After a Newton step the next tries Newton first; a Gauss-Newton line search stays the
  fallback, and solves whose Gauss-Newton steps all land whole are unchanged.
- Convergence is checked after every step and cancellation before it (and before each further
  Newton shift), so the diagnosis budget counts steps taken; the analysis polls cancellation before each part, constraint and null-space
  solve. Tests bound cost with `solve/tally.rs` work units (`fit.rs` adds its passes over the
  samples), never wall-clock time.
- An attempt ends converged, at a least-squares minimum, pressed against a collapse (such a minimum
  with a moving line or arc span, or a radius, near the collapse length) or unfinished. A minimum
  is a Newton step under the least shift predicting under a thousandth of the cost
  (`SETTLED_FALL`), no step lowering the cost, or three steps each gaining under a thousandth. Retries
  perturb each part by a fraction of its own extent.
- Each part solves at its own scale (largest coordinate or length dimension, at least 1, rounded up
  to a power of two; `system.rs`), which sets tolerance, step limit and degenerate lengths.
- Parts (`numeric::Parts`) are worked out once per solve from the starting values and shared by
  memo, solver and analysis; only a sketch with spline parameters redoes them from current values,
  since a parameter moving along its spline changes which control points its equations reach.
- A `Component` lists the spans with an end among its variables (`System::spans_at_variable`), so
  collapse checks look at a part's own spans.
- A solve that would collapse a line or an arc's radius to nothing is not converged, reported as a
  conflict; if it already had no length, `SketchError::NoLength` naming it.

## Dragging

- `solve_dragging` takes `Drag`s (a point or a circle's radius with its target). It starts from the
  targets and first holds them there (`FROZEN`); when that cannot work they are only `STIFF` times
  as willing to leave their targets as free geometry is to move. Geometry already satisfying its
  constraints does not move; under-constrained geometry moves as little as possible.
- A part whose `STIFF` solve fails (a pointer past what the geometry reaches) solves from where it
  was drawn (`solve_from_drawn`); only a part failing from both is diagnosed. Every part with a
  target then reaches toward it (`numeric/reach.rs`) from whichever of its solved and drawn poses
  holds and lies closer (the drawn one is the last frame's while dragging): the closest
  least-squares pose. A first-order step cannot tell an arm turning toward the pointer from one
  bending away, so each step is Newton's on the constraints, curvature being the targets' plus the
  constraints' second derivatives weighted by their least-squares multipliers, by finite
  differences of the analytic Jacobian (`PROBE` of the part's scale). Every accepted pose is a
  solution.
- Up to `DENSE_LIMIT` variables the step is exact: Newton in the null space of the normalised rows
  (SVD), negative or flat curvatures replaced by their size or `FLATTEST`, projected back onto the
  constraints by unit-weight Gauss-Newton and halved until the pose holds and lies closer.
- Above it (`numeric/reach/augmented.rs`) nothing is dense. The multipliers come from a sparse
  Cholesky of the Gram matrix `J Jᵀ`; the step is Newton's on the augmented Lagrangian at the
  current pose, `(H + ρ JᵀJ) p = -(g + Jᵀλ)` with `ρ` `AUGMENTATION` times the steepest curvature,
  factored by `sparse::Cholesky` (the multiplier Hessian probed by colour as in `newton.rs`) under a
  shift from `LEAST_SHIFT` of that curvature, raised until it factors, then its remainder off the
  null space removed through the Gram factor. A tiny shift matters: a long chain turning about its
  end bends in modes whose curvature falls with the square of its length. Steps are capped by
  their largest single move (`MAX_STEP` of the scale), not their norm, since swinging a long chain
  moves everything. The step is halved (each step starting from twice the last fraction) until a
  restoration lands closer: Gauss-Newton corrections in the metric of the constraints' own
  unit-weight curvature plus `RESTORING_SHIFT`, `(M + ρ JᵀJ) Δ = -ρ Jᵀc`, refactored each
  correction and each required to halve the residuals, until every residual is within `CLOSE_SHARE`
  of the tolerance (stopping at the tolerance itself lets the steps creep along it). The
  minimal-norm correction fails there: in a chain of nearly parallel links it shortens them by
  folding the joints sideways, amplified with the chain's length, where the metric pulls each link
  back along itself.
- A frame's reaching stops once it has spent `REACH_WORK` units (`ReachWork`, counting what
  `tally` counts, refused factorisations and orderings included); the pose it stops at holds and
  the next frame goes on from it. A 500-line zig-zag chain dragged past its reach lands on the ray
  toward the pointer at full extension in about 31 million units for the whole frame (5 million
  reaching), a 2,000-line one in about 505 million (92 million reaching, the rest the `FROZEN` and
  `STIFF` solves; an ignored test in `reach_tests.rs`, run nightly).
- `drag_solver.rs` runs it on its own worker thread through `solve_geometry_from`: the same solve
  without rank and degrees-of-freedom analysis, returning a memo whose parts are marked not
  analysed, so they warm-start a later solve but never stand in for its analysis.

## Equation forms

- Two-branch equations take their branch from the starting geometry, so a solve never flips:
  tangent side, signed and horizontal/vertical distance, the side of a circle or line a point keeps
  its distance on. Internal circle tangency instead follows whichever circle is currently larger.
- A tangent whose curves share a point (directly or through coincidences) is the radius there
  perpendicular to the line (or both radii along one line), keeping full rank where the distance
  form has none. A zero distance between points is solved as a coincidence.

## Splines (`solve/spline.rs`)

- `System::add_splines` builds every spline's `SplineHandle` once, before the constraints (control
  point handles, degree, knots, weights); the forms read only the handle, so each constraint works
  on every kind. Open control spline: its points; closed: its points taken round again (`points +
  3` handles on `periodic_knots`, gradients arriving at each copy); a piece (`SplineKind::Piece`)
  its points on `piece_knots`; conic: three points with
  rational weights, from the active `Rho` dimension else the stored rho, and `Key::weights` holds
  them in the memo key since rho is no variable.
- A fit-point spline's control points are variables of their own (`System::hidden`, `Variable::Control`
  in the memo, counted in `entity_variables`), started from the interpolation of the fit points,
  which implicit `Form::Through` equations hold on the curve (an open spline shares its end points
  and holds the inner ones, a closed one holds all). Knots and parameters are the `FitLayout` where
  the solve starts. It counts two degrees of freedom per fit point, which take constraints and
  dimensions like any point.
- A centripetal spline's knots depend on its fit points, which no equation differentiates, so
  `solve_with` solves in rounds: when the solved fit points give knots more than `KNOT_SETTLING`
  from the round's start (`respaced`), it solves again from the solved sketch with centripetal fit
  points held stiff (`spaced_fit_variables`, frozen first like dragged points), at most
  `SPACING_ROUNDS`, the last system finishing the solve. A point on the spline then lies on the
  drawn curve.
- A spline's ends (`SplineEnd`) take their legs from the handle (`end_legs`), so tangent or
  curvature at a fit-point spline's end uses its hidden control points and a conic's end curvature
  its weights (`end_factor` of the knots at that end, times `w_end w_after / w_next²`). Closed
  splines have no ends; their handles are `periodic`.
- Curvature at a clamped spline's end is `end_factor` times the cross product of its first two
  legs over the first leg's length cubed. `Curvature` with a line holds the first two legs
  parallel; with a circle or arc, `Form::EndCurvature` holds curvature times radius at the side the
  centre started on; between splines, `Form::MatchedCurvature` holds the two curvatures, each
  measured going into its spline, opposite.

## Curve parameters (splines and ellipses)

- Some constraints get a parameter of their own along a curve: an extra variable after the
  geometry's, counted in the degrees of freedom, never perturbed, kept in the `SolveMemo`
  (`System::parameters` lists each constraint's). A spline's is clamped (a point held beyond the end
  of a spline that cannot move is a conflict); a closed spline's and an ellipse's wrap
  (`System::wrapping_parameters`: taken modulo one turn after each step, never pushed against a
  bound), so a point slides across the seam. An ellipse's is its angle over the whole ellipse, as a
  point on an arc lies on its whole circle.
- It starts at the closest point (or the stationary point of the distance to the other curve),
  refined by Newton so geometry that already holds does not move; among starts equally near (within
  `TIE`) one with a non-vanishing first derivative wins, and where it vanishes a tangency uses the
  second derivative, never a fixed fallback. A step pushing a parameter already at an end of its
  range further out is retaken with it held.
- Forms read a `CurveHandle` (a spline's or an ellipse's) evaluated at its parameter (`CurveAt`:
  point, tangent, second derivative, gradients pushed into the handle), so the same forms serve
  both. Spline cases: a point on a spline, at a distance from it (`Form::SplineFoot` square to it
  there, `Form::SplineDistance` signed distance along its normal on the starting side), a tangent
  between a spline and a curve not sharing one of its end points, a distance from a spline to a
  line or circle (the tangency forms `Form::SplineOnLine` and `SplineOnCircle`, which carry a side
  from the start and the value, tangency being side 1 and value 0, with `SplineAlongLine` or
  `SplineAcrossRadius` keeping it square to the gap).
- Two curves sharing no point take a parameter on each (`pair::closest_pair` over `pair::Course`:
  nearest pair of samples refined by Newton on the squared distance; `System::pair_parameter_start`,
  in the constraint's order). Tangent: `Form::CurvesMeet` along x and y plus `Form::CurvesAlong`.
  Distance: `Form::CurvesFoot` (second curve's point square to the first's tangent),
  `Form::CurvesGap` (along the first's normal at the value, on the starting side) and `CurvesAlong`.
  Three equations for two parameters either way. Where they share a point a tangent needs no
  parameter: two ellipses through one point are `Form::EllipsesTouch` (normals parallel, via
  `ellipse_touch_along`), a spline ending on an ellipse `Form::EllipseTouch` with its first leg
  (`Joints::pair_joint`). `Sketch::ellipse_gap` and `spline_gap` give the measured points of such a
  pair from the same `closest_pair` (`curve_pair_gap`).
- Equal lengths with a spline (`Form::SameLength` of `LengthOf::Line` or `Spline`) differentiate
  the Gauss-Legendre sum of the speed, five nodes per knot span (`length_nodes`), as
  `BSpline::length` sums it.
- An angle to an arc is `Form::Angle` on the radius from the centre to the joint, turned a right
  angle (counter-clockwise at the arc's start, clockwise at its end) through the target value.

## Ellipses (`solve/ellipse.rs`)

- The minor radius is a variable beside the circles' radii (`System::radii`, fixed when projected)
  and the centre to axis point a span, so an ellipse counts five degrees of freedom and an
  elliptical arc seven: its start and end are held on it by two implicit `Form::OnEllipse`, which
  is `b/2 (x²/a² + y²/b² − 1)` in the ellipse's frame, equal to the distance for a circle.
  `MajorRadius` is a `PointDistance` and `MinorRadius` a `Radius` on the minor radius.
- A line: `Form::EllipseTangent` holds the signed distance from the centre to the line at
  `sqrt(a²(n·u)² + b²(n·v)²)` plus its value on the starting side, so a `Distance` from a line to
  an ellipse is that form with the gap as value (`System::ellipse_line_gap`), needing no parameter.
  Sharing a point, a tangency is `Form::EllipseTouch` (line along the ellipse's tangent there). A
  circle or arc tangent at a shared point is `Form::EllipseTouchCircle` (the touch with the radius
  turned a right angle, one equation for either side).
- Elsewhere (circle or arc tangent sharing no point, point or circle at a distance) it takes a
  parameter (`System::ellipse_parameter_start`, evaluated by `EllipseHandle::at`, gradients via
  `push_point` and `push_tangent`). A point's distance is `Form::EllipseFoot` and
  `Form::EllipseDistance` (even at zero); a circle's `Form::EllipseOnCircle` (the point at the
  radius plus the value from the centre, side 1 and value 0 for a tangency) and
  `Form::EllipseAcrossRadius`. A point's parameter starts at `EllipseGeometry::closest_parameter`,
  a circle's at the stationary point of the distance from its centre whose gap to the circle is
  smallest (`touching_parameter`, slope sign changes over `STATIONARY_SAMPLES`, bisected).
- An angle between a line and an elliptical arc sharing an end is `Form::NormalAngle`: from the
  line to the ellipse's outward normal at the joint, turned a right angle as an arc's radius is
  (`Joints::arc_joint` takes elliptical arcs), so a chamfer by distance and angle holds on one.
- `Form::OnMinorAxis` (`OnMinorAxis { point, ellipse }`): the point's offset from the centre along
  the major axis is zero, so with a `Coincident` it holds the point at a minor axis end however the
  axis is turned.
- `Equal` between ellipses is `EqualLength` of the centre-to-axis spans and `EqualRadius` of the
  minor radii (`EllipseHandle::minor_circle`), only the latter when both share centre and axis
  point, so a split elliptical arc's pieces get no zero row.
- `Midpoint` on an elliptical arc is `Form::OnEllipse` and `Form::EllipseMiddle`: the major radius
  times the wrapped difference between the point's parameter (`atan2(y/b, x/a)`) and the start's
  plus half the counter-clockwise sweep, a single branch since the opposite point is half a turn
  away.

## Point targets (`solve/target.rs`)

- A target is a `TargetHandle`: a weighted sum of point handles (a point, a line's middle, the
  centre of corners and sides, with a hidden point per arc), a circle's centre plus its radius along a fixed direction (`Rim`, a
  quadrant, the radius through `push_radius`) or an ellipse turned to a fixed cosine and sine
  (`Turned`, an axis end, through `EllipseHandle::turned` and `push_point`).
- A crossing and the middle of an arc or elliptical arc have no closed form, so the constraint gets
  a hidden point of its own (`System::targets`: two variables allocated after the curve
  parameters, `Variable::Target` in the memo numbered on per hidden point, perturbed like geometry, owned by no entity) held by
  the forms defining it, owned by the constraint: on each curve (`Form::OnLine`, the circle form,
  `OnEllipse`, or `OnSpline` along x and y at a curve parameter of the constraint started at the
  closest point, `target_parameter_start`), or the forms `Midpoint` uses (`middle_forms`, shared).
  It starts at `Sketch::target_position`, so a solve keeps the crossing the drawing chose; two
  variables and two equations, it adds no freedom while the curves cross at an angle.
- `HorizontalPoints` and `VerticalPoints` on a sketch point stay `Form::SameY` and `SameX`; on any
  other target they are `Form::Level` (`along · (point − target)`). `OnDirection` is
  `Form::OnDirection`: the offset from the target dotted with the line's unit direction (square)
  or its normal, the line's turn reaching its ends through `Direction::back_from_unit`. `Centroid`
  is `Form::Through` along x and y over its members at equal weights (`centroid_terms`): a point
  itself, a line as half of each end, an arc as a hidden point held at its middle by
  `middle_forms`; a tracked `PointTarget::Centroid` sums the same terms. Hidden points are per
  constraint (`System::targets`, one per arc member or one for a crossing or arc middle,
  `hidden_target_starts`), each two variables and two equations. Their rows belong to their
  constraint, so degrees of freedom, redundancy and conflict diagnosis need nothing of their own.

## Constraint state

- Degrees of freedom and each entity's state come from the rank and null space of the Jacobian at
  the solution. A constraint adding no rank over older ones is redundant, naming what it duplicates.
- Above `DENSE_LIMIT` the analysis (`sparse::Triangular`) merges the normalised rows, in
  constraint order, into an upper-triangular basis by Givens rotations, columns in minimum-degree
  order, so fill stays within the symbolic Cholesky structure and a closed chain analyses in
  near-linear work. A row adds rank when an entry above `RANK_TOLERANCE` reaches a column with no
  basis row. A variable is fixed when its null-space row (each free column set to one,
  back-substituted) stays within `sqrt(NULL_SPACE_TOLERANCE)`; a part with no free column is fixed
  throughout, a column reaching no free column fixed outright, the rest take one back-substitution
  per free column or one transposed solve per column, whichever is fewer. Dense and sparse analysis
  agree on the same parts, closed chains included.

## Conflict diagnosis (`diagnosis.rs`)

- A conflict is named only on evidence that its constraints cannot hold together, never because one
  solve from one start failed: constraints the solver merely cannot reach together from the drawn
  shape (a line that would have to fold back) are solved from where diagnosis reached, or
  `SketchError::Unsolvable` when it reached nowhere. The evidence is numerical, not a proof.
- The first candidate is the support of the failed solve's least-squares point (skipped when
  pressed against a collapse), probed from the drawn shape so that probe is also the verdict's
  descent from the drawing; when it holds, or there is none, QuickXplain-style divide and conquer
  over the part's constraints holding the newest one looks for one. Other probes solve the
  sub-parts from the closest known state; all remember each outcome by its equations and share
  one budget, `DIAGNOSIS_WORK`.
- A probe's failure only steers the search. The set found is trimmed, oldest first, of every
  constraint without which it still fails; each kept constraint comes with a witness, a solution of
  the set without it. A descent is conclusive when its unperturbed attempt ends at a least-squares
  minimum clearly above tolerance, or pressed against a collapse, and no retry converges. The set
  cannot hold when its own probe and every descent from the drawn shape (also with spline
  parameters mid-range and held) and from the closest witnesses are conclusive; any descent that
  converges means it holds (remembered, the search reruns); otherwise inconclusive.
- Trimming tries a step before a descent (`witness.rs`): the failing part is linearised and
  factored once at its probe's least-squares end (`sparse::Triangular` over the normalised rows,
  semi-normal equations, `Triangular::normal_solution`). A constraint's witness is that point moved
  through the pseudo-inverse columns of its own rows, weighted so the linearisation of the rest
  holds and otherwise shortest; chord steps with fresh residuals follow (at most `CHORD_STEPS`,
  each halving what the rest leaves) until every part of the rest holds within its tolerance. A
  constraint whose step does not get there falls back to a descent, and dropping one factors the
  smaller set again. The factorisation costs one step and each solve through it
  `1/SOLVES_PER_STEP`; a solve is as long as the part, so stepping every constraint costs work
  quadratic in the conflict's size.
- So a witness is first moved from the last one (`Factored::transferred`): the strain the previous
  witness left on its own constraint is shifted onto the next, solving only the variables of the
  two constraints (then 1, 2, 4 and 8 hops further through the rest's equations, up to a quarter
  of the part) and the rest's equations touching them by Gauss-Newton (at most `TRANSFER_STEPS`,
  each halving what they leave), everything else held where it was. A witness serves as the start
  (`Strained`) only when every equation of the part but its constraint's holds as a part of its
  own (the smallest scale, so the strictest tolerance) and the part's spans are clear of a
  collapse at the part's own scale (the largest, so the strictest), so the moved witness need only
  check what it moved. A transfer may spend no more than the step it replaces (its constraint's
  rows times a solve), else the step is taken. Along a chain each transfer moves one joint: a
  300-line chain trims in about 3,600 units against 27,000 by steps (a normal-suite test bounds it
  at 20 per line, and checks every witness holds the rest), 1,000 lines in about 12,000 against
  285,000.
- The verdict's descents settle in a few Newton steps: from the drawing of a 300-line chain about
  46,000 units for all three attempts, against 192,000 by line search alone. A 300-line chain is
  diagnosed in about 103,000 units (the support's descent from the drawing 46,000, trimming 3,600,
  the closest witness's descent 53,000), a 1,000-line chain in about 456,000, so both are named
  within the budget (ignored stress tests in `diagnosis_tests.rs`, run by the nightly job).
- Only a set judged unable to hold is reported, minimal by its witnesses; recompute reports it as
  the feature's error with `FeatureError.constraints` and `FixTarget::Constraint`.
- Every failed part is diagnosed on its own budget, newest first, up to `DIAGNOSED_PARTS` (the rest
  are `Unsolvable`). Two or more reports are `SketchError::Several`; recompute merges them into one
  `FeatureError`.
- When the search ends with no such set (or the budget is spent) and the part holds after all (a
  probe of all its equations, else one more descent of the whole part from the closest known state,
  converges), that solution is the part's (`diagnosis::Found`): the solve succeeds when every
  failed part found one, and the memo keeps it under the drawn start so the next solve starts there.
  A part that found one is never reported.
- A failure with neither is `SketchError::Unsolvable` with the part's curves, free points and
  newest constraint; recompute words it as that geometry not solving from its current shape.

## SolveMemo (`memo.rs`)

- `Sketch::solve_from` takes the previous solve's `SolveMemo` (recompute passes the feature's last
  good `SketchResult::memo`). Each part is keyed by its entities, constraints, dimension values,
  starting values and own scale, and remembered under both starting and solved values, so an
  untouched part starts from its old solution and reuses its rank analysis once its solved values
  match exactly, however the rest of the sketch grew or moved. The memo also holds the sketch's
  fixed values (projected geometry) once and is recalled only under the same ones.
- An outcome lines up with its key's variables in order, so recalling maps nothing by name; a part
  whose equations or variables differ from its key's is analysed afresh.
