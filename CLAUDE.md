# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

caditor is a parametric CAD application for Linux, written in Rust (edition 2024) with wgpu for
rendering. Licensed AGPL-3.0-only. User experience and never losing the user's work are the two
product priorities that outrank everything else; see `.claude/rules/ux.md` and
`.claude/rules/reliability.md`.

## Commands

```sh
cargo run -p caditor
cargo build --workspace
cargo test --workspace
cargo test -p <crate> <test_name>
cargo clippy --workspace --all-targets -- -D warnings
rust-formatter
rust-formatter --check
cargo deny check
(cd fuzz && cargo +nightly fuzz run <target> corpus/<target> seeds/<kind> -- -dict=dictionaries/<kind>.dict -max_total_time=60)
packaging/build-release.sh --snapshot
packaging/check-install.sh target/dist/caditor-<version>-snapshot-linux-x86_64.tar.zst
```

`rust-formatter` formats both `.rs` and `.toml` files. It replaces `cargo fmt` and `rustfmt`
entirely; see `.claude/rules/rust-style.md`.

`.github/workflows/ci.yml` runs on every push to `master` and every pull request, and is called
by the release workflow before it builds, in Ubuntu 22.04 containers on the toolchain pinned by
`RUST_TOOLCHAIN` (shared with the release workflow; actions are pinned by commit, and
`cargo-deny`, `cargo-fuzz` and `cargo-about` are their release binaries checked against pinned
SHA-256 sums): the tests with `--locked` and `CADITOR_REQUIRE_GPU=1` on the lavapipe software
Vulkan driver (without that variable the offscreen render tests skip when no adapter exists),
clippy, `cargo deny` (`deny.toml`: licences, sources and advisories, each ignored advisory with
its reason), a snapshot archive checked by `packaging/check-install.sh`, and a minute of fuzzing
per target. Every job has a timeout, and a newer push to a pull request cancels its older run. `fuzz/` is its own cargo
workspace for `cargo fuzz` on nightly, with targets for the expression parser (`expression`),
DXF (`dxf`), model files as they are (`model`, which mostly exercises the container's damage
scan) and with every chunk checksum recomputed (`model_sealed`, which reaches the value decoder,
the record loaders and the version history), the recovery journal and its replay (`journal`,
also resealed), `caditor-zstd` with and without a prefix (`zstd`, which also checks that frames
round-trip), `read_step` (`step`, which covers the Part 21 parser) and STEP import end to end
(`step_import`). The byte-based entry points they need are `caditor_file::fuzzing`, behind
`caditor-file`'s `fuzzing` feature. Seeds are committed in `fuzz/seeds/<kind>` and dictionaries
in `fuzz/dictionaries/<kind>.dict`, shared by targets reading the same kind of input; the
model, journal and zstd seeds are written by `CADITOR_WRITE_FUZZ_SEEDS=1 cargo test -p
caditor-file write_fuzz_seeds -- --ignored`, and tests keep every seed loading cleanly.
`fuzz/corpus` stays local.

## Architecture

The Cargo workspace is `crates/*`. Dependencies point in one direction only:

```
caditor-expression  ←──────────────────┐
       ↑                               │
caditor-geometry  ←  caditor-sketch  ←  caditor-document  ←  caditor-file  ←  caditor (bin)
   ↑   ↑                                   │                  ↑  ↑                │
   │   └──────────────  caditor-render  ←──┼──────────────────┼──┼────────────────┘
   └──  caditor-kernel  ←──────────────────┘                  │  caditor-zstd
              ↑                                               │
              └───────────────────────  caditor-step  ←───────┘
```

`caditor-expression` has no workspace dependencies; the sketch, document, file and app crates all
use it. `caditor-file` and the app also use the geometry and sketch crates directly.
`caditor-kernel` depends only on `caditor-geometry`, never on the sketch or document crates; the
document and file crates use it for solid features, and the app for face and edge names and
meshes. `caditor-zstd` has no workspace dependencies and only `caditor-file` uses it.
`caditor-step` depends only on the kernel and geometry crates, and only `caditor-file` uses it.

- **caditor-zstd**: safe `compress`, `compress_after`, `decompress` and `decompress_after` over
  Trifecta Tech Foundation's pure-Rust zstd port (`libzstd-rs-sys`), the `_after` pair taking a
  raw prefix (the newer version) that makes the frame a delta: its window is sized to reach
  across prefix and data, with long-distance matching, and decoding accepts windows up to zstd's
  maximum since the output is allocated at the checked size anyway. It is the only crate with
  `unsafe`: its lints set `unsafe_code = "deny"` and each FFI-style call is allowed at its own
  item. Contexts are owned by guards that free them on drop, frames must record their content
  size, and decompression refuses a frame larger than the caller's limit or one that decodes to
  a different size than it records.
- **caditor-step**: STEP (ISO 10303-21, AP214 `AUTOMOTIVE_DESIGN`). `write_step` writes named kernel
  solids as one product (named after the model, or after the body when there is only one) whose
  `ADVANCED_BREP_SHAPE_REPRESENTATION` holds one `MANIFOLD_SOLID_BREP` per lump, or a
  `BREP_WITH_VOIDS` whose voids are `ORIENTED_CLOSED_SHELL`s of inverted faces; shells are told
  apart by the sign of their meshed volume (only bodies with several shells are meshed, and one
  whose shells cannot be sorted is refused as `WriteError::Shells`). Millimetres and radians,
  uncertainty `LINEAR_RESOLUTION`, no author or organisation. Every kernel surface and curve has an
  exact STEP form: planes, cylinders, spheres and tori as they are, cones with a negative half angle
  on a flipped axis, extrusions and revolutions as `SURFACE_OF_LINEAR_EXTRUSION` and
  `SURFACE_OF_REVOLUTION`, B-splines with knot runs (rational ones as the complex entity), and
  intersection curves as the cubic B-spline of their Hermite segments over the edge. Face
  `same_sense` is the face sense, since the kernel's normals are STEP's. Reals print as the shortest
  round-tripping decimal with a point, and text escapes quotes, backslashes and non-ASCII (`\X2\`).
  The output was checked against OpenCascade (valid, closed, same volume) for every kind of face.
  - Reading (`part21.rs`, `read/`): a Part 21 parser (header, named and repeated data sections,
    edition 3 `ANCHOR`, `REFERENCE` and `SIGNATURE` sections skipped byte by byte past strings and
    comments, complex instances sorted by name, typed values, comments, the `\X\`, `\X2\`, `\X4\`
    and `\S\` encodings, nesting limit) feeds `read_step`, which returns every
    `MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS`, `FACETED_BREP` and `SHELL_BASED_SURFACE_MODEL` with
    closed shells (each closed shell a lump) as named kernel solids plus notes, or a `ReadError` in
    words. Units come from each representation's context (SI prefixes and conversion-based units
    such as inches and degrees, whose factor may be a simple or a complex `MEASURE_WITH_UNIT`), and
    a note names every length unit other than millimetres that was converted. Assemblies are
    followed from each solid's representation up to the roots through
    `REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION` (the child is the representation of the
    occurrence's child definition, found through `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` and
    `NEXT_ASSEMBLY_USAGE_OCCURRENCE`, else guessed from which side is some assembly's child; `rep_1`
    is carried into `rep_2`, so the transform is inverted when the parent is listed first),
    untransformed relationships and `MAPPED_ITEM`s, giving one solid per placement. A product's only
    body takes the product's name and several bodies keep their own; placements of one solid take
    their occurrences' names when each has a distinct one, else a number. Placements are memoised
    per representation (so layered assemblies cost one visit per part), assemblies deeper than
    `MAX_DEPTH` or placing a part only inside itself leave that solid out with a note, and the whole
    file yields at most `MAX_INSTANCES` solids. Spline degrees above the kernel's
    `MAX_SPLINE_DEGREE` are refused as they are read, and knot multiplicities must sum to points
    plus degree plus one (with checked arithmetic) before any knot is expanded. Geometry covers
    every kernel surface and curve including B-spline surfaces and curves in all their forms (Bézier
    ones with the standard piecewise knots, degree-fold at every joint; uniform and other unclamped
    ones clamped by knot insertion), trimmed and surface curves by their basis, polylines, and
    `COMPOSITE_CURVE`s (each segment trimmed by point or parameter and followed in its sense) and
    `OFFSET_CURVE_3D`s as dense polylines that edge healing then rebuilds on the faces. A spindle
    `DEGENERATE_TOROIDAL_SURFACE` is the revolution of the rational arc of its tube on one side of
    the axis (the apple or, mirrored, the lemon), so its normal stays the torus's. Faces
    bounded by `POLY_LOOP`s get line edges shared by corner position (and a plane from the polygon
    when a plain `FACE` names no surface). Topology is surveyed first (which faces use each edge and
    vertex), then vertices off their faces are moved onto all of them by damped least squares, edges
    not within a quarter of the resolution of both faces are rebuilt with
    `IntersectionCurve::through`, loops take their orientation from bounds, oriented edges and
    `same_sense` (voids from `ORIENTED_CLOSED_SHELL`), the outer loop is the `FACE_OUTER_BOUND`,
    else the one using a seam, else the largest by area, and faces bounded only by `VERTEX_LOOP`s
    get a pole-to-pole seam (spheres, closed spline surfaces and revolutions with two poles). Every solid then goes through
    `SolidBuilder::build`, so an import is valid or a sentence naming the entity; faces that meet
    only farther apart than `LINEAR_RESOLUTION` are refused in those words, and a solid whose faces
    cross (`Solid::find_crossing`) is refused naming the two face entities, or the one face whose
    edges cross.
- **caditor-geometry**: the math vocabulary, as f64 `glam` aliases (`Point3`, `Rotation3`, …)
  plus `Plane` (origin, normal and in-plane x axis, also used as the frame of every circle and
  rotational surface), `Ray`, `Aabb`, `Aabb2` and the rigid transforms `RigidTransform` and
  `RigidTransform2`. The world is Z-up and
  model data is f64 throughout; conversion to f32 happens only at the GPU boundary in the
  renderer.
- **caditor-expression**: units and expressions. A `Quantity` is an f64 in base units
  (millimetres and degrees) with a `Dimension` of length and angle powers. A plain number takes
  the dimension of whatever it is added to, and a field that expects a length takes a plain
  result as millimetres. Trigonometry reads a plain number as radians, so an angle field takes
  a plain literal as degrees but refuses a computed plain result (`PlainAngle`), asking for deg
  or rad. A unit binds to the primary before it (a number, a parenthesised group, a name or a
  call, `Expression::WithUnit`), and `mm²` and `mm³` name areas and volumes. Typed text accepts
  SI units only (`in` and `ft` stay readable in stored text) and names the units that exist
  when one is misspelled. Every intermediate value must be finite and real, and a literal too large for an f64 is refused
  as it is parsed (it could not be stored). Besides the
  arithmetic and trigonometry there are comparisons (plain 1 or 0, equality within 1e-9) with
  a lazy `if`, lazy `and` and `or`, `not`, `mod`, `hypot`, `exp`, `ln`, `log10`, `log2`,
  `cbrt`, `sign`, `clamp`, `round`, `floor`, `ceil` and `trunc` with an optional step (the
  quotient snapped to a whole number within the comparison tolerance, so `floor(0.3, 0.1)` is
  0.3), and the constants `pi`, `tau` and `e`. Chained comparisons, decimal commas, `mm2` and
  `width²` get messages of their own, and names that read as units (`mm²`) are refused. An `Expression` refers to
  parameters by `ParameterId`, never by name, so renaming a parameter rewrites every
  expression's text. Parsing limits length, nesting and the depth of the tree it builds (checked
  as each operator is added, so long chains stop at the limit) so that hostile input cannot
  overflow the stack, and errors are plain-language clauses.
- **caditor-sketch**: 2D sketches on a `Plane` and caditor's own constraint solver.
  - Entities are points, lines, circles (centre point and radius), arcs (centre, start and end
    points, counter-clockwise) and clamped B-splines through control points. Every sketch also
    has a fixed origin and two axes under reserved IDs (`EntityId::ORIGIN`, `HORIZONTAL_AXIS`,
    `VERTICAL_AXIS`) that the counter never reaches; stored IDs stay below 2^63.
  - Constraints have stable `ConstraintId`s: coincident (point–point or point on a curve),
    horizontal, vertical, parallel, perpendicular, tangent, equal, and the dimensions distance,
    angle and radius, whose values are expressions. An angle measures from its first line's
    direction (or its reverse when `reversed`, which the UI sets so a corner of a chain is
    measured inside it) to its second's. `check_constraint` refuses constraints that
    do not fit the entity kinds, so the UI can ask before offering one. `insert_entity` and
    `insert_constraint` take explicit IDs and check references, for loading. The sketch counts
    how often each entity is used by curves and constraints, so refusing to remove a used one
    never scans the sketch and undoing a large import stays fast; `remove_entity` removes a
    whole cascade in one pass and updates those counts incrementally.
  - Sketch splines are clamped with uniform knots and degree min(3, points − 1). `BSpline::fit`
    approximates a dense polyline by one of these (chord-length parameters corrected by
    projection, banded least squares with fixed ends, doubling the control points until within
    a tolerance, else the best found), `BSpline::interpolate` passes one through given points at
    evenly spaced parameters, and `BSpline::through` follows unevenly spaced points without
    loops: a chord-length interpolation with averaged knots, sampled and fitted. Knot spans are
    found by binary search and every system is solved by banded elimination (`banded.rs`).
  - `solve` evaluates the dimensions, then runs damped Gauss–Newton with minimal-norm steps on
    each independent part of the system (SVD from `nalgebra` for parts of up to 48 variables;
    above that CGLS from zero on the sparse Jacobian, which converges to the same minimal-norm
    step, and the analysis uses sparse forward elimination, `sparse.rs`). `solve_dragging` starts
    from dragged points placed at their targets and first holds them there while everything
    else solves; when that cannot work they only weigh a hundred times more than free geometry,
    so they end as near their targets as the constraints allow, so geometry that already
    satisfies its constraints does not move and under-constrained geometry moves as little as
    possible. Every equation has an analytic gradient; two-branch equations (tangent side,
    signed distance) take their branch from the starting geometry, so a solve never flips,
    while internal circle tangency follows whichever circle is currently larger. A tangent
    whose curves share a point (directly or through point–point coincidences) is written as
    the radius there being perpendicular to the line (or both radii along one line), which
    keeps full rank where the distance form has none, and a zero distance between points is
    solved as a coincidence. A solve that would collapse a line or an arc's radius to nothing
    counts as not converged, so it is reported as a conflict. Retries perturb each part by a
    fraction of its own extent.
    Degrees of freedom and each entity's constraint state come from the rank and null space
    of the Jacobian at the solution; a constraint whose equations add no rank over older ones
    is reported as redundant, naming what it duplicates. When a part does not converge,
    QuickXplain-style divide and conquer over its constraints (newest preferred, re-solving
    only the failed parts) finds a minimal set of conflicting constraints, which recompute
    reports as the feature's error with `FeatureError.constraints` and `FixTarget::Constraint`.
    `Sketch::solve_from` takes the `SolveMemo` of the previous solve (recompute passes the
    feature's last good `SketchResult::memo`): each part is keyed by its entities, constraints,
    dimension values, starting values and the solver's scale, and remembered under both its
    starting and its solved values, so a part an edit did not touch starts from its old
    solution and reuses its rank analysis once its solved values match exactly.
- **caditor-kernel**: caditor's own B-rep geometry kernel (no truck, no OpenCascade), the base of
  solid modelling.
  - Cancellation (`interrupt.rs`): `interruptible(interrupt, work)` installs a check for the
    current thread while `work` runs, and booleans (per edge, face pair, face and fragment and
    between phases) and tessellation (per face) poll it, failing with a `Cancelled` variant of
    their error; a boolean cancelled while validating its result says so too
    (`BuildError::interrupted`), rather than reporting an invalid solid. The document installs its `CancelToken` around every evaluation and meshing,
    and export around its meshing and STEP writing.
  - Tolerances live in `tolerance.rs`: `LINEAR_RESOLUTION` is 1e-6 mm and `ANGULAR_RESOLUTION`
    is the angle that moves a point at `MODEL_EXTENT` (10 m) by it. `SamplingTolerance` (chord
    and angle) drives every sampling, and `Solid::default_tolerance` derives one from the size.
    Constructors reject non-finite and degenerate input, every iteration has a fixed bound, and
    failures are errors, never panics.
  - `Curve` (line, circle, ellipse, B-spline, intersection) and `Curve2` (line, circle, B-spline)
    share one `BSpline<P>` (clamped, optionally rational, degree up to 9) and generic sampling,
    length and closest-point code. Lines run by arc length along a unit direction, circles and
    ellipses by angle in a `Plane` frame (period 2π), splines over their knot range. Reversal maps
    t to `reversal_pivot() - t`. Closest points are analytic for lines and circles, otherwise
    seeded by sampling and refined by bracketed Newton.
  - `Curve::Intersection(IntersectionCurve)` lies on two surfaces it carries: nodes refined onto
    both (point, unit tangent, uv on each) joined by cubic Hermite segments in approximate arc
    length, subdivided until the midpoint of every segment is within `INTERSECTION_TOLERANCE`
    (a quarter of `LINEAR_RESOLUTION`) of the true intersection, so edges built on it validate.
    A closed one is periodic over its length. `uv_at` and `refined_point` re-project onto both
    surfaces; `trimmed` returns a sub-range as a new curve with the same parameters and shape.
    `IntersectionCurve::through` rebuilds one from rough points (an imported edge a little off
    its faces): each point is solved onto both surfaces in its normal plane, and where the
    surfaces only touch (a tangent fillet edge) by alternating projection, accepting the middle
    of a gap up to `LINEAR_RESOLUTION`; only this path follows touching surfaces.
  - `Surface`: plane, cylinder, cone, sphere, torus, extrusion, revolution and `BSplineSurface`
    (tensor-product, clamped, optionally rational, degree up to 9, points stored row by row with u
    along a row). A spline surface whose first and last rows or columns meet is periodic in that
    direction over its knot range (C0 at the seam is enough), and a boundary row collapsed to a
    point is a pole; a collapsed column cannot be a pole, so importers transpose such surfaces and
    flip the face. It evaluates second derivatives exactly (rational by the quotient rule), bounds a
    uv box by its own control net, cut out of the spans by knot insertion (in homogeneous
    coordinates, so the hull holds for rational surfaces; the spans' net when the box wraps a closed
    direction), so sub-patches shrink as they are divided, and projects from the three nearest
    samples of a precomputed grid (searched by blocks of 8×8 with their boxes) plus the hint,
    refined by Newton, keeping the hint's foot only when it is as close as the best. Its poles are
    found once, when it is built. u is the angle around the axis (the frame normal) on every rotational surface; the cone's
    v is slant distance from its reference circle, the sphere's v latitude, the torus's v the tube
    angle, and a revolution's v the profile parameter. An extrusion is (profile parameter,
    distance). du × dv points outward on every elementary surface. Singularities are always `Pole`s:
    v isolines where du vanishes (sphere poles, cone apex, a revolution profile ending on its axis).
    `project` returns the periodic representative nearest a hint, else the principal one in [0,
    period); on spline profiles it keeps the closest point near the hint when no other is closer by
    more than the resolution, so self-crossing profiles project consistently. `same_surface` gives
    the `Sense` between the normals of two coincident surfaces whatever their frames and seams:
    analytic for elementary pairs, and by mutual sampled projection when an extrusion or revolution
    is involved.
  - Topology: a `Solid` arena of vertices, edges, coedges, loops, faces and shells behind typed
    ids and accessors, built through `SolidBuilder`, whose `build` validates. An edge is a
    curve, an interval and two vertices (one for a closed edge). A coedge has a sense and a
    pcurve: a uv polyline carrying the edge parameter of each sample, with exact end points,
    refined until its chords stay within `PCURVE_TOLERANCE` in space, continuous across
    periodic seams. A face's first loop is its outer one, and loops run counter-clockwise about
    the face normal, so in uv the outer loop is counter-clockwise when the face sense is `Same`.
    A face that wraps around a periodic surface has a seam edge used twice in its loop with
    opposite senses, one period apart in uv; `add_loop` fits pcurves by chaining projection
    hints and moves the second copy of a seam by a period when the chain put both on one side.
    Poles have no degenerate edges: the pole is a vertex, and the uv loop is closed along the
    pole line between the two coedges that meet there, a gap that validation and tessellation
    both accept; a fitted pcurve end at a pole takes the pole's v exactly. `add_loop` places the
    first loop of a face with its lowest u and v in the principal period and shifts every later
    loop by whole periods into the outer loop's range, so holes lie inside the outer loop in uv
    whatever the fitting chain started from. Faces carry a `FaceName` and an optional
    `FaceOrigin`, edges an `EdgeName`.
  - `Solid::validate` checks a closed, oriented 2-manifold whose geometry agrees with its
    topology (edge uses and senses, loop chaining in space and in uv, vertices on curve ends,
    edges on both surfaces, pcurves on their edges, loop winding and nesting, shell
    connectivity, Euler–Poincaré per shell, positive volume for lumps and voids inside a lump)
    and returns the first `ValidationError`, with ids. The volume checks run on a coarse mesh and
    retry finer before reporting a void outside its lump. Validation does not intersect faces
    with each other, since every build runs it; `Solid::find_crossing` does, for importers: the
    edges of each face (seams aside) are intersected with each other, and a transversal point or
    an overlap away from the vertices they share is a `Crossing` of that face with itself; then
    every pair of faces whose boxes overlap is intersected, and a point of a branch strictly
    inside both faces (or, for coincident faces, a sample strictly inside both) is a `Crossing`.
    Neighbours skip the costly surface pair, whose branch along their shared edge is known: each
    one's other edges are intersected with the other's surface, and a transversal point or an
    overlap strictly inside the other face is a `Crossing`. A pair whose intersection fails is not
    skipped: the result is a `CrossingCheck` (`Clear`, `Crossing`, or `Inconclusive` naming the first
    such pair when no crossing was found), and STEP import keeps an inconclusive solid with a note
    naming the face entities. `bounding_box` covers the edges and, for
    doubly curved faces, a grid of points inside each face plus a sphere's axis extremes.
  - Tessellation samples each edge once and shares its positions between both faces. Each face is a
    constrained Delaunay triangulation (spade) of its loops in (u, v), scaled by the mean surface
    speeds, plus a uniform grid of interior points spaced by curvature (normal curvature and twist,
    sampled on a lattice that also covers every knot span; spline, revolution, extrusion and cone
    faces are then refined, by the square root of the excess, until the grid's cells stay within
    the chord tolerance) and kept clear of
    the boundary (a direction without curvature gets cells at most four times longer than the curved
    one's, so no triangle spans far across a curved direction); triangles are kept by the parity of
    constraint crossings from outside. Consecutive boundary points at the same vertex whose
    parameters differ by a spatially negligible gap (an edge ending within the resolution of its
    vertex) are merged, so such joints do not become spikes. Pole-line points share the pole's
    position and the triangles that collapse there are dropped, so the mesh stays watertight. A
    straight edge ending at a pole (a ruling to a cone's apex) is sampled at the grid's row spacing
    of the faces it bounds, since with only its ends the triangles between it and the next grid
    column would fan from the apex along one ruling and have no area. `Mesh` holds shared positions,
    per-face vertices with exact surface normals, triangles, each face's triangle range and each
    edge's polyline, and computes volume, area and centroid by the divergence theorem. When a face
    boundary crosses itself at the requested tolerance (loops closer than the sampling error),
    tessellation retries a few times before failing, halving chord and angle for every face that
    crossed and for the edges they bound (an edge takes the finest tolerance of its faces).
  - Naming (`naming/`): `FaceName`, `EdgeName` and `VertexName` are 128-bit FNV-1a digests over a
    canonical little-endian encoding with a tag byte per constructor; they are stored in files, so
    the encoding and the pinned digests in `naming/tests.rs` never change. Faces: `side(feature,
    PieceId)`, `start_cap` and `end_cap(feature, RegionKey)`. Edges: `between` (unordered face
    pair), `seam(face)` for the profile seam of a full revolution, and, when several edges share a
    name, `between_at(left, right, from, to)` with the vertex names (sets of faces around each end)
    and the faces oriented by the edge, then `occurrence` ordered by position (midpoints on a grid
    of a hundred resolutions, so rounding noise cannot swap them) as a last resort. `FaceOrigin`
    (side of an entity, start or end cap, with the raw feature and entity ids) says in words what a
    face came from. Later generators (a fillet face named by the edge it replaced, boolean fragments
    that keep their name) are new constructors with new tags. `Solid::imported(feature)` names an
    imported solid: `FaceName::imported(feature, index)` by the face's position in the solid (its
    order in the stored STEP text, which never changes), `FaceOrigin::Imported`, and edges `between`
    their faces, disambiguated like sweeps.
  - References (`naming/reference.rs`) are how later features keep hold of generated topology. A
    `FaceReference` is a face's name, origin and the set of its neighbours' names. It resolves to
    the one face with that name when it still shares a neighbour (or none were recorded); among
    fragments of a split face, to the one whose neighbours match best (most shared, then fewest
    differences); and when the name is gone (its region key or piece id changed), to the face of the
    same origin sharing at least one neighbour. An `EdgeReference` is an edge's name, its two face
    names and its end vertex names, resolved by name, else among the edges between the same faces by
    matching ends (an edge sharing no end with the reference is never taken for it). A tie is `ReferenceError::Ambiguous` with the candidates and no match is
    `Missing`: resolution never guesses between equals.
  - Profiles (`profile/`): `Profile::new` takes `ProfileCurve`s (lines, circles, counter-clockwise
    arcs, clamped B-splines with an explicit knot vector) tagged with the sketch entity id as a
    plain u64, and builds the planar arrangement with tolerance 1e-7 of the profile size (at
    least `LINEAR_RESOLUTION`), finding candidate curve pairs, curves under endpoints and faces
    around nested components through a box tree: analytic line and circle intersections,
    subdivision on monotone
    spans (pairs pruned by their boxes before any budget is spent, running out of it is its own
    `TooIntricate` error) plus damped Newton from every leaf for splines (self-crossings
    included, crossings merged only within tolerance), endpoints landing on curves,
    clustering of nearby points into vertices, merging of overlapping collinear or co-circular
    pieces (the lowest entity id is kept), pruning of dangling pieces and bridges, and faces
    traced by angle at each vertex (ties between tangent curves decided by the position a short
    way along). A `Region` has a CCW outer `ProfileLoop` and CW holes of `Piece`s (entity, 2D
    curve, parameter range, reversed), with the region on the left of every piece. A `PieceId` is
    the entity plus what bounds each end: its own start or end, or the sorted ids of the curves
    that cut it there with an occurrence counted along the curve. A `RegionKey` digests the set of
    (entity, side) pairs of its boundary; regions sharing a key are told apart by their piece
    ids. Depth counts nesting of connected components inside faces of others. `select` with
    `Selection::EvenDepth` (the default) or explicit keys returns the union of the chosen regions
    as new regions keyed the same way, so adjacent regions sweep as one lump. Errors name the
    entity ids. Whether a key is tie-broken is decided once over the whole arrangement (any two
    regions sharing it), so a region keeps its key whatever else is selected with it.
    `Region::triangulate` samples the loops and keeps the constrained Delaunay
    triangles inside by the parity of constraint crossings, for drawing regions as fills.
  - Intersections (`intersect/`) take a `SurfacePatch` (a surface and a finite uv box; periodic
    boxes wrap, poles accept any u) so booleans intersect face patches, and restrict curves to a
    parameter interval. Coincidence within tolerance is detected, never guessed: a curve lying
    in a surface or on another curve is an overlap interval, and coincident surfaces return
    `SurfaceIntersection::Coincident(Sense)` from `same_surface`; patches of the sampled kinds
    (extrusion, revolution, spline) are also `Coincident` when they share only part of their
    extent: every grid sample of either whose foot lands inside the other must lie on it (a
    foot pushed against the other's edge, with a residual off the normal, is skipped), and at
    least four must land there. Points within
    `LINEAR_RESOLUTION` are one point; one at a range end takes the exact end parameter, and a
    closed curve's wrap point is reported once. `tangent` flags touches (no sign change, or
    parallel tangent within 1e-7), and clusters of roots closer than the resolution collapse to
    one tangent point.
    - `intersect_curve_surface` (points with curve parameter and uv, overlaps): analytic for a
      line against plane, cylinder, cone (its own nappe) and sphere, the torus quartic isolated
      through its derivatives' roots, circles and ellipses against planes, circles against
      spheres and coaxial cylinders, cones and tori, and an intersection curve on its own
      surfaces. Otherwise the curve is subdivided on piece boxes (within one spline span, the
      samples widened by a second-derivative sagitta, since span hulls do not shrink) pruned by
      the surface's Lipschitz distance until flat relative
      to both curvatures, then each leaf brackets sign changes of the signed distance (roots
      verified by true distance) and minimises it for touches. Swept surfaces project locally
      from the previous foot point inside a leaf.
    - `intersect_curves` and `intersect_curves2` (parameters on both, overlaps): analytic for
      lines and 2D circles, else paired subdivision to flat pieces and Newton on the squared
      distance from several starts per leaf.
    - `intersect_surfaces` returns `IntersectionBranch`es (curve, increasing range inside both
      boxes, closed flag, end uv on both sides, tangent flag) and isolated `IntersectionPoint`s.
      Analytic: plane/plane, plane/cylinder (circle, ellipse, two lines, tangent line), plane/cone
      (circle, ellipse, rulings through the apex or a tangent ruling, the apex alone), plane
      through a torus axis (two circles), plane/extrusion (lines when parallel to the direction,
      else the profile's exact affine image: B-spline, line, conic), parallel cylinders (lines or
      a tangent line), equal cylinders with crossing axes (two ellipses and the two tangent
      points), and every coaxial pair of rotational surfaces (plane normal to the axis, sphere
      centred on it, cylinder, cone, torus, revolution with a planar profile), whose meridians are
      intersected in (r, z) as 2D curves: each point is a circle, tangent points give tangent
      circles, points on the axis give isolated points. Everything else is marched: seeds come
      from paired subdivision of both patches (sub-patches cached with their boxes, pruned by box
      overlap and Lipschitz distance) down to leaves of half a curvature radius, each solved by
      minimal-norm Gauss–Newton, then a sign scan of the distance for tiny loops; a leaf already
      holding a transversal seed is skipped. Branches march both ways from each seed not already
      on a branch, with steps limited by the turn of the tangent, stop exactly on the box boundary
      (a parameter-constrained solve), close loops through the seed and end where the normals
      become parallel (reported as tangent points). A step that collapses where the branch runs
      off a bounded surface ends on the boundary ahead (the nearest patch bound along the
      tangent, within one maximum step, by a parameter-constrained solve); otherwise it, and a
      branch longer than the step cap, fails as `IntersectionError::Unfollowable`.
      Near poles the contact is solved with one surface as carrier and the other's signed
      distance. A marched branch that is a line, circle
      or ellipse within half the resolution is returned as that curve.
  - Point classification (`topology/classify.rs`, `SolidClassifier` to reuse per solid):
    `classify_point` gives `Inside`, `Outside` or `OnBoundary(face)` exactly, or `Undecided` when
    every ray was ambiguous (a boolean then tries the fragment's other points): a point on a face's
    surface and inside its boundary is on it, otherwise rays from a fixed list of directions are
    intersected with each face's surface through `intersect_curve_surface` (over each face box's
    window widened a little, so a crossing is never snapped to the window's end), and the nearest
    crossing's outward normal decides; a ray that grazes, is tangent, lies in a face or meets an
    edge or vertex no farther than its nearest clean crossing is discarded for the next direction,
    and when none is left the point is `Undecided`, never a guess. `point_in_face(face, uv)` (`Inside`,
    `Outside`, `OnBoundary`) uses the pcurve polygons by parity over periodic shifts (poles probed
    just off the pole line, inwards from whichever end of the domain is nearer), and near the
    boundary (within a few `PCURVE_TOLERANCE`) the exact edge: the side of the nearest non-seam
    coedge, or of both coedges at a vertex (convex corners need both).
    `classify_boundary_point(point, normal)` adds `Coincident { face, sense }` for a point on a face
    whose normal is parallel, and `Touching(face)` otherwise.
  - Builders (`build/`): `extrude(plane, regions, LinearExtent, feature)` and `revolve(plane,
    regions, Axis2, AngularExtent, feature)` plan vertices, edges and faces, merge coincident
    vertices within a shell (pinched regions), name every face and edge, group faces into shells by
    shared edges and emit through `SolidBuilder`, so a result is valid or an error (`SweepError`).
    Extrusion sides are planes, cylinders or extrusion surfaces; revolution sides are planes,
    cylinders, cones, spheres, tori or revolution surfaces (splines, and arcs whose circle reaches
    the axis, converted to rational splines). Faces on extrusion and revolution surfaces get exact
    straight pcurves; the rest are fitted. The start cap is the one at the extent's start (the
    sketch plane for `one_side`), whichever way the sweep runs, so flipping the direction keeps
    every name. A profile on the right of the revolution axis is revolved about the reversed axis;
    lines on the axis become shared cap edges or nothing, endpoints on it poles, and a full turn has
    no caps (holes become void shells). The document is expected to convert a solved sketch to
    `ProfileCurve`s, keep the chosen `RegionKey`s in the feature, and call these with the feature
    id. `build::plan::Plan` is also how booleans emit their result, with explicit pcurves.
  - Booleans (`boolean/`): `boolean(first, second, BooleanOperation)` for union, difference and
    intersection, valid or an error (`BooleanError`), never a bad solid.
    - Imprinting pools vertices within `LINEAR_RESOLUTION`: those of both solids, edge–face hits
      inside or on the face, the ends of an edge lying in a face's surface and its crossings with
      that face's edges, and the tangent points of face pairs. Each edge is split at the pooled
      vertices of the other solid lying on it (the ends of a piece shorter than the resolution
      become one vertex) and each face–face branch at every pooled vertex on
      it; a branch piece is kept where its midpoint is strictly inside both faces, and an edge piece
      lying in the surface of a face of the other solid and inside it is a cut in that face. Pieces
      with the same end vertices and geometry are one edge, so an intersection along an existing
      edge and coincident faces need no special case. Edge–face and face–face candidates come
      from a tree of face boxes (`box_tree.rs`, also used by `Solid::find_crossing`).
    - A face with no cuts whose edges are all unsplit passes through with its own loops and
      pcurves (refitted only on an edge merged with one of the other solid). Otherwise it is
      traced into loops from its boundary pieces (hinted by the original pcurves) and
      its cuts (both ways, dangling ones pruned): at each vertex the next edge is the first one
      clockwise from the arriving one about the outward normal (at a pole, the mean normal of a ring
      around it, so rulings through a cone apex are ordered by azimuth), with ties and cusps decided
      by chords at a common distance. Loops are fitted in the face's chart; a run of cuts leaving a
      pole is shifted by whole periods to meet the next boundary edge, pcurve ends are snapped to
      their vertices, and a hole goes to the smallest outer loop containing a point of it that is
      not on that loop.
    - Each fragment is classified against the other solid at up to three interior points (inside
      or outside wins over coincident or touching; inside and outside together is `Ambiguous`) and
      kept by the operation. Faces that passed through share one class across unsplit edges that
      no cut or other piece shares, and one whose box misses the other solid's is outside. Of coincident faces only the first solid's fragment can stay: with
      the same orientation for union and intersection, the opposite one for difference. A
      difference reverses the second solid's fragments it keeps. Every edge of the result then has
      one use each way, else `Open`, or `NonManifold` when solids would meet only along an edge.
    - Adjacent faces on the same surface with the same orientation are merged by retracing them
      without the edges between them (left apart when that fails, as for a ring around a periodic
      surface), and two edges meeting at a vertex between the same faces are joined when they are
      pieces of one curve, collinear lines or arcs of one circle. Faces keep their names and
      origins (fragments of a split face share its name), pieces keep their edge's name and new
      edges are named `between` their two faces, before the plan disambiguates duplicates.
  - Blends (`blend/`): `blend(solid, edges, BlendShape, feature)` rounds (`Fillet`) or bevels
    (`Chamfer`) edges by sweeping a tool per edge, valid or a `BlendError` that names the edge.
    Tools are united pairwise in rounds (a pair that cannot be united, such as tools meeting only
    along an edge, stays apart) and each group is applied in one boolean, or tool by tool when
    that fails. Chosen edges first grow along tangent-continuous chains (`blend_chain`) and
    smooth edges are dropped. Supported edges are straight ones whose faces run along them (planes,
    parallel cylinders), swept by extrusion, and circles whose faces share their axis (planes,
    cylinders, cones, spheres, tori), swept by revolution; the blend must fit on both faces at a
    quarter, half and three quarters of the edge, and the cross-section is solved in 2D
    (`section.rs`: fillet circle from the offset curves, chamfer points at equal distance). Convex
    tools are lifted clear of the faces they cut and subtracted; concave ones are flush and added,
    all concave edges first, then the convex ones re-found by reference in the filled solid (one
    that cannot be found fails as `Lost`, and errors about edges of the filled solid that are not
    chosen ones come back as `AfterFill` without an id). Ends continuing into another chosen edge
    stop flush, ends on a face perpendicular to the edge stop there, ends on a slanted face extend
    past it when the extension lies where the operation changes nothing, else are clipped by the
    face's plane. Three convex straight edges filleted at a vertex of three planes get a spherical
    corner (`corner.rs`: a hexahedron minus the rolling ball, built through `Plan`); other corners
    mitre. Faces are named `FaceName::blend(feature, edge)` and `corner(feature, vertex)` with
    `FaceOrigin::Fillet` or `Chamfer`.
  - Shell (`shell/`): `shell(solid, open, thickness, feature)` offsets every face by the thickness
    (`inner.rs`: each vertex solved by minimal-norm Newton on the offset surfaces, each line or
    circle edge rebuilt through its offset ends and checked on both offset surfaces) and subtracts
    the result. The topology is kept except where the offset changes it: a cylinder, sphere or
    torus curving more tightly than the thickness collapses and is dropped (edges around its axis,
    centre or spine vanish and merge their vertices, whose position is solved on every surviving
    face around them, and the two edges along it become one between the faces on either side), and
    a vertex of four to eight faces whose offsets do not meet, all its edges convex or all concave,
    is split along the triangulation of its cycle of faces whose corners lie inside (or outside)
    every other offset, each diagonal a line between its two faces. Only flat faces open. An opened face with no smooth edge to a closed face
    is offset outward, so the inner solid passes through it and the body needs room only across its
    walls; this attempt counts only when every closed face's inner face survives the subtraction.
    Otherwise (or when it fails) every face is offset inward and a prism swept outward from the
    offset copy of each opened face is unioned before subtracting, which needs the thickness below
    half the body in every direction. Inner faces are `FaceName::shell(feature, original)` with
    `FaceOrigin::Shell`. Failures are told apart: a face curving more tightly than the thickness
    that cannot be dropped (`TooCurved`), a corner whose walls cannot meet (`Corner`), an edge whose wall shrinks to
    nothing, an opening that cannot be cut, walls that cross (`Walls`) and a thickness too large for
    the body.
- **caditor-document**: the parametric model: parameters, the ordered feature tree and
  everything that changes or recomputes it.
  - Every mutation is a `Transaction` of `Edit`s passed to `Document::apply`, the only public
    mutator of content (`reserve_ids_below` only raises the ID counters, for loading). `apply` is
    atomic and returns the inverse transaction, and `Editor` keeps undo and redo as stacks of these
    inverses. Edits carry their IDs, so redo restores the same IDs, and ID counters never move
    backwards. Parameter and feature IDs stay below 2^63 (`FIRST_UNSTORABLE_ID`, as in sketches):
    edits refuse larger ones as `ReservedId` and `reserve_ids_below` clamps to it, so a stored
    counter can never wrap. Edits refuse to break invariants: unknown references, parameter cycles, deleting
    something still in use, moving a feature past one it depends on, or two features sharing a name
    (names are trimmed on every insert and rename; loading renames the second with a report). `same_content` compares documents without their ID
    counters, which is what decides whether a model is unsaved. `Document::check` runs a transaction
    on a clone so the UI can report the error before committing; `can_remove_parameter` and
    `can_remove_feature` answer the common case without one. Parameter dependencies are built once
    per `apply` as a `DependencyGraph` that also knows each parameter's users, so a cycle check
    searches back from the edited parameter (nothing to search while no one uses it), and
    evaluation orders parameters topologically before looking for cycles, so both
    stay near linear in the parameter count. `Document::transaction_to` (which keeps every sketch's
    ID counter at least where it is, so restoring never reuses IDs) builds the transaction that
    turns one document into another (every feature and parameter removed, then the target's inserted
    with their IDs), which is how an earlier version is restored as one undoable change.
  - Sketch content changes only through sketch edits (add, remove or set an entity, add or
    remove a constraint, set a dimension). Removing an entity that something still uses is
    refused rather than cascaded; `TransactionBuilder::remove_sketch_items` expands a user's
    deletion into constraints first, then curves, then points. Setting an entity changes only
    its value, never its kind or the points it uses. `settle_sketch` moves the definition to a
    solved shape so the next solve starts from what the user sees.
  - Import features (`import.rs`, `FeatureKind::Import`) make a body from an imported solid:
    they keep the source file's name, the solid and the canonical single-solid STEP text it was
    read from, which is what the file stores; equality compares the text, not the solid. The
    body is the solid named by `Solid::imported`, so later features (blends, shells, sketches on
    faces, datums, adding and removing) hold its faces and edges like any other body's. An
    import cannot change kind; one whose shape could not be read back fails with a sentence.
  - Blend features (`blend.rs`, `FeatureKind::Blend`) keep a `BlendKind` (fillet or chamfer,
    switchable through `SetFeatureKind`), the body, the chosen `EdgeReference`s and a size
    expression; recompute resolves the references in the body's state before the feature (a
    split edge contributes all its pieces, a lost one fails the feature) and maps kernel errors
    to sentences naming the edge by its faces (`describe.rs`). The state each blend starts from
    is kept (`Evaluation::body_before`) and meshed so the app can show it while choosing edges.
  - Shell features (`shell.rs`, `FeatureKind::Shell`) keep the body, the opened faces as
    `FaceReference`s (possibly none, for a closed hollow body) and a thickness expression.
    Like blends they modify a body, resolve their references in the state before them (a
    reference tied between fragments opens all of them, a lost one fails the feature) and have
    that state meshed; kernel errors become sentences naming the face or edge involved.
  - Solid features (`solid.rs`, `FeatureKind::Solid`) are an `Extrude` or a `Revolve` of a
    sketch's regions (`RegionChoice::All` for even depth, or chosen `RegionKey`s) with a
    `BodyOperation`: `NewBody`, or `Add`, `Remove` or `Intersect` on the body of the feature
    that made it. A body is named by that feature's ID. Extents are expressions (lengths, or
    angles in degrees) that must be above zero, both distances of a two-sided extrusion
    included; one-sided extents flip with `reversed`, and a
    revolve's axis (`RevolveAxis`) is a line of its sketch, one of the sketch axes, or an
    `AxisReference` to a model axis that must lie in the sketch plane. Inserting one checks that
    its sketch is a sketch and its target makes a body; `SetFeatureKind` replaces its settings
    but never its kind, and a feature whose body others change keeps making a new body. A sketch
    line used as a revolve axis cannot be deleted, and edits refuse an axis that is not a line or
    axis of the revolve's own sketch (`AxisNotALine`).
  - Sketches (`FeatureKind::Sketch(SketchFeature)`) keep their `Sketch` and optionally a
    `SketchAttachment`: a datum plane they lie on and follow, or, when they lie on a body, a
    `FaceAttachment` (`attachment.rs`: the body's feature ID and a `FaceReference`).
    The stored plane is where the sketch was placed; recompute resolves the reference in the
    body's state at the sketch's place in the tree (`FeatureKind::body_input`, shared with solid
    features that change a body) and gives the solved geometry the face's plane, outward normal
    and surface frame, so the sketch follows the face. Fragments of a split face are accepted
    when they lie in one plane; a lost, split or curved face fails the sketch alone with a fix
    pointing at it. `SetSketchPlacement` sets the plane and attachment together (attach, move
    to another face, or detach where it is); a body with attached sketches cannot be deleted or
    stop making a body.
  - Datums (`datum.rs`, `FeatureKind::Datum`) are planes and axes with a `DatumResult` (a
    `Plane` or a `Ray`). References to model geometry are a `PlaneReference` (principal plane,
    datum plane, or flat face as a `FaceAttachment`) and an `AxisReference` (principal axis,
    datum axis, straight edge as an `EdgeReference`, or the axis of a cylindrical, conical,
    toroidal or revolved face as a `FaceReference`), resolved in each body's state at the
    feature's place in the tree; pieces of a split edge or face count when they lie on one line.
    A `DatumPlane` starts from its base plane, optionally moves it to pass through an axis and
    turns it about the axis (which must run along the plane) by an angle, then offsets it along
    its normal; a `DatumAxis` runs
    along an axis reference or where two planes meet. Plane stays plane and axis stays axis
    under `SetFeatureKind`, and edits refuse a sketch or plane based on something that is not a
    datum plane (`NotAPlane`) or an axis reference to something that is not a datum axis
    (`NotAnAxis`). `FeatureKind::bodies_used`, `planes_used` and `axes_used` extend
    `features()`, so dependents, moves and deletions account for them.
  - Recompute: `ParameterValues` evaluates parameters in dependency order and reports cycles rather
    than following them. `Recompute` walks the features in tree order and reuses a cached result
    when the feature's content (an `Arc`, compared by pointer first, then its ID and kind by
    `same_content`, so a sketch's ID counter does not count, but not its name), the values of the parameters it uses and its upstream results are unchanged; a failed
    result is also recomputed when the names in its message changed. Upstream results count as
    unchanged when they are the same `Arc` or, for sketches, have the same plane and entities
    (datums: the same result), so an edit that leaves geometry alone (a satisfied constraint, a
    settle) stops there. A failing feature is `Failed` with a `FeatureError` (reason, remedy and a
    `FixTarget`) and keeps its last good result. Its dependents fail with a pointer back to it, and
    everything else is unaffected. A panic inside an `Evaluator` is caught and becomes that
    feature's error. Each body's latest good state is carried through the tree and is part of the
    next change's upstream of every feature that uses the body (`bodies_used`): a feature that
    changes a body gets its current solid through `Inputs::body`, and a failing one is skipped, so
    later features of the body build on the state before it. `Evaluation::body` gives each body's
    final solid and `body_result` the shared result holding it; a body whose creating feature failed
    or was not reached keeps the last good state of its latest feature as a stale body (`is_stale`),
    drawn tinted and still exported. `body_seen_by` gives the state of a body a given feature used,
    which is where a revolve's model axis is drawn. Solid features map profile, sweep and boolean
    errors to sentences naming the sketch curves involved.
  - Display data is computed on the worker at the end of each run and cached inside the shared
    results (`OnceLock`), so the UI only reads it: each body's final state is tessellated
    (`SolidResult::mesh`; intermediate states are not), and every sketch that a solid feature
    sweeps gets its regions with a triangulation each (`SketchResult::regions`). A sketch's
    profile arrangement is built once per result and shared by every feature that sweeps it and
    by the display. The state before an open blend or shell is meshed only when the app asks
    (`Recomputer::mesh`, sent by `Model::mesh_before` for the open feature). A panic or
    failure while meshing leaves the body without a mesh (`mesh_failed`) but keeps its shape for
    later features, and a run cancelled before every shown body was meshed is not complete.
  - `Recomputer` runs recompute on a worker thread. A newer submission or `cancel` stops the
    running job between features (evaluators also receive a `CancelToken`), and features that
    were not reached are reported as `Outdated`. The worker calls a wake callback after each
    report so the UI can redraw. Each run is contained: a panic outside any evaluator runs it
    again without the cache, and a second panic reports `Outcome::Failed` with the last good
    evaluation (shown as stopped, with Restart), so the worker lives on. Requested meshes run
    after the queued recompute under a token that a newer submission or `cancel` trips, and stay
    queued until they finish.
- **caditor-file**: persistence. Our own formats are binary, compressed with zstd and checked
  with xxh3; there is no text model format. The format number restarted at 1 with this design,
  and from here on every version that ships stays readable.
  - Container (`binary/`): an 8-byte magic (`\x89CAD\r\n\x1a\n` for models, `\x89CJL…` for
    journals), a little-endian `u32` format version, then chunks. A chunk is the sync marker
    `CDCK`, its kind, its codec (stored, zstd, or zstd against the next newer version as a raw
    prefix), a flags byte (`MUST_UNDERSTAND`; unknown bits are ignored) and a reserved zero byte,
    stored and content lengths and an xxh3-64 of the header fields and payload, followed
    by the payload. A reader that meets a bad chunk scans forward to the next marker whose
    checksum holds, so damage loses only the chunks it touches; the payload bytes hashed while
    scanning are capped at four times the file size, so forged headers cannot make the scan
    quadratic. Content is capped at 256 MiB per chunk, records are decoded one at a time, and
    one load, listing or restore decompresses at most 2 GiB in all, so a hostile file cannot
    force a huge allocation or endless work.
  - Values (`binary/value.rs`) are a self-describing serde encoding: tagged null, booleans,
    LEB128 unsigned and negative integers, little-endian f64, strings, bytes, and sequences and
    maps closed by an end tag, with nesting limited. Enums are encoded like JSON (a unit variant
    is its name, others a one-entry map), so `Lenient` reads any record through a
    `serde_json::Value`, fields a reader does not know are ignored and records of an unknown kind
    are reported as coming from a newer version. A newer version that adds something whose loss
    would change the model's meaning must use a new record kind or a must-understand chunk, since
    an older reader drops unknown fields of any record it changes.
  - A model file holds a head chunk (when it was saved, the name of the last change, and a
    blake3 digest of its records), one zstd chunk per record (parameters, features, the ID
    counters) and the version history. Saving writes a record whose understood content is
    unchanged back exactly as it was stored (fields from newer versions included, and without
    compressing it again), and carries chunks of kinds it does not know unless they are flagged
    must-understand, which loading reports as left out (so the original is kept as `.damaged`).
    Records carry their stable IDs; expressions are stored
    as canonical text that refers to parameters as `$<id>` (`Expression::to_stored_text` and
    `parse_stored`), region keys and topology names as 32-digit hex digests, and numbers as
    exact f64. An unreadable extent falls back to 10 mm or 360°, an unreadable blend edge is left
    out, an unreadable opened face is left closed, a sketch whose face or datum plane cannot
    be restored stays on its stored plane, and a revolve whose axis line is gone turns about its
    sketch's vertical axis, each with a report.
  - Version history: every save that changes the model keeps the state it replaces as a version
    inside the file (`save_with` reads the earlier versions from the session's own file, so Save As
    carries them along; saving an unchanged model adds none). A version is an info chunk (time, last
    change, blake3 digest) and a data chunk compressed with the next newer version as a zstd prefix,
    so a version costs only its difference; every eighth one is stored whole, which bounds the chain
    a damaged chunk can break. An info chunk belongs only to the data chunk right after it: data
    whose info was damaged is kept (unlisted) so the older deltas built on it still decode, and info
    whose data was damaged is dropped. Each rebuilt version is checked against its digest before it is
    offered. `history` lists the versions (with whether each can still be rebuilt), keeping only the
    rolling newer snapshot, and `load_version` rebuilds one starting from the nearest whole version
    at or after it. When the head cannot be rebuilt, the next save drops the deltas that depended on
    it and keeps the rest.
  - Saving writes a temporary sibling, fsyncs it, renames it over the target and fsyncs the
    directory, keeping the target's permissions, group and extended attributes (ACLs included;
    through `xattr`, each one that cannot be set is skipped), with the temporary created
    owner-only until they are applied. A symbolic link is followed to the file it
    names, which is what gets replaced. A failed directory fsync after the rename is logged, not
    reported as a failed save. Temporary siblings are named
    `.<name>.<boot>-<pid>-<n>.tmp`, `<boot>` being the start of the kernel's `boot_id`, and those
    of this boot whose process no longer exists are removed after each write, so another
    machine's save in progress on a shared folder is left alone. Names that would pass the
    file system's 255 bytes (temporaries, `.damaged` copies) are cut and end in a hash
    (`paths::fitting`), and a model whose name leaves no room for `.<name>.journal` keeps its
    journal in the recovery directory. A save refuses to go ahead when the earlier versions in the file
    it replaces cannot be read, since it would drop them. Overwriting a file that loaded with
    problems first keeps the original as `<name>.damaged.caditor` (`keep_copy`, shared with the
    preferences: a hard link, or where links are unsupported a copy synced under a temporary name
    and moved into place whole, so a failed copy leaves nothing behind).
  - Every file caditor reads (models, journals, imports, preferences, recent files) goes through
    `read.rs`: only regular files, at most `MAX_FILE_SIZE` (2 GiB), reserved fallibly, so a
    device, a pipe or a huge sparse file is refused in words instead of hanging or aborting on
    allocation; saving over anything but a regular file is refused too.
  - Loading is partial. Each record and each sketch item is read on its own (`Lenient`), and the
    pieces are assembled through `Document::apply`, so a loaded model always satisfies the
    document invariants. Damaged or unknown (newer) records are left out, a lost parameter that
    something still uses becomes a stand-in with value 0, unusable or duplicate names are
    renamed, a parameter cycle is broken at the parameter that closes it, and an unreadable
    dimension takes its drawn length. Each of these is reported in plain language. When no chunk
    is damaged but the records do not match the head's digest (a file cut cleanly between
    chunks), the file is reported as ending early, so it loads with problems and keeps its
    `.damaged` copy on the next save. Assembly stays near linear on hostile files: names are
    indexed, duplicate IDs and cycles (through `DependencyGraph`) are found before applying, each
    kind of record is applied as one transaction that is halved only where it fails, and at most
    `MAX_RECORDS` (10 000) parameters and features are loaded, the rest reported.
  - The recovery journal uses the same container: a header chunk naming the file, a snapshot of
    the last saved state, then one chunk per change (`apply`, `undo` or `redo` with the
    transaction that was applied). The header also carries the path as raw bytes (so
    non-UTF-8 names survive) and whether the file loaded with problems, so a recovered session
    still keeps the damaged original on its first save. Replay stops at the first damaged or
    unreadable chunk, so a
    torn tail loses only the changes after it, and replaying through an `Editor` restores the
    undo history. New edit kinds do not bump the journal version: an older reader stops at the
    first entry it cannot read and keeps everything before it. The journal lives next to the
    file as `.<name>.journal`, falling back to `$XDG_STATE_HOME/caditor/recovery/`, where
    untitled documents keep theirs. Its owner holds an exclusive lock on it, which is how the
    startup scan and other instances tell a live journal from an orphan. Locks are taken on
    read-write handles (`lock.rs`; a read-only journal gets a shared lock), since NFS emulates
    `flock` with byte-range locks that need a writable descriptor. A spawned child shares every
    locked descriptor until it execs, so a scan meanwhile takes a live journal for locked; tests
    therefore never spawn processes (FIFOs are made in-process through `rustix`). A journal is only put in
    place by linking it where none exists, or by renaming over the inode the writer holds
    locked and has checked is still at that path, so two windows cannot take one path from each
    other; the owner also checks the path still names its file after every sync, and otherwise
    reports itself unprotected and retakes the path when it can. Journals take the
    model's permissions, or owner-only for untitled documents.
  - `Storage` is one worker thread per open document. It owns the journal and performs saves,
    so appends, saves and the rebase of the journal onto the saved snapshot stay in order, and
    it fsyncs after each batch of entries. It keeps the saved snapshot and the entries since, so
    after a write error (`Report::JournalFailed`) it keeps holding the lock, stops appending and
    rewrites the whole journal every few seconds until that succeeds
    (`Report::JournalRestored`). A journal it replaces (after a save or a restore) is removed
    only once the new one is written, and a failed directory fsync after the rename is logged
    rather than treated as a failed write, since the new journal is already in place. Saving to another path is refused while another window
    holds that model's journal, and a journal is never renamed over one another window holds.
    A `Flusher` lets the panic hook and the signal handler wait for pending entries.
  - Preferences (`settings.rs`): `Settings` is a JSON key/value file in
    `$XDG_CONFIG_HOME/caditor/preferences.json` (`config_dir`), read leniently and written
    atomically; keys a version does not know are kept, so an older caditor never erases a newer
    one's settings. `save_changes` re-reads the file and applies only the keys changed since the
    last save, so two windows keep each other's changes, and an unreadable file is kept as
    `preferences.unreadable.json` before it is replaced. Recent files store paths that are not
    UTF-8 as byte arrays, and each window writes them as `RecentChange`s applied to what is on
    disk (`RecentFiles::save_changes`), so windows keep each other's entries.
  - Recovery (`scan`, `journal_for`) inspects unlocked journals in the recovery directory, next
    to recent files and wherever a marker names one: a journal kept next to its model leaves an
    `adjacent-<hash>.location` file holding its absolute path in the recovery directory, removed
    with the journal (and pruned by the scan once the journal is gone), so a crashed model's
    unsaved work is offered even when no recent file names it. The scan deletes those with nothing to recover (no net change, or already in
    the file, and every entry read) and returns the rest with a replayed `Editor`. A journal
    with entries it could not read (damaged, or from a newer version) is offered, never deleted.
    Before deleting, the locked file is compared with the path by inode, so a journal an owner
    renamed into place meanwhile is left alone. A journal of a model that cannot be read at all
    (bad header or snapshot, or a newer journal version) is renamed aside by `journal_for` to
    `<journal>.<seconds>.unreadable`, which no scan picks up, before the new session's journal
    takes its place (`FileJournal::SetAside`), and opening the model reports where it was kept.
  - DXF import (`import/`): `parse_dxf` reads ASCII and binary DXF (group codes with typed values,
    UTF-8 or single-byte text) into a `Drawing` of 2D `DrawingCurve`s in millimetres plus notes in
    plain language. It reads `$INSUNITS` (none is read as millimetres, with a note, unless
    `$MEASUREMENT` is 0, imperial, when it is read as inches, also with a note), layers (entities on
    off or frozen layers are left out), blocks and INSERTs (base point, scale, rotation, column and
    row arrays, nested with cycle and depth limits and at most `MAX_EXPANDED_OBJECTS` objects and
    cells visited in all, block content on layer 0 taking the insert's layer), and the entities
    LINE, POINT, CIRCLE, ARC, ELLIPSE, LWPOLYLINE and POLYLINE (bulges become arcs, 3D polylines
    lines) and SPLINE (control points with knots and weights up to degree 9, or fit points). Object
    coordinate systems follow the arbitrary axis algorithm. Everything becomes a 3D shape (point,
    line, parametric conic, NURBS or fit points), is transformed, then flattened onto XY: conics
    that project to circles become circles and arcs (counter-clockwise), other conics and splines
    that are not already in the sketch's uniform form are fitted within a millionth of the drawing's
    size. Paper space and invisible entities are skipped silently; text, dimensions, hatches and
    other annotations are counted in a note. The cap is `MAX_DRAWING_CURVES`. `drawing_transaction`
    turns a drawing into one transaction on an existing or new sketch, dropping curves shorter than
    the joint tolerance and joining ends closer than a millionth of the drawing's size with
    `Coincident` constraints.
  - STEP import (`import/model.rs`): `read_step_file` reads a STEP file through `caditor-step`
    and canonicalises each solid (written by caditor's own writer and read back, so what is
    stored is exactly what later loads), giving one `ImportedBody` per solid plus notes;
    `bodies_transaction` adds an `Import` feature per body under unique names. The model file
    stores an import as its source name and STEP text (`import` records), and an unreadable one
    loads as an empty import with a report.
  - Export (`export/`), the one place besides import that follows foreign formats:
    `export_bodies` writes each `ExportBody` (a name and a solid) as STEP through `caditor-step`
    (resolution ignored, no triangle count), or tessellates it at a `MeshResolution` (coarse,
    standard or fine: a chord that is a fraction of the largest body's diagonal, and 20°, 10° or 5°
    between triangles), keeps only the positions the triangles use and drops collapsed triangles,
    then writes binary STL (every body in one surface, facet normals from the winding) or 3MF (one
    named object per body, millimetres) and saves it atomically like a model. Cancellation is
    checked between bodies and before writing, and failures are sentences naming the body. The 3MF
    package is written by a small ZIP writer (`zip.rs`: deflate through `miniz_oxide` unless storing
    is smaller, CRC32, no ZIP64).
- **caditor-render**: wgpu device and surface ownership, the camera and the viewport. It does
  not depend on winit or on the document: it takes any `Arc<dyn WindowTarget>` and draws a
  `Scene` of shaded meshes, lines, markers, triangle fills and a grid built by the app.
  `begin_frame` draws the 3D viewport into its rect and hands back a `Frame` whose encoder the
  app draws the UI into; `submit` presents it.
  - Precision: every position is converted relative to the eye in f64 before the cast to f32,
    and the view matrix is rotation only, so geometry far from the origin stays exact. Meshes
    are the exception that keeps the rule: a `ShadedMesh` stores f32 positions relative to its
    own centre, and the offset from the eye to that centre is computed in f64 each frame.
  - Meshes: a `MeshInstance` is an `Arc<ShadedMesh>` (faces of points with normals) plus a
    `FaceStyle` (colour, pick id) per face. Vertex and index buffers are uploaded once per
    `Arc` and dropped when the mesh leaves the scene; only the per-face styles, read from a
    storage buffer by face index, are rewritten each frame, so hover and selection cost nothing
    in geometry. Faces are lit two-sided by a key light above and to the left of the camera, a
    headlight and a small specular term, and write depth, so edges and sketches behind them are
    hidden in the view and in picking alike.
  - Depth is reverse-Z with an infinite far plane and `Depth32Float`, with 4x MSAA when the
    adapter supports it. The surface is `Bgra8Unorm` or `Rgba8Unorm` when offered (never a float
    or snorm format an HDR setup lists first), else the first non-sRGB one. Model geometry draws
    over reference geometry (datum planes, axes) through a per-`Layer` depth bias, and model-layer
    fills (sketch regions) over the faces they lie on.
  - Picking renders a small window around the cursor into ID and depth targets and reads it
    back asynchronously, so hover never blocks the UI thread; `poll_pick` says `Pending`, `Ready`
    or `Failed` (a failed readback, or a pick whose frame was dropped before `submit`, which the
    next `begin_frame` abandons), and the app asks again after a failure. Hits carry their world position,
    which navigation uses as the orbit pivot, pan grab point and zoom anchor. Reference-layer
    fills (principal and datum planes) are drawn in a pass of their own first, nearest winning,
    and everything else is drawn over them, so a translucent plane owns a pixel only where no
    face, line, marker or model fill covers it and a face seen through a plane is picked.
  - Navigation has a single model: right-drag orbits (turntable around world Z), middle-drag or
    Shift+right-drag pans, the wheel and pinch zoom toward the point under the cursor, and
    view changes from the view cube or fit animate.
- **caditor**: the winit `ApplicationHandler` (`app.rs`), the egui integration drawn over the
  viewport (`overlay.rs`), the menu bar (`menu_bar.rs`), the tool ribbon (`toolbar.rs`), the
  status bar (`status_bar.rs`), the side panel (`panels.rs`) with the feature tree
  (`feature_tree.rs`) and parameter table (`parameter_table.rs`), the viewport widget with
  navigation, hover and selection (`viewport.rs`), the view cube (`view_cube.rs`) and the
  conversion of documents and results to a `Scene` (`scene.rs`). Selectable things are
  `Pickable` values built from stable IDs. The hover remembers the cursor position and view its
  pick result was made for, and a click that depends on it (anything but a drawing tool's) waits
  until a result for the current cursor and view has arrived, so it never acts on a stale hover.
  - Look (`fonts.rs`, `appearance.rs`, `icons.rs`, `widgets.rs`): the interface is set in Inter (the
    variable font in `assets/fonts`, OFL, registered at weights 400, 500 and 600 through its `wght`
    axis as the proportional, `medium` and `semibold` families) with Phosphor icons
    (`egui-phosphor`) in their own `icons` family, since Inter's private-use glyphs would otherwise
    shadow them. The fonts are installed on the first frame, which draws nothing, and the style is
    applied from the next. `appearance.rs` holds the theme as `Tokens` (surfaces, text, a blue
    accent, error, warning and success with tinted backgrounds) for dark, light and both
    high-contrast variants, builds each egui `Style` from them (Inter text styles plus a `section`
    style, spacing, radii, shadows), and tests every text pairing against its background. Panels
    read colours from `appearance::tokens(ui)` or the visuals, never fixed values, and take their
    icons from `icons.rs` (one per command, tool, constraint and feature kind). `widgets.rs` is the
    shared kit every panel and dialog is built from: `ToolButton` (icon above label), `section`,
    `properties`/`property`/`error_row`, `card`, `callout` and `pill` with a `Tone`, `icon_button`,
    `small_button`, `primary_button`, `menu_item`, `link_label`, `choose_in_view`, and
    `dialog`/`footer` (a titled modal with a close button and the primary action rightmost); dialog
    widths and list heights are clamped to the screen (`fitting_width`, `list_height`) so nothing
    clips at 200%. Icons and labels are separate text atoms, so tests find a button by its bare
    label. The 3D view keeps a dark canvas in every theme, so text drawn on it takes its colours
    from `canvas.rs` and sits on its translucent backdrop (`canvas::label`); a test holds every
    canvas colour to 4.5:1 (body text 7:1) over that backdrop on black, white and highlight colours.
  - Layout: the menu bar holds File, Edit, View, Model, Sketch and Help, built from the
    commands with their icons and shortcuts; items trigger their command and are enabled from
    the previous frame's offers (`Workspace::last_offers`), with the reason on hover. Its right
    side shows the document name (with an Unsaved pill) and the Search commands field. The
    ribbon groups Undo and Redo, New sketch, Extrude and Revolve, Fillet, Chamfer and Shell, and
    Plane and Axis as `ToolButton`s that wrap. While a sketch is edited a tinted sketch ribbon
    shows its name and state pill, the drawing tools, the constraints in a grid that wraps, Delete
    and Finish sketch. The status bar shows recompute progress (or Up to date, or a failed pill
    that focuses the first failed feature), saving, importing and exporting, the current notice
    (an edit clears info and refused-edit notices, never a `Notice::failure` from saving, opening,
    importing, exporting or the journal), the selection, the length unit (opening Preferences) and the interface size when it is not
    100%. The side panel has collapsible Features and Parameters sections; a feature row is its
    kind icon, name, state icon, edit and more buttons, highlighted with an accent bar while
    open, with failures and outdated states as callouts under it and its properties in a card.
    Clicking or tabbing to a row's name selects it in the tree (`PanelState::selected`, cleared
    when the view selection changes); Rename feature (F2), Move feature up or down and Delete
    feature act on it, else on the open feature (`feature_tree::current_feature`), and Delete
    selection (Delete) deletes it outside sketch editing. A section opens by itself when a
    rename or a focus request needs something inside it.
  - `Model` (`model.rs`) owns the `Editor` and the `Recomputer`. The UI gets `&Model` and
    returns `Action`s, which the app performs after the UI pass, so the UI never mutates the
    document directly. Each change submits a snapshot to the worker. Feature geometry is drawn
    from the last good result, tinted when the feature failed or is outdated.
  - Bodies (`bodies.rs`): `BodyMeshes` converts each body's final mesh into a `ShadedMesh` with
    its edge polylines (seams left out) once per result, keyed by the result's `Arc`, and keeps
    the previous one while a new mesh is not ready. A face is picked and selected as
    `Pickable::Face` with a `FaceKey` (its `FaceName` and its occurrence among faces sharing
    the name, in solid order), an edge as `Pickable::Edge` with its `EdgeName`; both are
    described in words from the `FaceOrigin` (for example "Extrude 1 side from Line 3"). While
    a sketch is edited, bodies are dimmed and not pickable.
  - Solid modelling (`solid_tools.rs`, `solid_panel.rs`): the toolbar's Extrude and Revolve
    take the edited sketch, else the sketch of the selected entities, else the last sketch, and
    a selected line or sketch axis as the revolve axis, else a selected principal axis, datum
    axis, straight edge or round face (the vertical axis otherwise); the panel's Use selected
    axis does the same for an existing revolve. A new
    feature is one-sided 10 mm or a full turn and adds to the last body, or makes a new one
    when there is none. It then opens: `SketchEditing` holds at most one open solid feature,
    never together with an edited sketch, and `editing::Context` carries both to the scene and
    to availability checks. An open feature's row in the tree is its property panel (sketch,
    regions, extent, axis, result and target body), where every change is one `SetFeatureKind`
    checked before it is offered, and its sketch's regions are drawn as fills that
    `Pickable::Region` clicks add or leave out, turning `RegionChoice::All` into the explicit
    keys. Double-clicking a face opens the feature that made it; Escape closes it last.
  - Fillets and chamfers (`blend_tools.rs`, `blend_panel.rs`): the toolbar's Fillet and Chamfer
    take the selected edges of one body and create a 1 mm feature that opens. While open, the
    body is drawn as it was before the feature with its edges as `Pickable::BlendEdge`: chosen
    edges and the chains they pull in are highlighted, and a click adds an edge or removes the
    references whose chain contains it. The panel switches between fillet and chamfer, edits
    the size and lists the edges in words.
  - Shells (`shell_tools.rs`, `shell_panel.rs`): the toolbar's Shell takes the selected flat
    faces of one body as the faces to open and creates a 1 mm feature that opens. While open,
    the body is drawn as it was before the feature (`BodyMeshes::body_before`, shared with
    blends) with its flat faces as `Pickable::ShellFace`, opened ones highlighted, and a click
    opens a face or closes it again. The panel edits the thickness and lists the open faces.
  - Datums (`datum_tools.rs`, `datum_panel.rs`): the toolbar's Plane starts from the selected plane
    or flat face (the XY plane otherwise), turned 45° about the selected axis, straight edge or
    round face when there is one (offset 0 mm), else offset 10 mm; Axis runs along the selected
    axis, straight edge or round face, or where two selected planes or flat faces meet. A selected
    face or edge that gives neither (curved, round rim, made later, not yet recomputed) makes both
    refuse with that reason. The new feature opens, and its panel has Use selected for its base and
    rotation axis (or for the whole axis) and fields for the angle and offset. Datums are drawn
    outside sketch editing as translucent squares and lines centred where the world origin projects
    onto them, picked as `Pickable::Datum`, tinted when failed, and double-clicking one opens it.
  - Sketches on faces and planes (`sketch_placement.rs`): New sketch starts on a selected principal
    plane, datum plane or flat face, and while choosing a plane a click on any of them does the
    same. The attachment is captured from the body's state where the sketch sits in the tree, so a
    face made further down is refused with the reason, and one whose body is not recomputed that
    far says so (`body_state_before` tells the two apart). Every refused New sketch, including a
    click on a curved face while choosing, and every Fillet, Chamfer, Shell or datum that cannot
    be created, leaves a notice with the reason. A sketch's row says which face or plane it
    lies on and offers Detach, and Place on selected plane or Place on selected face when one is
    selected. Everything that draws or maps onto a sketch takes its plane from the solved result
    (`scene::sketch_plane`, `displayed_sketch`), since an attached sketch's stored plane is only
    where it was placed.
  - `Model` also owns the file session: the path, the last saved document (the model is
    unsaved exactly when its document differs from it), the journal entries since then and the
    `Storage` worker, to which every change is recorded. `files.rs` is the file workflow: the
    File menu and shortcuts, native dialogs through the XDG desktop portal (`rfd`) on their own
    thread, loading and recovery scans on a background worker (each job contained by
    `catch_unwind`, a panic becoming that job's failure event), the unsaved-changes prompt before
    New, Open, Open Sample, Restore and Quit, the recovery offer and the load report. Save As
    appends `.caditor` to any other name, checks the target on the worker (refused while open in
    another window or holding unrecovered changes) and asks before replacing a file the dialog
    did not name. Quitting waits for the storage worker without blocking the UI, offering Quit
    Anyway after a few seconds. While the journal cannot be written the status bar shows a
    Not protected pill. `main.rs` installs the panic hook and a SIGTERM, SIGHUP and SIGINT
    handler (`signal-hook`) that flush the journal before the process ends, and starts with an
    empty model. Release builds unwind (`panic = "unwind"`), since containment relies on it, and
    `recompute.rs` fails to compile otherwise. The viewport
    fits the view once the first recompute of a newly opened model (another `Model::session`)
    is up to date.
  - Samples (`samples.rs`): three parametric models built through the document API, so they are
    always in the current format: a plate with two holes (extrude), a flanged spool (full
    revolve about the sketch's vertical axis) and an angle bracket (symmetric extrude with a
    hole removed by a second one). Every sketch is fully constrained and its dimensions refer to
    named parameters; a test recomputes each and checks its volume. File › Open Sample, the
    palette and the welcome dialog open one as an untitled, unmodified model after the
    unsaved-changes prompt.
  - Onboarding (`onboarding.rs`): the welcome dialog appears until it is closed once
    (`onboarding.welcomed`) and again from Help › Welcome and samples…; it offers an empty model
    (New, through the unsaved-changes prompt, unless the model already is empty and untitled), the
    samples and Open. Tips are shown one at a time as a card at the bottom centre of the
    viewport when no dialog is open, first of those that apply and were not dismissed: start with a
    sketch (empty model), draw (edited sketch with no geometry), constrain (edited sketch that can
    still move), extrude or revolve (a sweepable sketch and no body), navigate (a body exists), and
    the palette. Got it dismisses one (`onboarding.dismissed_hints`, unknown ids kept), Hide tips
    turns them off (`onboarding.hints`), and Preferences turns them back on or restores the
    dismissed ones.
  - About and the command line (`about.rs`, `cli.rs`): Help › About caditor shows the version
    and licences. `caditor [MODEL]` opens one model; `--version` and `--help` print and exit, and
    unknown options or several paths are refused. Screen readers reach the interface through
    egui's AccessKit integration (`egui-winit`'s `accesskit` feature): the window is created
    hidden, the adapter is attached (`Overlay::enable_accessibility`) and then it is shown, and
    `AppEvent::Accessibility` carries the adapter's requests to the overlay. The window's
    Wayland app ID and X11 class are
    `about::APP_ID` (`caditor`), which must match the desktop entry's name.
  - Export (`export.rs`): File › Export… (Ctrl+E) opens a dialog with the format (STL, 3MF or
    STEP), for meshes the resolution (showing the resulting deviation in millimetres) and a
    checkbox per body, all on by default.
    It waits for a running recompute, warns when features failed (each body is exported as its
    last good state), and after the save dialog runs on its own thread, shown in the status bar
    with a Cancel button. A path without the format's extension gets it appended, so an
    export never replaces a model file (`.stp` counts as STEP). The outcome is a notice with the
    body count and, for meshes, the triangle count.
  - Import (`import.rs`): File › Import… (Ctrl+I) picks a DXF or STEP file (by extension, else
    by whether it starts like STEP) and reads it on the files worker. A drawing becomes one
    "Import <file>" change to the sketch being edited when the command was given, else to a new
    sketch on the XY plane named after the file, which is then entered; a STEP model becomes one
    change adding an import feature per body. The outcome is a notice with the curve or body
    count; the file's notes (units, left-out objects, fitted curves, repaired edges) are shown in
    the same report dialog as a damaged file's problems, also when nothing could be imported. A result that arrives after another
    document was opened is dropped.
  - Version history (`history.rs`): File › Version History… (for a saved model) reads the
    versions from the file on the files worker and lists them newest first as "Saved 2 hours ago
    after “Edit width”", marking damaged ones. Restore loads that version in the background and
    applies `Document::transaction_to` (remove every feature and parameter, then insert the
    version's, keeping ID counters) as one "Restore earlier version" change (a refused one keeps
    its error, and one that arrives after another model was opened says it was not restored), so Undo brings back
    what was there and the next save keeps the replaced state as a version too.
  - Preferences (`preferences.rs`, `units.rs`): File › Preferences… (Ctrl+,) sets the length unit
    (millimetres, centimetres or metres; SI only, see `.claude/rules/ux.md`), the `Appearance`
    (theme: system, dark or light, the 3D view keeping its dark canvas; interface size from 75% to
    200% in eighths, also on Ctrl+Plus, Ctrl+Minus and Ctrl+0, applied as the egui zoom factor,
    whose own keyboard zoom and quit shortcut are switched off; high contrast), orbit and zoom speed
    with the zoom direction, and opens the shortcut editor. Every text colour of the theme is tested
    against its background (4.5:1, and 7:1 for body text and pills in high contrast, whose button
    and focus outlines reach 3:1). Changes apply at once and are saved on the files worker, a
    dragged slider only when it is released (`PreferencesCommand::Preview` until then).
    `Workspace` owns the `Preferences`, `Model` carries the length unit so every panel can use it,
    and `Action::Preferences` is performed with the workspace. The unit is for display and input
    only: models stay unit-explicit. Values and previews are shown in it (`LengthUnit::show`), a
    plain number typed where a length is expected gets it attached (`2` becomes `2 cm`), as does a
    plain value typed for a parameter that holds a length (one holding an angle gets degrees,
    `field::parameter_expression`), measured dimensions are written in it and new features start
    from round numbers in it.
  - Every numeric input is a `field::commit_field`: it commits on Enter or loss of focus,
    reverts on Escape, and keeps invalid text with its error inline instead of discarding it.
    Expression fields parse, evaluate and check the dimension before building a transaction;
    sketch dimensions go through `field::dimension_transaction`, which also applies the
    constraint's own rule (a radius above zero). Plain-key shortcuts (and Escape, Enter and
    Backspace in the viewport) run only when no widget held keyboard focus at the start of the
    frame or the end of the previous one, so Escape or Enter in a field never reaches the
    viewport; shortcuts with Ctrl or Alt, and plain keys that widgets do not use (letters, Delete,
    function keys; not Space, Enter, Tab, Escape, arrows, Page Up or Down, Home or End), need
    only that no text field has focus.
  - Commands (`commands.rs`): every toolbar, menu and sketch-toolbar action is a `Command` with
    a stable id, a title, a category, a `Scope` (anywhere, or only while a sketch is edited) and
    default shortcuts. The `Keymap` in `Preferences` holds the user's bindings as overrides of
    the defaults, stored as `keys.<id>` lists of text such as `Ctrl+Shift+Z` (only changed
    commands are written, so unreadable or newer entries survive, and a command none of whose
    stored bindings can be read keeps its defaults). Each frame `app::show`
    dispatches key presses to commands (exact modifiers first; extra Shift or Alt is ignored only
    for punctuation keys; sketch-scope bindings win while a sketch is edited; Backspace and
    Delete are left to drawing while a shape is in progress; a held key repeats only camera moves,
    highlight steps, undo, redo and interface size) and hands a `CommandFrame` to the
    toolbars and the viewport. Whoever draws a command's button calls `invoke` with its
    availability, which records an `Offer` and says whether it was triggered; a triggered
    command that is unavailable becomes a notice with the reason. Hover texts and menu items
    show the current binding (`commands::display`). Esc, Enter and Tab are reserved and cannot
    be bound.
  - The command palette (`palette.rs`, Ctrl+Shift+P or the menu bar's Search commands) lists
    this frame's offers, so only commands that fit the context appear: matches by title start,
    word starts, substring, then scattered letters, available before unavailable, recently used
    first; arrows move, Enter runs, and an unavailable highlighted command shows its reason. The
    chosen command is triggered on the next frame. The shortcut editor (`shortcut_editor.rs`,
    from Preferences, the File menu or the palette) lists every command by category with its
    bindings: Add records the next key press (Esc cancels), a binding already used in an
    overlapping scope asks before moving it, clicking a binding removes it, and Reset or Reset
    all go back to the defaults.
  - Keyboard-only operation: every command is in the palette, including recompute (F5) and its
    cancel, adding a parameter, opening each recent model (an `Offer` may carry a detail, here
    the file name, shown after the title), recovering unsaved work, cancelling an export and
    dismissing the notice; Tab moves between widgets and Escape
    leaves them; each feature row has a "⋯" menu with what its right-click menu holds. In the
    viewport, standard views (Alt+0 to Alt+6), orbit (arrows), pan (Shift+arrows) and zoom (Page Up
    and Page Down) are commands; N and Shift+N step a keyboard highlight through the scene's
    pickables in pick-table order, each once (it is drawn and described like hover, a pointer move
    or Escape clears it), Space acts on it as a click would (toggling it in the selection, or a
    region, blend edge, shell face or sketch plane as in those modes, through `pick_action`), and
    Enter opens what it belongs to as a double-click would. While a drawing tool is active, typing a
    digit, sign, point, `(` or `@` opens the typed-point field (`typed_point.rs`): two length
    expressions in the preferred unit, split at top-level commas, `@` for an offset from the last
    placed point; Enter places the point through `Drawing::type_point` (landing exactly on an
    existing point or the pending one snaps to it, like a click), an error keeps the field open with
    the reason, and Escape closes it without touching the shape.
  - Sketch editing is a context, not a mode: `editing.rs` holds which sketch is edited and the
    active `Tool`, changed by `Action::Editing` commands that `app::perform` routes after the UI
    pass; it ends by itself when the sketch disappears or another document is opened. The
    viewport watches it: entering turns the camera to face the sketch plane and fits it, the
    grid moves to that plane, the sketch's origin and axes become pickable references, other
    features are dimmed and unpickable, and the selection keeps only that sketch. Clicks go to
    selection unless the tool `draws`. Escape backs out one step at a time: plane choice, shape
    in progress, tool, selection, then editing.
  - Drawing tools (`drawing.rs`: point, line, rectangle, circle, arc, spline) keep their clicked
    points, hover and arc sweep as viewport UI state and build one transaction per finished
    shape (`Draw line`, …), settled first like any sketch transaction. Lines chain, each new
    line joined to the last end by `Coincident`, until Escape, a click on the last point or a line
    ending on the chain's first point (or the point it snapped to), which closes the outline;
    splines finish on Enter or a click on the last control point. An arc runs the way the
    pointer swept around its centre, and its end is projected onto the circle through its
    start. Every inferred constraint is checked with `Sketch::check_constraint` on a shadow of
    the sketch and skipped if refused. A shape with no size (a flat rectangle, a line, circle or
    arc ending where it starts) is refused with a `Degenerate` reason: a notice for a click, the
    field's error for a typed point.
  - Snapping (`snap.rs`) runs on the UI thread against the displayed sketch, in screen space
    through the view: the shape's own pending point first, then existing points and the origin
    within 8 logical pixels, then lines, circles, arcs and the axes within 6, projecting onto
    the curve. A snapped point gets a `Coincident` with its target. A line end that snapped to
    nothing becomes exactly horizontal or vertical within 3° or 6 pixels and gets that
    constraint. The preview, snap marker and snap label are drawn from this state, and the
    snap target replaces the GPU hover while a drawing tool is active.
  - `sketch_tools.rs` turns the selection into candidate constraints checked by
    `Sketch::check_constraint`; `sketch_toolbar.rs` offers them as buttons and commands
    (Shift+letter by default), disabled with what to select, and the drawing tools on plain
    letters (P, L, R, C, A, S). Dimensions start at the value measured on the
    displayed geometry. Every sketch transaction first settles the sketch to the last result,
    but only when that result is up to date (`Model::settled_sketch`). The UI never solves; it
    reads constraint states, degrees of freedom and redundancies from the last evaluation
    (`sketch_status.rs`, colouring in `scene.rs`). `scene::displayed_sketch` is the definition
    with solved positions wherever the last result has the same entity.
  - The edited sketch is annotated over the viewport with the egui painter (`annotations.rs`,
    placement in `annotation_layout.rs`), from the displayed geometry projected through the
    current view, with offsets and sizes in screen points and no stored positions. Distances
    between points are parallel dimension lines with extension lines, point–line distances are
    perpendicular, angles are arcs at the lines' intersection (between their closest ends when
    nearly parallel), radii are leaders with an `R` prefix; dimensions sit away from the
    sketch's centre. Other constraints are glyphs stacked beside each constrained entity on the
    opposite side, painted as shapes or as letters the default fonts carry. Labels show the
    expression in the document's naming, followed by its value when it is not a literal.
    Conflicting and redundant constraints take the error and warning colours.
  - Labels and glyphs are `Pickable::SketchConstraint`: hovering highlights the constrained
    entities, clicking selects (Shift or Ctrl toggles), and Delete removes selected constraints
    and entities in one transaction. They are painted but not interactive while a drawing tool
    is active. Double-clicking a label opens an inline `commit_field` on the canvas with the
    value selected; so does a new dimension from the constraint tools and any
    `Focus::Dimension` of the edited sketch, which the app takes from the panels and hands to
    the viewport, waiting until the dimension can be drawn.
  - `ui_tests.rs` drives the real toolbars, panels and viewport through a headless egui context
    with synthetic input; picking needs the GPU, so the harness answers each pick request as the
    renderer would, with nothing under the cursor unless a test hovers a chosen `Pickable`
    (`hover_pickable`, through `hover_through_pick`) until the pointer moves, and can hold answers
    back like a slow GPU (`picks_held`). Tests also set the viewport selection directly, drawing
    tests click sketch positions mapped to the screen through the view and annotation tests click
    the painted labels and glyphs.

Entities, constraints, parameters and features are referred to by stable IDs (`EntityId`,
`ConstraintId`, `ParameterId`, `FeatureId`). IDs come from a per-container counter and are never
reused, never positional, and survive the removal of anything else. Anything that references
model geometry must keep this property, because positional naming is the root of FreeCAD's
topological naming failures.

## Releases

caditor ships as plain release binaries: a `.tar.zst` per release with an installer, built on
Ubuntu 22.04 by `.github/workflows/release.yml` when a `v<version>` tag is pushed, and published
as a GitHub release. `packaging/` holds the desktop entry, icon, metainfo, MIME type, installer,
the cargo-about licence template and `build-release.sh`; `docs/RELEASING.md` explains the choice
and the release steps. `CHANGELOG.md` lists every release: a change users notice adds a line
under `## [Unreleased]` in the same commit, written for users.

## Roadmap

`docs/TODO.md` holds the roadmap, the open design decisions and the project's direction. Check
it before starting new work. It lists only what remains: the change that implements an item
deletes it (never ticks it), and a resolved decision is removed once it is recorded where it
belongs, such as the Architecture section or a rules file.

## Rules

- `.claude/rules/ux.md`: UX requirements and the FreeCAD failure modes to avoid.
- `.claude/rules/reliability.md`: crash and data-loss policy, panic lints.
- `.claude/rules/rust-style.md`: formatting, imports, comments, collections and locks, `unsafe`,
  edition.
- `.claude/rules/dependencies.md`: how dependencies are declared and versioned.
