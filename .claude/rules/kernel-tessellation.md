---
paths:
  - "crates/caditor-kernel/src/tessellation/**"
---

# Tessellation

- Each edge is sampled once; both faces share its positions. Edges are sampled sparingly
  (`Curve::sample_sparingly`): the adaptive samples, seeded with 2×degree points in every knot
  span, are merged afterwards while a chord from the last kept sample still holds every dropped
  sample and sub-span middle within the chord tolerance and turns within the angle
  (`merged_parameters`, at most `MAX_MERGED_SAMPLES` merged into one), never across a sample where
  the tangent jumps (a degree-1 spline's corners). So a traced thread edge stored as a spline with
  a knot at every node takes the 65 points a turn 6° asks for, not one per seed (670 on a VZ330
  screw, and ten times the face triangles). Validation and profiles keep `Curve::sample`.
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
- Exactly cocircular points send spade's in-circle predicate down its slow exact path, and round
  holes and the interior grid are full of them. So a helper point is inserted first at the vertex
  centroid of every inner loop turning against the outer one that sees each of its edges from
  that centroid (`constrained::outline`, a `Hole`): the hole's interior becomes a fan around it
  instead of triangles among its own cocircular samples, its triangles never cross the loop's
  constraints, so they are outside the face and dropped (any kept triangle on a helper
  triangulates again without helpers), and no point of the face is moved. Interior grid points
  are nudged in the mapped plane only, by a fixed `scatter` of their cell of at most a millionth
  of a cell (`GRID_NUDGE`), so grid squares are no longer cocircular; their uv and positions are
  untouched. On the top of a plate with 113 holes the whole triangulation fell from 11 to 3 ms,
  and exact predicates from about a quarter of all tessellation time to under a tenth.
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
- A face of at least `SPLIT_POINTS` points holding at least a `DOMINANT_SHARE`th of the body's
  positions placed before it (`pieces::allowed`) is triangulated in pieces (`pieces.rs`), so the
  largest face no longer bounds the body's time alone. The mapped plane is cut into vertical strips
  of equal point counts, each triangulated on a thread of its own (`parallel.rs`) from the points
  within its reach, every loop segment overlapping it, and every hole whose span meets the reach
  whole, with its helper, so a reach edge never leaves a hole's cocircular samples to the exact
  predicates. A reach is a distance, not a share of points: it widens the core each side by
  `REACH_GAPS` of the face's widest gap, its edge moved within `EDGE_SLACK` of that to where the
  fewest holes straddle it, then to the nearest gap between points. The widest gap (`gaps.rs`) is
  the radius of the largest disc empty of points and loop segments centred inside the face, read
  from an occupancy grid of about one cell per point (points and loop segments mark cells, a scan
  of row crossings keeps the cells inside the outer loop and outside the holes, a two-pass chamfer
  measures each cell's distance to a marked one), plus `CELL_ERROR` cells so it never falls short;
  it is computed beside the sort of the points along x (`parallel::both`), taking about 0.12 ms on
  the plate top below. Strips number one per `POINTS_PER_PIECE` points, at most `MAX_PIECES`, and
  no more than leave each core `STRIP_REACHES` reaches wide, so a face of fine features is cut
  finely and one of wide gaps coarsely. A cut sits `CUT_SHARE` of the way between two neighbouring
  points, not halfway: a mirror-symmetric face puts its cut on its axis otherwise, where the
  circumcentres of the cocircular cells straddling it lie, and every such triangle failed the
  strict check below.
- A strip keeps a triangle inside the face (parity spread among its triangles within the reach,
  seeded by a vertical ray cast against those segments) whose circumcircle stays within its reach,
  which makes it a triangle of the whole face's constrained Delaunay triangulation, and which it
  owns: the strip whose core holds the circumcentre, computed from the corners in index order so
  every strip computes the same bits. Its corners may lie in several cores, so triangles across a
  cut are kept. Triangles of one cocircular cell share a circumcentre, so one strip owns them all;
  as computed centres differ by rounding, a centre within `BAND_SHARE` of the face's width of a cut
  or with an error bound (`Circle::error`) over a quarter of that is kept only when strictly
  Delaunay: every neighbour across an edge that is not a constraint has its far corner outside the
  circle by more than the in-circle error bound, which a triangle in a cell of four or more
  cocircular points never has. So no two strips keep overlapping or equal triangles. One more
  triangulation covers the rest: the points no kept triangle uses or that lie on the border of
  the kept triangles (edges two strips both border cancel), constrained by that border and the
  loop segments no kept triangle covers, with the helpers of the holes some of whose segments it
  holds. No point is added, so the face's boundary stays the one its neighbours share; the
  decision ignores the thread count, so one thread makes the same mesh; and any trouble (a
  remainder constraint that would split, a kept triangle on a helper) triangulates the face whole
  instead. On the 6,784-point top of a plate with 113 holes six strips keep 6,686 of its 7,008
  triangles and twelve keep 6,654 (a reach a quarter of a strip's points kept 82% in thirteen),
  leaving the remainder about 0.2 ms: the fans along its long straight sides, whose circumcircles
  leave every strip
  (`a_large_face_triangulated_in_pieces_is_covered_once_without_gaps` and
  `a_reach_sized_by_the_widest_gap_keeps_the_triangles_of_narrow_strips`, at least nine tenths
  kept, on staggered and mirror-symmetric plates, the latter in sixteen strips narrower than two
  reaches). On 16 threads of 8 cores six strips still take that face from 3 ms to about 1.4 ms
  (the share-sized reach took 1.3 ms) and more strips do not pay, since each strip triangulates its
  core and two reaches; on a face of 44,000 points with an interior lattice sixteen strips each
  triangulate a quarter fewer points than with the share-sized reach, 5.1 ms against 5.3 ms.
- A face makes a `FacePatch` (`patch.rs`): its interior points, its vertices, each on a boundary
  position or one of its own interior points, and its triangles in its own indices. Patches are
  placed in face order, numbering positions, vertices and triangles exactly as one thread would
  (`faces_meshed_on_many_threads_make_the_mesh_of_one_thread`); the point budget is checked as each
  is placed, against the positions placed before it. Its `PatchShape` records the grid size, the
  points triangulated and whether pieces were allowed.
- A vertex is made only for a triangle that is kept, in the order triangles are emitted, so each
  face's vertices and interior positions are contiguous in the mesh and in order of first use. The
  app's `ShadedMesh` relies on it to read a body's mesh in place (`render.md`).
- `Solid::display_mesh_reusing(quality, earlier)` returns a `DisplayMesh`: the mesh and, per face,
  a key (`reuse.rs`) of everything triangulating it reads: surface, sense, tolerance, pole
  density, and its boundary loops in uv with their position labels. A face whose name has a key
  in the earlier mesh equal in all of that (uv to the bit, labels equal in the same pattern) reads
  its patch back from the earlier mesh, labels mapped, instead of being triangulated, and the mesh
  is the one meshing afresh gives (a patch whose pieces decision would differ in the new body is
  made again). The display tolerance follows the solid's extent rounded to the
  nearest power of two (`MeshQuality::display_tolerance`, within a factor √2 of the quality's
  chord), so an edit that changes the body's box a little (a longer extrusion) keeps the tolerance
  and every face it left alone (`a_lengthened_extrusion_reuses_the_mesh_of_the_faces_it_left_alone`);
  one crossing a step meshes every face again. `display_mesh_costs` (ignored, release) times both:
  on 16 threads a 119-face plate takes 9 ms (11.5 ms with its top triangulated whole) against
  44 ms on one, and 5.3 ms when one face is added.

## Quality

- `MeshQuality` (`tolerance.rs`) is a chord as a fraction of the solid's extent (bounding-box
  diagonal) plus an angle per segment. `COARSE` is `Solid::default_tolerance` and
  `SamplingTolerance::for_extent` (validation, profiles, tests); `SMOOTH` is the default display
  quality. The angle bounds segments per turn whatever the radius; the relative chord takes over on
  radii large against the solid.
- `Solid::display_mesh(quality)` meshes within `DISPLAY_POINTS` at the stepped display tolerance.
  Any failure but cancellation (over the budget, a boundary still crossing after the retries)
  meshes again at the quality made at least as coarse as `COARSE` (`MeshQuality::at_least`) within
  `MAX_POINTS`: a body is shown coarser rather than not at all.
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
