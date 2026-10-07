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
  (pinched regions), name every face and edge, group faces into shells and emit through
  `SolidBuilder`: valid or a `SweepError`. `build::plan::Plan` also builds boolean results, blend
  corners and the shell's inner solid, with explicit pcurves.
- A `Plan` takes labels (`Plan::label`) that apply to the faces added after them. When building
  fails, `ValidationError::faces` maps the error to the faces it involves and the failure comes
  back as `PlanError::Labelled` with the labels of those faces: the sketch entities of the region
  for `extrude` and `revolve` (`SweepError::Invalid { entities }`, which the document names as the
  region bounded by those curves), the source face index for the shell's inner solid.
- A circle reaches the revolution axis when its gap is within the revolve's tolerance, so a circle
  tangent to a slanted axis makes a horn torus with a pole, never a `Torus` whose radii differ by
  rounding. Faces on extrusion and revolution surfaces get exact straight pcurves; the rest are
  fitted.
- The start cap is the one at the extent's start (the sketch plane for `one_side`) whichever way
  the sweep runs, so flipping the direction keeps every name.
- A `LinearBound` is an offset along the sketch normal or a `Plane` where the profile, moved along
  the normal, meets it (`LinearExtent::between`). A plane level over the profile is an offset;
  otherwise its cap is tilted (a line's edge the line between its moved ends, a circle's the
  ellipse it maps to, a spline's the spline of its moved control points). Caps keep the names they
  have at a distance, so switching an end between a distance and a plane renames nothing.
- A plane along the direction is `EndAlongDirection`, ends that meet or cross within the profile
  `EndsCross`, a height past `MAX_SIZE` `TooLong`.
- A profile on the right of the revolution axis is revolved about the reversed axis. Lines on the
  axis become shared cap edges or nothing, endpoints on it poles; a full turn has no caps (holes
  become void shells).
- `heights(plane, regions, target)` gives the least and most signed height of a target plane over
  the profile, which the document uses to tell a plane ahead from one behind or across.
- `next_face(solid, plane, regions, reversed)` casts rays from the regions' triangle centroids
  (about `RAY_SAMPLES`, spread by area) and from the corners around each of the solid's vertices
  that project into a region (`VERTEX_NUDGE` of the profile's size away, at most
  `MAX_VERTEX_RAYS`, so a small boss between the centroids is met) to the first crossing and groups the faces met by plane
  (coplanar fragments are one face): several groups are `SeveralFaces`, one curved face `Curved`,
  else the plane with its outward normal and whether the rays enter. Undecided rays are skipped;
  none decided is `Undecided`, every ray missing `Nothing`, some `Partly`.

# Booleans (`boolean/`)

- `boolean(first, second, BooleanOperation)`: valid or a `BooleanError`, never a bad solid
  (`Empty` when nothing is left). Phases: `imprint.rs`, `faces.rs`, `select.rs`, `heal.rs`,
  `assemble.rs`. Candidates come from a tree of face boxes (`box_tree.rs`, also used by
  `find_crossing`).
- Face boxes (uv and the 3D box from it) are widened by `PCURVE_TOLERANCE` converted through the
  slowest surface speed, since the box of a face's pcurve samples can fall that far short of the
  edges, and a branch clipped to it would stop before the edge vertex.
- Imprint: vertices within `LINEAR_RESOLUTION` are pooled; every edge and face–face branch is split
  at the pooled vertices on it. A branch piece is kept when no sample along it lies outside either
  face and some sample lies strictly inside each, so a piece a micrometre or two long, whose middle
  is within the resolution of a face boundary, still cuts; a piece no longer than twice the
  resolution, which can have no sample that far inside a face it starts on the boundary of, is kept
  when no sample lies outside either face. Pieces with the same end vertices and geometry within
  `LINEAR_RESOLUTION` (the bound validation holds edges to) are one edge, so an intersection along
  an existing edge and coincident faces need no special case, and pieces a few micrometres apart
  stay two edges bounding a sliver.
- Face tracing: a face with no cuts and no split edges passes through with its own loops and
  pcurves. Otherwise it is traced into loops from its boundary pieces and cuts; at each vertex the
  next edge is the first clockwise from the arriving one about the outward normal (at a pole, the
  mean normal of a ring around it); ties and cusps (`ANGLE_TIE`, the noise of intersection
  tangents) are decided by chords at a common distance.
- Traced pcurves are sharpened until they hold the face's shape (`untangle.rs`): coedges whose uv
  polygons cross (compared only where two coedges' boxes overlap) are refined to half their
  tolerance, and a fragment whose depth (`trace::depth`, the deepest point's reach to the boundary
  along both uv directions) is under twice the tolerance of some coedge gets those coedges halved,
  round after round down to a tenth of the resolution, so a sliver or a coaxial annulus narrower
  than `PCURVE_TOLERANCE` is sampled inside itself. A fragment whose area over perimeter is
  `DEEP_GATE` times its coarsest tolerance skips the depth scan.
- Selection: each fragment is classified against the other solid at up to `INTERIOR_POINTS`
  interior points (`classify_fragment_point`; inside or outside wins over coincident or touching; inside and outside
  together, or coincident samples of opposite senses, are `Ambiguous`). A sample counts as
  coincident only when it lies exactly inside the coincident face
  (`SolidClassifier::exactly_inside_face`: the side of the nearest edge, without the resolution
  band); one within the resolution of that face's boundary, as on a strip a micrometre wide beside
  it, is decided by the thin-strip test below and only failing that taken as coincident. A fragment whose samples
  only touch the other solid (a strip narrower than twice the resolution) is decided at its deepest
  points (`trace::deepest_points`) by `SolidClassifier::side_of_touched_faces`: the sign of the
  offset along the outward normal of every face it touches, which must agree. Faces that passed through
  share one class across unsplit edges that no cut shares; one whose box misses the other solid's
  is outside. Of coincident faces only the first solid's fragment can stay: with the same
  orientation for union and intersection, the opposite for difference. A difference reverses the
  second solid's kept fragments. Every result edge must then have one use each way, else `Open`,
  or `NonManifold` when solids would meet only along an edge.
- `Intersection`, `Split`, `Ambiguous`, `Open` and `NonManifold` carry a boxed `BooleanSite`: the
  input faces of each operand involved and a model point. The innermost step that knows a point
  sets it (the vertex where tracing stuck, the sample classified both ways, the middle of an edge
  used unevenly, the faces' common box projected onto the first) and outer steps only fill what is
  still empty (`or_faces`, `or_point`): a face's split names that face and falls back to its uv
  centre.
- Healing: adjacent faces on the same surface with the same orientation are merged by retracing
  without the edges between them (left apart when that fails, as for a ring around a periodic
  surface); two edges meeting between the same faces, not at a pole, are joined when they are
  pieces of one curve. Names: `kernel-naming.md`.

## Interference (`boolean/interference.rs`)

- `interference(first, second)` is `Apart`, `Touching(point)` or `Overlapping(solid)`, the
  boolean intersection, or the boolean's error. Boxes farther apart than ten times the resolution
  are apart without a boolean; an empty intersection is then touching when some pair of faces with
  near boxes comes within that distance.
- The touch point is taken from the touching face pair whose shared box is largest, ties going to
  the one whose point (the shared box's centre projected onto the first face, confirmed on the
  second) lies nearest that centre, so faces flush with each other report the middle of the
  contact rather than a corner or an edge beside it.

# Blends (`blend/`)

- `blend(solid, edges, BlendShape, feature)` rounds (`Fillet`) or bevels (`Chamfer`) edges by
  sweeping a tool per edge: valid or a `BlendError` naming the edge. A failing tool's `Profile`,
  `Sweep` or `Boolean` carries the edge it was built for, except when the failing step is the
  pairwise union of tools or a corner's, which name none.
- `tangent_chain(solid, edges)` follows tangent-continuous edges sharing a face through their end
  vertices, smooth or sharp, for selection; it shares `follow` with `blend_chain`, which admits only
  sharp edges.
- Chosen edges grow along tangent-continuous chains (`blend_chain`); smooth edges are dropped, and
  only when every chosen edge is smooth is it `Smooth`. Tools are united pairwise in rounds (a
  pair that cannot be united stays apart) and each group is applied in one boolean, or tool by tool
  when that fails.
- Supported: straight edges whose faces run along them (planes, parallel cylinders), swept by
  extrusion; circles whose faces share their axis, swept by revolution; else `Unsupported`.
- `TooLarge` covers a blend that does not fit on both faces at sampled points along the edge, whose
  foot on a face crosses an edge of that face (other than seams and edges at the blended edge's
  ends), whose foot crosses another blend's on that face (`feet.rs`; edges sharing a vertex
  excepted), and a knife edge (faces with opposite normals).
- Convex tools are lifted clear of the faces they cut and subtracted; concave ones are flush and
  added. All concave edges go first, then the convex ones are re-found by reference in the filled
  solid (one not found fails as `Lost`; errors about unchosen edges of the filled solid come back
  as `AfterFill` without an id).
- Ends continuing into another chosen edge stop flush, ends on a perpendicular face stop there,
  ends on a slanted face extend past it when the extension lies where the operation changes
  nothing, else are clipped by its plane. A circular edge whose extended sweep would pass a full
  turn is `WrapsAround`.
- Three convex straight edges filleted at a vertex of three planes get a spherical corner
  (`corner.rs`: a hexahedron minus the rolling ball); other corners mitre.

# Patterns (`pattern/`)

- `pattern(solid, copies, feature)` places a copy for each `PatternCopy` (an index `[column, row]`
  and a `Similarity`, so a copy may be mirrored or scaled through `Solid::mapped`) and unions the
  original with every copy: valid or a `PatternError`.
- Copies are unioned in pairs, round by round, so n copies take about log n rounds of booleans on
  neighbours rather than n booleans against an ever larger body; copies that do not touch stay
  separate lumps of one body, and coincident faces of touching copies merge by healing. Copies
  meeting only along an edge or at a point (a union that is `NonManifold`) are kept as separate
  shells instead (`Solid::beside`, which appends the arenas with shifted ids, accepted when it
  validates). A union that fails otherwise is `PatternError::Union` naming the copies of both
  sides, the original as `[0, 0]`.
- Copies are renamed and given `FaceOrigin::Copy` (`kernel-naming.md`); the original keeps every
  name and origin.

# Shell (`shell/`)

- `shell(solid, open, thickness, feature)` offsets every face by the thickness and subtracts the
  result. Only flat faces open. Offsets are exact planes, cylinders, spheres, tori and cones (a
  cone offset past its apex is framed again on the same nappe).
- `inner.rs` solves each vertex on the offset surfaces of its faces; a vertex its faces leave free
  is also held to the axial plane through any round seam at it, so both ends of a seam stay on one
  ruling. `edge.rs` rebuilds a line or circle edge through its offset ends when that lies on both
  offset surfaces, else takes the branch of the offset surfaces' intersection through both ends;
  an open edge whose ends pass each other shrinks to nothing.
- The topology is kept except where the offset changes it (`collapse.rs`):
  - a cylinder, sphere or torus curving more tightly than the thickness, a flat face bounded only
    by circles of one cone whose offset tip passes the face's offset, and a one-loop face whose
    offset edges all shrink to nothing or all but two apart from each other (a chamfer, a narrow
    top, a cone band closing into a ridge) are dropped; shrinking faces are found from the solved
    vertices, then everything is solved again without them, round after round;
  - vanishing edges merge their vertices; a merged vertex must lie strictly inside the offset of
    each face that shrank away there, and beyond an opened face offset outward, so the cavity still
    opens through it, else the edge shrinking to nothing is reported;
  - a vertex of four to `MAX_SPLIT_FACES` faces whose offsets do not meet is split along a
    triangulation of its cycle of faces (`split.rs`) when its edges are all convex, all concave, or
    convex but for one concave edge; other corners are `Corner`.
- An opened face with no smooth edge to a closed face is offset outward, so the inner solid passes
  through it and the body needs room only across its walls; the attempt counts only when every
  closed face's inner face, dropped ones aside, survives the subtraction. A face of a void
  (`Solid::void_shells`) is never offset outward: its opening is a prism of its own outline
  reaching the thickness into the material.
- Otherwise (or when that fails) every face is offset inward and a prism swept outward from the
  offset copy of each opened face is unioned before subtracting, which needs the thickness below
  half the body in every direction.
- `ShellError::Walls` names the faces whose offsets failed to build or to settle, up to
  `NAMED_WALL_FACES`; a failure spread over more of the body names none.
- When the outward attempt only fails to keep every wall and the inward one fails with an error
  that names its cause (`ShellError::names_the_cause`), that error is reported instead of
  `TooThick`.
