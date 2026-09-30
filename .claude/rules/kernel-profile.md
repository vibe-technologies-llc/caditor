---
paths:
  - "crates/caditor-kernel/src/profile/**"
  - "crates/caditor-kernel/src/build/**"
---

# Profiles (`profile/`)

- `Profile::new` takes `ProfileCurve`s (lines, circles, counter-clockwise arcs, clamped B-splines
  with an explicit knot vector), each tagged with the sketch entity id as a plain u64, and builds
  the planar arrangement.
- Tolerance is 1e-7 of the profile size (at least `LINEAR_RESOLUTION`). Candidate curve pairs,
  curves under endpoints and faces around nested components come from a box tree.

## Arrangement

- Intersections: analytic for lines and circles; for splines, subdivision on monotone spans plus
  damped Newton from every leaf (self-crossings included, crossings merged only within tolerance).
  - Pairs are pruned by their boxes before any budget is spent; running out of budget is its own
    `TooIntricate` error.
- Endpoints landing on curves are found; nearby points cluster into vertices; overlapping collinear
  or co-circular pieces merge (lowest entity id kept); dangling pieces and bridges are pruned.
- Faces are traced by angle at each vertex; ties between tangent curves are decided by the position
  a short way along.

## Regions

- A `Region` has a CCW outer `ProfileLoop` and CW holes of `Piece`s (entity, 2D curve, parameter
  range, reversed), with the region on the left of every piece.
- A `PieceId` is the entity plus what bounds each end: its own start or end, or the sorted ids of
  the curves that cut it there with an occurrence counted along the curve.
- A `RegionKey` digests the set of (entity, side) pairs of its boundary. Regions sharing a key are
  told apart by their piece ids.
  - Whether a key is tie-broken is decided once over the whole arrangement (any two regions sharing
    it), so a region keeps its key whatever else is selected with it.
- Depth counts nesting: a face lies one deeper than the single face its whole boundary borders (a
  hole touching its outline, a circle tangent inside another), else than the face its connected
  component lies in, so the cells of a grid keep their outline's depth.
- `select` with `Selection::EvenDepth` (the default) or explicit keys returns the union of the
  chosen regions as new regions keyed the same way, so adjacent regions sweep as one lump.
- `Region::triangulate` samples the loops and keeps the constrained Delaunay triangles inside by the
  parity of constraint crossings, for drawing regions as fills.

## Errors

- Errors name the entity ids.
- An arrangement that fails without knowing which curves caused it is rebuilt from fewer curves
  (`culprits.rs`, at most 64 attempts) until the failing set is small enough to name.

## Use by builders

- The document converts a solved sketch to `ProfileCurve`s, keeps the chosen `RegionKey`s in the
  feature and calls `extrude`/`revolve` with the feature id (`kernel-operations.md`).
