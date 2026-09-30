---
paths:
  - "crates/caditor-sketch/**"
---

# Sketches

## Entities

- A sketch lies on a `Plane`. Entities are points, lines, circles (centre point and radius), arcs
  (centre, start and end points, counter-clockwise) and clamped B-splines through control points.
- Every sketch has a fixed origin and two axes under reserved IDs (`EntityId::ORIGIN`,
  `HORIZONTAL_AXIS`, `VERTICAL_AXIS`) the ID counter never reaches. Stored IDs stay below 2^63.
- Any curve can be construction geometry (`Sketch::set_construction`); never a point or the
  reference geometry.
  - It is a set kept beside the entities, forgotten when the curve is removed, and part of
    `same_geometry`.
  - It solves, snaps and takes constraints like any other curve, but `solid::profile_curves` leaves
    it out, so a centreline neither splits regions nor changes their keys.
  - It can still be a revolve axis.
- `insert_entity` and `insert_constraint` take explicit IDs and check references, for loading.
- The sketch counts each entity's uses by curves and constraints, so refusing to remove a used one
  never scans the sketch and undoing a large import stays fast. `remove_entity` removes a whole
  cascade in one pass, updating the counts incrementally.

## Constraints

- Constraints have stable `ConstraintId`s. The kinds:
  - coincident (point–point or point on a curve, splines included);
  - horizontal and vertical (a line, or two points as `HorizontalPoints` and `VerticalPoints`);
  - parallel, perpendicular and equal;
  - tangent (a spline with a line, circle or arc too);
  - midpoint (a point halfway along a line);
  - concentric (two circles or arcs, or a point at one's centre);
  - collinear;
  - symmetric (two points about a line or a point);
  - fix (a point held at a stored position);
  - dimensions, whose values are expressions: distance, horizontal and vertical distance, angle,
    radius and diameter.
- Distance is between points, a point and a line or circle, or two lines. Between two lines it
  holds both ends of the second at the distance from the first, so it implies parallel.
- An angle measures from its first line's direction (or its reverse when `reversed`, which the UI
  sets so a corner of a chain is measured inside it) to its second's.
- `check_constraint` refuses constraints that do not fit the entity kinds; the UI asks before
  offering one.
- `Sketch::measured` gives a dimension's drawn value: new dimensions start from it and unreadable
  stored ones fall back to it.

## Splines

- Sketch splines are clamped with uniform knots and degree min(3, points − 1).
- `BSpline::fit` approximates a dense polyline by one: chord-length parameters corrected by
  projection, banded least squares with fixed ends, doubling the control points until within a
  tolerance, else the best found.
- `BSpline::interpolate` passes one through given points at evenly spaced parameters.
- `BSpline::through` follows unevenly spaced points without loops: a chord-length interpolation
  with averaged knots, sampled and fitted, passing once through points repeated within a billionth
  of the polyline's length.
- Knot spans are found by binary search and every system is solved by banded elimination
  (`banded.rs`).
- Splines evaluate their basis and its first two derivatives exactly (`curve.rs`).
