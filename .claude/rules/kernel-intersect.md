---
paths:
  - "crates/caditor-kernel/src/intersect/**"
  - "crates/caditor-kernel/src/topology/classify.rs"
  - "crates/caditor-kernel/src/topology/classify_tests.rs"
  - "crates/caditor-kernel/src/boolean/**"
---

# Intersections (`intersect/`)

- Functions take a `SurfacePatch` (a surface and a finite uv box; periodic boxes wrap, poles accept
  any u), so booleans intersect face patches, and restrict curves to a parameter interval.
- Coincidence within tolerance is detected, never guessed: a curve in a surface or on another curve
  is an overlap interval; coincident surfaces return `SurfaceIntersection::Coincident(Sense)` from
  `same_surface`. Patches of the sampled kinds (extrusion, revolution, spline) are also
  `Coincident` when sharing only part of their extent: every grid sample of either whose foot lands
  inside the other must lie on it (a foot pushed against the other's edge, with a residual off the
  normal, is skipped), and at least four must land there.
- Points within `LINEAR_RESOLUTION` are one point; one at a range end takes the exact end
  parameter; a closed curve's wrap point is reported once. `tangent` flags touches (no sign change,
  or parallel tangent within 1e-7); clusters of roots closer than the resolution collapse to one
  tangent point.

## `intersect_curve_surface`

- Returns points (curve parameter and uv) and overlaps.
- Analytic: a line against plane, cylinder, cone (its own nappe) and sphere; the torus quartic,
  isolated through its derivatives' roots; circles and ellipses against planes; circles against
  spheres and coaxial cylinders, cones and tori; an intersection curve on its own surfaces.
- Otherwise the curve is subdivided on piece boxes (within one spline span the samples are widened
  by a second-derivative sagitta, since span hulls do not shrink), pruned by the surface's
  Lipschitz distance until flat relative to both curvatures. Each leaf brackets sign changes of the
  signed distance (roots verified by true distance) and minimises it for touches. Swept surfaces
  project locally from the previous foot inside a leaf.

## `intersect_curves`, `intersect_curves2`

- Parameters on both, and overlaps. Analytic for lines and 2D circles; else paired subdivision to
  flat pieces and Newton on the squared distance from several starts per leaf.

## `intersect_surfaces`

- Returns `IntersectionBranch`es (curve, increasing range inside both boxes, closed flag, end uv on
  both sides, tangent flag) and isolated `IntersectionPoint`s.
- Analytic cases:
  - plane/plane; plane/cylinder (circle, ellipse, two lines, tangent line); plane/cone (circle,
    ellipse, rulings through the apex or a tangent ruling, the apex alone); a plane through a torus
    axis (two circles); plane/extrusion (lines when parallel to the direction, else the profile's
    exact affine image: B-spline, line, conic);
  - parallel cylinders (lines or a tangent line); equal cylinders with crossing axes (two ellipses
    and the two tangent points);
  - every coaxial pair of rotational surfaces (plane normal to the axis, sphere centred on it,
    cylinder, cone, torus, revolution of a planar profile): meridians are intersected in (r, z) as
    2D curves (a cylinder's reaching well past the window, so no crossing is pulled onto its end);
    each point is a circle, tangent points tangent circles, points on the axis isolated points.
- Everything else is marched:
  - Seeds come from paired subdivision of both patches (sub-patches cached with their boxes, pruned
    by box overlap and Lipschitz distance) to leaves of half a curvature radius, each solved by
    minimal-norm Gauss–Newton, then a sign scan of the distance for tiny loops; a leaf already
    holding a transversal seed is skipped.
  - Branches march both ways from each seed not already on a branch, steps limited by the tangent's
    turn; they stop exactly on the box boundary (a parameter-constrained solve), close loops through
    the seed and end where the normals become parallel (reported as tangent points).
  - A step that collapses where the branch runs off a bounded surface ends on the boundary ahead
    (the nearest patch bound along the tangent within one maximum step, by a parameter-constrained
    solve); one collapsing at a pole (a cone apex on the other surface) ends there; otherwise it,
    and a branch longer than the step cap, fails as `IntersectionError::Unfollowable`.
  - Contact solves keep each periodic coordinate at the turn nearest their start; near poles the
    contact is solved with one surface as carrier and the other's signed distance.
  - Every branch is clipped to both patches by sampling only its spans whose boxes reach the window
    the patches share, so a long curve through a small face is found there.
  - A marched branch that is a line, circle or ellipse within half the resolution is returned as
    that curve.

# Point classification (`topology/classify.rs`)

- `classify_point` (`SolidClassifier` to reuse per solid) gives `Inside`, `Outside` or
  `OnBoundary(face)` exactly, or `Undecided` when every ray was ambiguous (a boolean then tries the
  fragment's other points), never a guess.
  - A point on a face's surface and inside its boundary is on it.
  - Otherwise rays from a fixed list of directions are intersected with each face's surface through
    `intersect_curve_surface`, over the face box's window widened a little so a crossing is never
    snapped to the window's end; the nearest crossing's outward normal decides.
  - A ray that grazes, is tangent, lies in a face, or meets an edge or vertex no farther than its
    nearest clean crossing is discarded for the next direction.
- `point_in_face(face, uv)` (`Inside`, `Outside`, `OnBoundary`) uses the pcurve polygons by parity
  over periodic shifts (poles probed just off the pole line, inwards from the nearer end of the
  domain); within a few `PCURVE_TOLERANCE` of the boundary it uses the exact edge: the side of the
  nearest non-seam coedge, or of both coedges at a vertex (convex corners need both).
- `classify_boundary_point(point, normal)` adds `Coincident { face, sense }` for a point on a face
  with a parallel normal, and `Touching(face)` otherwise.
