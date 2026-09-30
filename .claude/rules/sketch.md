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

## Trim and extend

- `trim.rs` on top of `intersect.rs` (lines against segments, circles and arcs analytically, against
  splines by sampling and bisection). Cutters and targets are every other curve of the sketch,
  construction curves and splines included; the reference axes and lone points are not. A crossing
  at a curve's own end (within `1e-7` of the curves' extent) is a joint, not a cut.
- `trim_pieces` splits a line, circle or arc at its crossings (a circle needs two); `trim_piece`
  finds the one nearest a point, `trim` removes it on a copy and keeps the result only if every
  step succeeded. Splines are refused in words (`TrimError::Spline`).
- The kept part keeps the curve's ID, its surviving end points and every constraint still true of
  it: a changed curve is removed and inserted again with the same ID (a circle becomes an arc), and
  its constraints return with their IDs; `Midpoint` and `Equal` on a shortened line are dropped.
  A piece split off gets fresh IDs from the counter and the curve's construction flag; a line piece
  is `Collinear` with the kept part, or, when the line was horizontal or vertical, takes that
  constraint too and puts its new end on the kept line; an arc piece gets its own centre,
  `Concentric` and `Equal` with the kept arc. Tangent, parallel, perpendicular and angle
  constraints to a curve joined only at the far end move to the piece holding that end.
- A new end joins its cutter: `Coincident` with the cutter's end point when the cut lies there
  (replacing that point's now implied place on the trimmed curve), else a point on the cutter.
  An end point no curve uses any more is removed with its constraints, as are a deleted curve's.
- `extension` picks the end nearer the given point and the nearest crossing beyond it, along the
  line or round the arc's circle (never past its own start); `extend` moves that end point there,
  drops its constraints (and `Midpoint` and `Equal` on a line) and joins it to the target the same
  way. An end shared with another curve, coincident with another point or fixed is refused
  (`ExtendError::Joined`, `Fixed`), as are circles, splines and ends with nothing beyond them.

## Offset

- `offset.rs`. `offset_chain` orders the chosen lines and arcs into one chain, open or closed, by
  their ends meeting within `1e-7` of their extent (a lone circle is a closed chain); points and
  the reference geometry are ignored, splines refused, and branches or separate chains refused in
  words. The chain is walked from a free end (or its lowest curve), so the same curves give the
  same order and `Side` (left or right of that walk) means the same thing on the working copy.
- `Chain::outline` offsets each curve (an arc or circle refused if its radius would reach nothing)
  and joins them: where the original is smooth the offsets meet; line–line corners meet sharp,
  extended or trimmed to where the offset lines cross; a convex corner involving an arc gets a
  round arc about the original corner; a concave one is trimmed to where the offset carriers cross
  nearest the corner, refused when they miss. A curve used up by trimming, or offsets of curves
  not next to each other crossing, is refused.
- `offset` adds the outline in one pass on a copy: separate end points joined by `Coincident`,
  offset arcs and circles on their original's centre point, round corners on the original corner
  point. The first curve and every curve after a sharp corner hold the distance with the typed
  expression: `Distance` between the lines, from the arc's start to the original arc, or, for a
  circle, from a point on it held level with its centre (`HorizontalPoints`) to the original.
  After a smooth joint or a round corner the distance follows from the `Tangent` there, so a line
  only gets `Parallel` and an arc nothing, and concentric arcs meeting smoothly get no tangent.
  The closing `Coincident` of a closed chain is added last, so the one relation a closed loop
  repeats is part of it and never a whole redundant constraint.

## Mirror

- `mirror.rs`. `mirror` copies the chosen points and curves about a sketch line or an axis:
  points on the mirror line are shared (and held there by `Coincident` unless already an end of
  the line, on it, at its end or the origin of an axis, or its midpoint), others copied with
  `Symmetric` to the original; arcs swap their ends to stay counter-clockwise, circles add `Equal`
  for the radius, and a curve that is its own image is left out. No other constraint is copied,
  since symmetry holds the copy. `mirror_image` gives the same result as polylines for a preview.

## Sketch fillet

- `fillet.rs`. A `Corner` is where exactly two lines or arcs end (shared point or ends within
  `1e-7`), kept by one of its points; `corner_at`, `corner_between` and `fillet_corners` find
  them, and one curve, three, a spline or a smooth meeting is refused in words.
- `rounding` puts the arc's centre where the two curves' carriers offset by the radius toward the
  inside of the corner cross, nearest the corner, and refuses a radius whose touching point would
  not lie on a curve short of its far end (`TooLarge`, naming the curve). `radius_through` gives
  the radius of the fillet passing through a point on the corner's bisector at its distance.
- `fillet` moves each curve's corner end to its touching point (restructured like Trim, dropping
  `Equal` and `Midpoint` of a shortened line), adds the arc with its own end points joined by
  `Coincident`, `Tangent` to both and a `Radius` dimension, and keeps the corner point as a sharp
  held on both carriers by `Coincident`, so dimensions, fixes and symmetry on the corner still
  hold; the other corner point, if separate, is merged into it.

## Faceting

- Curves are drawn as polylines within a chord tolerance (`Faceting`, `curve.rs`): arcs and circles
  take the fewest equal steps whose sagitta stays within it, at least 12 and at most 1024 a turn;
  splines take equal parameter steps from a bound on their second derivative (the convex hull of
  its B-spline control points, a chord deviating at most h²·max|C''|/8), at least one per span and
  enough for 12 a turn of the control polygon's turning, at most 4096. `Faceting::within` takes
  any value (NaN or below zero as the finest), and every count is at least one.
- `Sketch::faceted` and `facet_segments` give an entity's polyline and its count; trim pieces and
  extensions are faceted the same way (`Piece::faceted`, `Extension::faceted`). `polyline` with a
  largest step angle remains for sampling that is not drawn.

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
