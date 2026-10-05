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
  than the spacing is still found; ends are then bisected exactly.
- Points within `LINEAR_RESOLUTION` are one point; one at a range end takes the exact end
  parameter; a closed curve's wrap point is reported once. `tangent` flags touches (no sign change,
  or parallel tangent); clusters of roots closer than the resolution collapse to one tangent
  point. Branches shorter than `MIN_BRANCH_LENGTH` are dropped.
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
  subdivision and Newton on the squared distance.

## `intersect_surfaces`

- Returns `IntersectionBranch`es (curve, increasing range inside both boxes, closed flag, end uv on
  both sides, tangent flag) and isolated `IntersectionPoint`s.
- Analytic cases (`analytic.rs`): a plane against a plane, cylinder, cone, extrusion or a torus
  through its axis, and cylinders against cylinders. `coaxial.rs` takes every coaxial pair of
  rotational surfaces by intersecting meridians in (r, z) as 2D curves (a cylinder's reaching well
  past the window, so no crossing is pulled onto its end): each point is a circle, tangent points
  tangent circles, points on the axis isolated points.
- Everything else is marched (`march.rs`):
  - Seeds come from paired subdivision of both patches, each solved by Gauss–Newton, then a sign
    scan of the distance for tiny loops.
  - Points of the arrangement lying on both surfaces (where an edge of one solid pierces a face of
    the other) are also seeds (`intersect_surfaces_through`), so a short branch between two such
    points near a corner of a patch is found even when no subdivision leaf seeds it; a failing
    hint is ignored.
  - Branches march both ways from each seed not already on a branch, stop exactly on the box
    boundary, close loops through the seed and end where the normals become parallel (reported as
    tangent points). A step that collapses where the branch runs off a bounded surface ends on the
    boundary ahead, at a pole (a cone apex on the other surface) ends there; otherwise it, and a
    branch longer than the step cap, fails as `Unfollowable`. A step is accepted only when the
    point found halfway along its chord (on the plane normal to the heading) lies within
    `MAX_SAGITTA` of the chord's middle, so a step cannot jump across the neck between two
    nearby branches onto the other one.
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
  a vertex: convex corners need both). The classifier keeps a tree of face boxes (`box_tree.rs`)
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
