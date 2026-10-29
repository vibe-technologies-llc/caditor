---
paths:
  - "crates/caditor-kernel/**"
  - "crates/caditor-geometry/**"
---

# Kernel

- Own B-rep kernel, no truck or OpenCascade; depends only on `caditor-geometry`.
- `caditor-geometry` is the f64 vocabulary under it and every crate above: `glam` aliases
  (`Point3`, `Vector3`, `Rotation3`), `Plane` (origin, normal, x axis, orthonormal), `Ray`, `Aabb`,
  `Aabb2`, `RigidTransform`, `RigidTransform2` and `Similarity` (rigid motion with a scale and an
  optional mirror). Its only tolerances keep frames orthonormal; model tolerances are the kernel's.
- Operations return a valid solid or an error naming what failed; iterations are bounded and
  constructors reject non-finite and degenerate input with a `GeometryError` each.
- `LINEAR_RESOLUTION` is the one absolute length tolerance; `ANGULAR_RESOLUTION` moves a point at
  `MODEL_EXTENT` by it; `MAX_SIZE` bounds radii and extents. `SamplingTolerance` (chord, angle)
  drives sampling; `MeshQuality` scales it to a solid's extent (`kernel-tessellation.md`).

## Cancellation (`interrupt.rs`)

- `interruptible(interrupt, work)` installs a per-thread check; every loop that can run long polls
  `interrupt::check`. A new long loop must poll.
- Each operation's error has a `Cancelled` variant, and a nested cancellation becomes the outer
  one's (a cancelled pcurve fit is `BuildError::Cancelled`, not a missing pcurve). A boolean, shell
  or blend failing for any reason while its interrupt is set reports `Cancelled`
  (`BuildError::interrupted` covers validation).
- A classification or `first_crossing` whose ray intersection is cancelled answers `Undecided` at
  once; its caller's next poll reports the cancel.
- The document installs its `CancelToken` around evaluation and meshing; export around meshing and
  STEP writing; import around reading STEP and meshes. `check_interrupt` lets a crate above the
  kernel poll the same check, and `current_interrupt` lets work fanned out to other threads
  install it there.
- Tests bound an operation by its interrupt polls (`test_support::polls_during`), which are
  deterministic and count work from every thread, or by `tally` units for what has no poll: box
  tree nodes visited, intersection-curve segments handed to a closest-point search, half-edges of
  the cycles `Arrangement::contains` tests. Never by time (`rust-style.md`).

## Curves and surfaces

- `Surface` boxes its extrusion, revolution and B-spline variants, so it is sized by a cone or
  torus and a `Face` stays small (`faces_and_their_geometry_stay_small_with_large_surfaces_boxed`
  bounds `Face`, `Surface` and `Curve`); build one with `Surface::from`. `Curve`'s variants are
  close in size and unboxed. `Solid::owned_size` counts each boxed surface (`Surface::boxed_size`).
- `Curve` and `Curve2` share one `BSpline<P>` and generic sampling, length and closest-point code.
  Lines run by arc length, circles and ellipses by angle in a `Plane` frame, splines over their
  knots; reversal maps t to `reversal_pivot() - t`.
- `CurveDerivatives::curvature` (and `Curve::curvature`) is the curvature vector: towards the
  centre, length one over the radius, zero on a line, `None` at zero speed. An intersection
  curve's is its Hermite segments', so approximate.
- Closest points and `project` take a hint: the search refines near it first and samples the whole
  range only when that foot is not within the resolution; the hint's foot is kept unless another is
  closer by more than the resolution, so self-crossing profiles project consistently.
- Whole-range searches are pruned by box trees, built on first use and shared by clones: a spline
  of at least `MIN_PRUNED_SPANS` spans samples only runs of spans whose control hull comes no
  farther than the nearest any hull reaches (`BSpline::nearby_runs`); an extrusion or revolution
  whose profile has at least `MIN_INDEXED_SPANS` spans indexes its span hulls
  (`surface/profile_spans.rs`); an intersection curve of at least `MIN_INDEXED_SEGMENTS` node
  segments indexes its segments' hulls and keeps the length travelled to each node, so its length
  is a difference and two partial integrals.
- Surface `project` returns the periodic representative nearest the hint, else the principal one in
  [0, period). A spline surface seeds from three samples per knot span (capped at 48 a direction);
  a net with more spans also keeps a box tree of span hulls, and a point no seed reaches within the
  resolution is sought from a few samples in each span whose hull comes nearer than half the best
  foot so far (`SPAN_SEARCH_SHARE`), nearest hull first, stopping at the first one farther
  (`spans_near`, at most `MAX_SPAN_SEARCHES`); hulls not holding the point count only within
  `OUTSIDE_SHARE` of the net's size, and at most `MAX_OUTSIDE_SPANS` of them. A point just beyond
  the side of a strip winding many turns (a thread flank) lies in no hull, and its seeds' foot was
  a whole turn away. Refining is Newton on the squared distance, Gauss–Newton when the Hessian
  is not positive definite; a one-direction step is taken only when the other direction's term is
  zero, never negative (that stuck projections on a spline's corner).
- Splines reach `MAX_SPLINE_DEGREE` (25). Evaluation keeps its basis tables on the stack at the
  width the degree needs (`CUBIC_WIDTH`, `NARROW_WIDTH` up to degree 9, `WIDE_WIDTH`).
- `BSpline::interpolating` passes a clamped spline through points at chord-length parameters
  (banded solve, no pivoting); `interpolating_at` does it at given increasing parameters so grid
  rows share knots (the STEP reader's fitted offsets).
- `BSpline::restricted` is the exact piece over a range, by knot insertion in homogeneous
  coordinates (drawing export uses it). `BSpline::restarted_at` traces a closed spline (ends within
  the resolution) from a parameter round to it again, joining the two restricted pieces C0. `BSplineSurface::restricted` is the same over a uv box,
  `side` gives a side as a curve, `extended_evaluate` continues the boundary spans' polynomials past
  the domain for closest points that must not clamp, and `extended` makes that continuation a
  surface: each open side not a pole, nor within a given distance of its opposite, grows by a
  fraction of its boundary span, shrinking the reach while a weight would turn non-positive.
- `BSplineSurface::bent_side` refits one side (`SideBend`): the side curve, with the requested
  knots (snapped to an existing one within `KNOT_SNAP`) and a C0 knot at each interior pin, is
  fitted by banded least squares to sampled `BendTarget`s, pins and held corners fixed and the
  displacement smoothed; spans missing a strict target are halved up to `MAX_REFINEMENTS` rounds and
  `MAX_SIDE_CONTROL_POINTS`. Only the side row moves, so the surface changes only across its first
  span from the side. A linear side is raised to cubic first; a seam, pole or closed side is
  `BendError::Unbendable`.
- Lengths are compared through `is_longer_than` (chord, then seed polyline, then the integral) and
  capped with `length_up_to`; building, validation, imprinting and profile arrangement never
  measure a whole spline to compare it with a tolerance. Tracing bounds candidates at a vertex by
  the seed polyline (`polyline_length_up_to`, polling every `SEEDS_PER_POLL` seeds), shortest chord
  first, each capped by the shortest so far.
- `Curve::Intersection(IntersectionCurve)` lies on two surfaces it carries, within
  `INTERSECTION_TOLERANCE`. Building one halves each Hermite segment at the point on both surfaces
  across its middle until the points across its middle and quarters lie within the tolerance and its
  tangents turn at most `MAX_SEGMENT_TURN`; the middle alone passed a segment bowing to opposite
  sides of it, found only by validation as `EdgeOffSurface`. A segment that cannot be halved (no
  point across its middle, that point at one of its ends, `MAX_REFINE_DEPTH` or `MAX_NODES`) is kept
  only when those three points lie within the tolerance of both surfaces, else building fails as
  `IntersectionError::Unrefined` with the farthest point, its distance and the `RefineStop`.
  Segments within twice `LINEAR_RESOLUTION` are kept. `IntersectionCurve::through` rebuilds one from
  rough points (an imported edge off its faces) and is the only path that follows surfaces that
  merely touch (a tangent fillet edge).
- A spline surface whose first and last rows or columns meet is periodic there (C0 suffices). A row
  collapsed to a point is a pole; a column cannot be, so importers transpose and flip the face.
  Singularities are always `Pole`s (sphere poles, cone apex, a revolution profile ending on its
  axis); a point within the resolution of one takes the hint's u.
- u is the angle around the frame normal on rotational surfaces; v: cone slant distance from its
  reference circle, sphere latitude, torus tube angle, revolution profile parameter; an extrusion
  is (profile parameter, distance). du × dv points outward on every elementary surface.
- A `Torus` takes any positive radii: a tube wider than its ring radius is a spindle torus whose
  points past the axis are the self-crossing lemon. Faces on one stay on the outer side, where
  parametrization, normal and projection are a ring torus's; `revolve` makes one only for an arc
  clear of its axis, and the coaxial intersection skips meridian points past the axis.
- A `Revolution`'s profile must lie in a plane through its axis, else
  `GeometryError::ProfileOutsideMeridian` (`project_seed` relies on it; STEP import refuses a skew
  `SURFACE_OF_REVOLUTION` with it).
- `same_surface` gives the `Sense` between coincident surfaces' normals: analytic for elementary
  pairs, sampled for an extrusion, revolution or spline (7×7 grid, plus for a spline surface one
  sample per control point at its Greville abscissae, at most 64 a direction, so one control
  point's bump cannot hide).

## Topology

- A `Solid` is an arena of typed ids built by `SolidBuilder::build`, which validates. An importer
  whose geometry does not validate may still mesh what it built (`SolidBuilder::unvalidated_mesh`,
  a `FaceMesh` naming the face of each triangle) for `faceted_solids`; the unvalidated arena never
  leaves the builder.
- A coedge's pcurve is a uv polyline carrying each sample's edge parameter, exact ends, chords
  within its tolerance in space, continuous across seams. A fitted pcurve's tolerance is
  `PCURVE_TOLERANCE`, or half of how far the edge bows from the chord between its ends when less
  (never below `LINEAR_RESOLUTION`), so a nearly straight short arc keeps an interior sample.
  Fitting halves a step whose ends, or whose middle against either end, lie more than a quarter
  period apart on a periodic surface: a cone's section passing near its apex winds most of a turn
  within one of an ellipse's π/4 seeds, and with the ends alone the pcurve went round the short way.
- A spline edge that is a side of its face's spline surface (neither direction periodic; same
  degree and knots, control points within `SIDE_MATCH`, weights alike, either way round) is
  fitted as the straight pcurve along that side without sampling (`pcurve::along_side`), exact
  since the side's parameter is the surface's: a loft's rows and a fillet's feet and joint arcs,
  whose many spans would otherwise each take six seeds and their projections.
- Geometry data is shared, never copied: pcurve samples, spline knots, control points and weights,
  a spline surface's net and an intersection curve's nodes are `Arc` slices, so cloning is a
  pointer copy and successive solids share what an operation did not change. Anything producing a
  changed copy builds new data; one that would change nothing hands back the same data
  (`Pcurve::with_ends` and `shifted`, `refined_within` inserting no sample, `BSpline::map_points`
  and `with_points` keeping knots and weights, `Solid::mapped` keeping every pcurve whose edge and
  uv maps are identities). `Solid::approximate_size` counts shared data in full in each solid
  holding it; `owned_size` is the solid's own arrays and `shared_buffers` reports each reached
  `Arc` as a `SharedBuffer` of address and bytes, so a caller holding many solids counts each
  buffer once.
- A face's first loop is its outer one; loops run counter-clockwise about the face normal. A face
  wrapping a periodic surface has a seam edge used twice in its loop, opposite senses, one period
  apart in uv.
- Poles have no degenerate edges: the pole is a vertex, the uv loop closes along the pole line
  between the two coedges meeting there, and a pcurve end at a pole takes its v exactly. A seam
  that runs out to a pole and straight back mid-loop (a screw's cone tip) has its return set one
  period over and the later coedges carried along (`SolidBuilder::align_seams`).
- `Solid::bounding_box` is a bound, not an estimate: the box of edges and vertices, widened by each
  doubly curved face's interior extremes. Sphere and torus extremes are closed-form
  (`torus_extremes`) and count when inside the face; every other extreme of those faces lies on an
  edge. A torus whose axis is the box axis has a ring of extremes, sampled (`RING_SAMPLES`).
  Revolution and spline faces add `patch_bounds` of their uv box (loose, never short).
  `interference` answers `Apart` from this box, so it must never fall short.

## Transforms (`mapping.rs`, `topology/mapping.rs`)

- `RigidTransform` moves geometry and leaves every parameter alone (`transformed`); a `Similarity`
  goes through `Solid::mapped`, which keeps every face, edge and vertex name and returns a
  validated solid or a `TransformError`.
- Each curve and surface maps with an affine change of its parameters: lines and intersection
  curves run by length, so their parameters scale; a mirrored circle or ellipse flips its frame
  normal to keep its parameter; a mirrored plane negates v and a mirrored rotational surface turns u
  back (`2π − u`), so du × dv stays outward. A face whose uv map keeps its orientation under a
  mirror (extrusions, spline surfaces) reverses its sense instead. A mirror reverses every loop:
  coedge order, senses and pcurve samples.
- `Curve::similar` and `Surface::similar` map one curve or surface by a `Similarity`; the surface
  comes with the `Sense` of its normal against the mapped parameterisation's (`Reversed` when the
  uv map turns orientation, so a STEP `SURFACE_REPLICA` face flips its sense).
- Enlarging re-traces every intersection edge and any edge drifting beyond `INTERSECTION_TOLERANCE`
  from its faces (`IntersectionCurve::through`, ends pinned to the vertices) and refits its
  pcurves; other pcurves are refined back to `PCURVE_TOLERANCE`. A vertex beyond `MAX_SIZE` or a
  shrunk edge shorter than the resolution is refused.

## Validation

- `Solid::validate` checks a closed, oriented 2-manifold whose geometry agrees with its topology
  and returns the first `ValidationError` with ids. Non-obvious checks:
  - Euler–Poincaré per shell, each fan of faces at a vertex counting as one vertex, so a pinched
    shell has the characteristic of the surface it pinches;
  - an edge used twice by one face only as a seam, its two pcurves apart in the domain: farther
    apart on the surface than `SEAM_SEPARATION`, or half a period or more apart in uv, so a seam
    piece ending at a cone's apex is not taken for a slit;
  - positive volume per lump, each void inside exactly one lump, lumps neither overlapping,
    nesting nor coinciding.
- Volume checks mesh coarsely, sized by the box of edges and vertices (never the classifier-based
  `bounding_box`), retrying finer (`VALIDATION_COARSENESS`) before reporting a void outside its
  lump, an empty shell (a thin lens is empty on a coarse mesh) or lumps that overlap or coincide.
- Lumps (`topology/lumps.rs`) are checked by probes exactly on each shell: the material depth of the
  other shells (outward +1, voids −1, by mesh parity) must be 0 on a lump and 1 on a void. A probe
  within `DEVIATION_ALLOWANCE` times another shell's mesh deviation is touching and decides
  nothing, so lumps touching at a point, line or face validate; an overlap shallower than that band
  or narrower than the probe spacing is not seen. Nearest triangle and ray parity come from a box
  tree (`TriangleIndex`, boxes widened to every triangle a ray may cross or graze) so up to
  `MAX_PROBES` probes stay cheap on fine retries; each probe polls the interrupt.
- Validation never intersects faces with each other, since every build runs it; `find_crossing`
  does, for importers.
- `Solid::lumps` takes a solid apart into one solid per outward shell, each void going with the
  smallest outward shell whose box holds it (`void_shells`, `keeping_shells`); a solid of one shell
  is its own lump. The document makes a body of each separate piece through it. `Solid::joined`
  is the reverse, appending solids' arenas into one solid of their shells and validating it, so
  lumps that overlap are refused.
- A coedge a builder marks settled (`SolidBuilder::settle`, from `PlanPcurve::Settled`) was carried
  unchanged (edge curve, interval, face surface, pcurve) from a valid solid, so its geometry check
  against the edge is skipped; nothing else is.

## `Solid::find_crossing` (for importers)

- Returns a `CrossingCheck`: `Clear`, `Crossing`, or `Inconclusive` naming the first face pair it
  could not decide when no crossing was found (a failed pair is never skipped); STEP import keeps
  an inconclusive solid with a note naming the face entities.
- Edges of one face are intersected with each other; face pairs with overlapping boxes
  (`box_tree.rs`) are intersected, a branch point strictly inside both faces being a `Crossing`
  (neighbours share the known branch, so their other edges are tested against the other face).
- Past `PAIRS_PER_THREAD` pairs a thread, pairs are probed in parallel (`probed`, scoped threads
  with the caller's interrupt), each worker taking the next pair in order and none taking one beyond
  the first crossing found so far, so the answer is the sequential one: first crossing in pair
  order, else the first inconclusive pair. A lead screw's thread is the case it serves.

## Faceted solids (`faceted/`)

- `faceted_solids` turns a triangle mesh into one solid per closed shell. It welds vertices within
  a billionth of the diagonal or `LINEAR_RESOLUTION`, whichever is larger, looking in the
  neighbouring cells too (a corner's copies a hair apart across a cell border once survived as an
  edge too short to build), drops degenerate and repeated triangles, closes holes of up
  to `MAX_FILLED_HOLE_EDGES` edges by a fan, orients triangles and shells consistently, and leaves
  out open, self-folding and empty shells and a shell inside out within a larger one (a sealed
  hollow), counting each in `MeshRepairs`. A corner past `MAX_SIZE` is `FacetedError::TooFar`,
  since the weld's cells would no longer tell points apart.
- Edge-joined triangles with agreeing normals and corners within coordinate-magnitude noise of the
  first one's plane become one planar face; each shared corner moves to the least-squares point on
  the fitted planes, so every edge lies within `LINEAR_RESOLUTION` of its faces. A face whose
  corners cannot be placed, whose boundary pinches or has no single outer loop is split back into
  triangles; after `MERGE_ATTEMPTS`, or when the build fails, the shell keeps all its triangles. A
  corner on a straight edge between two such faces is left out, so a box imports with 12 edges.
  More than `MAX_FACETED_FACES` faces in all is refused. `FacetedSolids::sources` lists, per solid
  and face, the input triangles it was made of (hole-closing fans have none).

## Measuring (`measure/`)

- `distance` returns a `Separation` (closest points and an `Accuracy`): `Exact` only when every
  step was closed form, `Approximate` as soon as seeding and alternating projection found it.
  `angle` of straight edges sharing an end is the angle inside the corner (0 to 180°), otherwise
  the acute angle between the lines or planes.
- Besides points, edges and faces of a solid, an `Element` is a free `Curve` over an interval
  (sketch geometry, measured like an edge; `curve_measure` gives its length and circle form), an
  unbounded `Axis` or an unbounded `Plane`. Axis–axis, axis–plane and plane–plane are closed form.
  Against anything bounded, an axis is the segment over the projection of the other element's
  bounds, and a plane is a face that contains every point and has no edges, kept second in a face
  pair so the bounded face seeds the search.
- `mass_properties` (`measure/mass.rs`) integrates area, volume and first and second moments
  exactly over each trimmed face: the divergence theorem gives a surface integral, and Green's
  theorem in the face's uv domain a line integral along its coedges of a strip integral across u
  from the middle of the u range (`∮ H dv`, so edges of constant v and pole lines contribute
  nothing). Each boundary point is the edge's exact point projected onto the surface with the
  pcurve as hint, its dv/dt from the surface's Jacobian. Both integrals are adaptive Gauss–Kronrod
  (`quadrature.rs`, 15 points, bisecting until the 7-point error is within a relative tolerance of
  its own magnitude or an absolute floor scaled to the body), broken at quarter turns, curve and
  surface knots and where a boundary crosses a knot line, so each piece is smooth. Values are about
  the centre of the box of edges, which keeps second moments from cancelling. A face that does not
  converge within `FACE_EVALUATIONS`, or meets a non-finite value, takes its mesh triangles instead
  and is listed in `SolidMass::meshed_faces`.
  `mass_properties_tessellating` takes a sampling tolerance and tessellates only when some face does
  not converge (failure is `MeasureError::Meshing`). `face_area` is the same integral for one face
  (`None` when it gives up).
- `extent` is the tight box, not a bound like `Solid::bounding_box`: vertices, conic edges'
  closed-form extremes, spline and intersection edges' critical points along each axis, sphere and
  torus extremes inside their faces, and for revolution and spline faces whose hull reaches past
  that box, a monotone Newton ascent from the grid's local peaks along each axis, kept when inside
  the face.
