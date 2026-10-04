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
  `DENSE_LIMIT` variables, above that CGLS on the sparse Jacobian with sparse elimination for the
  analysis (`sparse.rs`). Every equation has an analytic gradient.
- Convergence is checked after every step and cancellation before it, so the diagnosis budget
  counts steps taken. Tests bound cost with `solve/tally.rs` work units, never wall-clock time.
- An attempt ends converged, at a least-squares minimum, pressed against a collapse (such a minimum
  with a moving line or arc span, or a radius, near the collapse length) or unfinished. Retries
  perturb each part by a fraction of its own extent.
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

## Spline parameters (`solve/spline.rs`)

- A point on a spline, and a tangent between a spline and a curve not sharing one of its end
  points, get a parameter of their own along the spline: an extra variable after the geometry's,
  counted in the degrees of freedom, never perturbed, clamped to the spline's range (a point held
  beyond the end of a spline that cannot move is a conflict), kept in the `SolveMemo`.
- It starts at the closest point (or the stationary point of the distance to the other curve),
  refined by Newton so geometry that already holds does not move; among starts equally near (within
  `TIE`) one with a non-vanishing first derivative wins, and where it vanishes a tangency uses the
  second derivative, never a fixed fallback that could line up by chance.
- A step pushing a parameter already at an end of its range further out is retaken with it held, so
  the rest moves instead of the step being spent on the clamp.

## Constraint state

- Degrees of freedom and each entity's state come from the rank and null space of the Jacobian at
  the solution. A constraint whose equations add no rank over older ones is redundant, naming what
  it duplicates.

## Conflict diagnosis (`diagnosis.rs`)

- A conflict is named only on evidence that its constraints cannot hold together, never because one
  solve from one start failed: constraints the solver merely cannot reach together from the drawn
  shape (a line that would have to fold back) are `SketchError::Unsolvable`. The evidence is
  numerical, not a proof.
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
- Only a set judged unable to hold is reported, minimal by its witnesses; recompute reports it as
  the feature's error with `FeatureError.constraints` and `FixTarget::Constraint`.
- Every failed part is diagnosed on its own budget, newest first, up to `DIAGNOSED_PARTS` (the rest
  are `Unsolvable`). Two or more are `SketchError::Several`, so the user meets every problem at
  once; recompute merges them into one `FeatureError`.
- A failure with no such set (budget spent, the part holding after all from a warm start,
  inconclusive evidence) is `SketchError::Unsolvable` with the part's curves, free points and newest
  constraint; recompute words it as that geometry not solving from its current shape.

## SolveMemo (`memo.rs`)

- `Sketch::solve_from` takes the previous solve's `SolveMemo` (recompute passes the feature's last
  good `SketchResult::memo`). Each part is keyed by its entities, constraints, dimension values,
  starting values and own scale, and remembered under both starting and solved values, so an
  untouched part starts from its old solution and reuses its rank analysis once its solved values
  match exactly, however the rest of the sketch grew or moved.
