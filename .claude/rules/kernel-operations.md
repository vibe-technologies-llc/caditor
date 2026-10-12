---
paths:
  - "crates/caditor-kernel/src/build/**"
  - "crates/caditor-kernel/src/boolean/**"
  - "crates/caditor-kernel/src/blend/**"
  - "crates/caditor-kernel/src/shell/**"
  - "crates/caditor-kernel/src/pattern/**"
  - "crates/caditor-kernel/src/enclose/**"
  - "crates/caditor-kernel/src/box_tree.rs"
  - "crates/caditor-kernel/src/thread/**"
---

# Builders (`build/`)

- `extrude(plane, regions, LinearExtent, feature)` and `revolve(plane, regions, Axis2,
  AngularExtent, feature)` plan vertices, edges and faces, merge coincident vertices within a shell,
  name every face and edge and emit through `SolidBuilder`: valid or a `SweepError`.
  `build::plan::Plan` also builds boolean results, blend corners and the shell's inner solid, with
  explicit pcurves.
- `Plan::label` labels the faces added after it. When building fails, `ValidationError::faces` maps
  the error to its faces and the failure returns as `PlanError::Labelled` with their labels: the
  region's sketch entities for `extrude` and `revolve` (`SweepError::Invalid { entities }`), the
  source face index for the shell's inner solid.
- Revolve: a circle reaches the axis when its gap is within the revolve's tolerance, so a circle
  tangent to a slanted axis makes a horn torus with a pole, never a `Torus` with radii differing by
  rounding. An arc whose circle passes the axis but which stays clear of it by more than the
  tolerance (`nearest_to_axis`) is an exact spindle `Torus` (so a shell or offset face can move a
  rim fillet); one ending on the axis, or a circle centred behind it, is the `Revolution` of its
  rational arc. A profile right of the axis is revolved about the reversed axis. Lines on the axis
  become shared cap edges or nothing, endpoints on it poles; a full turn has no caps (holes become
  void shells). Extrusion and revolution faces get exact straight pcurves; the rest are fitted.
- The start cap is the one at the extent's start (the sketch plane for `one_side`) whichever way
  the sweep runs, so flipping the direction keeps every name.
- A `LinearBound` is an offset along the sketch normal or a `Plane` met by the profile moved along
  the normal (`LinearExtent::between`). A plane level over the profile is an offset; otherwise the
  cap is tilted (a line's edge the line between its moved ends, a circle's or ellipse's the ellipse
  from the moved conjugate diameters, a spline's the spline of its moved control points; cap
  pcurves shift by the edge's parameter offset). An ellipse's side is an `Extrusion` of its
  `Curve::Ellipse`; a revolved ellipse is first the rational quadratic B-spline of its arc.
  Caps keep their names across distance and plane ends, so switching renames nothing.
- A plane along the direction is `EndAlongDirection`, ends that meet or cross within the profile
  `EndsCross`, a height past `MAX_SIZE` `TooLong`.
- `extrude_along(plane, regions, extent, direction, feature)` sweeps along a slanted direction:
  heights and offsets stay measured square to the sketch (each point moves by the direction scaled
  to rise one unit per unit of height), so caps are the profile shifted; `heights_along` gives a
  target plane's heights likewise. A line's side is the plane through it and the direction; a
  circle's, ellipse's or spline's an `Extrusion` along it (pcurves stretched by the direction's
  length per unit height; a square circle keeps its `Cylinder`). A direction in the sketch plane is
  `DirectionAlongSketch`.
- `extrude_tapered(plane, regions, extent, angle, feature)` (`taper.rs`) offsets each loop's pieces
  to their left (inward, so a positive angle draws in) by `tan(angle)` times the height from the
  sketch plane clamped into the extent: one-sided extrusions taper from the profile as drawn, ones
  spanning the sketch plane taper both ways (two side faces per piece, middle ring shared). Lines
  sweep planes, arcs and circles cones about their own centre; corners are re-met at each level
  (`profile::offset_strands`), straight where both pieces are lines or tangent, else an
  `IntersectionCurve` through `JOINT_SAMPLES` levels. Below `STRAIGHT_TAPER` it is `extrude`.
  Errors: splines and ellipses `TaperedUnsupportedCurve`, a slanted end `TaperedTiltedEnd`,
  `MAX_TAPER_DEGREES` or steeper `TaperTooSteep`, a piece used up, an arc shrunk to nothing,
  corners no longer meeting, a loop turned over or loops crossing at a cap `TaperCloses`.
- `wrap_regions(plane, regions, surface, feature)` (`wrap.rs`) wraps a plane's regions onto a
  cylinder or cone by its unrolled form, for a split face's tool; other surfaces are
  `NotUnrollable`, a plane not running along the axis `AcrossAxis`. Distance along the axis becomes
  distance along the slope (from the middle of the curves' reach), distance across it becomes arc
  length round the surface at that slope distance (a rectangle keeps its area on a cone); the
  plane's side of the axis stays in front, so a decal is never mirrored. Curves reaching the apex
  are `PastApex`. The result is an extrusion's topology between coaxial copies of the surface
  offset a quarter of the least radius inside and outside it (seam opposite the outlines): each
  piece's side is a ruled spline surface, cubic along the curve at stations at most `STEP_ANGLE`
  apart, overhanging both copies by `WALL_OVERHANG`; rims are `IntersectionCurve`s of side and copy,
  every pcurve fitted, and a closed piece (a circle) is halved first since a periodic side cannot be
  intersected reliably. Outlines within `OVERLAP_GAP` of a full turn are `BeyondFullTurn`; a piece
  needing more than `MAX_STATIONS` `TooIntricate`.
- `wrap_chain(plane, curves, surface, solid, faces, feature)` (`wrap_cut.rs`) builds the tool that
  cuts the chosen faces along one open chain wrapped the same way (`NotOneChain` otherwise). Ends
  inside the faces' reach are carried straight on until they leave it (`EndRunsRound` when one runs
  round a whole face). The cut is drawn in a window of the unrolled surface one turn wide: starting
  on the seam all chosen faces share, else (no face runs right round) in the gap common to all, the
  window then narrower than a turn; `NoSeamPlace` when neither exists. Every turn of the chain is
  folded into the window (shifted back by whole turns: a translation on a cylinder, a shear on a
  cone, arcs and ellipses then rational splines), at most `MAX_TURNS` turns. Regions inside are
  coloured two ways across the chain (`CrossesItself` when inconsistent) and the side left of the
  chain walked from its first end is wrapped (`wrapped`). A full window's edges both lie on the
  seam, so a choice whose regions meet across it at the same height is refused and the other side
  tried, then a window starting at an angle the chain never reaches; else `ClosesRound`. A
  selection of no region or all of them is `Misses`. The tool's faces along the window's edges
  coincide with the seam, so the cut adds no edge there.
- `heights(plane, regions, target)` gives the least and most signed height of a target plane over
  the profile, to tell a plane ahead from one behind or across.
- `next_face(solid, plane, regions, reversed)` casts rays from the regions' triangle centroids
  (about `RAY_SAMPLES`, spread by area) and from the corners around each solid vertex projecting
  into a region (`VERTEX_NUDGE` of the profile's size away, at most `MAX_VERTEX_RAYS`, so a small
  boss between centroids is met) to the first crossing, grouping faces met by plane (coplanar
  fragments are one face): several groups `SeveralFaces`, one curved face `Curved`, else the plane
  with its outward normal and whether the rays enter. Undecided rays are skipped; none decided is
  `Undecided`, every ray missing `Nothing`, some `Partly`.
- `stop_at_body(tool, body, plane, regions, reversed, far)` cuts a sweep made to `far` back to
  where it first meets a body: it classifies up to `MAX_START_PROBES` points just ahead of the
  profile (all outside entering, all inside leaving, else `Straddles`), takes the difference with
  the body when entering or the intersection when leaving, and keeps the shells whose mesh reaches
  back to the start plane (`Solid::shell_spans`, `keeping_shells`, which renumbers and keeps every
  name). A void shell is `Enclosed`, a kept shell reaching `far` `PassesBeside`, nothing kept
  `Nothing`.

# Booleans (`boolean/`)

- `boolean(first, second, BooleanOperation)`: valid or a `BooleanError`, never a bad solid
  (`Empty` when nothing is left). Phases: `imprint.rs`, `faces.rs`, `select.rs`, `heal.rs`,
  `assemble.rs`. Candidates come from a tree of face boxes (`box_tree.rs`, also used by
  `find_crossing`). Face boxes (uv and the 3D box from it) are widened by `PCURVE_TOLERANCE`
  through the slowest surface speed, since pcurve samples can fall that far short of the edges.
- Operands whose enclosing boxes (edges, vertices, plus the uv patch box of each doubly curved
  face) lie more than ten times the resolution apart skip the pipeline (`apart.rs`): a union is
  `Solid::beside`, a difference the first solid, an intersection `Empty`; only when the pipeline
  would change nothing: no edge between two faces healing would merge, no vertex of exactly two
  edges between the same faces, no edge name repeated across both operands.
- What no tool touches is carried rather than rebuilt, so work follows the size of the contact: a
  face's loops with unsplit, unshared edges and vertices no cut or other loop reaches keep their
  pcurves and skip tracing and refitting; a fragment with an edge whose box misses the other
  solid's extent is outside without sampling; coedges reaching assembly with the operand's own
  edge, surface, sense and pcurve go to the `Plan` as `PlanPcurve::Settled`, skipping geometry
  validation (`SolidBuilder::settle`). Every other check, volume and lump checks over the whole
  result included, still runs. `Input::with_carrying(.., false)` runs the full pipeline;
  `carry_tests.rs` compares the two.
- Imprint: vertices within `LINEAR_RESOLUTION` are pooled; every edge and face-face branch is split
  at the pooled vertices on it. A branch piece is kept when no sample along it lies outside either
  face and some sample lies strictly inside each (so a piece a micrometre or two long still cuts);
  a piece no longer than twice the resolution is kept when no sample lies outside either face.
  Pieces with the same end vertices and geometry within `LINEAR_RESOLUTION` (the bound validation
  holds edges to) are one edge, so an intersection along an existing edge and coincident faces need
  no special case; pieces a few micrometres apart stay two edges bounding a sliver.
- Face tracing: a face with no cuts and no split edges passes through with its own loops and
  pcurves. Otherwise it is traced into loops from its boundary pieces and cuts; at each vertex the
  next edge is the first clockwise from the arriving one about the outward normal (at a pole, the
  mean normal of a ring around it); ties and cusps (`ANGLE_TIE`) are decided by chords at a common
  distance.
- Traced pcurves are sharpened until they hold the face's shape (`untangle.rs`): coedges whose uv
  polygons properly cross (compared only where boxes overlap; a turn within `COLLINEAR_SINE` counts
  as none, so the two traversals of one iso-parametric edge never cross) are refined to half their
  tolerance, and a fragment whose depth (`trace::depth`) is under twice the tolerance of some
  coedge gets those coedges halved, round after round down to a tenth of the resolution, so a
  sliver or coaxial annulus narrower than `PCURVE_TOLERANCE` is sampled inside itself. A fragment
  whose area over perimeter is `DEEP_GATE` times its coarsest tolerance skips the depth scan.
- Selection: each fragment is classified against the other solid at up to `INTERIOR_POINTS`
  interior points (`classify_fragment_point`; inside or outside wins over coincident or touching;
  inside and outside together, or coincident samples of opposite senses, are `Ambiguous`). A sample
  is coincident only when exactly inside the coincident face (`SolidClassifier::exactly_inside_face`,
  no resolution band); one within the resolution of that face's boundary is decided by the
  thin-strip test and only failing that taken as coincident. A fragment whose samples only touch
  the other solid is decided at its deepest points (`trace::deepest_points`) by
  `SolidClassifier::side_of_touched_faces`: the sign of the offset along the outward normal of
  every face it touches, which must agree. Faces that passed through share one class across
  unsplit edges no cut shares; one whose box misses the other solid's is outside. Of coincident
  faces only the first solid's fragment can stay: same orientation for union and intersection,
  opposite for difference. A difference reverses the second solid's kept fragments. Every result
  edge must then have one use each way, else `Open`, or `NonManifold` when solids meet only along
  an edge.
- `Intersection`, `Split`, `Ambiguous`, `Open` and `NonManifold` carry a boxed `BooleanSite`: the
  input faces of each operand involved and a model point. The innermost step that knows a point
  sets it and outer steps only fill what is empty (`or_faces`, `or_point`).
- Healing: adjacent faces on the same surface with the same orientation are merged by retracing
  without the edges between them (left apart when that fails, as for a ring around a periodic
  surface); two edges meeting between the same faces, not at a pole, are joined when pieces of one
  curve. Names: `kernel-naming.md`.

## Split faces (`boolean/split_faces.rs`)

- `split_faces(solid, faces, tool, feature)` divides the chosen faces where the tool's surface
  crosses them and leaves the shape alone: valid or a `FaceSplitError` (`NoFaces`, `Undivided`
  when no chosen face comes out in more than one piece, `Boolean`, `Cancelled`). It is the boolean
  pipeline restricted (`Input::imprinting`): only chosen faces of the first operand are
  intersected (`Input::imprints_on`), only edges bounding a chosen face split
  (`Input::splits_edge`), only the first operand traced (`Input::traces`), every fragment of it
  kept, the tool's dropped, no healing (it would rejoin the pieces), only `check_closed` before
  assembly.
- Each piece is classified against the tool like a boolean fragment and named
  `FaceName::split(feature, original, SplitPiece)`: `Inside` (or on the tool) or `Outside`,
  keeping the original's `FaceOrigin`. Every edge used by a chosen face is renamed from its faces
  (`between`, or `seam` for a face using it twice; `assemble` takes the per-piece names), the
  `Plan` disambiguating repeats, so a later feature holds a piece by name.
- The document builds the tool: a plane's half-space block, an open sketch chain's swept half
  space, closed outlines swept through the body (`split::Sweep`, square to the sketch or along a
  direction through `extrude_along`) or wrapped round a cylinder or cone (`wrap_regions`, an open
  chain `wrap_chain`), or another body as it stands; a sketch mixing a chain with outlines calls
  `split_faces` twice (`document.md`).

## Interference (`boolean/interference.rs`)

- `interference(first, second)` is `Apart`, `Touching(point)` or `Overlapping(solid)` (the boolean
  intersection), or the boolean's error. Boxes farther apart than ten times the resolution are
  apart without a boolean; an empty intersection is touching when some pair of faces with near
  boxes comes within that distance.
- The touch point comes from the touching face pair whose shared box is largest, ties going to the
  one whose point (the shared box's centre projected onto the first face, confirmed on the second)
  lies nearest that centre, so flush faces report the middle of the contact.

# Blends (`blend/`)

- `blend(solid, edges, BlendShape, feature)` rounds (`Fillet`) or bevels edges by sweeping a tool
  per edge: valid or a `BlendError` naming the edge. A bevel is `Chamfer` (one distance on both
  faces), `TwoDistanceChamfer` (`first` on the measured face, `second` on the other) or
  `AngledChamfer` (`distance` on the measured face, the cut turned `angle` radians from that face,
  met with the other face's section line or circle; `AngleMisses` when it never meets, an angle
  outside (0, pi) `InvalidAngle`). The measured face of each edge is the one more of the pass's
  edges share, ties going to the smaller `FaceName`; `flipped` swaps it for every edge. Concave and
  convex edges are separate passes. A failing tool's `Profile`, `Sweep` or `Boolean` carries its
  edge, except when the failing step is the pairwise union of tools or a corner's.
- Selection helpers: `tangent_chain(solid, edges)` follows tangent-continuous edges sharing a face
  through their end vertices, smooth or sharp (shares `follow` with `blend_chain`, which admits
  only sharp edges). `hole_faces(solid, faces)` (`hole_faces.rs` at the kernel root) spreads from
  each selected concave cylinder, cone or torus (outward normal pointing at its axis) to concave
  faces of the same axis and to flat faces square to the axis bounded only by circles about it whose
  every neighbour is such a wall (a counterbore's floor), so a hole is taken whole while the plate
  and tube ends are not; a boss is no hole. `tangent_faces(solid, faces)` spreads across every edge
  where the two faces' normals agree within `TANGENT_FACE_ANGLE` (0.01 rad, looser than blending's
  test so fitted imported blends count) at a quarter, half and three quarters along it.
- Chosen edges grow along tangent-continuous chains (`blend_chain`); smooth edges are dropped, and
  only when every chosen edge is smooth is it `Smooth`. Tools are united pairwise in rounds (a pair
  that cannot be united stays apart; a pair whose boxes lie more than `APART_TOOLS` apart is joined
  as separate lumps by `Solid::beside`) and each group is applied in one boolean, or tool by tool
  when that fails.
- Swept exactly: straight edges whose faces run along them (planes, parallel cylinders), by
  extrusion; circles whose faces share their axis, by revolution. A revolved profile may reach the
  axis (the touching point becoming a pole) when neither end extends; crossing the axis, or reaching
  it with an end that extends, is `TooLarge`.
- Every other sharp edge between two faces of any surface kind (ellipses, splines, intersection
  curves) is blended by a lofted tool (`general.rs`), as is the whole tangent chain holding one
  (`chain.rs` orders a chain into runs, a closed edge alone split in two), so joints never pair a
  swept tool with a lofted one. When the swept tools of a choice fail in profile, sweep or boolean,
  the same edges are blended again lofted before the error is reported (`planned_and_applied`).
  `Unsupported` is left for an edge not between two faces and a chain turning from convex to
  concave.
- A lofted run is solved in sections (`station.rs`): at each station, in a plane through the edge
  point square to the tangent (the cross product of the faces' normals, smooth where a marched
  edge's own tangent is not), Newton on the true surfaces finds the rolling ball's centre and
  contacts (fillet), or the feet at the chord distances or angle (chamfers), started from the
  section of the faces' tangent planes. Contact faces searched are the run's own and those reached
  across smooth edges (`tangent_faces`), the nearest solution with both feet inside its faces
  winning. A run is split where a contact moves onto the next face, at the parameter where it
  crosses the shared edge (bisection on its signed distance), so every piece is smooth on one face
  per side; a section unsolvable on the earlier faces while bisecting leaves the switch where the
  faces changed rather than failing (past a tapered rim's corner the ball no longer meets the
  corner's cylinder). A chamfer's split is moved `BREAK_OFFSET` past that edge, as a tool joint on
  a body edge leaves the boolean nothing to divide; a fillet's is not, its contact moving by the
  radius over the face's radius times the offset. Sections double from `FIRST_STATIONS` until the
  interpolated rows hold the sections between them within `OUTLINE_FIT` (chamfer) or `FOOT_FIT`
  and `ARC_FIT` (fillet), at most `MAX_STATIONS` (`Intricate` beyond). Station tangents of a piece
  blend linearly into the joint tangents, and the two sections at a joint, solved on slightly
  different faces, are averaged when within `JOINT_GAP` (else `Intricate`), so neighbouring pieces
  share it exactly. Either refusal is `TooLarge` instead when a section's foot or ball centre
  reaches past `1 - FOLDING_SPEED` of the way to the edge's centre of curvature (`folds`), since
  the blend then folds back on itself (a rim fillet over rounded corners no larger than it).
- The tool (`loft.rs`) is a prism of ruled B-spline faces through each section's outline: the chord
  between the feet overhanging both by `OVERHANG` of its length (crossing the faces transversally),
  an apex `APEX_REACH` out along the feet's normal bisector (into the body for a concave edge), and
  planar caps at an open chain's ends. Each section carries the edge's curvature (`Cross::bend`);
  an outline point moves along the chain at `1 - offset * bend` times the edge's speed, turning back
  past the centre of curvature, so both reaches are cut for the whole chain at once
  (`Reach::unfolded`) until every chord end and apex keeps `KEPT_SPEED` of its origin's speed;
  reaches cut below `LEAST_OVERHANG` and `LEAST_APEX` are `TooLarge`. Square, slanted and round ends
  take the swept tool's `End`s, except that a square end is extended and, where that would change
  something, clipped rather than flush (`SquareEnd::Past`): stations run on past the end along the
  faces' own intersection (`on_both`), so the clip cuts where the loft is smooth. A lofted chain
  ending at a rounded corner (a `Setback`) is `UnsupportedEnd`; a section with no ball or feet, or a
  foot outside its faces before an end plane, is `TooLarge`; `feet.rs` does not check lofted feet
  against other blends' feet.
- A chamfer is the tool's boolean. A fillet's boolean leaves chord faces, which `rounding.rs` turns
  into the fillet: each face named after a lofted run is matched to its chord surface and given the
  rolling-ball surface lofted through the same sections (rational B-spline, cubic along the run,
  the exact circular arc across it), its sense voted by the normals at its edges' middles; edges
  along the feet take the feet's rows, joints the section's arc, and every other edge (against an
  end face or another lofted fillet) is traced again between its vertices on the new surfaces
  (`IntersectionCurve::through`). The solid is rebuilt through `SolidBuilder` with every pcurve on
  a changed face or edge refitted, and validated.
- `TooLarge` covers a blend that does not fit on both faces at sampled points along the edge, whose
  foot crosses an edge of its face (other than seams and edges at the blended edge's ends), whose
  foot crosses another blend's on that face (`feet.rs`; edges sharing a vertex excepted), and a
  knife edge (opposite normals). A sample past the plane of a flat face at one of the edge's ends
  (`end_planes`) fits whatever face it lands on, since that end extends or clips the tool there.
- Convex tools are lifted clear of the faces they cut and subtracted; concave ones are flush and
  added. All concave edges go first, then the convex ones are re-found by reference in the filled
  solid (one not found is `Lost`; errors about unchosen edges of the filled solid are `AfterFill`
  without an id).
- Ends continuing into another chosen edge stop flush, ends on a perpendicular face stop there, ends
  on a slanted face extend past it when the extension changes nothing, else are clipped by its
  plane. Where the extension would change something and the end face belongs to another chosen
  straight edge of the same kind sharing one flat face (concave edges round a boss's outside corner,
  convex edges into an inside corner), both ends are mitred (`mitre`): each tool extends past the
  vertex and is clipped by the plane through it bisecting the two edges.
- An end clipped by a curved face (a notch's floor edge running out through a round wall) is
  `End::Trimmed` when that face is a cylinder, cone, sphere or torus (`round_end.rs`, `Round`): near
  the end (`Cut`: from the vertex, `behind` back along the edge, `span` across) the tool loses what
  lies beyond the face's surface, a revolved profile's region (for a torus seen from outside, the
  quadrant corner the face faces, so the tube's other quarters keep their material). The quadrant is
  read from the torus face's own normal at the middle of its uv box (`facing`), never from the
  cut's direction; a face facing no one quadrant (leaning less than `LEANING`) is cut by the whole
  tube's outside. A concave tool also loses what lies beyond each round face beside the end
  (`beside_ends`), anchored at the corner those three share, and its feet may cross the edges to
  those faces without `TooLarge`; an end beside a face neither flat nor round keeps none.
- Refused corners, each pinned by a test: a convex edge rising from bevelled concave edges ends,
  after the fill, where two fill faces meet and is `UnsupportedEnd`; feet meeting exactly across a
  fill are `TooLarge`; fills that use up a whole face (a pocket's walls) lose the edges on it
  (`Lost`). A circular edge whose extended sweep would pass a full turn is `WrapsAround`.
  `survey.rs` (ignored) chamfers every corner of twenty bodies.
- Three convex straight edges filleted at a vertex of three planes get a spherical corner
  (`corner.rs`: a hexahedron minus the rolling ball); other corners mitre.

# Patterns (`pattern/`)

- `pattern(solid, copies, feature)` places a copy for each `PatternCopy` (an index `[column, row]`
  and a `Similarity`, so copies may be mirrored or scaled through `Solid::mapped`) and unions the
  original with every copy: valid or a `PatternError`. `pattern_copies` unions the copies alone,
  leaving the original out, for the document to cut or join a repeated feature's tool.
- Copies are unioned in pairs, round by round (about log n rounds of booleans on neighbours, not n
  booleans against an ever larger body); copies that do not touch stay separate lumps of one body,
  and coincident faces of touching copies merge by healing. Copies meeting only along an edge or at
  a point (a union that is `NonManifold`) are kept as separate shells (`Solid::beside`, which
  appends the arenas with shifted ids, accepted when it validates). Any other union failure is
  `PatternError::Union` naming the copies of both sides, the original as `[0, 0]`.
- Copies are renamed and given `FaceOrigin::Copy` (`kernel-naming.md`); the original keeps every
  name and origin.

# Closing faces (`enclose/`)

- `enclose_faces(solid, faces, feature)` turns chosen faces into a closed tool: valid or an
  `EnclosureError`. Their open edges (used by a chosen face and an unchosen one) are chained into
  loops; a vertex where two open edges start is `OpenBoundary`. Each loop takes the first support
  holding it within `LINEAR_RESOLUTION`: the plane of an unchosen flat face beside it, the surface
  of an unchosen curved face beside it (the face a pocket was cut into or a boss stands on), else a
  plane fitted to its edges; none is `NotFlat`. A curved support must be a cylinder, cone, sphere or
  torus (`NotElementary` otherwise), and a loop not closing in its uv (around the cylinder or a
  pole) is `AroundSurface`.
- Loops on one support (one plane, or one surface by `same_surface` with the same face sense) are
  nested by containment, in the plane's frame or the surface's uv (samples projected in a chain,
  then shifted by whole periods next to the first loop), into cap faces: even depth an outer loop,
  odd a hole that must wind against it (`Openings` otherwise). A flat cap's normal follows its
  outer loop's winding. A curved cap is a patch of the neighbour's surface with its sense reversed;
  an outer loop winding the other way in uv is `Openings`.
- Chosen faces are copied with surfaces, names, origins and settled pcurves; caps are added with
  pcurves fitted in their surface's uv (`SolidBuilder::add_loop`, which places a loop across a seam
  in one piece), named `FaceName::closure(feature, edge names)` with the origin of the unchosen
  face beside them. A shell meshing inside out (`SolidBuilder::inside_out`) bounds a cavity: every
  face and loop is turned over and the result is `Bounds::Cavity`; otherwise `Bounds::Material`.
  Validation catches the rest (a lone flat face closing on itself) as `Build`.

# Shell (`shell/`)

- `shell(solid, open, thickness, feature)` offsets every face by the thickness and subtracts the
  result. Only flat faces open. Offsets are exact planes, cylinders, spheres, tori and cones (a cone
  offset past its apex is framed again on the same nappe).
- `inner.rs` solves each vertex on the offset surfaces of its faces; a vertex its faces leave free
  is also held to the axial plane through any round seam at it, so both ends of a seam stay on one
  ruling. `edge.rs` rebuilds a line or circle edge through its offset ends when that lies on both
  offset surfaces, else takes the branch of the offset surfaces' intersection through both ends;
  an open edge whose ends pass each other shrinks to nothing.
- The topology is kept except where the offset changes it (`collapse.rs`):
  - dropped faces: a cylinder, sphere or torus curving more tightly than the thickness; a flat
    face bounded only by circles of one cone whose offset tip passes the face's offset; a one-loop
    face whose offset edges all shrink to nothing or all but two apart from each other (a chamfer,
    a narrow top, a cone band closing into a ridge). Shrinking faces are found from the solved
    vertices, then everything is solved again without them, round after round. A four-sided face
    whose edges all shrink first tries closing into a ridge along either pair of opposite edges
    (`narrow_to_ridge`): the pair is kept when, solved again with the other pair vanishing, its two
    edges, merged into one between the faces beyond them, neither turn over nor shrink to nothing;
    only when neither pair holds do all four vertices merge into one;
  - a one-loop face whose kept edges form two sides apart, at least one a chain (a rim split by
    another face's seam), is dropped only when its offset outline turns over: the loop's offset
    vertices projected onto the original surface enclose a uv area of opposite sign or none
    (`Layout::outline_inverts`), since counting chains alone closes faces of a chamfered box that
    survive. Such a dropped face is a `Band` closing into a ridge between the faces beyond its two
    sides (`Rim`s, both walked from one end of the band to the other): each joint of one side lies
    across the rim of the other side nearest to it on the original body, which orders the joints of
    both sides along the ridge (`ridge`, refused when they do not follow each other in order), and
    is solved on the offsets of the faces beyond the rims on both sides (a vertex of two offset
    surfaces held by a seam plane when its own faces leave it free). Each stretch of ridge between
    consecutive joints becomes a new edge (a `Span`, its curve rebuilt from the open rim of the
    two) between the faces beyond the rims covering it, and each rim maps to the stretches it
    covers in order. A band whose rim's far face drops as well has no surface for its joints:
    `ShellError::ClosesBesideClosing`, naming both faces;
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
  half the body in every direction. The prism's sides are square to the opened face, so a closed
  face meeting it at a sharp edge and leaning over it (outward normal along the opened face's, a
  chamfer round the rim) would be cut thinner than the thickness: `ShellError::Overhang`, refused
  before building.
- `ShellError::Walls` names the faces whose offsets failed to build or settle, up to
  `NAMED_WALL_FACES`; a failure spread over more names none. When the outward attempt only fails to
  keep every wall and the inward one fails with an error naming its cause
  (`ShellError::names_the_cause`), that error is reported instead of `TooThick`.

# Offset faces (`shell/offset.rs`)

- `offset_faces(solid, faces, distance)` moves each face along its outward normal (positive grows
  the body) and lets neighbours extend or trim to meet it: valid or an `OffsetError`. It is the
  shell's inner solid with one distance per face (`Offsets::moving`: chosen faces `-distance`
  inward, every other face 0), so vertex solving, edge rebuilding, collapses and corner splitting
  are the shell's. A face at distance 0 keeps its surface whatever it is (splines, extrusions and
  revolutions included) and an edge between such faces whose ends did not move keeps its curve;
  only a moved face must be a plane, cylinder, cone, sphere or torus (`UnsupportedFace`).
- Faces keep their names and origins (`Naming::Kept`), and edges keep theirs (only edges the offset
  makes new are named `between` their faces, disambiguated by the plan), so edges told apart by
  `between_at` or `occurrence` stay apart and references survive; the shell names its inner faces
  after the shell instead and renames every edge from them.
- A face that would vanish (`Collapses` drops a cylinder, sphere or torus gone to nothing, a face a
  moved neighbour closes up) is `Vanishes`, never dropped silently. After a valid build,
  `find_crossing` must find no face pair crossing (`Crosses`); a body failing validation first is
  `Invalid`. A moved face beside an unmoved face it was tangent to cannot keep that tangency, so its
  edge is `UnsupportedEdge`; moving the whole tangent chain (`tangent_faces`) keeps every tangency
  (radii grow by the distance).

# Threads (`thread/`)

- `thread(solid, ThreadSpec, feature)` cuts a modelled helical thread into a shaft or bore and
  returns a valid solid or a `ThreadError`; `thread_tools` gives the tools it subtracts. A
  `ThreadSpec` holds the frame (origin on the axis at the thread's start, normal along the thread
  into the material, x axis where the groove's angle is zero), the `ThreadedFace` (`Shaft`, `Bore`)
  and its radius, a `ThreadForm` (pitch, flank half angle, root radius, axial groove width at the
  root, crest radius), the `Handedness`, the length and how each end finishes (`ThreadEnd`):
  `RunsOut` carries the groove `RUN_OUT` (1.5) pitches past the end, for an open end; `Stops` ends
  it square in the plane through the axis where its widest part reaches the end (a shoulder or
  blind bottom).
- The tool is the groove swept along the helix, built directly as a `Plan` per turn (pieces of at
  most one turn, so no face's projection lands on the wrong turn): a root and a far face on exact
  `Cylinder`s and two flanks, each a ruled rational B-spline between two helices. The helices are
  rational quadratics, `SPANS_PER_TURN` (16) per turn with double knots, whose plan view is an
  exact circle and whose axial position follows the angle within 8e-5 of the pitch, so every edge
  lies exactly on its cylinders and is a boundary row of its flank (straight pcurves). The two caps
  are planes through the axis. Where an end runs out, a piece boundary is placed where the groove's
  middle crosses that end plane, so the plane never passes a hair from a corner.
- The far face lies `FAR_REACH` (5 %) of the pitch beyond the face (outside a shaft, inside a
  bore), so flanks cross the face cleanly. A shaft wider than the crest is first turned down to it,
  and a bore narrower than the crest (the minor diameter) bored out to it, over the thread's length
  by a revolved ring (`RING_ENTITY` entities), then the groove is cut. Errors: a groove at least
  `1 - LEAST_LAND` of the pitch wide at the far face `GrooveOverlaps`, a root not inside the face
  `RootOutsideFace`, more than `MAX_THREAD_TURNS` turns `TooManyTurns`, a length its form cannot fit
  `TooShort`.
- `thread_tools_numbered` gives the same tools with every face's entity renumbered by the caller
  (`Solid::with_sides_renumbered`, `FaceName::thread` of the new number, ring faces included), so a
  hole threading several bores keeps their faces apart.
- Faces are `FaceName::thread(feature, part)` with `FaceOrigin::Side { feature, entity: part }`:
  `GROOVE_PARTS` numbers root, leading flank, far face and trailing flank 1 to 4 plus `per_turn`
  (8) times the turn from the start, and the start and end caps 5 and 6.
- Cost: an M4x20 shaft threaded its whole length takes about 0.7 to 0.9 s in release, an M8 bore
  12 mm deep about 0.35 s (`thread/tests.rs` prints the times); volumes agree with the analytic
  groove volume within 1e-4. Measuring a threaded body's mass exactly (`mass_properties`) does not
  converge on the flanks and falls back to the mesh after a long search (roadmap).
