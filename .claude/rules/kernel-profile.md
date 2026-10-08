---
paths:
  - "crates/caditor-kernel/src/profile/**"
  - "crates/caditor-kernel/src/build/**"
---

# Profiles (`profile/`)

- `Profile::new` takes `ProfileCurve`s (a line, circle, counter-clockwise arc or clamped B-spline),
  each tagged with the sketch entity id as a plain u64, and builds the planar arrangement. The
  tolerance is `RELATIVE_TOLERANCE` of the profile size, at least `LINEAR_RESOLUTION`.
- The document converts a solved sketch to `ProfileCurve`s, keeps the chosen `RegionReference`s in
  the feature, resolves them and calls `extrude`/`revolve` with the feature id
  (`kernel-operations.md`).

## Arrangement

- Spline intersections (self-crossings included) are subdivision plus damped Newton; pairs are
  pruned by their boxes before any budget is spent, and running out is `TooIntricate`.
- Overlapping collinear or co-circular pieces merge (lowest entity id kept); dangling pieces and
  bridges are pruned. When pruning removed anything, the arrangement is built once more without
  the cuts where only the cut curve itself survives (a stray line touching an outline, the foot of
  a bridge), and cutter sets list only curves with pieces surviving at the vertex, so stray
  geometry never adds a vertex to a side or renames it.
- An arrangement that fails without knowing which curves caused it is rebuilt from fewer curves
  (`culprits.rs`, within `PROBE_BUDGET` attempts) until the failing set is small enough to name
  (`NAMED_AT_MOST`).
- Errors name entity ids. `NoClosedProfile` carries the `OpenEnd`s (the curve, the point, the
  nearest end of another curve with the gap), capped at `MAX_REPORTED_OPEN_ENDS`.

## Regions

- A `Region` has a CCW outer `ProfileLoop` and CW holes of `Piece`s (entity, 2D curve, parameter
  range, reversed), with the region on the left of every piece.
- A `PieceId` is the entity plus what bounds each end (`PieceBound`): its own start or end, or the
  sorted ids of the curves that cut it there with an occurrence counted along the curve (`Cut`).
  A circle has no start, so a cutter set meeting it more than once gives `Crossing` bounds instead
  (a tag of its own, so no older name can mean another arc), ranked by the side each cutter heads
  to across the circle, then the position along the lowest-id open cutter, and only last the
  circle's own angle, so moving a crossing past the parameter origin never swaps its arcs' names.
- A `RegionKey` digests the set of (entity, side) pairs of its boundary; regions sharing one are
  told apart by their piece ids. Whether a key is tie-broken is decided once over the whole
  arrangement, so a region keeps its key whatever else is selected with it.
- A cycle is a face only when its area exceeds tolerance² and its mean width, twice its area over
  its perimeter, exceeds `SLIVER_TOLERANCES` tolerances; a thinner one (a line a hair beside an
  edge, two nearly equal arcs) is no region and no outer boundary, so it adds no phantom region.
- Depth counts nesting: a face lies one deeper than the single face its whole boundary borders (a
  hole touching its outline, a circle tangent inside another), else than the face its connected
  component lies in, so the cells of a grid keep their outline's depth.
- `select` with `Selection::EvenDepth` (the default) or explicit keys returns the union of the
  chosen regions as new regions keyed the same way, so adjacent regions sweep as one lump. A hole
  goes to the outer loop of its lump through the faces joined across pieces chosen on both sides,
  so selection is linear in loops; a probe point's containment decides only when a lump has
  several outer loops or none.
- `Region::triangulate` samples each loop into a polygon whose consecutive points closer than
  `LINEAR_RESOLUTION` are one (an arc sampled from a crossing starts a rounding error off the line
  ending there, and keeping both made the constrained triangulation refuse the region, leaving it
  without a fill to draw or click).
- A `RegionReference` keeps what a feature chose: the key, the boundary pieces and an anchor (the
  centroid of the largest triangle of `Region::triangulate`, `RegionMesh::anchor`). `resolve` gives
  `Same` when the key still exists, else `Healed` to the region most like it among those sharing at
  least one (entity, side): most pieces shared, then holding the anchor, then most pairs shared; a
  tie is `AmbiguousRegion`, nothing shared is `Gone`: a hole drawn inside a chosen region keeps the
  region around it, a line splitting it keeps the part holding the anchor, and a disc whose circle
  was deleted is gone rather than taken for the region it was a hole of. A reference of a key alone
  (older files) is `Same` or `Gone`. `resolve_regions` resolves a feature's whole choice
  and fails only on a tie or when every chosen region is gone (`MissingRegion`).
