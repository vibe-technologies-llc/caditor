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
  otherwise its cap is tilted (a line's edge the line between its moved ends, a circle's or an
  ellipse's the ellipse it maps to, found from the moved conjugate diameters, a spline's the spline
  of its moved control points; the given cap pcurves are shifted by the edge's parameter offset).
  An ellipse's side face is an `Extrusion` of its `Curve::Ellipse`, mapped like a spline's; a
  revolved ellipse is first the rational quadratic B-spline of its arc (the affine image of the
  circle's), so it revolves through a `Revolution` like any spline. Caps keep the names they
  have at a distance, so switching an end between a distance and a plane renames nothing.
- A plane along the direction is `EndAlongDirection`, ends that meet or cross within the profile
  `EndsCross`, a height past `MAX_SIZE` `TooLong`.
- `extrude_along(plane, regions, extent, direction, feature)` sweeps the same way along a slanted
  direction: heights and offsets stay measured square to the sketch, each point moving by the
  direction scaled to rise one unit per unit of height, so caps are the profile shifted and
  `heights_along` gives a target plane's heights the same way. A line's side is the plane through
  it and the direction; a circle's, ellipse's or spline's an `Extrusion` along the direction
  (mapped pcurves stretched by the direction's length per unit of height; a square circle keeps
  its `Cylinder`). A direction in the sketch plane is `DirectionAlongSketch`.
- A profile on the right of the revolution axis is revolved about the reversed axis. Lines on the
  axis become shared cap edges or nothing, endpoints on it poles; a full turn has no caps (holes
  become void shells).
- `extrude_tapered(plane, regions, extent, angle, feature)` (`taper.rs`) offsets each loop's
  pieces to their left (inward, so a positive angle draws in) by `tan(angle)` times the height from
  the sketch plane clamped into the extent, so a one-sided extrusion tapers from the profile as
  drawn and one spanning the sketch plane tapers both ways from it (two side faces per piece, the
  middle ring shared). Lines sweep planes, arcs and circles cones about their own centre; corners
  are re-met at each level (`profile::offset_strands`), straight where both pieces are lines or meet
  tangentially, else an `IntersectionCurve` through `JOINT_SAMPLES` levels on the two side
  surfaces. Below `STRAIGHT_TAPER` it is `extrude`; splines and ellipses are
  `TaperedUnsupportedCurve`, a slanted end `TaperedTiltedEnd`, `MAX_TAPER_DEGREES` or steeper
  `TaperTooSteep`, and a piece used up, an arc shrunk to nothing, corners that no longer meet, a
  loop turned over or loops crossing at a cap `TaperCloses` (naming the pieces' entities when
  known).
- `heights(plane, regions, target)` gives the least and most signed height of a target plane over
  the profile, which the document uses to tell a plane ahead from one behind or across.
- `next_face(solid, plane, regions, reversed)` casts rays from the regions' triangle centroids
  (about `RAY_SAMPLES`, spread by area) and from the corners around each of the solid's vertices
  that project into a region (`VERTEX_NUDGE` of the profile's size away, at most
  `MAX_VERTEX_RAYS`, so a small boss between the centroids is met) to the first crossing and groups the faces met by plane
  (coplanar fragments are one face): several groups are `SeveralFaces`, one curved face `Curved`,
  else the plane with its outward normal and whether the rays enter. Undecided rays are skipped;
  none decided is `Undecided`, every ray missing `Nothing`, some `Partly`.
- `stop_at_body(tool, body, plane, regions, reversed, far)` cuts a sweep made to `far` back to
  where it first meets a body, whatever the faces there: it classifies up to `MAX_START_PROBES`
  points just ahead of the profile (all outside: entering, all inside: leaving, else `Straddles`),
  takes the difference with the body when entering or the intersection when leaving, and keeps the
  shells whose mesh reaches back to the start plane (`Solid::shell_spans`, `keeping_shells`, which
  renumbers the kept topology and keeps every name). A void shell is `Enclosed`, a kept shell
  reaching `far` `PassesBeside`, nothing kept `Nothing`.

# Booleans (`boolean/`)

- `boolean(first, second, BooleanOperation)`: valid or a `BooleanError`, never a bad solid
  (`Empty` when nothing is left). Phases: `imprint.rs`, `faces.rs`, `select.rs`, `heal.rs`,
  `assemble.rs`. Candidates come from a tree of face boxes (`box_tree.rs`, also used by
  `find_crossing`).
- Operands whose enclosing boxes (edges and vertices, plus the uv patch box of each doubly curved
  face) lie more than ten times the resolution apart skip the pipeline (`apart.rs`): a union is
  `Solid::beside`, a difference the first solid, an intersection `Empty`. Only when the pipeline
  would change nothing in them, though: no edge between two faces healing would merge, no vertex
  of exactly two edges between the same faces, no edge name repeated across both operands
  (`Plan` would disambiguate it); otherwise they go through the pipeline.
- What no tool touches is carried rather than rebuilt, so the work follows the size of the contact
  and not of the body: a face's loops whose edges are unsplit, shared with nothing and whose
  vertices no cut or other loop reaches keep their pcurves and skip tracing and refitting; a
  fragment with an edge whose box misses the other solid's extent is outside without sampling;
  coedges reaching assembly with the operand's own edge, surface, sense and pcurve go to the
  `Plan` as `PlanPcurve::Settled`, whose geometry validation skips (`SolidBuilder::settle`).
  Every other check, the volume and lump checks over the whole result included, still runs, so a
  hole in a body of many holes costs about the volume check's coarse mesh of the body.
  `Input::with_carrying(.., false)` runs the full pipeline; `carry_tests.rs` compares the two.
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

## Split faces (`boolean/split_faces.rs`)

- `split_faces(solid, faces, tool, feature)` divides the chosen faces where the tool solid's
  surface crosses them and leaves the shape alone: valid or a `FaceSplitError` (`NoFaces`,
  `Undivided` when no chosen face comes out in more than one piece, `Boolean` for the pipeline's
  own failures, `Cancelled`). It is the boolean pipeline restricted (`Input::imprinting`): only the
  chosen faces of the first operand are intersected with the tool (`Input::imprints_on`), only
  edges bounding a chosen face are split (`Input::splits_edge`), only the first operand is traced
  (`Input::traces`), every fragment of it is kept as it is, the tool's are dropped, and nothing is
  healed (merging would join the pieces again), only `check_closed` before assembly.
- Each piece of a chosen face is classified against the tool like a boolean fragment and named
  `FaceName::split(feature, original, SplitPiece)`: `Inside` (or lying on the tool) or `Outside`,
  keeping the original's `FaceOrigin`. Every edge used by a chosen face is renamed from its faces
  (`between`, or `seam` of a face using it twice; `assemble` takes the per-piece names), the
  `Plan` disambiguating repeats, so a later feature holds a piece by name.
- The document builds the tool: a plane's half-space block, an open sketch chain's swept half
  space, closed sketch outlines swept through the body (`split::Sweep`, square to the sketch or
  along a direction through `extrude_along`) or another body as it stands; a sketch mixing a
  chain with outlines calls `split_faces` twice (`document.md`).

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

- `blend(solid, edges, BlendShape, feature)` rounds (`Fillet`) or bevels edges by sweeping a tool
  per edge: valid or a `BlendError` naming the edge. A bevel is `Chamfer` (one distance on both
  faces), `TwoDistanceChamfer` (`first` on the measured face, `second` on the other) or
  `AngledChamfer` (`distance` on the measured face, the cut turned `angle` radians from that face
  inside the cut-off corner, met with the other face's section line or circle; a cut that never
  meets it is `AngleMisses`, an angle outside (0, π) `InvalidAngle`). The measured face of each
  edge is the one more of the pass's edges share (the top of a chosen rim), ties going to the
  smaller `FaceName`; `flipped` swaps it for every edge. Concave and convex edges are separate
  passes, each counting its own edges. A failing tool's `Profile`,
  `Sweep` or `Boolean` carries the edge it was built for, except when the failing step is the
  pairwise union of tools or a corner's, which name none.
- `tangent_chain(solid, edges)` follows tangent-continuous edges sharing a face through their end
  vertices, smooth or sharp, for selection; it shares `follow` with `blend_chain`, which admits only
  sharp edges.
- `hole_faces(solid, faces)` (`hole_faces.rs`) spreads from each selected concave cylinder, cone or
  torus (its outward normal pointing at its axis) to the faces of the same axis that are concave
  too, and to flat faces square to the axis bounded only by circles about it whose every
  neighbour is such a wall (a counterbore's floor, a flat bottom), so a hole is taken whole while
  the plate around it and the ends of a tube are not; a boss is no hole.
- `tangent_faces(solid, faces)` spreads from the given faces across every edge where the two faces'
  normals agree within `TANGENT_FACE_ANGLE` (0.01 rad, looser than blending's test so fitted
  imported blends count) at a quarter, half and three quarters along it, for selection.
- Chosen edges grow along tangent-continuous chains (`blend_chain`); smooth edges are dropped, and
  only when every chosen edge is smooth is it `Smooth`. Tools are united pairwise in rounds (a
  pair that cannot be united stays apart; a pair whose bounding boxes lie more than `APART_TOOLS`
  apart is joined as separate lumps by `Solid::beside`, without a boolean, valid by construction)
  and each group is applied in one boolean, or tool by tool when that fails.
- Supported: straight edges whose faces run along them (planes, parallel cylinders), swept by
  extrusion; circles whose faces share their axis, swept by revolution; else `Unsupported`. A
  revolved profile may reach the axis (a fillet as large as the round fill it runs along, where a
  notch's back edge continues up the ends of its rounded sides) when neither end extends, the
  touching point becoming a pole; one crossing the axis, or reaching it with an end that extends,
  is `TooLarge`.
- `TooLarge` covers a blend that does not fit on both faces at sampled points along the edge, whose
  foot on a face crosses an edge of that face (other than seams and edges at the blended edge's
  ends), whose foot crosses another blend's on that face (`feet.rs`; edges sharing a vertex
  excepted), and a knife edge (faces with opposite normals). A sample past the plane of a flat face
  at one of the edge's ends (`end_planes`, oriented along the edge out of that end) fits whatever
  face it lands on, since that end extends or clips the tool there: a foot running off an acute
  end onto the face it meets is not too large.
- Convex tools are lifted clear of the faces they cut and subtracted; concave ones are flush and
  added. All concave edges go first, then the convex ones are re-found by reference in the filled
  solid (one not found fails as `Lost`; errors about unchosen edges of the filled solid come back
  as `AfterFill` without an id).
- Ends continuing into another chosen edge stop flush, ends on a perpendicular face stop there,
  ends on a slanted face extend past it when the extension lies where the operation changes
  nothing, else are clipped by its plane. Where that extension would change something and the end
  face belongs to another chosen straight edge of the same kind sharing one flat face (concave
  edges round a boss's outside corner, convex edges into an inside corner), both ends are mitred
  instead (`mitre`): each tool extends past the vertex and is clipped by the plane through the
  vertex bisecting the two edges, so the corner is filled or cut to where the two blends meet
  rather than left notched or standing.
- An end that must be clipped by a curved face (a notch's floor edge running out through a round
  wall) is `End::Trimmed` when that face is a cylinder, cone, sphere or torus (`round_end.rs`,
  `Round`): near the end (`Cut`: from the vertex, `behind` back along the edge, `span` across) the
  tool loses what lies beyond the face's surface, the region a revolved profile of the surface
  bounds (the solid of the cylinder, cone or sphere, or for a torus seen from outside its tube
  the corner of the quadrant the face faces, a revolved spandrel, so the tube's other quarters
  keep their material). The quadrant is read from the torus face's own normal at the middle of
  its uv box (`facing`), never from the direction of the cut, which runs level along a rim; a
  face facing no one quadrant (leaning less than `LEANING` either way) is cut by the whole
  tube's outside instead. A concave tool also loses what lies beyond each round face beside the end
  (`beside_ends`: a neighbour of an end face and of a blended face, a rim's fillet the notch runs
  out under), anchored at the corner those three share, and its feet may cross the edges to those
  faces without being `TooLarge`; an end beside a face that is neither flat nor round keeps
  none.
- Refused corners, each pinned by a test: a convex edge rising from bevelled concave edges (a
  boss's corner edge chosen with its base) ends, after the fill, where two fill faces meet and is
  `UnsupportedEnd`; feet meeting exactly across a fill (a rim chamfer meeting a boss's skirt on the
  face between them) are `TooLarge`, and fills that use up a whole face (a pocket's walls) lose the
  edges on it (`Lost`). `survey.rs` chamfers every corner of twenty bodies (ignored). A circular edge whose extended sweep would pass a full
  turn is `WrapsAround`.
- Three convex straight edges filleted at a vertex of three planes get a spherical corner
  (`corner.rs`: a hexahedron minus the rolling ball); other corners mitre.

# Patterns (`pattern/`)

- `pattern(solid, copies, feature)` places a copy for each `PatternCopy` (an index `[column, row]`
  and a `Similarity`, so a copy may be mirrored or scaled through `Solid::mapped`) and unions the
  original with every copy: valid or a `PatternError`. `pattern_copies` unions the copies alone,
  leaving the original out (none for no copies), for the document to cut or join a repeated
  feature's tool.
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

# Offset faces (`shell/offset.rs`)

- `offset_faces(solid, faces, distance)` moves each face along its outward normal (positive grows
  the body) and lets the faces beside it extend or trim to meet it: valid or an `OffsetError`.
  It is the shell's inner solid with one distance per face (`Offsets::moving`: the chosen faces
  `-distance` inward, every other face 0), so vertex solving, edge rebuilding, collapses and
  corner splitting are the shell's. A face at distance 0 keeps its surface whatever it is
  (splines, extrusions and revolutions included) and an edge between such faces whose ends did not
  move keeps its curve; only a moved face must be a plane, cylinder, cone, sphere or torus
  (`UnsupportedFace` otherwise).
- Faces keep their names and origins (`Naming::Kept`), so edges and references to them survive
  and a later feature finds them as before; the shell names its inner faces after the shell instead.
- A face that would vanish (`Collapses` drops a cylinder, sphere or torus gone to nothing, a face a
  moved neighbour closes up) is refused as `Vanishes`, never dropped silently. After a valid build,
  `find_crossing` must find no face pair crossing (`Crosses`); a body failing validation first
  (a moved face through the opposite one) is `Invalid`. A moved face beside an unmoved face it
  was tangent to cannot keep that tangency, so its edge is `UnsupportedEdge`; moving the whole
  tangent chain (`tangent_faces`) keeps every tangency (radii grow by the distance).

