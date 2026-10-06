---
paths:
  - "crates/caditor-kernel/**"
---

# Kernel

- Own B-rep kernel, no truck or OpenCascade; depends only on `caditor-geometry`.
- Operations return a valid solid or an error naming what failed; iterations are bounded and
  constructors reject non-finite and degenerate input with a `GeometryError` each.
- `LINEAR_RESOLUTION` is the one absolute length tolerance; `ANGULAR_RESOLUTION` moves a point at
  `MODEL_EXTENT` by it; `MAX_SIZE` bounds radii and extents. `SamplingTolerance` (chord, angle)
  drives sampling; `MeshQuality` scales it to a solid's extent (`kernel-tessellation.md`).

## Cancellation (`interrupt.rs`)

- `interruptible(interrupt, work)` installs a per-thread check; every loop that can run long polls
  `interrupt::check`. A new long loop must poll.
- Each operation's error has a `Cancelled` variant, and a nested cancellation becomes the outer
  one's (a cancelled pcurve fit is `BuildError::Cancelled`, not a missing pcurve or a split
  failure). A boolean, shell or blend failing for any reason while its interrupt is set reports
  `Cancelled` (`BuildError::interrupted` covers validation).
- A classification or `first_crossing` whose ray intersection is cancelled answers `Undecided` at
  once rather than trying the other directions; its caller's next poll reports the cancel.
- The document installs its `CancelToken` around evaluation and meshing; export around meshing and
  STEP writing.

## Curves and surfaces

- `Curve` and `Curve2` share one `BSpline<P>` and generic sampling, length and closest-point code.
  Lines run by arc length, circles and ellipses by angle in a `Plane` frame, splines over their
  knots; reversal maps t to `reversal_pivot() - t`.
- Closest points and `project` take a hint: the search refines near it first and samples the whole
  range only when that foot is not within the resolution, so chained projections stay cheap, and
  the hint's foot is kept unless another is closer by more than the resolution, so self-crossing
  profiles project consistently. Surface `project` returns the periodic representative nearest the
  hint, else the principal one in [0, period). A spline surface seeds from a grid of three samples
  per knot span, capped at 48 a direction; a net with more spans than that also keeps a box tree of
  each span's control hull, and a point no seed reaches within the resolution is sought from the
  nearest of a few samples in each span whose hull holds it (at most `MAX_SPAN_SEARCHES`), so a
  point on a dense, rough net always finds its foot.
- `Curve::Intersection(IntersectionCurve)` lies on two surfaces it carries, within
  `INTERSECTION_TOLERANCE`. `IntersectionCurve::through` rebuilds one from rough points (an
  imported edge off its faces) and is the only path that follows surfaces that merely touch (a
  tangent fillet edge).
- A spline surface whose first and last rows or columns meet is periodic there (C0 suffices). A row
  collapsed to a point is a pole; a column cannot be, so importers transpose and flip the face.
  Singularities are always `Pole`s (sphere poles, cone apex, a revolution profile ending on its
  axis); a point within the resolution of one takes the hint's u.
- u is the angle around the frame normal on rotational surfaces; v: cone slant distance from its
  reference circle, sphere latitude, torus tube angle, revolution profile parameter; an extrusion
  is (profile parameter, distance). du × dv points outward on every elementary surface.
- A `Revolution`'s profile must lie in a plane through its axis, else
  `GeometryError::ProfileOutsideMeridian` (`project_seed` relies on it; STEP import refuses a skew
  `SURFACE_OF_REVOLUTION` with it).
- `same_surface` gives the `Sense` between coincident surfaces' normals: analytic for elementary
  pairs, sampled for an extrusion, revolution or spline: a 7×7 grid, plus for a spline surface one
  sample per control point at its Greville abscissae (at most 64 a direction), so a bump of one
  control point cannot hide between samples.

## Topology

- A `Solid` is an arena of typed ids built by `SolidBuilder::build`, which validates.
- A coedge's pcurve is a uv polyline carrying each sample's edge parameter, exact ends, chords
  within its tolerance in space, continuous across seams. A fitted pcurve's tolerance is
  `PCURVE_TOLERANCE`, or half of how far the edge bows from the chord between its ends when that is
  less (never below `LINEAR_RESOLUTION`), so a short arc that is nearly straight keeps an interior
  sample and never collapses onto a straight edge between the same vertices.
- A face's first loop is its outer one; loops run counter-clockwise about the face normal. A face
  wrapping a periodic surface has a seam edge used twice in its loop, opposite senses, one period
  apart in uv.
- Poles have no degenerate edges: the pole is a vertex, the uv loop closes along the pole line
  between the two coedges meeting there, and a pcurve end at a pole takes its v exactly.

## Transforms (`mapping.rs`, `topology/mapping.rs`)

- `RigidTransform` moves geometry and leaves every parameter alone (`transformed`); a
  `Similarity` (rotation, optional mirror, uniform scale, translation) goes through
  `Solid::mapped`, which keeps every face, edge and vertex name and returns a validated solid or a
  `TransformError`.
- Each curve and surface maps with an affine change of its parameters: lines and intersection
  curves run by length, so their parameters scale; a mirrored circle or ellipse flips its frame
  normal to keep its parameter; a mirrored plane negates v and a mirrored rotational surface turns u
  back (`2π − u`), so du × dv stays outward on elementary surfaces. A face whose uv map keeps its
  orientation under a mirror (extrusions and spline surfaces) reverses its sense instead.
- A mirror reverses every loop: coedge order, coedge senses and pcurve samples.
- Enlarging re-traces every intersection edge and any other edge that drifts beyond
  `INTERSECTION_TOLERANCE` from its faces (`IntersectionCurve::through`, ends pinned to the
  vertices) and refits its pcurves; other pcurves are refined back to `PCURVE_TOLERANCE`. A vertex
  beyond `MAX_SIZE` or a shrunk edge shorter than the resolution is refused.

## Validation

- `Solid::validate` checks a closed, oriented 2-manifold whose geometry agrees with its topology
  and returns the first `ValidationError` with ids. Non-obvious checks:
  - Euler–Poincaré per shell, each fan of faces at a vertex counting as one vertex, so a pinched
    shell has the characteristic of the surface it pinches;
  - an edge used twice by one face only as a seam, its two pcurves apart in the domain (a dangling
    slit otherwise);
  - positive volume per lump, each void inside exactly one lump, lumps neither overlapping,
    nesting nor coinciding.
- Volume checks mesh coarsely, sized by the box of edges and vertices (never the classifier-based
  `bounding_box`), retrying finer (`VALIDATION_COARSENESS`) before reporting a void outside its
  lump, an empty shell (a thin lens is empty on a coarse mesh) or lumps that overlap or coincide.
- Lumps (`topology/lumps.rs`) are checked by probes exactly on each shell: the material depth of
  the other shells (outward +1, voids −1, by mesh parity) must be 0 on a lump and 1 on a void. A
  probe within `DEVIATION_ALLOWANCE` times the mesh deviation of another shell is touching and
  decides nothing, so lumps touching at a point, line or face validate; an overlap shallower than
  that band or narrower than the probe spacing is not seen.
- Validation never intersects faces with each other, since every build runs it; `find_crossing`
  does, for importers.

## `Solid::find_crossing` (for importers)

- Returns a `CrossingCheck`: `Clear`, `Crossing`, or `Inconclusive` naming the first face pair it
  could not decide when no crossing was found (a failed pair is never skipped); STEP import keeps
  an inconclusive solid with a note naming the face entities.
- Edges of one face are intersected with each other; face pairs with overlapping boxes
  (`box_tree.rs`) are intersected, a branch point strictly inside both faces being a `Crossing`
  (neighbours share the known branch, so their other edges are tested against the other face).

## Measuring (`measure/`)

- `distance` returns a `Separation` (closest points and an `Accuracy`): `Exact` only when every
  step was closed form, `Approximate` as soon as seeding and alternating projection found it.
  `angle` of straight edges sharing an end is the angle inside the corner (0 to 180°), otherwise
  the acute angle between the lines or planes.
