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
- Projected geometry (`Sketch::set_projected`, a set beside the entities, part of
  `same_geometry`) is a curve and its points, or a lone point, whose position the document
  supplies. The solver holds it fixed like the origin: its points are `PointHandle::Fixed` and a
  projected circle's radius `RadiusHandle::Fixed`, so it adds no freedom and reads as fully
  constrained, and every fixed value is part of each `SolveMemo` key. A constraint on projected or
  reference geometry alone is `OnlyReference`. Trim, extend and the sketch fillet refuse a
  projected curve (`Projected`); it still cuts, offsets, mirrors and takes constraints. It is
  ordinary profile geometry unless made construction.
- `Sketch::free_points` lists the points no curve uses (a constraint using one does not count), in one
  pass; the hole feature drills at them.
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
  sketch itself still accepts both, since stored files may hold them.
- A dimension is a magnitude: distances are never negative, the side coming from the drawn
  geometry. `add_constraint` refuses a literal value that could never hold
  (`SketchError::DimensionValue`, so it fails when typed, not at solve); loading and undo go through
  `insert_constraint` and stay lenient, and a parameter-driven value that turns negative fails that
  dimension at solve with its own message.
- `Sketch::measured` gives a dimension's drawn value: new dimensions start from it and unreadable
  stored ones fall back to it.
- `Midpoint { point, curve }` takes a line or an arc, never a circle. On an arc it is two
  single-branch equations (`Form::OnBisector`, the point on the chord's perpendicular bisector, and
  `Form::ArcBulge`, its signed distance from the centre across the chord equal to the radius on the
  side a counter-clockwise arc bulges), so a solve never lands on the opposite side of the circle.

## Editing operations

Trim, extend, offset, mirror and fillet work on a copy and replace the sketch only if every step
succeeded. A changed curve is removed and inserted again under the same ID (`restructure`), with
every constraint still true of it. Joints are judged by a `TOLERANCE` relative to the extent.

- Trim and extend (`trim.rs`): cutters and targets are every other curve, construction curves and
  splines included; reference axes and lone points are not. A crossing at a curve's own end is a
  joint, not a cut. Splines cannot themselves be trimmed or extended.
  - `Midpoint` and `Equal` on a shortened line, and `Midpoint`, `ArcLength` and `Sweep` on a
    shortened or extended arc (`keeps_sweep`; the sketch fillet does the same), are dropped, as is any distance dimension between
    its two old ends or points joined to them by `Coincident` (one end often outlives the trim,
    shared with another curve, and would keep measuring to the far end).
  - A split-off piece gets fresh IDs and the construction flag. A line piece is `Collinear` with
    the kept part (or keeps horizontal or vertical and a new end on the kept line); an arc piece
    gets its own centre, `Concentric` and `Equal`. Tangent, parallel, perpendicular and angle
    constraints to a curve joined only at the far end move to the piece holding that end.
  - A new end joins its cutter: `Coincident` with the cutter's end point when the cut lies there,
    else a point on the cutter. End points no curve uses any more are removed.
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
- Fillet (`fillet.rs`): a `Corner` is where exactly two lines or arcs end, kept by one of its
  points. `rounding` refuses a radius whose touching point would not lie on a curve short of its far
  end (`TooLarge`). `fillet` adds the arc `Tangent` to both with a `Radius` dimension and keeps the
  corner point as a sharp held on both carriers by `Coincident`, so dimensions, fixes and symmetry
  on the corner still hold.

## Faceting and splines (`curve.rs`, `fit.rs`)

- Curves are drawn as polylines within a chord tolerance (`Faceting`, bounded counts per turn and
  `MAX_SEGMENTS`); `Faceting::within` accepts any value (NaN or below zero as the finest). Trim
  pieces and extensions are faceted the same way.
- Sketch splines are clamped, uniform knots, degree `min(MAX_SPLINE_DEGREE, points - 1)`, exact
  derivatives, banded elimination (`banded.rs`). `BSpline::fit` approximates a dense polyline
  within a tolerance, `interpolate` passes through points at evenly spaced parameters, `through`
  follows unevenly spaced points without loops.
