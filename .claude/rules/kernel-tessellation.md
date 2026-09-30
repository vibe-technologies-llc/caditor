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

## `Mesh`

- Holds shared positions, per-face vertices with exact surface normals, triangles, each face's
  triangle range and each edge's polyline.
- Computes volume, area and centroid by the divergence theorem.

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
