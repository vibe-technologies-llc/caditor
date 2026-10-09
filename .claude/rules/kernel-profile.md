---
paths:
  - "crates/caditor-kernel/src/profile/**"
  - "crates/caditor-kernel/src/build/**"
---

# Profiles (`profile/`)

- `Profile::new` takes `ProfileCurve`s (a line, circle, counter-clockwise arc, clamped B-spline,
  ellipse or counter-clockwise elliptical arc, the last two as a centre, a major axis vector and a
  minor radius, `Curve2::Ellipse` running by its angle parameter, the arc between its end points),
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
- `section_of` (and `Region::section`) gives the section properties of a set of regions of one
  profile: area, perimeter, centroid, and second moments about the centroid along the sketch axes
  (`AreaMoments`: Ix = ∫y², Iy = ∫x², the product Ixy, the polar moment) with the principal moments
  and the angle from x to the axis of the larger (`PrincipalMoments`, no angle when the two agree
  within `ISOTROPIC_SHARE`, as for a circle or square). Green's theorem over each piece as the
  signed fan from a reference point at the centre of the bounds (so far-off sketches keep their
  digits): lines and arcs in closed form (an arc as two triangles through its centre plus the
  sector), splines and ellipses by adaptive Gauss-Legendre quadrature against
  `QUADRATURE_TOLERANCE` of the size, which makes the whole section `Accuracy::Approximate`.
  Ellipses meet other curves through the generic subdivision and Newton path, as splines do. Regions of a profile are disjoint,
  so their integrals add; a piece met on both sides (the same `PieceId` left and right) is inside
  the union and adds no perimeter.
- A `RegionReference` keeps what a feature chose: the key, the boundary pieces and an anchor (the
  centroid of the largest triangle of `Region::triangulate`, `RegionMesh::anchor`). `resolve` gives
  `Same` when the key still exists, else `Healed` to the region most like it among those sharing at
  least one (entity, side): most pieces shared, then holding the anchor, then most pairs shared; a
  tie is `AmbiguousRegion`, nothing shared is `Gone`: a hole drawn inside a chosen region keeps the
  region around it, a line splitting it keeps the part holding the anchor, and a disc whose circle
  was deleted is gone rather than taken for the region it was a hole of. A reference of a key alone
  (older files) is `Same` or `Gone`. `resolve_regions` resolves a feature's whole choice
  and fails only on a tie or when every chosen region is gone (`MissingRegion`).

## Offsets and walls (`strand.rs`, `wall.rs`)

- A `Strand` is a line or an arc (signed sweep, a full circle sweeping a whole turn) in the
  direction of travel. `offset_strands` moves every strand of a chain or loop to its left by a
  distance and re-meets each corner: tangent corners at the moved ends' midpoint, others at the
  crossing of the moved carriers nearest an aim (the previous level's corner when tapering),
  failing as `OffsetFailure` (`Shrinks`: a strand reversed or an arc gone, `Apart`, `Folds`).
  `polygons_cross` finds proper crossings between non-adjacent segments of sampled loops.
- `wall_regions(curves, thickness, WallSide)` chains the curves by their ends (within the
  arrangement's relative tolerance; three ends at a point are `Branches`), offsets each chain to
  both sides (`Inside` is the side a closed chain encloses, or the side an open chain bends around,
  by the sign of its area closed by its chord; `Centred` half each way) and returns one region per
  chain: an open chain's outline closed by straight caps at its ends, a closed chain's ring. Pieces
  keep their entity with bounds that no arrangement makes, so every face is named stably whichever
  way the chain is drawn: the entity's own right side `(Start, End)`, its left `(End, Start)`, the
  cap at its own start `(Start, Start)` and end `(End, End)`. Splines are `Spline`; a wall too thick
  for a curve, corners that no longer meet, a turned-over loop or walls crossing each other
  (`CrossesItself`) are refused. The regions extrude, taper and revolve like any other.

