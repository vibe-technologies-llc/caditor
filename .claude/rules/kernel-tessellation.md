---
paths:
  - "crates/caditor-kernel/src/tessellation/**"
---

# Tessellation

- Each edge is sampled once; both faces share its positions.
- Each face is a constrained Delaunay triangulation (`spade`) of its loops in (u, v), scaled by the
  mean surface speeds, plus a uniform grid of interior points:
  - spaced by curvature (normal curvature and twist), sampled on a lattice that also covers every
    knot span;
  - spline, revolution, extrusion and cone faces are then refined, by the square root of the excess,
    until the grid's cells stay within the chord tolerance;
  - kept clear of the boundary;
  - a direction without curvature gets cells at most four times longer than the curved one's.
- Triangles are kept by the parity of constraint crossings from outside.
- Consecutive boundary points at the same vertex whose parameters differ by a spatially negligible
  gap (an edge ending within the resolution of its vertex) are merged, so such joints do not become
  spikes.
- A vertex a face's loops pass more than once (a pinch) takes the uv of its first pass wherever the
  others lie within that gap, so the triangulation sees one point.
- Pole-line points share the pole's position and the triangles that collapse there are dropped, so
  the mesh stays watertight.
- A straight edge ending at a pole (a ruling to a cone's apex) is sampled at the grid's row spacing
  of the faces it bounds; with only its ends, the triangles beside it would fan from the apex along
  one ruling and have no area.
- A face with a pole computes its density once per tolerance, for its pole edges and its grid alike.

## Quality

- `MeshQuality` (`tolerance.rs`) is a chord as a fraction of the solid's extent (its bounding box's
  diagonal) plus an angle per segment; `tolerance(extent)` turns it into a `SamplingTolerance`.
  `COARSE` (1e-3, 0.35 rad) is `Solid::default_tolerance` and `SamplingTolerance::for_extent`, for
  validation, profiles and tests; `SMOOTH` (2.5e-4, 6°) is the default display quality.
- The angle bounds every circle to at least 60 segments per turn whatever its radius, and every
  curved grid direction alike; the relative chord takes over on radii large against the solid.
- `Solid::display_mesh(quality)` meshes within `DISPLAY_POINTS` (2^20). Any failure but
  cancellation (over the budget, a boundary still crossing after the retries) meshes again at the
  quality made at least as coarse as `COARSE` in both terms (`MeshQuality::at_least`) with the full
  `MAX_POINTS`, so a body is shown coarser rather than not at all.
- Export does not use it: it has its own `MeshResolution` (`file-import-export.md`).

## `Mesh`

- Holds shared positions, per-face vertices with exact surface normals, triangles, each face's
  triangle range and each edge's polyline.
- Normals are the surface's analytic normal at each vertex (flipped by the face sense, nudged into
  the triangle at a pole), so curved faces shade smoothly; vertices are per face, so a position
  shared by a cylinder and its cap carries each face's own normal and the edge between them stays
  crisp.
- Edge polylines are the edge samplings the faces were triangulated with, so drawn outlines follow
  the faces' silhouettes exactly.
- Computes volume, area and centroid by the divergence theorem.
- Records the chord it was asked for (`Mesh::chord`; faces retried finer only get closer), which the
  app quotes as the accuracy of mass properties.

## Retries

- When a face boundary crosses itself at the requested tolerance (loops closer than the sampling
  error), tessellation retries a few times before failing.
- Each retry halves chord and angle for every face that crossed and for the edges they bound; an
  edge takes the finest tolerance of its faces.
- A retry samples again only the edges whose tolerance or least count changed, and triangulates only
  the faces that crossed or bound such an edge, keeping every other face's triangles. Points left
  unused are dropped at the end.

## Limits and cancellation

- A solid's mesh holds at most `MAX_POINTS` (2^22) points. Edges and each face's interior grid are
  counted before they are inserted; a mesh that would need more fails as `TooLarge`, which export
  names as a body too fine for the resolution.
- Cancellation is polled per edge, per grid row and every 1024 points inserted.
