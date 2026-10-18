---
paths:
  - "crates/caditor-kernel/src/intersect/**"
  - "crates/caditor-kernel/src/topology/classify.rs"
  - "crates/caditor-kernel/src/topology/classify_tests.rs"
  - "crates/caditor-kernel/src/topology/polygons.rs"
  - "crates/caditor-kernel/src/topology/isoparametric.rs"
  - "crates/caditor-kernel/src/boolean/**"
---

# Intersections (`intersect/`)

- Functions take a `SurfacePatch` (a surface and a finite uv box; periodic boxes wrap, poles accept
  any u), so booleans intersect face patches, and restrict curves to a parameter interval.
- Coincidence within tolerance is detected, never guessed: a curve in a surface or on another curve
  is an overlap interval; coincident surfaces return `SurfaceIntersection::Coincident(Sense)`.
  Sampled kinds (extrusion, revolution, spline) are also `Coincident` when sharing only part of
  their extent (`surface_surface/overlap.rs`); the grid projected onto the cheaper surface (not a
  sampled kind) is checked first, as one point lying apart decides. Before any projection, a pair
  whose grid points within the resolution of the other patch's box number fewer than the four
  shared points needed is not coincident (`reaching`), since only those can be shared.
- Clipping (`clip.rs`) does not depend on sample spacing: a piece between two outside samples is
  refined unless a bound on its path misses the patch; ends are then bisected exactly. Guided
  clipping polls `interrupt::check` before every sample and probe; a branch's probes project from
  the previous probe's foot.
- Points within `LINEAR_RESOLUTION` are one point; one at a range end takes the exact end
  parameter; a closed curve's wrap point is reported once. `tangent` flags touches (no sign change,
  or parallel tangent). Clusters of roots closer than the resolution collapse to one tangent
  point, except a cluster with exactly one distinct root (a sign change) is a crossing at that
  root, so a grazing crossing is met where it crosses. Branches shorter than `MIN_BRANCH_LENGTH`
  are dropped.
- Every search is budgeted; exhausting one is `IntersectionError::TooComplex`.

## Curves

- `intersect_curve_surface` (points with curve parameter and uv, and overlaps) is analytic for
  lines, circles and ellipses against elementary surfaces, else subdivides the curve on piece boxes
  (widened by a second-derivative sagitta within a spline span, since span hulls do not shrink),
  prunes by the surface's Lipschitz distance, brackets sign changes of the signed distance and
  minimises it for touches.
  - A spline piece holding fewer than `MAX_TIGHT_SPANS` knots is boxed by the tight bounds of each
    span part, not the control hull of every span it touches; otherwise a helix just off a cylinder
    over hundreds of spans halves at every knot and ends `TooComplex`.
  - Within a leaf, an extrusion, revolution or spline surface is projected from the previous foot;
    a foot whose offset is not along the normal (stalled on a domain boundary, where the signed
    distance reads zero far from the surface) is replaced by the global projection, so a stale hint
    never brackets a false root.
- `intersect_curves` and `intersect_curves2` are analytic for lines and 2D circles, else paired
  subdivision and Newton on the squared distance. A cluster of candidates is reported at the one
  where the curves come closest, since Newton clamped at a leaf boundary near a grazing crossing
  stops far from where it crosses.

## `intersect_surfaces`

- Returns `IntersectionBranch`es (curve, increasing range inside both boxes, closed flag, end uv on
  both sides, tangent flag) and isolated `IntersectionPoint`s.
- Analytic (`surface_surface/analytic.rs`): a plane against a plane, cylinder, cone, extrusion or a
  torus through its axis, and cylinders against cylinders. `coaxial.rs` takes every coaxial pair of
  rotational surfaces by intersecting meridians in (r, z) as 2D curves: each point is a circle,
  tangent points tangent circles, points on the axis isolated points.
- A plane cuts a cone in the exact ellipse (or circle) only when `|n·axis| > sin α`; crossing the
  axis behind the apex means it misses. Planes at `|n·axis| <= sin α` (parabolas and hyperbolas)
  are marched; through the apex they are rulings or the apex alone.
- Two cylinders of different radii whose axes cross at a distance equal to the sum or difference of
  the radii touch at a point on the axes' common perpendicular (`analytic::touches`), where the
  intersection crosses itself (two holes flush at the top or bottom). It is handed to the marcher,
  which ends a branch exactly there when a step's chord passes within `TOUCH_SLACK` of its length,
  or stalls tangent near it. Each lobe, from the point back to it, is returned as two halves (the
  boolean splits a branch at its pooled vertices by nearest parameter, so a branch passes each
  vertex once), and the point is a tangent end the boolean pools. Without it the marcher stepped
  across the point and returned one self-crossing loop with no vertex where it crosses.
- Everything else is marched (`march.rs`):
  - Seeds come from paired subdivision of both patches, each solved by Gauss–Newton, then a sign
    scan of the distance for tiny loops. Near-coincident patches prune little, so pairs are
    budgeted at `MAX_SEED_PAIRS`. A subdivision box is apart from a spline surface when it misses
    the control hull of every span (`BSplineSurface::hulls_miss`: the hull's box, then the axis
    across the span's corner diagonals; through the span tree on a large net), from any other
    surface when its centre's projection lies farther than its half diagonal (a projection onto
    a spline off its surface is a full seed search). A cylinder patch's box is
    the union of its two end arcs' boxes, exact for a ruled surface of constant radius.
  - Arrangement points lying on both surfaces (an edge of one solid piercing a face of the other)
    are also seeds (`intersect_surfaces_through`), so a short branch near a patch corner is found
    when no subdivision leaf seeds it; a failing hint is ignored.
  - Branches march both ways from each seed not already on a branch, stop exactly on the box
    boundary, close loops through the seed and end where the normals become parallel (tangent
    points). `end_on_boundary` drops contacts that slipped past the boundary within the inside
    test's slack, so a branch never ends with a backward hook whose node tangents point against
    the curve (which turns the boolean's trace the wrong way at that vertex).
  - A step whose solve on the plane ahead fails while its predicted uv lies outside the patches
    first tries ending exactly on the boundary it crosses (`boundary_within`: the bound solved as
    a parameter constraint, ahead of the current point, not a pole, turning at most `MAX_TURN`
    and hugging the chord), before the carried projections and halving. Without it a branch
    running off a spline face that spans its whole surface (a loft's) crept up on the domain end
    through some twenty halved steps, each projecting onto the spline from off its domain. A
    branch whose crossing lies within the resolution of its current point is already on the
    boundary it heads through (an arrangement hint on a patch edge marching outward): the step
    collapses at once and the branch ends there, rather than halving down to `MIN_STEP`.
  - A step that collapses where the branch runs off a bounded surface ends on the boundary ahead,
    at a pole (a cone apex on the other surface) ends there; otherwise it, and a branch longer than
    the step cap, fails as `Unfollowable`.
  - A step is accepted only when the points at a quarter, half and three quarters of its chord (on
    planes normal to the heading) lie within `MAX_SAGITTA` of the chord, so a step cannot jump
    across the neck between two nearby branches (the middle alone missed a jump between the tips
    of two U-shaped branches). A step landing outside the patches whose exit lies behind the
    current point is retried at half length, so a branch dipping into a patch for less than a step
    is followed.
  - A branch whose `IntersectionCurve` cannot be refined fails the intersection as `Unrefined`
    (`kernel.md`); the boolean takes its point as the error's site, so the curve is named where it
    strays instead of the solid failing validation. A branch whose contacts collapse to too few
    distinct points is dropped.
  - Every branch is clipped to both patches over only the spans reaching the window the patches
    share, so a long curve through a small face is found there. A marched branch that is a line,
    circle or ellipse within half the resolution is returned as that curve (`recognize.rs`).

# Point classification (`topology/classify.rs`)

- `classify_point` (`SolidClassifier` to reuse per solid) gives `Inside`, `Outside` or
  `OnBoundary(face)` exactly, or `Undecided` when every ray was ambiguous (a boolean then tries
  the fragment's other points), never a guess. A point on a face's surface and inside its boundary
  is on it; otherwise rays from a fixed list of directions are intersected with each face's
  surface and the nearest crossing's outward normal decides. A ray that grazes, is tangent, lies in
  a face, or meets an edge or vertex no farther than its nearest clean crossing is discarded for
  the next.
- `point_in_face(face, uv)` uses the pcurve polygons by parity over every periodic shift that
  brings the point into the face's uv box (at most `MAX_PERIOD_SHIFTS`, 4096, nearest first), since
  a curve winding many times round a pole or periodic surface (a thread's root, healed from a
  strip per turn) carries uvs many periods away. Near the boundary it uses the exact edge (the side
  of the nearest non-seam coedge, or of both coedges at a vertex: convex corners need both); within
  `LINEAR_RESOLUTION` of an edge it answers `OnBoundary`, while `exactly_inside_face` gives the side
  even there. Seams bound nothing (the face lies on both sides), so a point within the resolution of
  a seam edge and away from every other edge is inside: parity there sits on the seam's own polygon
  edge and answered either way, losing the vertex where two holes' axis-aligned seams cross. The classifier keeps a tree of face boxes (`box_tree.rs`) and each face indexes its
  boundary on first use (`PolygonIndex`, `topology/polygons.rs`): building is linear, a query
  logarithmic in the face's boundary.
- `classify_boundary_point(point, normal)` adds `Coincident { face, sense }` for a point on a face
  with a parallel normal, and `Touching(face)` otherwise. `classify_fragment_point` also requires
  the two elementary surfaces to be the same surface, so a plane tangent to a cylinder along a
  line is touching, not coincident.
- `Solid::isoparametric_runs(face, along, lines)` (`topology/isoparametric.rs`) gives a face's
  parameter lines: `lines` (at most `MOST_ISOPARAMETRIC_LINES`) values of the other parameter
  spaced evenly strictly inside the uv box, each sampled `SAMPLES_PER_LINE` times with
  `point_in_face` of a one-face classifier (`SolidClassifier::of_face`); a line through a hole is
  split into runs, each end bisected to the boundary. It polls the interrupt once a line
  (`IsoparametricError::Cancelled`).
- `first_crossing(origin, direction, beyond)` casts one ray the same way: the nearest clean
  crossing past `beyond` (face, distance, whether the ray enters), `Nothing`, or `Undecided` when a
  graze, tangent, overlap or edge comes no later. Hits up to `beyond` are ignored, so a ray may
  start on a face.
