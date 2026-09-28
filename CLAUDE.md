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
```

`rust-formatter` formats both `.rs` and `.toml` files. It replaces `cargo fmt` and `rustfmt`
entirely; see `.claude/rules/rust-style.md`.

## Architecture

The Cargo workspace is `crates/*`. Dependencies point in one direction only:

```
caditor-expression  ←──────────────────┐
       ↑                               │
caditor-geometry  ←  caditor-sketch  ←  caditor-document  ←  caditor-file  ←  caditor (bin)
   ↑   ↑                                   │                                      │
   │   └──────────────  caditor-render  ←──┼──────────────────────────────────────┘
   └──  caditor-kernel  ←──────────────────┘
```

`caditor-expression` has no workspace dependencies; the sketch, document, file and app crates all
use it. `caditor-file` and the app also use the geometry and sketch crates directly.
`caditor-kernel` depends only on `caditor-geometry`, never on the sketch or document crates; the
document and file crates use it for solid features, and the app for face and edge names and
meshes.

- **caditor-geometry**: the math vocabulary, as f64 `glam` aliases (`Point3`, `Rotation3`, …)
  plus `Plane` (origin, normal and in-plane x axis, also used as the frame of every circle and
  rotational surface), `Ray`, `Aabb`, `Aabb2` and the rigid transforms `RigidTransform` and
  `RigidTransform2`. The world is Z-up and
  model data is f64 throughout; conversion to f32 happens only at the GPU boundary in the
  renderer.
- **caditor-expression**: units and expressions. A `Quantity` is an f64 in base units
  (millimetres and degrees) with a `Dimension` of length and angle powers. A plain number takes
  the dimension of whatever it is added to, and a field that expects a length takes a plain
  result as millimetres. Trigonometry reads a plain number as radians. An `Expression` refers to
  parameters by `ParameterId`, never by name, so renaming a parameter rewrites every
  expression's text. Parsing limits length and nesting so that hostile input cannot overflow the
  stack, and errors are plain-language clauses.
- **caditor-sketch**: 2D sketches on a `Plane` and caditor's own constraint solver.
  - Entities are points, lines, circles (centre point and radius), arcs (centre, start and end
    points, counter-clockwise) and clamped B-splines through control points. Every sketch also
    has a fixed origin and two axes under reserved IDs (`EntityId::ORIGIN`, `HORIZONTAL_AXIS`,
    `VERTICAL_AXIS`) that the counter never reaches; stored IDs stay below 2^63.
  - Constraints have stable `ConstraintId`s: coincident (point–point or point on a curve),
    horizontal, vertical, parallel, perpendicular, tangent, equal, and the dimensions distance,
    angle and radius, whose values are expressions. `check_constraint` refuses constraints that
    do not fit the entity kinds, so the UI can ask before offering one. `insert_entity` and
    `insert_constraint` take explicit IDs and check references, for loading.
  - `solve` evaluates the dimensions, then runs damped Gauss–Newton with minimal-norm steps
    (SVD from `nalgebra`) on each independent part of the system, so geometry that already
    satisfies its constraints does not move and under-constrained geometry moves as little as
    possible. Every equation has an analytic gradient; two-branch equations (tangent side,
    signed distance) take their branch from the starting geometry, so a solve never flips.
    Degrees of freedom and each entity's constraint state come from the rank and null space
    of the Jacobian at the solution; a constraint whose equations add no rank over older ones
    is reported as redundant, naming what it duplicates. When a part does not converge, a
    deletion filter finds a minimal set of conflicting constraints, which recompute reports as
    the feature's error with `FeatureError.constraints` and `FixTarget::Constraint`.
- **caditor-kernel**: caditor's own B-rep geometry kernel (no truck, no OpenCascade), the base of
  solid modelling.
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
  - `Surface`: plane, cylinder, cone, sphere, torus, extrusion and revolution. u is the angle
    around the axis (the frame normal) on every rotational surface; the cone's v is slant
    distance from its reference circle, the sphere's v latitude, the torus's v the tube angle,
    and a revolution's v the profile parameter. An extrusion is (profile parameter, distance).
    du × dv points outward on every elementary surface. Singularities are always `Pole`s: v
    isolines where du vanishes (sphere poles, cone apex, a revolution profile ending on its
    axis). `project` returns the periodic representative nearest a hint, else the principal one
    in [0, period); on spline profiles it keeps the closest point near the hint when no other is
    closer by more than the resolution, so self-crossing profiles project consistently.
    `same_surface` gives the `Sense` between the normals of two coincident surfaces whatever
    their frames and seams: analytic for elementary pairs, and by mutual sampled projection when
    an extrusion or revolution is involved.
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
    both accept; a fitted pcurve end at a pole takes the pole's v exactly. Faces carry a
    `FaceName` and an optional `FaceOrigin`, edges an `EdgeName`.
  - `Solid::validate` checks a closed, oriented 2-manifold whose geometry agrees with its
    topology (edge uses and senses, loop chaining in space and in uv, vertices on curve ends,
    edges on both surfaces, pcurves on their edges, loop winding and nesting, shell
    connectivity, Euler–Poincaré per shell, positive volume for lumps and voids inside a lump)
    and returns the first `ValidationError`, with ids. The volume checks run on a coarse mesh and
    retry finer before reporting a void outside its lump.
  - Tessellation samples each edge once and shares its positions between both faces. Each face
    is a constrained Delaunay triangulation (spade) of its loops in (u, v), scaled by the mean
    surface speeds, plus a uniform grid of interior points spaced by curvature and kept clear of
    the boundary (a direction without curvature gets cells at most four times longer than the
    curved one's, so no triangle spans far across a curved direction); triangles are kept by the
    parity of constraint crossings from outside. Pole-line points share the pole's position and
    the triangles that collapse there are dropped, so the mesh stays watertight. `Mesh` holds
    shared positions, per-face vertices with exact surface normals, triangles, each face's
    triangle range and each edge's polyline, and computes volume, area and centroid by the
    divergence theorem. When a face boundary crosses itself at the requested tolerance (loops
    closer than the sampling error), tessellation retries with halved chord and angle a few times
    before failing.
  - Naming (`naming/`): `FaceName`, `EdgeName` and `VertexName` are 128-bit FNV-1a digests over a
    canonical little-endian encoding with a tag byte per constructor; they are stored in files, so
    the encoding and the pinned digests in `naming/tests.rs` never change. Faces: `side(feature,
    PieceId)`, `start_cap` and `end_cap(feature, RegionKey)`. Edges: `between` (unordered face
    pair), `seam(face)` for the profile seam of a full revolution, and, when several edges share
    a name, `between_at(left, right, from, to)` with the vertex names (sets of faces around each
    end) and the faces oriented by the edge, then `occurrence` ordered by position as a last
    resort. `FaceOrigin` (side of an entity, start or end cap, with the raw feature and entity
    ids) says in words what a face came from. Later generators (a fillet face named by the edge it
    replaced, boolean fragments that keep their name) are new constructors with new tags.
  - References (`naming/reference.rs`) are how later features keep hold of generated topology.
    A `FaceReference` is a face's name, origin and the set of its neighbours' names. It resolves
    to the one face with that name; among fragments of a split face, to the one whose neighbours
    match best (most shared, then fewest differences); and when the name is gone (its region
    key or piece id changed), to the face of the same origin sharing at least one neighbour.
    An `EdgeReference` is an edge's name, its two face names and its end vertex names, resolved
    by name, else among the edges between the same faces by matching ends. A tie is
    `ReferenceError::Ambiguous` with the candidates and no match is `Missing`: resolution never
    guesses between equals.
  - Profiles (`profile/`): `Profile::new` takes `ProfileCurve`s (lines, circles, counter-clockwise
    arcs, clamped B-splines with an explicit knot vector) tagged with the sketch entity id as a
    plain u64, and builds the planar arrangement with tolerance 1e-7 of the profile size (at
    least `LINEAR_RESOLUTION`): analytic line and circle intersections, subdivision on monotone
    spans plus damped Newton for splines (self-crossings included), endpoints landing on curves,
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
    entity ids. `Region::triangulate` samples the loops and keeps the constrained Delaunay
    triangles inside by the parity of constraint crossings, for drawing regions as fills.
  - Intersections (`intersect/`) take a `SurfacePatch` (a surface and a finite uv box; periodic
    boxes wrap, poles accept any u) so booleans intersect face patches, and restrict curves to a
    parameter interval. Coincidence within tolerance is detected, never guessed: a curve lying
    in a surface or on another curve is an overlap interval, and coincident surfaces return
    `SurfaceIntersection::Coincident(Sense)` from `same_surface`. Points within
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
      become parallel (reported as tangent points). Near poles the contact is solved with one
      surface as carrier and the other's signed distance. A marched branch that is a line, circle
      or ellipse within half the resolution is returned as that curve.
  - Point classification (`topology/classify.rs`, `SolidClassifier` to reuse per solid):
    `classify_point` gives `Inside`, `Outside` or `OnBoundary(face)` exactly: a point on a face's
    surface and inside its boundary is on it, otherwise rays from a fixed list of directions are
    intersected with each face's surface through `intersect_curve_surface`, and the nearest
    crossing's outward normal decides; a ray that grazes, is tangent, lies in a face or meets an
    edge or vertex is discarded for the next direction. `point_in_face(face, uv)` (`Inside`,
    `Outside`, `OnBoundary`) uses the pcurve polygons by parity over periodic shifts (poles probed
    just off the pole line), and near the boundary (within a few `PCURVE_TOLERANCE`) the exact
    edge: the side of the nearest non-seam coedge, or of both coedges at a vertex (convex corners
    need both). `classify_boundary_point(point, normal)` adds `Coincident { face, sense }` for a
    point on a face whose normal is parallel, and `Touching(face)` otherwise.
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
      vertices of the other solid lying on it and each face–face branch at every pooled vertex on
      it; a branch piece is kept where its midpoint is strictly inside both faces, and an edge piece
      lying in the surface of a face of the other solid and inside it is a cut in that face. Pieces
      with the same end vertices and geometry are one edge, so an intersection along an existing
      edge and coincident faces need no special case.
    - Each face is traced into loops from its boundary pieces (hinted by the original pcurves) and
      its cuts (both ways, dangling ones pruned): at each vertex the next edge is the first one
      clockwise from the arriving one about the outward normal, with ties and cusps decided by
      chords at a common distance. Loops are fitted in the face's chart; a run of cuts leaving a
      pole is shifted by whole periods to meet the next boundary edge, pcurve ends are snapped to
      their vertices, and a hole goes to the smallest outer loop containing a point of it that is
      not on that loop.
    - Each fragment is classified against the other solid at up to three interior points (inside
      or outside wins over coincident or touching; inside and outside together is `Ambiguous`) and
      kept by the operation. Of coincident faces only the first solid's fragment can stay: with
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
    (`Chamfer`) edges by sweeping a tool and one boolean per tool, valid or a `BlendError` that
    names the edge. Chosen edges first grow along tangent-continuous chains (`blend_chain`) and
    smooth edges are dropped. Supported edges are straight ones whose faces run along them
    (planes, parallel cylinders), swept by extrusion, and circles whose faces share their axis
    (planes, cylinders, cones, spheres, tori), swept by revolution; the cross-section is solved
    in 2D (`section.rs`: fillet circle from the offset curves, chamfer points at equal
    distance). Convex tools are lifted clear of the faces they cut and subtracted; concave ones
    are flush and added, all concave edges first, then the convex ones re-found by reference in
    the filled solid. Ends continuing into another chosen edge stop flush, ends on a face
    perpendicular to the edge stop there, ends on a slanted face extend past it when the
    extension lies where the operation changes nothing, else are clipped by the face's plane.
    Three convex straight edges filleted at a vertex of three planes get a spherical corner
    (`corner.rs`: a hexahedron minus the rolling ball, built through `Plan`); other corners
    mitre. Faces are named `FaceName::blend(feature, edge)` and `corner(feature, vertex)` with
    `FaceOrigin::Fillet` or `Chamfer`.
  - Shell (`shell/`): `shell(solid, open, thickness, feature)` offsets every face by the
    thickness with the topology kept (each vertex solved by minimal-norm Newton on the offset
    surfaces, each line or circle edge rebuilt through its offset ends and checked on both
    offset surfaces) and subtracts the result. Only flat faces open. An opened face with no
    smooth edge to a closed face is offset outward, so the inner solid passes through it and
    the body needs room only across its walls; this attempt counts only when every closed
    face's inner face survives the subtraction. Otherwise (or when it fails) every face is
    offset inward and a prism swept outward from the offset copy of each opened face is
    unioned before subtracting, which needs the thickness below half the body in every
    direction. Inner faces are `FaceName::shell(feature, original)` with `FaceOrigin::Shell`.
- **caditor-document**: the parametric model: parameters, the ordered feature tree and
  everything that changes or recomputes it.
  - Every mutation is a `Transaction` of `Edit`s passed to `Document::apply`, the only public
    mutator of content (`reserve_ids_below` only raises the ID counters, for loading). `apply`
    is atomic and returns the inverse transaction, and `Editor` keeps undo and redo as stacks of
    these inverses. Edits carry their IDs, so redo restores the same IDs, and ID counters never
    move backwards. Edits refuse to break invariants: unknown references, parameter cycles,
    deleting something still in use, or moving a feature past one it depends on.
    `Document::check` runs a transaction on a clone so the UI can report the error before
    committing.
  - Sketch content changes only through sketch edits (add, remove or set an entity, add or
    remove a constraint, set a dimension). Removing an entity that something still uses is
    refused rather than cascaded; `TransactionBuilder::remove_sketch_items` expands a user's
    deletion into constraints first, then curves, then points. Setting an entity changes only
    its value, never its kind or the points it uses. `settle_sketch` moves the definition to a
    solved shape so the next solve starts from what the user sees.
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
    angles in degrees) that must be above zero; one-sided extents flip with `reversed`, and a
    revolve's axis (`RevolveAxis`) is a line of its sketch, one of the sketch axes, or an
    `AxisReference` to a model axis that must lie in the sketch plane. Inserting one checks that
    its sketch is a sketch and its target makes a body; `SetFeatureKind` replaces its settings
    but never its kind, and a feature whose body others change keeps making a new body. A sketch
    line used as a revolve axis cannot be deleted.
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
    turns it about the axis by an angle, then offsets it along its normal; a `DatumAxis` runs
    along an axis reference or where two planes meet. Plane stays plane and axis stays axis
    under `SetFeatureKind`, and edits refuse a sketch or plane based on something that is not a
    datum plane (`NotAPlane`) or an axis reference to something that is not a datum axis
    (`NotAnAxis`). `FeatureKind::bodies_used`, `planes_used` and `axes_used` extend
    `features()`, so dependents, moves and deletions account for them.
  - Recompute: `ParameterValues` evaluates parameters in dependency order and reports cycles
    rather than following them. `Recompute` walks the features in tree order and reuses a
    cached result when the feature definition (an `Arc`, compared by pointer first), the values
    and names of the parameters it uses, and its upstream feature results are all unchanged. A
    failing feature is `Failed` with a `FeatureError` (reason, remedy and a `FixTarget`) and
    keeps its last good result. Its dependents fail with a pointer back to it, and everything
    else is unaffected. A panic inside an `Evaluator` is caught and becomes that feature's
    error. Each body's latest good state is carried through the tree and is part of the next
    change's upstream of every feature that uses the body (`bodies_used`): a feature that
    changes a body gets its current solid through `Inputs::body`, and a failing one is skipped, so later features of the body build on the
    state before it. `Evaluation::body` gives each body's final solid and `body_result` the
    shared result holding it. Solid features map profile, sweep and boolean errors to sentences
    naming the sketch curves involved.
  - Display data is computed on the worker at the end of each run and cached inside the shared
    results (`OnceLock`), so the UI only reads it: each body's final state is tessellated
    (`SolidResult::mesh`; intermediate states are not), and every sketch that a solid feature
    sweeps gets its regions with a triangulation each (`SketchResult::regions`). A panic or
    failure while meshing leaves the body without a mesh (`mesh_failed`) but keeps its shape for
    later features.
  - `Recomputer` runs recompute on a worker thread. A newer submission or `cancel` stops the
    running job between features (evaluators also receive a `CancelToken`), and features that
    were not reached are reported as `Outdated`. The worker calls a wake callback after each
    report so the UI can redraw.
- **caditor-file**: persistence. A model file (`.caditor`) is UTF-8 JSON Lines: a header
  `{"format":"caditor","version":N}`, one self-contained record per parameter and per feature
  carrying its stable ID, and the ID counters. Expressions are stored as canonical text that
  refers to parameters as `$<id>` (`Expression::to_stored_text` and `parse_stored`), so stored
  text never depends on names, and numbers round-trip exactly. Every format version that has
  shipped stays readable. Version 3 added `extrude` and `revolve` features; region keys are
  stored as 32-digit hex strings, and an unreadable extent falls back to 10 mm or 360° with a
  report. Version 4 added a sketch's `attachment` (body ID, face name, origin and neighbour names
  as hex digests); older readers ignore it and keep the sketch on its stored plane. An
  unreadable attachment, or one whose body could not be restored, leaves the sketch on its
  stored plane with a report. Version 5 added `fillet` and `chamfer` features (body, size and
  edges as name, face and end digests); an unreadable edge is left out with a report. Version 6
  added `shell` features (body, thickness and opened faces stored like an attachment's face);
  an unreadable face is left closed with a report. Version 7 added `plane` and `axis` features
  (references as tagged records, faces and edges stored like attachments and blend edges), a
  sketch's `datum`, which older readers ignore so the sketch stays on its stored plane, and a
  revolve `axis` that is either a sketch entity ID or an axis reference, which older readers
  cannot read, so they leave that revolve out with a report rather than turn it about the
  wrong axis. A sketch whose datum plane could not be restored stays where it was.
  - Saving writes a temporary sibling, fsyncs it, renames it over the target and fsyncs the
    directory, keeping the target's permissions. Overwriting a file that loaded with problems
    first keeps the original as `<name>.damaged.caditor`.
  - Loading is partial. Each line and each sketch item is read on its own (`Lenient`), and the
    pieces are assembled through `Document::apply`, so a loaded model always satisfies the
    document invariants. Damaged or unknown (newer) records are left out, a lost parameter that
    something still uses becomes a stand-in with value 0, unusable or duplicate names are
    renamed, a parameter cycle is broken at the parameter that closes it, and an unreadable
    dimension takes its drawn length. Each of these is reported in plain language.
  - The recovery journal is JSON Lines as well: a header naming the file, a snapshot of the last
    saved state, then one entry per change (`apply`, `undo` or `redo` with the transaction that
    was applied), each line carrying a CRC32 of its entry. Replay stops at the first bad line,
    so a torn tail loses only the changes after it, and replaying through an `Editor` restores
    the undo history. New edit kinds do not bump the journal version: an older reader stops at
    the first entry it cannot read and keeps everything before it, whereas a newer version
    number would make it refuse the whole journal. The journal lives next to the file as
    `.<name>.journal`, falling back to `$XDG_STATE_HOME/caditor/recovery/`, where untitled
    documents keep theirs. Its owner holds an exclusive lock on it, which is how the startup
    scan and other instances tell a live journal from an orphan.
  - `Storage` is one worker thread per open document. It owns the journal and performs saves,
    so appends, saves and the rebase of the journal onto the saved snapshot stay in order, and
    it fsyncs after each batch of entries. A `Flusher` lets the panic hook wait for pending
    entries.
  - Recovery (`scan`, `journal_for`) inspects unlocked journals in the recovery directory and
    next to recent files, deletes those with nothing to recover (no net change, or already in
    the file) and returns the rest with a replayed `Editor`.
  - Mesh export (`export/`): `export_mesh` tessellates each `ExportBody` (a name and a solid) at
    a `MeshResolution` (coarse, standard or fine: a chord that is a fraction of the largest
    body's diagonal, and 20°, 10° or 5° between triangles), keeps only the positions the
    triangles use and drops collapsed triangles, then writes binary STL (every body in one
    surface, facet normals from the winding) or 3MF (one named object per body, millimetres) and
    saves it atomically like a model. Cancellation is checked between bodies and before writing,
    and failures are sentences naming the body. The 3MF package is written by a small ZIP writer
    (`zip.rs`: deflate through `miniz_oxide` unless storing is smaller, CRC32, no ZIP64).
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
    adapter supports it. Model geometry draws over reference geometry (datum planes, axes)
    through a per-`Layer` depth bias, and model-layer fills (sketch regions) over the faces
    they lie on.
  - Picking renders a small window around the cursor into ID and depth targets and reads it
    back asynchronously, so hover never blocks the UI thread. Hits carry their world position,
    which navigation uses as the orbit pivot, pan grab point and zoom anchor.
  - Navigation has a single model: right-drag orbits (turntable around world Z), middle-drag or
    Shift+right-drag pans, the wheel and pinch zoom toward the point under the cursor, and
    view changes from the view cube or fit animate.
- **caditor**: the winit `ApplicationHandler` (`app.rs`), the egui integration drawn over the
  viewport (`overlay.rs`), the side panel (`panels.rs`) with the feature tree
  (`feature_tree.rs`) and parameter table (`parameter_table.rs`), the toolbar with undo, redo
  and recompute status (`toolbar.rs`), the viewport widget with navigation, hover and selection
  (`viewport.rs`), the view cube (`view_cube.rs`) and the conversion of documents and results to
  a `Scene` (`scene.rs`). Selectable things are `Pickable` values built from stable IDs.
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
  - Datums (`datum_tools.rs`, `datum_panel.rs`): the toolbar's Plane starts from the selected
    plane or flat face (the XY plane otherwise), turned 45° about the selected axis, straight
    edge or round face when there is one (offset 0 mm), else offset 10 mm; Axis runs along the
    selected axis, straight edge or round face, or where two selected planes or flat faces
    meet. The new feature opens, and its panel has Use selected for its base and rotation axis
    (or for the whole axis) and fields for the angle and offset. Datums are drawn outside
    sketch editing as translucent squares and lines centred where the world origin projects
    onto them, picked as `Pickable::Datum`, tinted when failed, and double-clicking one opens
    it.
  - Sketches on faces and planes (`sketch_placement.rs`): New sketch starts on a selected
    principal plane, datum plane or flat face, and while choosing a plane a click on any of them
    does the same. The attachment is captured from
    the body's state where the sketch sits in the tree, so a face made further down is refused
    with the reason. A sketch's row says which face or plane it lies on and offers Detach, and
    Place on selected plane or Place on selected face when one is selected. Everything that draws or maps onto a sketch takes its
    plane from the solved result (`scene::sketch_plane`, `displayed_sketch`), since an attached
    sketch's stored plane is only where it was placed.
  - `Model` also owns the file session: the path, the last saved document (the model is
    unsaved exactly when its document differs from it), the journal entries since then and the
    `Storage` worker, to which every change is recorded. `files.rs` is the file workflow: the
    File menu and shortcuts, native dialogs through the XDG desktop portal (`rfd`) on their own
    thread, loading and recovery scans on a background worker, the unsaved-changes prompt before
    New, Open, Restore and Quit, the recovery offer and the load report. `main.rs` installs the
    panic hook that flushes the journal.
  - Export (`export.rs`): File › Export… (Ctrl+E) opens a dialog with the format, resolution
    (showing the resulting deviation in millimetres) and a checkbox per body, all on by default.
    It waits for a running recompute, warns when features failed (each body is exported as its
    last good state), and after the save dialog runs on its own thread, shown beside the File
    menu with a Cancel button. A path without the format's extension gets it appended, so an
    export never replaces a model file. The outcome is a notice with the body and triangle count.
  - Every numeric input is a `field::commit_field`: it commits on Enter or loss of focus,
    reverts on Escape, and keeps invalid text with its error inline instead of discarding it.
    Expression fields parse, evaluate and check the dimension before building a transaction;
    sketch dimensions go through `field::dimension_transaction`, which also applies the
    constraint's own rule (a radius above zero). Viewport and toolbar shortcuts run only when
    no widget held keyboard focus at the start of the frame or the end of the previous one, so
    Escape or Enter in a field never reaches the viewport.
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
    line joined to the last end by `Coincident`, until Escape or a click on the last point;
    splines finish on Enter or a click on the last control point. An arc runs the way the
    pointer swept around its centre, and its end is projected onto the circle through its
    start. Every inferred constraint is checked with `Sketch::check_constraint` on a shadow of
    the sketch and skipped if refused.
  - Snapping (`snap.rs`) runs on the UI thread against the displayed sketch, in screen space
    through the view: the shape's own pending point first, then existing points and the origin
    within 8 logical pixels, then lines, circles, arcs and the axes within 6, projecting onto
    the curve. A snapped point gets a `Coincident` with its target. A line end that snapped to
    nothing becomes exactly horizontal or vertical within 3° or 6 pixels and gets that
    constraint. The preview, snap marker and snap label are drawn from this state, and the
    snap target replaces the GPU hover while a drawing tool is active.
  - `sketch_tools.rs` turns the selection into candidate constraints checked by
    `Sketch::check_constraint`; `sketch_toolbar.rs` offers them as buttons and Shift+letter
    shortcuts, disabled with what to select, and the drawing tools on plain letters (P, L, R,
    C, A, S). Dimensions start at the value measured on the
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
    with synthetic input; picking needs the GPU, so tests set the viewport selection directly
    or feed a pick result for a chosen `Pickable` (`hover_through_pick`),
    while drawing tests click sketch positions mapped to the screen through the view and
    annotation tests click the painted labels and glyphs.

Entities, constraints, parameters and features are referred to by stable IDs (`EntityId`,
`ConstraintId`, `ParameterId`, `FeatureId`). IDs come from a per-container counter and are never
reused, never positional, and survive the removal of anything else. Anything that references
model geometry must keep this property, because positional naming is the root of FreeCAD's
topological naming failures.

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
