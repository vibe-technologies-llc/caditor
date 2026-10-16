---
paths:
  - "crates/caditor-kernel/src/tessellation/**"
---

# Tessellation

## Sampling and triangulation

- Each edge is sampled once; both faces share its positions. Edges use `Curve::sample_sparingly`:
  adaptive samples seeded with 2×degree points per knot span are merged (`merged_parameters`, at
  most `MAX_MERGED_SAMPLES` into one) while a chord from the last kept sample holds every dropped
  sample and sub-span middle within the chord tolerance and angle, never across a tangent jump (a
  degree-1 spline's corners). A traced thread edge with a knot at every node so gets the points its
  angle asks for, not one per seed. Validation and profiles keep `Curve::sample`.
- Each face is a constrained Delaunay triangulation (`spade`) of its loops in (u, v) plus an
  interior tensor grid (`density.rs`) kept clear of the boundary. Grid lines are graded per
  direction by curvature: the need (cells per unit parameter) is sampled on a lattice (at most
  `MAX_LATTICE`), each lattice span takes the larger need of its ends, and lines sit at equal steps
  of accumulated need. No span may need more than `MAX_SEGMENTS` across the face (a pole's
  degenerate curvature must not attract every line). A direction without curvature gets
  `FLAT_SEGMENTS` cells whatever its length. On a rotational surface (u the angle round its axis)
  the need along u is at least one cell per `SamplingTolerance::angle` of turn, what a cylinder's
  curvature gives, since the normal curvature of a nearly flat cone's u lines is near zero while
  they still turn: with too few columns its cells spanned a half turn and triangles reached across
  a hole round the axis.
- Curved kinds are refined a whole direction at a time until sampled cells bow from their chords,
  measured along the surface normal (ignoring parameter stretch), by at most `GRID_SHARE` of the
  chord tolerance; a cell whose diagonal bows more than its sides refines both directions.
- A face with a grid is triangulated in cell space (each parameter mapped piecewise linearly to its
  grid line index), so an off-lattice boundary point never fans across a dense row (it spanned the
  tube of a thin torus); a face without a grid is scaled by the mean surface speeds. Gaps along a
  pole line or seam are filled in cell space, one point per cell crossed.
- Points are deduplicated by exact scaled coordinates (`DuplicateBoundaryPoint`), inserted in a
  biased randomised order (`insertion.rs`: rounds doubling in size from a fixed-seed shuffle, each
  Z-order sorted), then joined by the loops' constraint edges. Loop order flips edges
  quadratically and spade's bulk load walks its hull quadratically for points along one curve.
  Triangles are kept by the parity of constraint crossings from outside.
- Cocircular points (round holes, the interior grid) send spade's in-circle predicate down its slow
  exact path. So a helper point is inserted first at the vertex centroid of each inner loop
  turning against the outer one that sees all its edges (`constrained::outline`, a `Hole`): the
  hole becomes a fan whose triangles are outside the face and dropped (a kept triangle on a helper
  triangulates again without helpers); no face point moves. Interior grid points are nudged in the
  mapped plane only, by a fixed `scatter` of their cell of at most `GRID_NUDGE`; uv and positions
  are untouched.
- Boundary points at one vertex whose parameters differ by a spatially negligible gap are merged,
  and a pinched vertex takes the uv of its first pass. The gap may reach
  `OFF_POLE_JOINT_PARAMETER_GAP` (relative) away from poles, but only `JOINT_PARAMETER_GAP` at a
  pole, where every u is one point and the pole line must be kept.
- Where two boundary polylines leave a point along the same chord (a crescent tip, internally
  tangent circles, a cusp), the longer end segment is bisected first (at most `MAX_END_PARTINGS`
  rounds) for every face of the edge, so the loops part by the curves' own separation, not rounding.
- Pole-line points share the pole's position and collapsing triangles are dropped, keeping the mesh
  watertight. A straight edge ending at a pole (a ruling to a cone's apex) is sampled one piece per
  grid row crossed, else triangles beside it fan from the apex with no area.

## Threads and reuse

- Edges are sampled on the calling thread, which numbers their positions in edge order; pole
  samplings, and faces once overlapping ends are parted, run on scoped threads (`parallel.rs`: up
  to the available parallelism, at least `FACES_PER_THREAD` faces each). Helpers run under the
  caller's interrupt; a panic in one is resumed on the caller.
- A face of at least `SPLIT_POINTS` points holding at least a `DOMINANT_SHARE`th of the body's
  positions placed before it (`pieces::allowed`) is triangulated in pieces (`pieces.rs`), so the
  largest face does not bound the body's time. The mapped plane is cut into vertical strips of
  equal point counts, each triangulated on its own thread from the points within its reach, every
  loop segment overlapping it and every hole whose span meets the reach, whole with its helper.
  - A reach is a distance: the core widened each side by `REACH_GAPS` of the face's widest gap, the
    edge moved within `EDGE_SLACK` of that to where the fewest holes straddle it, then to the
    nearest gap between points. The widest gap (`gaps.rs`) is the radius of the largest empty disc
    centred inside the face, read from an occupancy grid of about one cell per point (chamfer
    distance), plus `CELL_ERROR` cells so it never falls short; it is computed beside the sort of
    points along x (`parallel::both`).
  - Strips number one per `POINTS_PER_PIECE` points, at most `MAX_PIECES`, and no more than leave
    each core `STRIP_REACHES` reaches wide. A cut sits `CUT_SHARE` of the way between neighbouring
    points, not halfway: a mirror-symmetric face otherwise cuts on its axis, where circumcentres of
    straddling cocircular cells lie, and every such triangle failed the strict check.
- A strip keeps a triangle inside the face (parity spread among its triangles within the reach,
  seeded by a vertical ray) whose circumcircle stays within its reach (so it belongs to the whole
  face's triangulation) and which it owns: the strip whose core holds the circumcentre, computed
  from corners in index order so every strip gets the same bits. A centre within `BAND_SHARE` of the
  face's width of a cut, or with `Circle.error` over a quarter of that, is kept only when strictly
  Delaunay: every neighbour across a non-constraint edge has its far corner outside the circle by
  more than the error bound, which a cell of four or more cocircular points never has. So no two
  strips keep overlapping or equal triangles. One more triangulation covers the rest: points no kept
  triangle uses or on the kept triangles' border (edges two strips both border cancel), constrained
  by that border and the loop segments no kept triangle covers, with the helpers of holes some of
  whose segments it holds. No point is added, so the boundary neighbours share is unchanged; the
  decision ignores the thread count, so one thread makes the same mesh; any trouble (a remainder
  constraint that would split, a kept triangle on a helper) triangulates the face whole. Tests
  `a_large_face_triangulated_in_pieces_is_covered_once_without_gaps` and
  `a_reach_sized_by_the_widest_gap_keeps_the_triangles_of_narrow_strips` (at least nine tenths kept,
  on staggered and mirror-symmetric plates).
- A face makes a `FacePatch` (`patch.rs`): interior points, vertices (each on a boundary position
  or its own interior point) and triangles in its own indices. Patches are placed in face order,
  numbering positions, vertices and triangles exactly as one thread would
  (`faces_meshed_on_many_threads_make_the_mesh_of_one_thread`); the point budget is checked as each
  is placed. Its `PatchShape` records grid size, points triangulated and whether pieces were allowed.
- A vertex is made only for a kept triangle, in emission order, so each face's vertices and interior
  positions are contiguous and in order of first use. The app's `ShadedMesh` relies on it to read a
  body's mesh in place (`render.md`).
- `Solid::display_mesh_reusing(quality, earlier)` returns a `DisplayMesh`: the mesh and, per face,
  a key (`reuse.rs`) of everything triangulating it reads besides its surface: tolerance, pole
  density (boxed, rare), a 128-bit digest (`Digest`) of the boundary loops' lengths and uv bits, and
  the loops' position labels in one array the whole mesh shares; faces are one `Vec` sorted by name.
  The surface and sense are not copied into the key: `earlier` is a `MeshedSolid`, the earlier mesh
  with the solid it was made from, and a candidate's surface and sense are read from that solid's
  face at the key's slot. A face key is a few dozen bytes plus four per boundary point, since an
  assembly of millions of faces keeps one per face (a full surface per key held 1.2 GiB of the
  VZ330). A face whose name has an equal key in the earlier mesh (uv equal by digest, labels equal
  in pattern) reads its patch back, labels mapped, instead of being triangulated; the result equals meshing afresh (a patch whose pieces decision would differ in the
  new body is made again). The display tolerance follows the solid's extent rounded to the nearest
  power of two (`MeshQuality::display_tolerance`, within √2 of the quality's chord), so a small edit
  of the box keeps the tolerance and every untouched face
  (`a_lengthened_extrusion_reuses_the_mesh_of_the_faces_it_left_alone`); crossing a step meshes
  every face again. `display_mesh_costs` (ignored, release) times both.

## Quality

- `MeshQuality` (`tolerance.rs`) is a chord as a fraction of the solid's extent (bounding-box
  diagonal) plus an angle per segment. `COARSE` is `Solid::default_tolerance` and
  `SamplingTolerance::for_extent` (validation, profiles, tests); `SMOOTH` is the default display
  quality. The angle bounds segments per turn whatever the radius; the relative chord takes over on
  radii large against the solid.
- `Solid::display_mesh(quality)` meshes within `DISPLAY_POINTS` at the stepped display tolerance.
  Any failure but cancellation (over budget, a boundary still crossing after the retries) meshes
  again at the quality made at least as coarse as `COARSE` (`MeshQuality::at_least`) within
  `MAX_POINTS`: a body is shown coarser rather than not at all.
- Export does not use it: it has its own `MeshResolution` (`file-import-export.md`).

## `Mesh`

- Vertices are per face and carry the surface's analytic normal (flipped by the face sense, nudged
  into the triangle at a pole): curved faces shade smoothly, the edge between a cylinder and its
  cap stays crisp. Edge polylines are the samplings the faces used, so outlines follow silhouettes.
- Positions stay f64 (`Point3`), but a normal is a display attribute, so a `MeshVertex` is a
  position index and a `MeshNormal` (`normal.rs`): octahedral, two `i32` snorms, zero or non-finite
  kept as the corner code (-1, -1) and a direction rounding onto it taking the opposite corner
  (both are -Z). `vector()` comes back within 1e-8 rad (finer than the f32 the app's analyses
  and the GPU read), the axes exactly. A vertex is 12 bytes, not 32
  (`a_packed_normal_is_eight_bytes_and_a_mesh_vertex_twelve`); a mesh of the VZ330 holds about 150
  MiB of them, not 400. Nothing exact reads normals: mass properties, validation's volume checks
  and every export (facet normals from positions) use positions alone.
- `mass_properties` (volume, area, centroid, second moments) by the divergence theorem over the
  triangles: what validation measures, and the fallback for a face exact integration gives up on
  (`kernel.md`). `Mesh::chord` records the chord asked for (retried faces only get closer); the app
  quotes it as the accuracy of such mass properties.

## Retries and limits

- A face boundary crossing itself at the requested tolerance retries up to `MAX_REFINEMENTS` times,
  halving chord and angle for the faces that crossed and the edges they bound (an edge takes the
  finest tolerance of its faces); only what changed is sampled or triangulated again.
- A mesh holds at most `MAX_POINTS`, counted before insertion; more fails as `TooLarge`, which
  export names as a body too fine for the resolution.
