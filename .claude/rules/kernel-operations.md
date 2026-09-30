---
paths:
  - "crates/caditor-kernel/src/build/**"
  - "crates/caditor-kernel/src/boolean/**"
  - "crates/caditor-kernel/src/blend/**"
  - "crates/caditor-kernel/src/shell/**"
  - "crates/caditor-kernel/src/pattern/**"
  - "crates/caditor-kernel/src/box_tree.rs"
---

# Builders (`build/`)

- `extrude(plane, regions, LinearExtent, feature)` and `revolve(plane, regions, Axis2,
  AngularExtent, feature)` plan vertices, edges and faces, merge coincident vertices within a shell
  (pinched regions), name every face and edge, group faces into shells by shared edges and emit
  through `SolidBuilder`: valid or a `SweepError`.
- Extrusion sides are planes, cylinders or extrusion surfaces; revolution sides are planes,
  cylinders, cones, spheres, tori or revolution surfaces (splines, and arcs whose circle reaches the
  axis, converted to rational splines). Faces on extrusion and revolution surfaces get exact
  straight pcurves; the rest are fitted.
- The start cap is the one at the extent's start (the sketch plane for `one_side`) whichever way the
  sweep runs, so flipping the direction keeps every name.
- A profile on the right of the revolution axis is revolved about the reversed axis. Lines on the
  axis become shared cap edges or nothing, endpoints on it poles; a full turn has no caps (holes
  become void shells).
- The document converts a solved sketch to `ProfileCurve`s, keeps the chosen `RegionKey`s in the
  feature and calls these with the feature id.
- `build::plan::Plan` is also how booleans emit their result, with explicit pcurves.

# Booleans (`boolean/`)

- `boolean(first, second, BooleanOperation)` for union, difference and intersection: valid or a
  `BooleanError`, never a bad solid.

## Imprinting

- Vertices within `LINEAR_RESOLUTION` are pooled, indexed by a grid of 256 cells across both solids
  and looked up along a curve piece by piece: both solids' vertices, edge–face hits inside or on the
  face, the ends of an edge lying in a face's surface and its crossings with that face's edges, and
  face pairs' tangent points.
- Each edge is split at the other solid's pooled vertices on it (the ends of a piece shorter than
  the resolution become one vertex), each face–face branch at every pooled vertex on it. A branch piece
  is kept where its midpoint is strictly inside both faces; an edge piece lying in a face of the
  other solid and inside it is a cut in that face.
- Pieces with the same end vertices and geometry are one edge, so an intersection along an existing
  edge and coincident faces need no special case.
- Edge–face and face–face candidates come from a tree of face boxes (`box_tree.rs`, also used by
  `Solid::find_crossing`).

## Face tracing

- A face with no cuts whose edges are all unsplit passes through with its own loops and pcurves
  (refitted only on an edge merged with one of the other solid).
- Otherwise it is traced into loops from its boundary pieces (hinted by the original pcurves) and
  its cuts (both ways, dangling ones pruned). At each vertex the next edge is the first clockwise
  from the arriving one about the outward normal (at a pole, the mean normal of a ring around it, so
  rulings through a cone apex are ordered by azimuth); ties and cusps (tangents within 1e-5 rad, the
  noise of intersection tangents) are decided by chords at a common distance.
- Loops are fitted in the face's chart: a run of cuts leaving a pole is shifted by whole periods to
  meet the next boundary edge, pcurve ends are snapped to their vertices, and a hole goes to the
  smallest outer loop containing a point of it not on that loop.

## Classification and selection

- Each fragment is classified against the other solid at up to three interior points (inside or
  outside wins over coincident or touching; inside and outside together is `Ambiguous`) and kept by
  the operation.
- Faces that passed through share one class across unsplit edges that no cut or other piece shares;
  one whose box misses the other solid's is outside.
- Of coincident faces only the first solid's fragment can stay: with the same orientation for union
  and intersection, the opposite for difference. A difference reverses the second solid's kept
  fragments.
- Every result edge then has one use each way, else `Open`, or `NonManifold` when solids would meet
  only along an edge.

## Healing and names

- Adjacent faces on the same surface with the same orientation are merged by retracing them without
  the edges between them (left apart when that fails, as for a ring around a periodic surface).
- Two edges meeting at a vertex between the same faces, not at a pole of either, are joined when
  they are pieces of one curve, collinear lines or arcs of one circle.
- Faces keep their names and origins (fragments of a split face share its name), pieces keep their
  edge's name and new edges are named `between` their two faces, before the plan disambiguates
  duplicates.

# Blends (`blend/`)

- `blend(solid, edges, BlendShape, feature)` rounds (`Fillet`) or bevels (`Chamfer`) edges by
  sweeping a tool per edge: valid or a `BlendError` naming the edge.
- Chosen edges first grow along tangent-continuous chains (`blend_chain`); smooth edges are dropped.
  Tools are united pairwise in rounds (a pair that cannot be united, such as tools meeting only
  along an edge, stays apart) and each group is applied in one boolean, or tool by tool when that
  fails.
- Supported: straight edges whose faces run along them (planes, parallel cylinders), swept by
  extrusion; circles whose faces share their axis (planes, cylinders, cones, spheres, tori), swept
  by revolution.
- The blend must fit on both faces at a quarter, half and three quarters of the edge, and its foot
  on each face (the line or circle it runs along) must cross no edge of that face other than seams
  and the edges at the blended edge's ends, so a hole or notch between the samples refuses it as
  `TooLarge`. The cross-section is solved in 2D (`section.rs`: the fillet circle from the offset
  curves, chamfer points at equal distance).
- Convex tools are lifted clear of the faces they cut and subtracted; concave ones are flush and
  added. All concave edges go first, then the convex ones are re-found by reference in the filled
  solid (one not found fails as `Lost`; errors about unchosen edges of the filled solid come back as
  `AfterFill` without an id).
- Ends continuing into another chosen edge stop flush; ends on a face perpendicular to the edge stop
  there; ends on a slanted face extend past it when the extension lies where the operation changes
  nothing, else are clipped by the face's plane. A circular edge whose extended sweep would pass a
  full turn is refused as `WrapsAround`.
- Three convex straight edges filleted at a vertex of three planes get a spherical corner
  (`corner.rs`: a hexahedron minus the rolling ball, built through `Plan`); other corners mitre.
- Faces are `FaceName::blend(feature, edge)` and `corner(feature, vertex)`, with
  `FaceOrigin::Fillet` or `Chamfer`.

# Patterns (`pattern/`)

- `pattern(solid, copies, feature)` places a copy of the solid for each `PatternCopy` (an index
  `[column, row]` and a `RigidTransform`) and unions the original with every copy: valid or a
  `PatternError` (`Placement`, `Union` carrying the boolean's error, `Cancelled`).
- Copies are unioned in pairs, round by round, so n copies take about log n rounds of booleans on
  neighbours rather than n booleans against an ever larger body; copies that do not touch stay
  separate lumps of one body, and coincident faces of touching copies merge by healing.
- A copy's faces are renamed `FaceName::pattern(feature, index, original)` and keep the original's
  `FaceOrigin`; its edges are named again from those faces (`Solid::with_face_names`, like
  imports). The original keeps every name.

# Shell (`shell/`)

- `shell(solid, open, thickness, feature)` offsets every face by the thickness and subtracts the
  result. Offsets are exact planes, cylinders, spheres, tori and cones; a cone whose offset passes
  the apex at its reference circle is framed again beyond the apex, on the same nappe.
- `inner.rs` solves each vertex by minimal-norm Newton on the offset surfaces of its faces. A
  vertex its faces leave free (fewer than three independent normals) is also held to the axial
  plane through any round seam at it, so both ends of a seam stay on one ruling; a vertex at a
  surface's pole, or where a disk closes into a cone's tip, goes to the offset surface's pole.
- `edge.rs` rebuilds a line or circle edge through its offset ends when that lies on both offset
  surfaces; any other edge (an ellipse where a slanted face cuts a round one, an intersection
  curve where a hole crosses a cone, a line or circle whose offset is neither) is the branch of
  the two offset surfaces' intersection through both ends, over windows around the original edge
  grown by three thicknesses, taken the way the original runs. An open edge whose ends pass each
  other (along a line, round a circle, or along the curve's parameter) shrinks to nothing.
- The topology is kept except where the offset changes it (`collapse.rs`):
  - a cylinder, sphere or torus curving more tightly than the thickness is dropped: every edge on
    it but the lines along its axis and the circles around its spine vanishes;
  - a flat face bounded only by circles of one cone is dropped when the cone's offset tip passes
    the face's offset; its vertex becomes that tip;
  - a face with one loop shrinks away when all its offset edges shrink to nothing (a corner facet
    closing into a point) or all but two apart from each other (a chamfer, a narrow top or a cone
    band closing into a ridge). Found from the solved vertices, then everything is solved again
    without it, round after round until no face shrinks.
  - Vanishing edges merge their vertices, placed on every surviving face around them; the two
    edges beside a dropped face become one between the faces beyond them. A merged vertex must
    lie strictly inside the offset of each face that shrank away there, and beyond an opened face
    offset outward, so the cavity still opens through it; otherwise the edge shrinking to nothing
    is reported.
  - A vertex of four to eight faces whose offsets do not meet is split along a triangulation of
    its cycle of faces (`split.rs`), each diagonal a line between its two faces, when its edges
    are all convex (every corner inside every other offset), all concave (outside), or convex but
    for one concave edge, as where a ridge meets an inside corner: that edge's faces are one
    union intersected with the rest, so a corner holding one of them lies outside the other, a
    corner holding neither lies inside at least one, and every corner lies inside the rest.
- Only flat faces open. An opened face with no smooth edge to a closed face is offset outward, so
  the inner solid passes through it and the body needs room only across its walls; this attempt
  counts only when every closed face's inner face, dropped ones aside, survives the subtraction.
- A face of a void (a shell of negative meshed volume, `Solid::void_shells`) is never offset
  outward: its opening is a prism of its own outline reaching the thickness into the material, so
  the walls beside it run down to the cavity.
- Otherwise (or when that fails) every face is offset inward and a prism swept outward from the
  offset copy of each opened face is unioned before subtracting, which needs the thickness below
  half the body in every direction.
- Inner faces are `FaceName::shell(feature, original)` with `FaceOrigin::Shell`.
- Failures are told apart: a face curving more tightly than the thickness that cannot be dropped
  (`TooCurved`), a corner whose walls cannot meet (`Corner`: its offsets do not meet and it cannot
  be split, as where convex and concave edges alternate or one convex edge meets concave ones, the
  offset there joining faces or giving an edge another pair of faces), an edge whose wall shrinks
  to nothing or a face that shrinks away but cannot be closed over (`EdgeCollapses`), an edge
  whose offset surfaces do not meet through its ends (`UnsupportedEdge`), an opening that cannot
  be cut, walls that cross (`Walls`), a body that cannot be meshed to find its voids (`Voids`) and
  a thickness too large for the body.
