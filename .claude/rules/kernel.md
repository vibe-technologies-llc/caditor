---
paths:
  - "crates/caditor-kernel/**"
---

# Kernel

- Own B-rep kernel, no truck or OpenCascade; depends only on `caditor-geometry`.

## Cancellation (`interrupt.rs`)

- `interruptible(interrupt, work)` installs a per-thread check while `work` runs. Polled in
  intersections (subdivision pair, march step, each midpoint refining a traced curve, which gives up
  the curve when interrupted), booleans (edge, face pair, face, fragment, split,
  branch, healing, between phases), profiles (curve, pair, spline segment, 256 box tests, face),
  sweeps and `Plan::build` (region, face), `next_face` (ray, and once before answering), shells
  (corner, edge, round of dropped faces), blends (edge, tool, corner), patterns (copy, union) and
  tessellation (face).
- Each fails with a `Cancelled` variant of its error; a nested cancellation becomes the outer one's.
  A boolean, shell or blend failing for any reason while its interrupt is set reports `Cancelled`
  (also while validating: `BuildError::interrupted`). `Solid::find_crossing` polls once more before
  answering.
- The document installs its `CancelToken` around evaluation and meshing; export around its meshing
  and STEP writing.

## Tolerances (`tolerance.rs`)

- `LINEAR_RESOLUTION` is 1e-6 mm; `ANGULAR_RESOLUTION` moves a point at `MODEL_EXTENT` (10 m) by it.
  `SamplingTolerance` (chord, angle) drives sampling; `Solid::default_tolerance` derives it, and
  `MeshQuality` scales one to a solid's extent (`kernel-tessellation.md`).
- Constructors reject non-finite and degenerate input (radii below `LINEAR_RESOLUTION` or above
  `MAX_SIZE`, a kilometre; extents beyond it), each with own error. Iterations are bounded; failures
  are errors, never panics.

## Curves

- `Curve` (line, circle, ellipse, B-spline, intersection) and `Curve2` (line, circle, B-spline)
  share one `BSpline<P>` (clamped, optionally rational, degree up to 9) and generic sampling, length
  and closest-point code.
- Lines run by arc length, circles and ellipses by angle in a `Plane` frame, splines over their
  knots. Reversal maps t to `reversal_pivot() - t`.
- Closest points are analytic for lines and circles, else seeded by sampling and refined by
  bracketed Newton.
- `Curve::Intersection(IntersectionCurve)` lies on two surfaces it carries: nodes refined onto both
  (point, unit tangent, uv on each), cubic Hermite segments subdivided until each midpoint is within
  `INTERSECTION_TOLERANCE` (a quarter of `LINEAR_RESOLUTION`) of the true one.
- Closed ones are periodic over their length; sampling seeds are the nodes less those within a
  thousandth of the resolution of each other or of the range ends. `uv_at` and `refined_point`
  re-project onto both surfaces; `trimmed` returns a sub-range with the same parameters and shape.
- `IntersectionCurve::through` rebuilds one from rough points (an imported edge off its faces): each
  point solved onto both surfaces in its normal plane; where they only touch (tangent fillet edge),
  by alternating projection accepting a gap's middle up to `LINEAR_RESOLUTION`. Only this path
  follows touching surfaces.

## `Surface`

- Kinds: plane, cylinder, cone, sphere, torus, extrusion, revolution, `BSplineSurface`
  (tensor-product, clamped, optionally rational, degree up to 9).
- A spline surface whose first and last rows or columns meet is periodic there (C0 suffices). A row
  collapsed to a point is a pole; a column cannot be, so importers transpose and flip the face.
  Poles are found once, when the surface is built.
- Spline surfaces evaluate second derivatives exactly (rational by the quotient rule), and a point
  alone without them, bit for bit the same point. A uv box is bounded by its own control net, cut
  out of the spans by knot insertion in homogeneous coordinates (so the hull holds for rational
  surfaces; the spans' net when the box wraps a closed direction), so sub-patches shrink as divided.
- Spline projection refines the hint first (its foot is the answer when on the surface), then the
  three nearest samples of a precomputed grid (searched in 8×8 blocks with their boxes), keeping the
  hint's foot only when as close as the best; a sample whose iterate comes within the resolution of
  a foot already found takes that foot.
- Refinement (`projection.rs`) is damped Newton on the squared distance: halving stops once the
  predicted decrease is within rounding of the distance (scaled by the point's magnitude), then up
  to two full Newton steps not measurably worse settle the foot, landing on the true foot to
  rounding instead of stalling.
- `project` returns the periodic representative nearest a hint, else the principal one in
  [0, period). On spline profiles it keeps the closest point near the hint unless another is closer
  by more than the resolution, so self-crossing profiles project consistently. A point within the
  resolution of a pole (`pole_at`) takes the hint's u, its own angle being rounding noise.
- u is the angle around the axis (frame normal) on rotational surfaces; v: cone slant distance from
  its reference circle, sphere latitude, torus tube angle, revolution profile parameter; an
  extrusion is (profile parameter, distance) and refuses a line within a millionth of a radian of
  its direction. du × dv points outward on every elementary surface.
- A `Revolution`'s profile must lie in a plane through its axis (sampled within ten resolutions),
  else `GeometryError::ProfileOutsideMeridian`; `project_seed` relies on it, and a STEP
  `SURFACE_OF_REVOLUTION` that is skew is refused with that message naming its entity.
- Singularities are always `Pole`s: v isolines where du vanishes (sphere poles, cone apex, a
  revolution profile ending on its axis).
- `same_surface` gives the `Sense` between coincident surfaces' normals: analytic for elementary
  pairs, sampled mutual projection with an extrusion or revolution.

## Topology

- A `Solid` is an arena of vertices, edges, coedges, loops, faces and shells behind typed ids, built
  by `SolidBuilder::build`, which validates. An edge: curve, interval, two vertices (one if closed).
- A coedge has a sense and a pcurve: a uv polyline carrying each sample's edge parameter, exact
  ends, chords within `PCURVE_TOLERANCE` in space, continuous across seams.
- A face's first loop is its outer one; loops run counter-clockwise about the face normal (in uv the
  outer loop is counter-clockwise when the face sense is `Same`). A face wrapping a periodic surface
  has a seam edge used twice in its loop, opposite senses, one period apart in uv.
- `add_loop` fits pcurves by chaining projection hints, bisecting steps over a quarter period;
  shifts a seam's second copy a period when the chain put both on one side; puts the first loop in
  the principal period and later loops, by whole periods, into the outer loop's range.
- Poles have no degenerate edges: the pole is a vertex; the uv loop closes along the pole line
  between the two coedges meeting there; a pcurve end at a pole takes its v exactly.

## Validation

- `Solid::validate` checks a closed, oriented 2-manifold whose geometry agrees with its topology and
  returns the first `ValidationError` with ids: edge uses and senses; loop chaining in space and uv;
  vertices on curve ends; edges on both surfaces; pcurves on their edges; loop winding and nesting;
  shell connectivity; Euler–Poincaré per shell (each fan of faces at a vertex counts as one vertex,
  so a pinched shell has the characteristic of the surface it pinches); an edge used twice by one
  face only as a seam, its two pcurves apart in the domain (a period, a closed spline extrusion's
  domain ends, a self-crossing profile's two parameters), never at the same place (a dangling
  slit); positive volume for lumps, each void inside exactly one lump, lumps neither overlapping,
  nesting nor coinciding.
- Volume checks use a coarse mesh sized by the box of the edges and vertices (never the
  classifier-based `bounding_box`), retrying finer before reporting a void outside its lump, a
  shell of no volume (a thin lens is empty on a coarse mesh) or lumps that overlap or coincide,
  over four chords (20, 1, 0.1 and 0.01 times the coarse one).
- Lumps (`topology/lumps.rs`) are checked only against shells whose boxes overlap theirs, by
  probes exactly on the shell: mesh corners, 15 points along each edge and triangle centroids
  projected onto their surfaces, spread evenly to at most 1,024 near another shell. At each probe
  the material depth of the other shells (outward ones +1, voids −1, by mesh parity) must be 0 on
  a lump and 1 on a void. A probe within twice the nearest triangle's deviation from its surface
  (plus `LINEAR_RESOLUTION`) of another shell is touching and decides nothing, so lumps touching
  at a point, along a line or over a face validate; a shell all of whose probes touch one other
  shell coincides with it. An overlap shallower than that band, or narrower than the probe
  spacing, is not seen.
- Validation never intersects faces with each other, since every build runs it; `find_crossing`
  does, for importers.

## Measuring (`measure/`)

- `distance` of two `Element`s (a point, an edge or a face of a solid) returns a `Separation`: the
  closest points and an `Accuracy`. It is `Exact` only when every step was closed form (points,
  closest points on lines and circles, projection onto planes, cylinders and spheres, segment to
  segment, a segment crossing a plane); anything found by seeding and alternating projection is
  `Approximate`.
- The minimum over a bounded item is its interior critical pair or one on its boundary, so faces
  recurse into their edges and edges into their ends; edge pairs whose boxes are farther apart
  than the best so far are skipped. Two flat faces need no interior search.
- Interior pairs are seeded (48 samples per curve, a 16×16 uv grid inside a face) and the best 4
  refined by alternating closest points, kept only when strictly inside the edge and inside the
  face (`SolidClassifier::point_in_face`).
- `angle`: straight edges sharing an end give the angle inside the corner (0 to 180°), otherwise
  the acute angle between the lines; flat faces the acute angle between the planes; a line and a
  plane the angle between them.
- `edge_measure` (length, exact for lines and circles; line, circle or ellipse form), `face_form`
  (plane with outward normal, cylinder, cone, sphere, torus), `planar_area` (Gauss–Legendre over
  the boundary, exact for lines and circles) and `axis_of`/`axis_separation` for round edges and
  faces.

## `Solid::find_crossing` (for importers)

- A face's edges (seams aside) are intersected with each other; a transversal point or overlap away
  from shared vertices is a `Crossing`.
- Each pair of faces with overlapping boxes (`box_tree.rs`) is intersected; a branch point strictly
  inside both faces (coincident faces: a sample) is a `Crossing`.
- Neighbours skip the surface pair (the shared edge is the known branch): each one's other edges are
  intersected with the other's surface; a transversal point or overlap strictly inside the other
  face is a `Crossing`.
- A failed pair is not skipped: the result is a `CrossingCheck` (`Clear`, `Crossing`, or
  `Inconclusive` naming the first such pair when no crossing was found); STEP import keeps an
  inconclusive solid with a note naming the face entities.
- `bounding_box` covers the edges, a grid inside each doubly curved face, a sphere's axis extremes.
