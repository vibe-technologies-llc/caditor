---
paths:
  - "crates/caditor-sketch/src/solve/**"
  - "crates/caditor-sketch/src/sketch.rs"
  - "crates/caditor/src/drag_solver.rs"
---

# Sketch solver

## Solving

- `solve` evaluates dimensions first; lengths are at most `MAX_LENGTH` (a kilometre), a huge value
  refused in words rather than overflowing the solver's scale.
- Then damped Gauss–Newton with minimal-norm steps on each independent part: SVD from `nalgebra`
  up to 48 variables; above that CGLS from zero on the sparse Jacobian (same minimal-norm step),
  with sparse forward elimination for the analysis (`sparse.rs`).
- Convergence is checked after every step and cancellation before it, so the diagnosis budget
  counts steps taken. Tests bound the solver's cost with `solve/tally.rs`, which counts the
  elimination, CGLS, SVD and linearisation work of a solve in test builds (a no-op otherwise), never
  with wall-clock time. An attempt ends converged, at a least-squares minimum (three steps in a row
  lower the squared residuals by under a thousandth, or no admissible lower point lies along the
  step), pressed against a collapse (such a minimum with a moving line or arc span, or a radius,
  within sixteen times the collapse length), or unfinished (out of steps while still descending, or
  no finite step). Retries perturb each part by a fraction of its own extent. Every equation has an
  analytic gradient.
- A solve that would collapse a line or an arc's radius to nothing is not converged, reported as a
  conflict; if the line or arc already had no length, `SketchError::NoLength` naming it.

## Scale

- Each part solves at its own scale: largest starting coordinate or length dimension, at least 1,
  rounded up to a power of two. It sets the convergence tolerance, step limit and the lengths below
  which a direction or line is degenerate, so no part depends on another. A zero distance is
  recognised against the scale of its two points.

## Dragging

- `solve_dragging` takes `Drag`s (a point or a circle's radius with its target); it starts from
  the dragged values at their targets and first holds them there while the rest solves. When that
  cannot work they only weigh a hundred times more than free geometry, ending as near their targets
  as the constraints allow. Geometry that already satisfies its constraints does not move;
  under-constrained geometry moves as little as possible.
- The app's `drag_solver.rs` runs this on a worker thread of its own.

## Equation forms

- Two-branch equations take their branch from the starting geometry, so a solve never flips:
  tangent side, signed distance, horizontal and vertical distance, the side of a circle a point
  keeps its distance on, the side of the first line the second keeps its spacing on. Internal
  circle tangency instead follows whichever circle is currently larger.
- A tangent whose curves share a point (directly or through point–point coincidences) is the radius
  there perpendicular to the line (or both radii along one line), keeping full rank where the
  distance form has none. A zero distance between points is solved as a coincidence.
- A tangent at a spline end joined to the other curve is the end's control leg along the line (or
  across the radius), like the line–arc joint.

## Spline parameters

- A point on a spline, and a tangent between a spline and a line, circle or arc not sharing one of
  its end points, get a parameter of their own along the spline (`solve/spline.rs`): an extra
  variable after the geometry's, counted in the degrees of freedom, never perturbed, clamped to the
  spline's range in the line search (a point held beyond the end of a spline that cannot move is a
  conflict), kept in the `SolveMemo`.
- It starts at the closest point, or at the stationary point of the distance to the other curve
  nearest touching, refined by Newton so geometry that already holds does not move. Among starts
  equally near (within 1e-9 of the control polygon's size, as along a stretch lying on the other
  curve), one where the spline's first derivative does not vanish wins.
- Where the first derivative vanishes (a cusp, or repeated control points), a tangency measures the
  spline's direction by its second derivative, the limit of its tangent line there, never by a fixed
  fallback that could line up with the other curve by chance.
- A step that would push a parameter already at an end of its range further out is taken again
  with that parameter held, so the rest moves instead of the step being spent on the clamp: a point
  held beyond the end of a free spline pulls the end along.

## Constraint state

- Degrees of freedom and each entity's state come from the rank and null space of the Jacobian at
  the solution. A constraint whose equations add no rank over older ones is redundant, naming what
  it duplicates.

## Conflict diagnosis

- A conflict is only named on evidence that its constraints cannot hold together, never because one
  solve from one start failed: constraints the solver merely cannot reach together from the drawn
  shape (a line that would have to fold back, a chain that would have to curl up) are
  `SketchError::Unsolvable`. The evidence is numerical, not a proof; its rules follow.
- When a part does not converge (`diagnosis.rs`), the first candidate is the support of its failed
  solve's least-squares point: the constraints owning equations whose residual one more
  (bound-aware) Gauss–Newton step would not remove, clearly above tolerance (a hundred times it)
  and, measured along each equation's own gradient, at least a thousandth of the largest. It is
  skipped when that point is pressed against a collapse. When the candidate holds, or there is none,
  QuickXplain-style divide and conquer over the part's constraints holding the newest one (newest
  preferred) looks for one.
- A probe solves the sub-parts its constraints form, each from the closest known state: the latest
  solution of a sub-part that held, the latest least-squares point of one that failed (never one
  pressed against a collapse) or the drawn shape, whichever leaves the smallest residual. Each
  sub-part's outcome and end state are remembered by its equations, so none is solved twice and
  each later probe starts from them. All probes share one budget, `DIAGNOSIS_WORK` (Gauss–Newton
  steps weighted by their equations); a whole part of about five hundred entities in conflict fits.
- A probe's failure only steers the search. The set found is trimmed, oldest first, of every
  constraint without which it still fails; each constraint kept comes with a witness, a solution of
  the set without it. Then the set is judged. A descent is conclusive when its unperturbed attempt
  ends at a least-squares minimum with a residual clearly above tolerance, or pressed against a
  collapse, and no retry converges.
  - It cannot hold together when its own probe was conclusive and so is every descent of it from
    the drawn shape (unless the probe started there), and from the drawn shape with every spline
    parameter set mid-range and held while the geometry settles, then released. The descent from the witness satisfying it most
    closely, and from every other witness satisfying it more closely than the lowest minimum
    reached so far, must fail conclusively or end no lower than that minimum.
  - It holds when any of these descents converges: that is remembered and the search runs again.
  - Otherwise the evidence is inconclusive.
- Only a set judged unable to hold is reported, and its witnesses make it minimal. Recompute
  reports it as the feature's error with `FeatureError.constraints` and `FixTarget::Constraint`.
- Every failed part is diagnosed on its own budget, newest part first, up to `DIAGNOSED_PARTS`
  (the rest are reported undiagnosed as `Unsolvable`). One failing part is reported as it is; two
  or more as `SketchError::Several` holding each part's `Conflict` or `Unsolvable`, so the user
  meets every problem at once. Recompute merges them into one `FeatureError` numbering each
  reason and remedy, with the union of the conflicting constraints (all highlighted) and the
  newest part's fix.
- A failure with no such set (budget spent, the part holding after all from a warm start,
  inconclusive evidence, nothing to blame) is `SketchError::Unsolvable` with the part's curves and
  free points and its newest constraint; recompute words it as that geometry not solving from its
  current shape, naming at most three, and points at the newest constraint (or the sketch when
  there is none).

## SolveMemo

- `Sketch::solve_from` takes the previous solve's `SolveMemo` (recompute passes the feature's last
  good `SketchResult::memo`). Each part is keyed by its entities, constraints, dimension values,
  starting values and own scale, and remembered under both starting and solved values, so an
  untouched part starts from its old solution and reuses its rank analysis once its solved values
  match exactly, however the rest of the sketch grew or moved.
