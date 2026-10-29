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
  - "crates/caditor-kernel/src/coil/**"
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
  rational arc. A line whose ends differ in height by at most `FLAT` is a plane, any steeper one a
  `Cone`, which therefore accepts slopes down to `MIN_FLAT_ANGLE` off square so no line falls between
  the two. A profile right of the axis is revolved about the reversed axis. Lines on the axis
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
  sweep planes, arcs and circles cones about their own centre; a spline, ellipse or elliptical arc
  (`profile::Course`) sweeps a ruled `BSplineSurface` between two cubic rows through its exact
  offset at the segment's two heights (`taper_bent.rs`, nodes graded to `ROW_TOLERANCE`), long
  enough at either end to hold the corner where a ray carries it past the curve's end (a drawn-out
  taper at a convex corner); each ring edge is the course's fitted spline at its level, its
  sense its own, since a curve cut at one level is not reversed at another. Corners are re-met at
  each level (`profile::offset_courses`), straight where both pieces are lines or tangent, else an
  `IntersectionCurve` through `JOINT_SAMPLES` levels. Below `STRAIGHT_TAPER` it is `extrude`.
  Errors: a loop that is one whole spline or ellipse `TaperedUnsupportedCurve`, a slanted end
  `TaperedTiltedEnd`, `MAX_TAPER_DEGREES` or steeper `TaperTooSteep`, a piece used up, an arc
  shrunk to nothing, corners no longer meeting, a loop turned over or loops crossing at a cap
  `TaperCloses`.
- `extrude_tapered_along(plane, regions, extent, angle, direction, feature)` is the same sweep
  slanted like `extrude_along`: every section parallel to the sketch is the square taper's section
  slid along the direction, and the taper is measured from the direction, the profile stepping in
  by `tan(angle)` for each unit travelled along it (`tan(angle)` times the direction's length per
  unit of height). Square to the sketch that is the ordinary taper; slanted, a wall running along
  the lean drafts by the angle from the direction and one facing it by about the angle times the
  lean's cosine (the offset is measured in the sketch's plane, as the slid sections need). Lines
  sweep planes through the line, the direction and the step in; an arc or circle sweeps an oblique
  circular cone, exactly a rational quadratic `BSplineSurface` ruled between its circles at the
  segment's two heights (`taper_bent::leaning_cone`, whole turn in four quarters, the seam at a
  circle's start or opposite an arc's middle); splines and ellipses keep their ruled rows, lifted
  along the direction. It refuses what the square taper refuses, a direction in the sketch plane
  being `DirectionAlongSketch`; below `STRAIGHT_TAPER` it is `extrude_along`.
- `extrude_flared(plane, regions, extent, flank, feature)` is the same sweep with the profile
  drawing in by `1 / tan(flank)` per unit of height, so the sides lean `flank` from the sketch
  plane instead of from its normal: a draft of a thin wall's broad faces, where the angle is
  small. `flank` below `MIN_FLANK_DEGREES` or not under a right angle is `FlankOutOfRange`.
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
- `wrap_regions_within(plane, regions, surface, Some(WrapBand), feature)` is the same with the
  copies at chosen normal distances instead of a quarter of the radius: `WrapBand` holds an `inward`
  and an `outward` distance (positive, `UnusableBand` otherwise) from the unrolled surface towards and
  away from its axis, the wrap's start and end caps lying at the one the sketch's side faces. An
  inward distance leaving no radius is `ReachesAxis`. The document's emboss builds its raised or
  recessed material with it.
- `wrapped_stand(plane, regions, surface, at)` (`wrap.rs`) gives where a sketch point lands on the
  surface the same regions wrap onto (the reference of their reach included) and the unit normal
  there pointing away from the axis, for the emboss's depth arrow; the errors are the wrap's.
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
- `sweep(plane, regions, path, SweepOrientation, feature)` (`sweep/`) sweeps regions along a path of
  `PathSegment`s (curve, range, the caller's key): valid or a `PathSweepError`, whose path errors
  name segments by their index in the input (`segments()`) and which wraps `SweepError` for the
  profile's. `path.rs` chains segments by their ends (`Branches`, `Apart`, `SegmentTooShort`),
  walks an open chain from the end nearest the profile plane (then nearest the profile's middle),
  and opens a closed one where it crosses the profile plane nearest the profile's middle, splitting
  that segment (`ClosedPathMissesProfile` when it never crosses), walked so it leaves along the
  plane's normal. Ellipses become rational splines and intersection curves interpolated ones. The
  profile is carried by a placement starting where it is drawn: `Square` keeps its angle to the path
  (rotation-minimising), `Parallel` only translates it; the path must leave the plane by
  `LEAST_RISE` (`StartsAlongProfile`). A segment may wind about an axis
  (`PathSegment::winding_about`, a `WindingAxis`; a helix's turns do): kept square, its leg carries
  the profile by the screw motion along it (`stations::winding`: turned about the axis by the
  angle the path point has gone round, unwrapped step by step, then moved so the path's start lands
  on the path point), so a profile keeps its place against the axis (a Frenet frame, which a
  rotation-minimising one would drift from by a turn's worth of torsion), and consecutive legs
  winding about the same axis join without a corner check. Such a leg is fitted like a spline and
  checked for folding from consecutive stations (`fold_check`: every profile sample must advance
  along the profile's normal by `FOLD_MARGIN` of the path's step, else `ProfileTooLarge`, a profile
  reaching the axis); turns too close for the profile are `CrossesItself`.
- Each path segment (a leg) is a ring of side faces, one per profile piece; joints are sections of
  the profile shared by the legs either side; an open path has extrude's start and end caps and
  their names. A line leg is an extrusion along it (`side_surface`); a square arc whose placed
  profile holds the arc's axis within `MERIDIAN_SLACK` takes revolve's surfaces (`Frame::turning`,
  `prepare`, `surface_of`: plane, cylinder, cone, torus, `Revolution`); a parallel arc or spline
  is the exact tensor product of the piece's spline and the path's (`sliding_surface`); a square
  spline, or an arc the profile leans to, is fitted (`stations.rs`): the pieces' control points are
  carried by the frames at stations doubling from `STATIONS_PER_SPAN` a span until the cubic rows
  hold the frames halfway between within `FIT_TOLERANCE`, at most `MAX_STATIONS` (`TooIntricate`),
  frames along a spline rotation-minimising by double reflection. Edges along a leg are lines,
  circles about the arc's axis, the path's spline translated or the stations' trajectory, so each
  is exactly a boundary of its faces.
- Joints within `SMOOTH_TURN` continue the placement. A parallel sweep takes any corner as it comes
  (the profile only translates). A square sweep mitres a corner between two lines: the profile
  turns by the corner, the section is it projected along the incoming line onto the bisecting
  plane (extrude's tilted `Cap`), refused as `CornerTooSharp` past `MOST_MITRE_TURN` or when the
  mitre reaches either line's far end; any other corner is `Corner` (rounding the path in its
  sketch is the fix). A closed path must bring the profile back within `FIT_TOLERANCE`
  (`ClosedPathTwists`; a planar one always does), the last fitted row then made the first exactly;
  one whole-arc leg is revolve's full turn (flat pieces annuli, sections seams).
- Folding: a square sweep checks the motion's Jacobian `(1 − κ·r)(T·n) + (κ·n)(T·r)` at each
  leg's start and every station for every profile sample, which must keep the start's sign above
  `FOLD_MARGIN` (`ProfileTooLarge`: the profile reaches the bend's centre); a parallel sweep must
  keep rising by `LEAST_RISE` along every leg (`TurnsAlongProfile`), which also makes it never cross
  itself. A square sweep of more than one leg or with a fitted leg runs `find_crossing` after
  validating (`CrossesItself`; inconclusive passes) when the path has several legs or its leg bends
  freely (`bends_freely_along`: not a line or a meridian arc, whatever the shape). Guide rails and
  round corners are roadmap entries.
- `sweep_shaped(plane, regions, path, (SweepOrientation, SweepShape), feature)` twists and scales
  the profile along the way (`sweep` is it with `SweepShape::NONE`). `SweepShape { twist, scale }`
  (radians in all, end scale; a non-finite twist or a scale outside `Similarity::scaling`'s range
  is `UnusableShape`) becomes a `Shaping`: at each station a `Similarity` scaling by
  `1 + (scale − 1)·f` and turning by `twist·f` about the line through where the path's start meets
  the profile plane along the plane's normal toward the travel (positive is counter-clockwise
  looking back along the path), `f` the fraction of the path's length travelled (`Curve::length`),
  applied before the rigid frame (`stations::shaped`). Every leg is then fitted (`shaped_motion`:
  lines by `sliding` translation, arcs by `rotating`, windings, splines rotation-minimising; at least
  `Shaping::stations_for` stations, one per `STATION_TURN` of twist), the placement chained by each
  leg's rigid `Stations::finish`, sections `Section::Placed(Similarity)` (scaled ones mapped by
  `Curve::mapped` with its parameter stretch) and the end cap `Plane::mapped`. A sharp corner
  between lines is `ShapedCorner` (no mitre). A closed path cannot scale (`ClosedPathScales`); one
  bringing the profile back turned within its plane is `ClosedPathTwists { turned }` with the angle
  about the travel (`turned_by`, none when tilted), and a turn within `CLOSING_SLACK` is planned
  once more with the twist less it (`Closing::Corrected`, accepting `FIT_TOLERANCE` plus twice the
  slack times the profile's reach), so a closed path out of one plane closes once the twist brings
  the profile back, the error giving the angle to type.
- `pipe(path, PipeSection, bend, up, feature)` (`sweep/pipe.rs`) is `sweep` of a section it
  builds: `PipeShape` round, square or equilateral triangle, centred on the path, with an inner
  outline `wall` inside for a hollow pipe (entities 1 up for the outer outline, `curves()` more for
  the inner, so the region is the depth-0 ring), placed square to the path's first tangent with
  `up` (rejected from the tangent, falling back to Z, X, Y) as its y axis so flat sides lie along
  the path's plane. It chains the path (`path::ordered`, the walk without orienting to a profile)
  and, for `bend` above zero, replaces each corner between two path curves of any kind turning
  more than `SMOOTH_TURN` (`bend.rs`). When both curves lie within `LINEAR_RESOLUTION` of the plane
  of the corner's two tangents (sketch curves, flat edge chains), the bend is the arc of that
  radius tangent to both, so a round section sweeps it as a torus: its centre lies the radius
  inside both curves (the side the corner turns to), found by Newton on the two curves' parameters
  from where the chord `bend · tan(turn / 2)` from the corner meets each (exact for two lines),
  held within their ranges. Otherwise the bend is a cubic Bézier from the points at that chord,
  tangent to both curves, its handles a circular arc's for the turn between them, swept fitted.
  The curves are cut back to the bend's ends (a curve used up exactly is dropped, more than its
  length, or no tangent arc within the curves, is `BendTooLong`; a turn past `MOST_MITRE_TURN`
  `CornerTooSharp`); bends are keyed `naming::path_bend` of the two curves' keys. Sweep errors
  come back naming input segments (`Rounded::remapped`), a fold on an inserted bend as
  `BendTooTight`; a wall leaving no inside is `WallTooThick`. With `bend` zero, corners touching
  an arc or spline are left to the sweep (`Corner`).
- `pipe_stand(path, bend, up)` walks the same chain and returns the `PipeStand` the app's value
  arrows stand on: the section plane `pipe` places (from the rounded chain, else the unrounded one)
  and the first `PipeCorner` turning above `SMOOTH_TURN` and up to `MOST_MITRE_TURN`, found whether
  or not `bend` rounds it. Its `middles` are `BendMiddles::AlongBisector` between two lines;
  where an arc, spline or edge curve meets the corner they are `Sampled`: the corner at radius zero,
  then the middle of the bend `bend::rounding` makes at `BEND_SAMPLES` radii up to the widest the
  shorter leg leaves room for, and at `bend` itself, radii it cannot round left out. It needs no
  section, so it works for sizes `pipe` refuses.
- `loft(profiles, apex, LoftSkin, feature)` (`build/loft/`) passes a solid through
  `LoftProfile`s (a plane and one `Region`, refused as `Holes` when it has any), closing to `apex`
  when given: valid or a `LoftError`. `rings.rs` turns each profile's outer loop to run
  counter-clockwise about the way the loft runs there (towards the next centre, from the previous
  one at the last; a plane within `LEAST_RISE` of that way is `AlongProfile`, centres within the
  tolerance `Coincident`), then matches counts: every profile is split to the most pieces any has,
  its longest span halved by parameter again and again, except that a lone circle or ellipse is
  split evenly starting at the point facing its neighbour's first vertex. From the first profile
  with the most pieces outward, each neighbour is turned (cyclic shift) so corresponding vertices
  lie nearest about their centres. `skin.rs` makes each column of matched pieces compatible in
  homogeneous coordinates: Bezier segments extracted, degree raised to the column's highest, the
  union of normalised breakpoints inserted, each segment reweighted to end weights 1 so every
  vertex and joint has weight 1. `Ruled` (and any loft of two rows) is a spline surface linear
  across each pair of neighbouring rows, its edges along the vertices straight lines and the
  profile edges between legs shared; `Smooth` interpolates every column through all rows at
  chord-length stations of the centres (degree up to 3), its edges along the vertices are the
  surfaces' u-start sides, and inner profiles leave no edge. Profile edges are each profile's own
  curves over the pieces' ranges; an apex is a pole row and one vertex. Faces are
  `FaceName::lofted` of the first profile's piece, part and leg (`kernel-naming.md`), caps are
  start and end caps of the first and last region keys. Sides running into each other are refused
  by `find_crossing` after validation (`CrossesItself`). At most `MAX_LOFT_PROFILES` profiles,
  `MAX_SPANS` pieces a profile and `MAX_SEGMENTS` segments a column (`TooIntricate`). Rails, a
  centreline and curvature-continuous or face-tangent ends are roadmap entries.
- `loft_through(profiles, apex, (LoftSkin, LoftEnds), feature)` closes and squares ends (`loft` is
  it with `LoftEnds::OPEN`). `closed` (at least three profiles, `TooFewToClose`; no apex,
  `ClosedWithApex`) orients every ring by the central difference of its neighbours' centres
  (`closed_travel`), repeats the first ring's curves as a last row with the closing chord among the
  stations, wraps ruled legs back to row 0 and runs one smooth leg through all with equal end
  slopes (the quadratic through the last, first and second rows, so the closing profile is passed
  smoothly), shares row 0's edges between the last and first legs (`Loft::wrapped`) and has no
  caps. `start` and `end` weights (finite, above zero, else `UnusableWeight`; smooth only,
  `SquareOnRuled`; never on a closed loft or toward an apex, `SquareOnClosed`, `SquareAtApex`) leave
  that end square: columns are interpolated cubically with end derivatives (`skin::hermite`: the
  stations as interior knots, the second and last-but-one control points from the slopes, a
  tridiagonal solve), a square end's slope the ring's oriented normal times the weight times the
  centres' total chord length, a free end beside it the quadratic through its three nearest rows
  (the chord for two). A two-profile loft with a square end is one cubic leg (`Leg::ruled` false, its
  edges along the vertices the surfaces' sides).
- `heights(plane, regions, target)` gives the least and most signed height of a target plane over
  the profile, to tell a plane ahead from one behind or across.
- `next_face(solid, plane, regions, reversed)` casts rays from the regions' triangle centroids
  (about `RAY_SAMPLES`, spread by area) and from the corners around each solid vertex projecting
  into a region (`VERTEX_NUDGE` of the profile's size away, at most `MAX_VERTEX_RAYS`, so a small
  boss between centroids is met) to the first crossing, grouping faces met by plane (coplanar
  fragments are one face): several groups `SeveralFaces`, one curved face `Curved`, else the plane
  with its outward normal and whether the rays enter. Undecided rays are skipped; none decided is
  `Undecided`, every ray missing `Nothing`, some `Partly`. `next_face_along(solid, plane, regions,
  direction, reversed)` casts the same rays along a slanted direction, the solid's vertices carried
  back to the sketch along it for their corner rays; a direction in the sketch plane is
  `AlongSketch`.
- `stop_at_body(tool, body, plane, regions, reversed, far)` cuts a sweep made to `far` back to
  where it first meets a body: it classifies up to `MAX_START_PROBES` points just ahead of the
  profile (all outside entering, all inside leaving, else `Straddles`), takes the difference with
  the body when entering or the intersection when leaving, and keeps the shells whose mesh reaches
  back to the start plane (`Solid::shell_spans`, `keeping_shells`, which renumbers and keeps every
  name). A void shell is `Enclosed`, a kept shell reaching `far` `PassesBeside`, nothing kept
  `Nothing`. Spans and `far` are heights square to the sketch, so a tool swept along a slanted
  direction (`extrude_along`, `extrude_tapered_along`) is cut back the same way.

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
- Snapping (`snap.rs`): a face of the second operand within `LINEAR_RESOLUTION` of a face of the
  first but more than `SNAP_FLOOR` (1e-9) off it is moved along its normal onto it before imprinting:
  a plane parallel to a plane, a cylinder whose axis runs along a plane it nearly touches (radius
  set to the axis's distance), a cylinder on another's axis (radius set to the other's). Of several
  candidates the nearest wins, and one within `SNAP_FLOOR` leaves the face where it is.
  `shell::snapped` builds the moved solid as the shell's inner solid with those offsets (neighbours
  follow, vertices settled to `SNAPPED_RESIDUAL` rather than the shell's hundredth of the
  resolution, which would leave edges where they were; every name kept). So offsets each within the
  resolution never compound past it: a block 8.3e-7 behind a plate's back and 6.2e-7 above its top
  had its corner 1.03e-6 off the plate's edge, neither pooled with it nor apart. Offsets up to
  `SNAP_FLOOR` count as exact; past the resolution faces are apart and nothing moves. Before that,
  a cylinder of the second operand parallel to one of the first, its axis more than `SNAP_FLOOR`
  and at most `SIDEWAYS_REACH` (four resolutions) beside the other's and its radius close enough
  for the walls to come within the resolution, moves the whole second operand sideways onto that
  axis by one translation (every such pair must ask for the same one); the radius is then matched
  by the offset above. Four resolutions, not one: axes 1.5e-6 to 4e-6 apart leave a crescent at
  most that thick whose walls lie within the resolution of each other over much of the round,
  which imprinting cannot split cleanly (`NonManifold`, `Open`), while a shift that small is
  invisible. The shift is dropped when it would take any face pair the offsets above would join
  (`contacts`) out of the resolution. When snapping fails or its boolean fails, the operands as
  given are combined; an `Empty` or cancelled snapped result stands.
- Each operand pair is combined with overlap cuts judged at the piece's middle and, only when that
  fails for every pair, again judged along it (`OverlapCuts`, below); the error reported is the
  first of the operands as given.
- What no tool touches is carried rather than rebuilt, so work follows the size of the contact: a
  face's loops with unsplit, unshared edges and vertices no cut or other loop reaches keep their
  pcurves and skip tracing and refitting; a fragment with an edge whose box misses the other
  solid's extent is outside without sampling; coedges reaching assembly with the operand's own
  edge, surface, sense and pcurve go to the `Plan` as `PlanPcurve::Settled`, skipping geometry
  validation (`SolidBuilder::settle`). Every other check, volume and lump checks over the whole
  result included, still runs. `Input::with_carrying(.., false)` runs the full pipeline;
  `carry_tests.rs` compares the two.
- Imprint: vertices within `LINEAR_RESOLUTION` are pooled; every edge and face-face branch is split
  at the pooled vertices on it. Where two branches of one face pair cross (sections meeting where
  the surfaces touch: tori whose equators touch, a torus and its copy turned a little, whose
  equators cross the meridians in the plane of the turn), the crossing inside both faces is pooled
  too (`branch_crossings`), so both branches split there and tracing has a vertex to turn at. An edge found lying on the other solid's surface that passes from
  one side of it to the other (at least `CROSSING_SIDE`, half the resolution, off at one end,
  `CROSSING_FLOOR`, a tenth, at the other, and changing side once, `shallow_crossing`) crosses it
  at a shallow angle instead: it is split where it changes side, where the faces' own intersection meets it, and is no overlap (a plug's rim
  1.5e-6 off its bore's axis was otherwise split where it came within the resolution, far from the
  walls' crossing). A piece of an edge lying on a face is a cut of it when its middle lies strictly
  inside the face, unless the edge only lies within the resolution's band of a face its own faces
  meet elsewhere (`met_elsewhere`): every face of the edge differs from that face's surface, runs
  along it (normals within `ALONG_SINE`), and meets it in branches, one passing through an end of
  the piece and none through its middle. A torus's seam on its equator, beside its copy turned
  3e-4 radians, starts where the equator sections cross it and drifts 8.7e-4 from them while staying
  2.6e-7 from the copy; cut, it left a sliver whose edge four fragments used (`NonManifold`). A
  plug's seam running beside the line where its wall crosses its bore's meets no branch at its
  ends and stays a cut, so the twin strips below decide it. The middle's rule is
  `OverlapCuts::AtMiddle`; on the retry a piece cuts when some of `INSIDE_SAMPLES` lies
  strictly inside and none outside (`WhereverInside`: a crescent rising past the resolution only
  near its end, a plug 1.1e-6 off its bore's axis). The middle stays the first rule, since a whole
  rim rising past the resolution in one bulge beside coincident walls must not cut. A branch piece is kept when no sample along it lies outside either
  face and some sample lies strictly inside each (so a piece a micrometre or two long still cuts);
  a piece no longer than twice the resolution is kept when no sample lies outside either face, as
  is one running within the resolution along an edge piece with the same end vertices when one of
  its two faces is bounded by no such edge (`cuts_beside_edge`; three plates turned about their
  shared edge each have their own edge there, and the kept piece, joined to another shell's edge,
  left a face that could not be divided, so a pattern of them failed as `Split` instead of keeping
  separate shells), which it then joins (a cylinder's cap meeting a block's bottom in a chord 2.9e-6 long along the block's
  edge, where the cap crosses that edge 1.4e-6 inside its rim: dropped, the bottom had no cut to
  turn along the edge's piece between the cap and the rim, and the result was `Open`).
  Pieces with the same end vertices and geometry within `LINEAR_RESOLUTION` (the bound validation
  holds edges to) are one edge, so an intersection along an existing edge and coincident faces need
  no special case; pieces a few micrometres apart stay two edges bounding a sliver. Pieces of two
  different edges of one operand stay apart (`pinched_apart`): a valid solid has such a pair only
  where it pinches, as two bores extruded from tangent circles meet in two coincident edges along
  the line where they touch, and joined into one they read as an edge used four times, so any cut
  or join beside the pinch failed as `NonManifold` there.
- Face tracing: a face with no cuts and no split edges passes through with its own loops and
  pcurves. Otherwise it is traced into loops from its boundary pieces and cuts; at each vertex the
  next edge is the first clockwise from the arriving one about the outward normal (at a pole, the
  mean normal of a ring around it); ties and cusps (`ANGLE_TIE`) are decided by chords at a common
  distance. A traced loop whose fitted pcurves enclose no area has its coedges halved round after round
  (`untangle::open_flat`) before it is refused as `Split`: a cylinder's cap meeting a frustum's
  wall in an ellipse that leaves the top rim and rejoins it 0.055 along, never more than 4.1e-5
  below it, had both edges fitted from their ends alone at a tolerance wider than that.
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
  every face it touches, which must agree. A face whose surface passes through the point but which
  ends short of it (`exactly_inside_face` false) says nothing and is left out, so a strip one to two
  resolutions wide beside a coincident face (a block 1.5e-6 proud of a plate it is flush with
  behind) is judged by the face it stands off. A fragment every sample of which touches a face of
  the other solid lying within the resolution of it, exactly inside that face and with normals
  within `LYING_ALONG_SINE` (1e-2) of parallel (`lying_along`; the surfaces differ, so the sample is
  not coincident) is coincident with the sense of those normals: first, before the deepest points,
  when the other solid has a fragment bounded by the same arrangement pieces (`Twin::Matched`, as
  the strips between two cylinders' seams lying within the resolution of the other wall are),
  otherwise only when nothing else decides it. So cylinders of one radius or radii within the
  resolution whose walls cross beside their seams (an extruded plug 1e-5 to 2e-3 off its bore's
  axis square to its seams) drop or keep both strips together. A fragment decided only by lying
  along takes instead the class of a fragment of its own face decided otherwise across one of that
  face's seams no cut shares (`settle_across_seams`), since the surface runs on unbroken there: a
  torus turned 1e-4 radians against its copy leaves each a sliver between its equator seam, its
  meridian seam and the equator section, which lies along the other torus throughout, and taken as
  coincident the second's dropped while the fragments across both its seams stayed (`Open`).
  Faces that passed through share one class across unsplit edges no cut shares; one whose box
  misses the other solid's is outside. Of coincident faces only the first solid's fragment can
  stay: same orientation for union and intersection, opposite for difference. A difference
  reverses the second solid's kept fragments. Every result edge must then have one use each way,
  else `Open`, or `NonManifold` when solids meet only along an edge. First, an edge two operands'
  edges were joined into whose four uses are, for each operand, one face either side of that face's
  own seam (`part_seams`) goes back to one edge per operand, the moved coedges refitted on the
  operand's own piece: the surface runs on unbroken across a seam, so its two sheets only run within
  the resolution of each other there. A torus turned 1e-4 radians about Y against its copy has both
  meridian seams within 1.6e-7 of each other from the equator section to the first's equator seam,
  and joined, the difference used that edge four times.
- `Intersection`, `Split`, `Ambiguous`, `Open` and `NonManifold` carry a boxed `BooleanSite`: the
  input faces of each operand involved and a model point. The innermost step that knows a point
  sets it and outer steps only fill what is empty (`or_faces`, `or_point`).
- Healing: adjacent faces on the same surface with the same orientation are merged by retracing
  without the edges between them (left apart when that fails, as for a ring around a periodic
  surface), their refitted pcurves untangled and sharpened as a split face's are, since a sliver
  loop left in the merged face (the crescent a plug 5e-6 to 3e-5 off its bore's axis leaves in the
  joined top and bottom) fitted at the default tolerance runs the wrong way; two edges meeting
  between the same faces, not at a pole, are joined when pieces of one curve, the joined pcurve
  sampled no coarser than the finer of the two it replaces: a frustum's rim vertex inside the
  crescent a sphere's equator leaves 5e-5 beside the rim, refitted at the default tolerance, had
  its chords sag across the crescent and turn the loop over. Names: `kernel-naming.md`.

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
- `measured_face(solid, edges, edge, flipped)` names the face a two-distance or angled chamfer
  measures its first distance on for one of the chosen edges, by the same rule as above (the face
  more of the chain-grown edges share, ties to the smaller `FaceName`, `flipped` swapping it), so
  the interface stands the first distance's arrow on the face the kernel will use.
- Selection helpers: `tangent_chain(solid, edges)` follows tangent-continuous edges sharing a face
  through their end vertices, smooth or sharp (shares `follow` with `blend_chain`, which admits
  only sharp edges). `hole_faces(solid, faces)` (`hole_faces.rs` at the kernel root) spreads from
  each selected concave cylinder, cone or torus (outward normal pointing at its axis) to concave
  faces of the same axis and to flat faces square to the axis bounded only by circles about it whose
  every neighbour is such a wall (a counterbore's floor), so a hole is taken whole while the plate
  and tube ends are not; a boss is no hole. `tangent_faces(solid, faces)` spreads across every edge
  where the two faces' normals agree within `TANGENT_FACE_ANGLE` (0.01 rad, looser than blending's
  test so fitted imported blends count) at a quarter, half and three quarters along it.
  `edge_convexity(solid, edge)` says `Convex`, `Concave` or `Smooth` for an edge between two faces,
  at the middle of the edge: smooth when the outward normals agree within `TANGENT_FACE_ANGLE`,
  else convex when (first normal x second normal) . tangent, the tangent taken along the first
  face's coedge, is positive and concave when negative; an edge with one face, a repeated face or
  other than two faces, a normal that cannot be found, or faces meeting back to back (outward
  normals opposite within `TANGENT_FACE_ANGLE`, `meets_back_to_back`: where two bores touch, whose
  sign was rounding noise) has none. The document's convex and concave edge rules use it
  (`document.md`).
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
  a changed face or edge refitted (the feet's rows and the joints' arcs are sides of the fillet
  surface, so their pcurves there are exact straight lines, `kernel.md`), every coedge whose edge,
  ends and face are unchanged settled, and validated.
- A fillet's tool is lofted through every 2^k-th section only, the coarsest stride keeping at
  least `FIRST_STATIONS` spans whose rows hold every section within `CHORD_FIT`
  (`chord_segment`), since rounding replaces the chord and the boolean's marching cost follows its
  spans (the rim of a rounded pocket: 17 of 129 sections). A foot edge is matched to the row of a
  named face nearest its middle within `ALONG_ROW`. Rounding first moves each vertex of a foot
  edge more than `ON_ROW` off its row onto it (`onto_rows`): to where the row crosses the vertex's
  one untouched edge (the body edge into an end face, a seam), solved across that edge in the
  face's tangent plane, whose interval follows; with no untouched edge, to where it crosses
  another fillet's row on the same face. Joints are kept sections, so their vertices never move.
- `TooLarge` covers a blend that does not fit on both faces at sampled points along the edge, whose
  foot crosses an edge of its face (other than seams and edges at the blended edge's ends), whose
  foot crosses another blend's on that face (`feet.rs`; edges sharing a vertex excepted). An edge
  whose faces meet back to back (opposite normals, where two bores touch) is `BackToBack`, since no
  size fits there. A sample past the plane of a flat face at one of the edge's ends
  (`end_planes`) fits whatever face it lands on, since that end extends or clips the tool there.
- Convex tools are lifted clear of the faces they cut and subtracted; concave ones are flush and
  added. All concave edges go first, then the convex ones are re-found by reference in the filled
  solid (one not found is `Lost`; errors about unchosen edges of the filled solid are `AfterFill`
  without an id).
- Ends continuing into another chosen edge stop flush, ends on a perpendicular face stop there, ends
  on a slanted face extend past it when the extension changes nothing, else are clipped by its
  plane. An end at a corner of two or more flat faces (where earlier chamfers met) extends past all
  of them when the tool leaves through each and every edge at the corner is of the edge's kind
  (`out_through_corner`), so the edge's tool reaches the corner as its chamfer's half-space would
  (three convex chamfers on flat faces then also take a triangle corner, below). Where the
  extension would change something and the end face belongs to another chosen straight edge of the
  same kind sharing one flat face (concave edges round a boss's outside corner, convex edges into
  an inside corner), both ends are mitred (`mitre`): each tool extends past the vertex and is
  clipped by the plane through it bisecting the two edges.
- An end at a corner earlier blends made (`out_through_blends`: the uprights of a box whose top
  edges were filleted before, a pocket's corners over its rounded floor) extends out through them
  when every sharp edge there is of the edge's kind, at least one end face rises smoothly from one
  of the edge's own faces (an earlier fillet) and the others are flat faces the tool leaves through:
  past the farthest of `BOUNDARY_SAMPLES` points along each rising face's edges by `END_MARGIN`
  of the reach, and only when that point lies outside the body for a convex edge, inside it for a
  concave one (so an inner edge under rounded top edges is not stretched up into the air). The
  tool then trims the earlier fillets where it crosses them: any fillet or chamfer size, its face
  meeting theirs along their intersection curve. The extension only measures the rising faces'
  edges, so it trusts a face to reach no farther inside than along its boundary. With only one
  top edge filleted the upright's chain grows on over that fillet's end arc instead, so a size
  above the arc's radius is `TooLarge` naming the arc.
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
  after the fill, where two fill faces meet and is `UnsupportedEnd`, as is the upright of a boss
  whose foot an earlier blend rounded (the fills' seam there is concave); feet meeting exactly across a
  fill are `TooLarge`; fills that use up a whole face (a pocket's walls) lose the edges on it
  (`Lost`). A circular edge whose extended sweep would pass a full turn is `WrapsAround`.
  `survey.rs` (ignored) chamfers every corner of twenty bodies.
- Three convex straight edges filleted at a vertex of three planes get a spherical corner
  (`corner.rs`: a hexahedron minus the rolling ball); other corners mitre.
- A convex straight edge filleted, alone at its vertex, with the radius of the two earlier
  cylinder fillets that rise from its faces there and rest on one flat face (`after_blends`: the
  uprights of a box whose top edges were filleted at the same radius) gets the same spherical
  corner: the three planes are its faces and that flat face, the virtual edges along the earlier
  fillets only give the setback planes' directions, and it applies only when the ball's centre lies
  on both cylinders' axes and level with the vertex (`ON_AXIS`), so the edge's tool ends flush there.
  The body equals filleting all the edges at once. A smaller or larger radius meets the earlier
  fillets along a crease (`out_through_blends`, above), not a smooth blend.
- Three convex straight (`Sweep::Along`) edges chamfered at a vertex of three planes end on a flat
  triangle (`chamfer_corner.rs`), as other CAD tools make it, rather than the three half-spaces
  meeting in a point: on each face the two chamfers' foot lines meet at a point, and the corner
  plane passes through those three. The corner's tool is a tetrahedron of the three face planes
  lifted by `CLEARANCE` times the reach and that plane, subtracted with the edges' tools; its face
  is `FaceName::corner(feature, VertexName::of_faces(..))` with `FaceOrigin::Chamfer`. Each strip
  then ends on one side of the triangle (a box with every edge chamfered has 6 + 12 + 8 faces). The
  corner is left to the half-spaces when the vertex or the three chamfers' tip does not lie beyond
  the plane, or the plane does not cross each edge within its length.

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
- `blend_copy(solid, edges, shape, copy, feature)` blends edges of one copy for a repeated fillet
  or chamfer: `blend` under the pattern's id (so the faces it makes are the only ones of that
  origin without a copy), then each such face's origin becomes a `Copy` of `copy` with the blend's
  `feature`. Its names stay the blend's own (`FaceName::blend(pattern, edge)` of the copy's edge).
- `shell_copies(solid, open, walls, pattern, feature)` hollows a body holding a pattern's copies
  for a repeated shell: `shell_walls` under the shell's `feature`, then each inner face named
  `FaceName::shell(feature, n)` of a face `n` whose origin is a copy of `pattern` becomes a `Copy`
  of that copy with origin `Shell` of the feature (`Solid::with_face_origins_named`, names kept).

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

- `shell_walls(solid, open, &Walls, feature)` offsets every face by its thickness and subtracts the
  result; `shell(solid, open, thickness, feature)` is `Walls::inside(thickness)`. `Walls` holds the
  thickness, a `ShellSide` and `faces`, a thickness of their own per face (`Walls::of`; refused on
  an opened face as `OpenedFaceThickness`, below the resolution as `InvalidFaceThickness`). Offsets
  are exact planes, cylinders, spheres, tori and cones (a cone offset past its apex is framed again
  on the same nappe); splines, extrusions and revolutions are `UnsupportedFace`.
- `ShellSide` splits each face's thickness between inside and outside (`inward_share`: `Inside` 1,
  `Outside` 0, `BothSides` half). The outward part first grows the body (`grown`, `Offsets::grown`:
  every closed face moved out by it, opened faces left where they are so the walls end flush with
  them, `Naming::Kept` so the outer faces keep their names, a concave round tighter than it dropping
  like any collapse); `find_crossing` on the grown body refuses walls running into each other as
  `Walls` naming the original faces (matched by name). The inward part is the inner solid as below,
  subtracted from the grown body (`Hollowing::body`); outside alone leaves every closed face at 0,
  so the cavity is the body as it was, its faces named after the shell. A void's opening prism
  starts the outward part inside the void (`Hollowing::beyond`) and reaches the opened face's
  thickness, so the lining grown into the void opens too.
- Flat faces open, and so do cylinder, cone, sphere and torus faces that can be offset outward
  (outside every void, no smooth edge to a closed face): their inner face passes through the opening.
  Any other curved opening is `CurvedOpening`, since the swept opening below needs a plane, and a
  curved opening never falls back to it (the outward attempt's error is reported).
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
  - when a settle round finds faces whose every edge reversed (a chamfer's corner triangle
    shrinking to a point) among other shrinking faces, those collapse alone first, if a layout
    with just them builds (`closing_whole_first`); otherwise the whole set collapses as before;
  - a one-loop face whose kept edges form two sides apart is dropped by how its offset outline
    turns: the loop's offset vertices projected onto the original surface enclose a uv area of
    opposite sign or none (`Layout::outline_inverts`). With at least one side a chain (a rim split
    by another face's seam) the face drops only when the outline is found turned over; with two
    single edges it drops unless the outline is found still the right way round, so the rim
    strips beside a corner triangle survive when the top above them opens outward. Counting
    edges alone closes faces of a chamfered box that survive. Such a dropped face is a `Band`
    closing into a ridge between the faces beyond its two sides (`Rim`s, both walked from one end
    of the band to the other): each joint of one side lies
    across the rim of the other side nearest to it on the original body, which orders the joints of
    both sides along the ridge (`ridge`, refused when they do not follow each other in order), and
    is solved on the offsets of the faces beyond the rims on both sides (a vertex of two offset
    surfaces held by a seam plane when its own faces leave it free). Each stretch of ridge between
    consecutive joints becomes a new edge (a `Span`, its curve rebuilt from the open rim of the
    two) between the faces beyond the rims covering it, and each rim maps to the stretches it
    covers in order. Bands stacked side by side, every rim of one side lying beyond the next band,
    close as one band (`stacked`): its rims are the outermost sides, the next band's far side reversed
    when its rims run the other way along the ridge (compared on the edge they share), and each
    across edge's end on a shared boundary is tied into the cluster of the nearest across end of
    the band before (`boundary_ties`), so seams turned apart on a narrow chamfer and the narrow lid
    above it still meet one ridge vertex. A band whose rim's far face drops but is no band beside
    it, or whose side lies beyond several faces of which one drops, has no surface for its joints:
    `ShellError::ClosesBesideClosing`, naming both faces;
  - vanishing edges merge their vertices; a merged vertex must lie strictly inside the offset of
    each face that shrank away there, and beyond an opened face offset outward, so the cavity still
    opens through it, else the edge shrinking to nothing is reported (`Collapses::closes_over`);
  - an edge between two faces that both close as bands (a bottom chamfer and the side above it)
    vanishes into the ridge, and the edges beyond them, now with the same ends, join across both;
  - when a round's layout cannot be built, a face whose every vertex lies in one cluster and on
    whose offset no corner of that cluster can be solved (`split::stranded`, a vertical chamfer
    left inside a corner whose sides closed) closes with all its edges, and the round is tried
    again; the split points must still lie inside its offset;
  - a vertex of four to `MAX_SPLIT_FACES` (24) faces whose offsets do not meet is split along a
    triangulation of its cycle of faces (`split.rs`) when its edges are all convex, all concave, or
    convex but for one concave edge; other corners are `Corner`. The triangulation is searched by
    dynamic programming over the cycle (`Search`, each triangle's point and each diagonal memoised),
    taking the valid one with the fewest diagonals between coincident split points, ties to the
    earliest apex: otherwise a corner where a whole top closes could map both ends of a ridge to
    one point instead of rising to the apex. A cluster of merged vertices that
    cannot be placed at one point is split the same way (`Cluster`): the cycle walks spokes (an
    edge and the cluster vertex it leaves), so a face may come round more than once and an edge
    with both ends in the cluster gets a split point at each end; dropped faces are walked through,
    a band's two rims contracted onto one slot; split points that coincide are merged with no edge
    between them, and a triangle naming one face twice is refused. Every split point must lie
    strictly inside each face that shrank away there, and at least one beyond an opened face
    offset outward: a top closing between converging rim chamfers becomes a roof whose ridge or
    apex alone pokes through the opening, its corners below it. A top whose roof stays below the
    opened face cannot open and is refused.
- An opened face with no smooth edge to a closed face is offset outward by the largest thickness
  (`Walls::reach`), so the inner solid passes through it and the body needs room only across its
  walls; the attempt counts only when every
  closed face's inner face, dropped ones aside, survives the subtraction. A face of a void
  (`Solid::void_shells`) is never offset outward: its opening is a prism of its own outline
  reaching the thickness into the material.
- Otherwise (or when that fails) every face is offset inward and a prism swept outward from the
  offset copy of each opened face is unioned before subtracting, which needs the thickness below
  half the body in every direction. The prism's sides are square to the opened face, so a closed
  face meeting it at a sharp edge and leaning over it (outward normal along the opened face's, a
  chamfer round the rim) would be cut thinner than the thickness: `ShellError::Overhang`, refused
  before building. An opened face the inner solid leaves where it is (walls outside) has no prism
  cutting into a wall, so it is not checked.
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

# Moving faces (`shell/moving.rs`)

- `move_faces(solid, faces, motion)` carries the chosen faces by one `RigidTransform` (a shift, a
  tilt about an axis, or both) and lets neighbours extend or trim to meet them: valid or a
  `MoveError` (`Unmoved` when no point of the faces' edges travels past the resolution, sampled at
  `TRAVEL_SAMPLES` per edge, so a tilt about an axis through a face's only vertex still counts;
  `NothingToMove`, `MissingFace`, `UnsupportedFace`, `UnsupportedEdge`, `Vanishes`, `Corner`,
  `EdgeCollapses`, `Invalid`, `Crosses`, `Cancelled`). It is the shell's inner solid with
  `Offsets::carrying` and one `Motion::Rigid`: the moved faces' surfaces are their own
  transformed (`Surface::transformed`, so any surface kind moves, splines included), their residual
  is the original's at the point pulled back by the inverse (`Offsets::pulled_back`) with the
  normal carried forward, and the reach is the farthest travel. A neighbour must still be one the
  shell can extend (the edge rebuilding and `intersect_surfaces` decide).
- `move_each(solid, groups)` is the same with a motion per group of faces (`Carried`: faces and
  their `Motion`; `Offsets::carrying` keeps one per group and each face's group in `Motions`). A
  `Motion` is `Rigid` (a transform and its inverse) or `Tapered` (a draft's `Taper`: points carried
  by scaling their distance from its axis by the new radius over the old at their height, pulled
  back by the reverse, no turn of directions, its new surface given and its residual the signed
  distance to that surface directly). An edge between faces of two groups is tried carried by each
  group's motion in turn and otherwise rebuilt or intersected like one between a moved and a still
  face, since `Offsets::shared_motion` takes an edge's carried curve unchecked only when all its
  faces share one motion; a taper carries a coaxial circle (radius scaled at its height) and a line
  in a meridian plane, nothing else. The reach is the farthest travel of any group. Draft and
  replace face are built on it.
- Corners touching a moved face start solving from their carried point (`Offsets::carried_with`), so
  a seam vertex of a shifted bore lands on its own side of the circle, not the antipode; hints on a
  moved face are taken on the pulled-back point and seam planes of a moved round face are carried
  with it. An edge with a moved face is first tried as its curve carried by the motion
  (`edge::carried`): taken when its ends land and it lies on both faces (always when both moved), so
  a shifted bore keeps exact circles where it meets a still plane; otherwise the shell's rebuilding
  (a degenerate rebuilt circle falls through rather than failing) and then the surfaces'
  intersection, so a tilted top meets a round wall in an ellipse. Edge reversal and outline inversion
  are judged on points pulled back when every face of the edge moved.
- Faces and edges keep their names (`Naming::Kept`); a face that would vanish is `Vanishes`, faces
  crossing afterwards `Crosses`. A moved face tangent to a still one (a fillet along it) cannot keep
  the tangency unless it moves along it: shifting a top up with the fillet along its rim works,
  the top alone does not.

# Drafting faces (`shell/draft.rs`, `shell/taper.rs`)

- `draft_faces(solid, faces, neutral)` drafts each `DraftedFace` (a face and its own angle in
  radians) so that it makes that angle with the pull (the neutral plane's normal), a positive angle
  leaning the outward normal toward the pull and narrowing the body that way; a face already at its
  angle is left out. `check_draftable` (public, the document's pre-check) takes planes, and
  cylinders and cones whose axis runs along the pull; any other surface is `NotDraftable`, a round
  face about another axis `OffPull`. An angle not finite or steeper than `STEEPEST_DRAFT` (89°) is
  `TooSteep`, a plane parallel to the neutral plane `AlongNeutral`, no face `NothingToDraft`, every
  face at its angle `AlreadyDrafted`, a face listed twice taken once.
- A plane turns rigidly about the line where it meets the neutral plane: the turn is the angle less
  its present angle (`asin` of the outward normal along the pull), about `normal × pull`. Planes
  sharing a plane (normal and offset within the resolutions) and a turn share one motion.
- A cylinder or cone is rebuilt as a `Taper`: about its own axis (origin on the neutral plane, its
  frame's normal and x-axis kept so the seam and parameter directions stay), its radius is linear in
  the height along the axis, the same at the hinge (where the surface meets the neutral plane, so a
  cylinder keeps the circle there) and with the slope `tan` of the new half-angle (minus the sense
  times the axis's direction along the pull, times the angle: a boss narrows toward the pull, a hole
  widens). The new surface is a cone, or a cylinder when the slope is nought, so a cone redrafted to
  0 stands square and a coaxial cone takes the new angle. A radius reaching nought within the
  face's height range (sampled along its edges) is `NarrowsToPoint`. Tapers with one axis, hinge and
  slopes share one motion.
- The groups go to `move_each` as `Motion`s, so neighbours extend or trim as they do for a move and
  every `MoveError` (wrapped as `DraftError::Move`) and name keeping are a move's. A tangent chain of
  planes and coaxial round faces (a slot or rounded outline) stays tangent: a tilted plane and a
  tapered cylinder tangent along a generator meet along the same generator after the draft. A draft
  steep enough to fold the body fails there rather than building a bad solid. Splitting at a
  parting line is the document's (`split_faces` first, `document.md`).

# Replacing faces (`shell/replace.rs`)

- `replace_faces(solid, faces, target)` puts each chosen flat face onto the target plane, keeping
  the side it faces: the target's normal is taken signed to agree with the face's outward normal,
  and the face turns about the line where its plane meets the target (by the angle between the
  normals, about `normal × target`) or, parallel, shifts along it, then a residual shift along the
  target's normal lands it exactly. Faces sharing a plane share one motion; the groups go to
  `move_each`, so neighbours extend or trim as they do for a move and every `MoveError` (wrapped as
  `ReplaceError::Move`) and name keeping are a move's.
- Refusals before building: no face `NothingToReplace`, a face not of the solid `MissingFace`, a
  curved face `NotFlat` (curved targets and faces are roadmap entries), a target leaning
  `STEEPEST_REPLACING_TURN` (89°) or more from a face `SquareToTarget`, a target on or past the far
  side of the body from a face (every sampled point of the edges not bounding it on or outside the
  target) `BeyondBody`, a replaced face meeting along an edge a face of another group or an unmoved
  face already on the target `Merging` (the two would become one face, which needs a topology
  change), every face already on the target `AlreadyOnTarget`. A face listed twice is taken once.
- A face beside the replaced one squeezed to nothing between it and the target (levelling a step
  with its riser standing) fails in the move as an edge collapsing; merging the faces either side is
  a roadmap entry.

# Delete faces (`shell/delete.rs`)

- `delete_faces(solid, faces)` removes faces and closes the gap by extending the faces around them:
  valid or a `DeleteError` (`NothingToDelete`, `MissingFace`, `Everything`, `CannotClose` naming the
  chosen face whose gap stays open, `Vanishes` for a kept face the closing swallows, `Invalid`,
  `Crosses`, `Cancelled`). Errors name faces of the input solid. The chosen faces are taken apart
  into components joined across their shared edges, each closed one of two ways:
  - A cap: every loop of a kept face touching the component borders only the component and is not
    that face's outer loop (a through or blind hole with its floor, a pocket, a boss with its top, a
    void shell). The component's faces and those loops are left out of a copy of the solid
    (`Capped::removed`, settled pcurves, names and origins kept), so the face the hole was cut into
    or the boss stood on closes over it.
  - A band (anything else, each face of one loop): the shell's inner solid at offset zero
    (`Offsets::still`, reach the diagonal of the deleted faces' edges) with each deleted face a
    forced collapse (`inner::healed_solid`, `Collapses::deleting`): its `across` edges vanish,
    merging their ends into vertices solved on the kept faces around them, and its two sides close
    into one edge between the faces beyond them (the shell's bands and ridges). A deleted face is
    never held to lie inside the closing as a shrinking face is (`Collapses::closes_over` skips it),
    since the gap may close outward (a convex fillet) or inward (a concave one).
- Which edges vanish is chosen per face (`candidates`): pairs of non-adjacent edges of its loop
  whose two sides lie beyond kept faces other than itself that are not parallel planes, the
  shortest across first (a fillet's or chamfer's ends), then every edge (a corner patch closing to a
  point, a chamfer's triangle or a fillet's sphere, whatever lies beyond). Combinations are tried in order, at most `MAX_TRIALS`, the first
  that builds without a kept face vanishing wins; when several faces fail together each is tried
  alone to name the culprit. Kept faces keep their names and origins (`Naming::Kept`), edges their
  names, new edges `between` their faces; `find_crossing` then refuses faces crossing.
- So a fillet or chamfer goes back to its sharp edge (a rim of fillets or mitred chamfers at once,
  every blend of a fully rounded or bevelled box with its corner patches),
  a hole's rim chamfer or round goes leaving the hole, a torus rim fillet of a drum goes, and a
  pocket's floor fillets go. One face of a tangent chain whose ends meet other blends, a face whose
  neighbours run parallel (a box's top), a hole's wall without its floor and a boss's side without
  its top are `CannotClose`.

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

# Coils (`coil/`)

- `coil(CoilSpec, feature)` winds a section around an axis: valid or a `CoilError`. A `CoilSpec`
  holds the frame (origin on the axis where the coil starts, normal along it, x axis where the
  angle is zero), the radius, `CoilShape` (`Round` of a diameter, `Square` of a width), the size,
  `CoilPlacement` (`Inside`, `Centred`, `Outside`: the section's middle at the radius less, at or
  plus half the size), the pitch, the turns, a taper angle (radius growing by `tan(taper)` per unit
  of rise; positive widens) and the kernel's `Handedness` (`Right` turns counter-clockwise about
  the frame normal). The section lies in the meridian plane and rests on the frame's plane at the
  start (its middle half a size above it), so the body spans the height plus the size.
- The solid is built directly as a `Plan` (`faces.rs`), the groove's construction generalised:
  pieces of at most one turn (`ceil(turns)` equal pieces, `SPANS_PER_TURN` spans each per turn), the
  section four pieces (a square's sides, a circle's quarters as rational quadratic arcs). Each side
  face is the exact tensor product of the helix's rational quadratic net (`helix_knots`, shared
  with threads) and the section piece's net: control point `(i, j)` lies at the helix angle of
  column `i`, its radial distance the column's stretch times the section point's radius plus the
  taper's radial growth there, its height the column's rise plus the section point's height, with
  weight `w_i v_j`. So every helix edge and every section edge at a piece boundary is exactly a
  boundary row or column of its faces (straight pcurves). Without a taper a square's upright sides
  are exact `Cylinder`s with fitted pcurves, as the groove's root and far face are. Caps are
  meridian planes. With a taper the radial part of a column is a conical spiral approximated the
  way the helix's rise is (exact at span ends and middles).
- Faces are `FaceName::coil(feature, part)` with `FaceOrigin::Side { feature, entity: part }`:
  `COIL_PARTS.side(turn, piece)` is `piece + 1 + 8 * turn` (`CoilFace::of_entity`: `Bottom`,
  `Outside`, `Top`, `Inside` from the section's bottom round counter-clockwise in the meridian
  plane; a round section's quarters start at its lowest point), the start and end caps 5 and 6.
- Refusals before building: non-finite values, no radius, size or pitch, fewer than
  `LEAST_COIL_TURNS` or more than `MAX_COIL_TURNS` turns, a taper at or past
  `MAX_COIL_TAPER_DEGREES`, a reach past `MAX_SIZE`, the section reaching the axis at either end
  (`ReachesAxis`), and turns closer than the section plus `LEAST_GAP` of it (`TurnsOverlap` with
  the least pitch: a square's turns apart axially or radially, a round's by the distance between
  their centres), so the shell never touches itself.
- `helix(HelixSpec)` (`coil/helix.rs`) gives the curve a coil's section point at the radius would
  follow: one `HelixTurn` per coil piece (at most one turn each), the coil's rational quadratic
  net (`faces::Motion::helix`, shared through `Winding`) over its `SPANS_PER_TURN` spans, the taper
  carried as the coil's; `HelixTurn::path_segment` keys it and winds it about the spec's axis.
  `start_angle` turns the frame counter-clockwise about its normal before winding
  (`HelixSpec::start_frame`), whatever the hand, so it moves the start without mirroring it;
  `HelixSpec::point_at` is the analytic point an angle round from the start (the app's arrows stand
  on it). Refusals as a coil's for the values it shares (a non-finite start angle `NonFinite`), a
  taper drawing it into the axis `ReachesAxis`.
- `spiral(SpiralSpec)` (`coil/spiral.rs`) winds a flat (Archimedean) spiral in the frame's plane:
  radius from the inner `radius` growing by `pitch` a turn, `hand`, `start_angle` and pieces as a
  helix's (`HelixTurn`s winding about the frame's normal, so a sweep or pipe follows it as a
  helix). Each span is a rational quadratic through the spiral's points at its ends with the
  spiral's tangents there (the shoulder where they cross) and through the spiral's crossing of
  the line from the chord's middle to the shoulder (`crossing`, by halving), so it is G1 and close
  to fifth order; the weights are scaled from span to span (`evenly_paced`) so the parameter's
  pace is continuous too, which a sweep's stations need to converge. The spans a turn start at
  `SPANS_PER_TURN` and double, up to sixteen times as many, until every span's radial deviation at
  four samples is within `SPIRAL_TOLERANCE` (`LINEAR_RESOLUTION`), else `Unfitted`.
  `SpiralSpec::point_at`, `radius_at`, `outer_radius` and `length` (the closed form of its arc
  length) are analytic; `turns_clear_of(across)` and `least_pitch_for(across)` say whether a
  section that wide in the spiral's plane keeps its turns apart: the section square to the path
  meets the meridian stretched by `sqrt(r² + b²) / r` (b the pitch over a turn), worst at the inner
  turn and the one outside it, plus the coil's `LEAST_GAP`. Refusals: non-finite values, no
  radius or pitch, turns out of range, a reach past `MAX_SIZE`. `coil/spiral_tests.rs` holds its
  points to the spiral, its length to the closed form, and a pipe along it to a valid watertight
  solid of the volume the screw motion's flux gives.
- `CoilSpec::volume` is the analytic volume (section area times the centroid's travel round the
  axis, the mean radius with a taper); `coil/tests.rs` holds tessellated volumes to it within 1e-3,
  a groove cut into a shaft to the half disc's, and names, hands and refusals.
