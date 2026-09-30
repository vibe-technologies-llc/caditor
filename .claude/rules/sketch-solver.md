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
- An attempt gives up once three steps in a row lower the squared residuals by under a thousandth
  (stalled at a least-squares minimum). Retries perturb each part by a fraction of its own extent.
  Every equation has an analytic gradient.
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
  spline's range in the line search (a point beyond its end is a conflict), kept in the
  `SolveMemo`.
- It starts at the closest point, or at the stationary point of the distance to the other curve
  nearest touching, refined by Newton so geometry that already holds does not move.

## Constraint state

- Degrees of freedom and each entity's state come from the rank and null space of the Jacobian at
  the solution. A constraint whose equations add no rank over older ones is redundant, naming what
  it duplicates.

## Conflict diagnosis

- When a part does not converge, QuickXplain-style divide and conquer over the failed part's
  constraints holding the newest one (newest preferred) looks for a conflict. A probe solves the
  sub-parts its constraints form from the starting shape; each sub-part's outcome is remembered by
  its equations, so none is solved twice; all probes share one budget, `DIAGNOSIS_WORK`
  (Gauss–Newton steps weighted by their equations).
- The set found must fail on its own and is trimmed, oldest first, of every constraint without
  which it still fails, so it is minimal. Recompute reports it as the feature's error with
  `FeatureError.constraints` and `FixTarget::Constraint`.
- A failure with no such set (budget spent, a set that solves alone, nothing to blame) is
  `SketchError::Unsolvable` with the part's curves and free points and its newest constraint;
  recompute words it as that geometry not solving from its current shape, naming at most three, and
  points at the newest constraint (or the sketch when there is none).

## SolveMemo

- `Sketch::solve_from` takes the previous solve's `SolveMemo` (recompute passes the feature's last
  good `SketchResult::memo`). Each part is keyed by its entities, constraints, dimension values,
  starting values and own scale, and remembered under both starting and solved values, so an
  untouched part starts from its old solution and reuses its rank analysis once its solved values
  match exactly, however the rest of the sketch grew or moved.
