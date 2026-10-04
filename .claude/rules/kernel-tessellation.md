---
paths:
  - "crates/caditor-kernel/src/tessellation/**"
---

# Tessellation

- Each edge is sampled once; both faces share its positions.
- Each face is a constrained Delaunay triangulation (`spade`) of its loops in (u, v), scaled by the
  mean surface speeds, plus a uniform interior grid (`density.rs`) spaced by curvature, covering
  every knot span, kept clear of the boundary and refined on curved kinds until its cells stay
  within `GRID_SHARE` of the chord tolerance (which keeps the triangles the triangulation actually
  picks within the chord). A direction without curvature gets cells at most `FLAT_ASPECT` times
  longer than the curved one's.
- Points are deduplicated by exact scaled coordinates (`DuplicateBoundaryPoint`), bulk-loaded, and
  only then joined by the loops' constraint edges (inserting boundary points one by one flipped
  edges quadratically). Triangles are kept by the parity of constraint crossings from outside.
- Boundary points at one vertex whose parameters differ by a spatially negligible gap are merged,
  and a pinched vertex takes the uv of its first pass, so joints do not become spikes.
- Where two boundary polylines leave one point along the same chord (tangent curves sampled at the
  same angular step: a crescent tip, internally tangent circles, a cusp), the longer end segment is
  bisected first (at most `MAX_END_PARTINGS` rounds) for every face of that edge, so the loops part
  by the curves' own separation, never by rounding.
- Pole-line points share the pole's position and the triangles collapsing there are dropped, so
  the mesh stays watertight. A straight edge ending at a pole (a ruling to a cone's apex) is
  sampled at the grid's row spacing, else the triangles beside it fan from the apex with no area.

## Quality

- `MeshQuality` (`tolerance.rs`) is a chord as a fraction of the solid's extent (bounding-box
  diagonal) plus an angle per segment. `COARSE` is `Solid::default_tolerance` and
  `SamplingTolerance::for_extent` (validation, profiles, tests); `SMOOTH` is the default display
  quality. The angle bounds segments per turn whatever the radius; the relative chord takes over on
  radii large against the solid.
- `Solid::display_mesh(quality)` meshes within `DISPLAY_POINTS`. Any failure but cancellation (over
  the budget, a boundary still crossing after the retries) meshes again at the quality made at
  least as coarse as `COARSE` (`MeshQuality::at_least`) within `MAX_POINTS`: a body is shown
  coarser rather than not at all.
- Export does not use it: it has its own `MeshResolution` (`file-import-export.md`).

## `Mesh`

- Vertices are per face and carry the surface's analytic normal (flipped by the face sense, nudged
  into the triangle at a pole), so curved faces shade smoothly while the edge between a cylinder
  and its cap stays crisp. Edge polylines are the samplings the faces were triangulated with, so
  outlines follow the silhouettes exactly.
- `mass_properties` (volume, area, centroid) by the divergence theorem; `Mesh::chord` records the
  chord asked for (retried faces only get closer), which the app quotes as the accuracy of mass
  properties.

## Retries and limits

- A face boundary crossing itself at the requested tolerance (loops closer than the sampling error)
  retries up to `MAX_REFINEMENTS` times, halving chord and angle for the faces that crossed and the
  edges they bound (an edge takes the finest tolerance of its faces); only what changed is
  sampled or triangulated again.
- A mesh holds at most `MAX_POINTS`, counted before insertion; more fails as `TooLarge`, which
  export names as a body too fine for the resolution.
