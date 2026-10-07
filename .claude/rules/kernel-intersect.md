---
paths:
  - "crates/caditor-kernel/src/intersect/**"
  - "crates/caditor-kernel/src/topology/classify.rs"
  - "crates/caditor-kernel/src/topology/classify_tests.rs"
  - "crates/caditor-kernel/src/topology/polygons.rs"
  - "crates/caditor-kernel/src/boolean/**"
---

# Intersections (`intersect/`)

- Functions take a `SurfacePatch` (a surface and a finite uv box; periodic boxes wrap, poles accept
  any u), so booleans intersect face patches, and restrict curves to a parameter interval.
- Coincidence within tolerance is detected, never guessed: a curve in a surface or on another curve
  is an overlap interval; coincident surfaces return `SurfaceIntersection::Coincident(Sense)`.
  Patches of the sampled kinds (extrusion, revolution, spline) are also `Coincident` when sharing
  only part of their extent (`surface_surface/overlap.rs`).
- Clipping to a patch or range (`clip.rs`) does not depend on the sample spacing: a piece between
  two outside samples is refined unless a bound on its path misses the patch, so one far shorter
  than the spacing is still found; ends are then bisected exactly. Guided clipping polls
  `interrupt::check` before every sample and refinement probe, and a branch's probes project from
  the previous probe's foot.
- Points within `LINEAR_RESOLUTION` are one point; one at a range end takes the exact end
  parameter; a closed curve's wrap point is reported once. `tangent` flags touches (no sign change,
  or parallel tangent); clusters of roots closer than the resolution collapse to one tangent
  point, except that a cluster with exactly one distinct root (a sign change) is a crossing at that
  root, so a curve crossing at a grazing angle, within the resolution of the surface over a long
  stretch, is met where it crosses and not at a sample that merely reads near zero. Branches
  shorter than `MIN_BRANCH_LENGTH` are dropped.
- Every search is budgeted; exhausting one is `IntersectionError::TooComplex`.

## Curves

- `intersect_curve_surface` (points with curve parameter and uv, and overlaps) is analytic for
  lines, circles and ellipses against elementary surfaces, else subdivides the curve on piece boxes
  (widened by a second-derivative sagitta within a spline span, since span hulls do not shrink),
  prunes by the surface's Lipschitz distance, brackets sign changes of the signed distance and
  minimises it for touches. Within a leaf, an extrusion or revolution is projected from the previous
  foot, and a foot whose offset is not along the normal (stalled on a boundary of the domain, where
  the signed distance reads zero far from the surface) is replaced by the global projection, so a
  stale hint never brackets a false root.
- `intersect_curves` and `intersect_curves2` are analytic for lines and 2D circles, else paired
  subdivision and Newton on the squared distance. A cluster of candidates is reported at the one
  where the curves come closest, since Newton clamped at a leaf boundary near a grazing crossing
  stops within the resolution of the other curve but a long way from where it crosses.

## `intersect_surfaces`

- Returns `IntersectionBranch`es (curve, increasing range inside both boxes, closed flag, end uv on
  both sides, tangent flag) and isolated `IntersectionPoint`s.
- Analytic cases (`analytic.rs`): a plane against a plane, cylinder, cone, extrusion or a torus
  through its axis, and cylinders against cylinders. `coaxial.rs` takes every coaxial pair of
  rotational surfaces by intersecting meridians in (r, z) as 2D curves (a cylinder's reaching well
  past the window, so no crossing is pulled onto its end): each point is a circle, tangent points
  tangent circles, points on the axis isolated points.
- A plane cuts a cone in the exact ellipse (or circle) only when `|n·axis| > sin α`: then the plane
  meets every ruling of one nappe, and crossing the axis behind the apex means it misses the cone.
  Planes at `|n·axis| <= sin α` (parabolas and hyperbolas) are marched; through the apex they are
  rulings or the apex alone.
- Everything else is marched (`march.rs`):
  - Seeds come from paired subdivision of both patches, each solved by Gauss–Newton, then a sign
    scan of the distance for tiny loops. Near-coincident patches (tori a tenth of a millimetre
    apart) prune little, so the pairs are budgeted at `MAX_SEED_PAIRS` (2^17).
  - Points of the arrangement lying on both surfaces (where an edge of one solid pierces a face of
    the other) are also seeds (`intersect_surfaces_through`), so a short branch between two such
    points near a corner of a patch is found even when no subdivision leaf seeds it; a failing
    hint is ignored.
  - Branches march both ways from each seed not already on a branch, stop exactly on the box
    boundary, close loops through the seed and end where the normals become parallel (reported as
    tangent points). A step that collapses where the branch runs off a bounded surface ends on the
    boundary ahead, at a pole (a cone apex on the other surface) ends there; otherwise it, and a
    branch longer than the step cap, fails as `Unfollowable`. A step is accepted only when the
    points found at a quarter, half and three quarters of its chord (on planes normal to the
    heading) lie within `MAX_SAGITTA` of the chord, so a step cannot jump across the neck between
    two nearby branches onto the other one (the middle alone missed a jump between the tips of two
    U-shaped branches). A step landing outside the patches whose exit lies behind the current
    point is retried at half its length, so a branch that dips into a patch for less than a step
    (a plane barely cutting a cone's rim) is followed rather than ended where it came in.
  - Every branch is clipped to both patches over only the spans reaching the window the patches
    share, so a long curve through a small face is found there. A marched branch that is a line,
    circle or ellipse within half the resolution is returned as that curve (`recognize.rs`).

# Point classification (`topology/classify.rs`)

- `classify_point` (`SolidClassifier` to reuse per solid) gives `Inside`, `Outside` or
  `OnBoundary(face)` exactly, or `Undecided` when every ray was ambiguous (a boolean then tries
  the fragment's other points), never a guess.
  - A point on a face's surface and inside its boundary is on it.
  - Otherwise rays from a fixed list of directions are intersected with each face's surface; the
    nearest crossing's outward normal decides. A ray that grazes, is tangent, lies in a face, or
    meets an edge or vertex no farther than its nearest clean crossing is discarded for the next.
- `point_in_face(face, uv)` uses the pcurve polygons by parity over every periodic shift that
  brings the point into the face's uv box (a curve winding several times around a pole carries
  uvs many periods away); near the boundary it uses the exact edge (the side of the nearest non-seam coedge, or of both coedges at
  a vertex: convex corners need both); within `LINEAR_RESOLUTION` of an edge it answers
  `OnBoundary`, while `exactly_inside_face` gives that side even there. The classifier keeps a tree of face boxes (`box_tree.rs`)
  and each face indexes its boundary on first use (`PolygonIndex`, `topology/polygons.rs`), so
  building stays linear and a query logarithmic in the face's boundary.
- `classify_boundary_point(point, normal)` adds `Coincident { face, sense }` for a point on a face
  with a parallel normal, and `Touching(face)` otherwise. `classify_fragment_point` also asks that
  two elementary surfaces be the same surface, so a plane tangent to a cylinder along a line is
  touching, not coincident.
- `first_crossing(origin, direction, beyond)` casts one ray the same way: the nearest clean
  crossing past `beyond` (face, distance, whether the ray enters), `Nothing`, or `Undecided` when a
  graze, tangent, overlap or edge comes no later. Hits up to `beyond` are ignored, so a ray may
  start on a face.
