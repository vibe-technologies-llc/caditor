---
paths:
  - "crates/caditor-kernel/src/tessellation/**"
---

# Tessellation

- Each edge is sampled once; both faces share its positions.
- Each face is a constrained Delaunay triangulation (`spade`) of its loops in (u, v), plus an
  interior tensor grid (`density.rs`) kept clear of the boundary. Each direction's grid lines are
  graded by curvature: the cells needed per unit parameter are sampled on a lattice of uniform
  points and points in every knot span (at most `MAX_LATTICE` a direction), each lattice span takes
  the larger need of its two ends, and lines sit at equal steps of the accumulated need, so a bump
  divides only the rows and columns through it and a flat stretch is one wide cell. No span may
  need more than `MAX_SEGMENTS` across the whole face, so a pole's degenerate curvature cannot
  draw every line to itself. A direction without curvature gets `FLAT_SEGMENTS` cells whatever its
  length: rulings are straight, so a long cylinder needs no rows along them.
- Curved kinds are then refined a whole direction at a time until the sampled cells (strided by
  index, plus some placed by parameter so wide cells are seen) bow from their chords, measured
  along the surface normal, by at most `GRID_SHARE` of the chord tolerance (which keeps the
  triangles the triangulation actually picks within the chord). Measuring along the normal ignores
  parameter stretch within the surface (a clamped spline's ends, a cone's rulings); a cell whose
  diagonal bows more than its sides refines both directions.
- A face with a grid is triangulated in cell space (each parameter mapped piecewise linearly to its
  grid line index, so every cell is a unit square), so a boundary point off the lattice joins its
  neighbours and never fans across a dense row of grid points, which on a torus much thinner than
  its ring spanned the tube; a face without a grid is scaled by the mean surface speeds. Gaps along
  a pole line or seam joint are filled in cell space, one point per cell they cross.
- Points are deduplicated by exact scaled coordinates (`DuplicateBoundaryPoint`), inserted in a
  biased randomised order (`insertion.rs`: rounds doubling in size from a fixed-seed shuffle, each
  sorted along a Z-order curve), and only then joined by the loops' constraint edges. Inserting
  boundary points in loop order flipped edges quadratically, and spade's bulk load walks its hull
  quadratically when the points lie along one curve (a cap cut in half: 18,000 boundary points took
  2 s, against 5 ms in this order). Triangles are kept by the parity of constraint crossings from
  outside.
- Boundary points at one vertex whose parameters differ by a spatially negligible gap are merged,
  and a pinched vertex takes the uv of its first pass, so joints do not become spikes.
- Where two boundary polylines leave one point along the same chord (tangent curves sampled at the
  same angular step: a crescent tip, internally tangent circles, a cusp), the longer end segment is
  bisected first (at most `MAX_END_PARTINGS` rounds) for every face of that edge, so the loops part
  by the curves' own separation, never by rounding.
- Pole-line points share the pole's position and the triangles collapsing there are dropped, so
  the mesh stays watertight. A straight edge ending at a pole (a ruling to a cone's apex) is
  sampled with one piece per grid row it crosses, else the triangles beside it fan from the apex
  with no area.

## Threads and reuse

- Edges are sampled on the calling thread, which numbers their positions in edge order; then pole
  samplings, and faces once the overlapping ends are parted, are made on scoped threads
  (`parallel.rs`: up to the available parallelism, at least `FACES_PER_THREAD` faces each, so a
  small body stays on the caller). Helpers run under the caller's interrupt and a panic in one is
  resumed on the caller.
- A face makes a `FacePatch` (`patch.rs`): its interior points, its vertices, each on a boundary
  position or one of its own interior points, and its triangles in its own indices. Patches are
  placed in face order, numbering positions, vertices and triangles exactly as one thread would
  (`faces_meshed_on_many_threads_make_the_mesh_of_one_thread`); the point budget is checked as each
  is placed, against the positions placed before it.
- A vertex is made only for a triangle that is kept, in the order triangles are emitted, so each
  face's vertices and interior positions are contiguous in the mesh and in order of first use.
- `Solid::display_mesh_reusing(quality, earlier)` returns a `DisplayMesh`: the mesh and, per face,
  a key (`reuse.rs`) of everything triangulating it reads: surface, sense, tolerance, pole
  density, and its boundary loops in uv with their position labels. A face whose name has a key
  in the earlier mesh equal in all of that (uv to the bit, labels equal in the same pattern) reads
  its patch back from the earlier mesh, labels mapped, instead of being triangulated, and the mesh
  is the one meshing afresh gives. The tolerance follows the solid's extent, so a change to the
  body's box meshes every face again. `display_mesh_costs` (ignored, release) times both: on 16
  threads a 119-face plate takes 18 ms against 69 ms on one, and 6 ms when one face is added.

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
- `mass_properties` (volume, area, centroid, second moments) by the divergence theorem over the
  triangles: what validation measures, and the fallback for a face exact integration gives up on
  (`kernel.md`); `Mesh::chord` records the chord asked for (retried faces only get closer), which
  the app quotes as the accuracy of such mass properties.

## Retries and limits

- A face boundary crossing itself at the requested tolerance (loops closer than the sampling error)
  retries up to `MAX_REFINEMENTS` times, halving chord and angle for the faces that crossed and the
  edges they bound (an edge takes the finest tolerance of its faces); only what changed is
  sampled or triangulated again.
- A mesh holds at most `MAX_POINTS`, counted before insertion; more fails as `TooLarge`, which
  export names as a body too fine for the resolution.
