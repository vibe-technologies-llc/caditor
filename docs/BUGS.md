# caditor bugs

Wrong results, silent losses and refusals of shapes and operations that should work today, each with
its reproduction or pinning test where one exists.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Kernel

- [high · hard] Booleans between the fixture solids in random placements all succeed on the survey's
  seed (`boolean::tests::random_placements_of_every_fixture`, ignored), and the failures other seeds
  (`Random::new(n)`, 7 to 19, 48, 52, 57, 76, 79, 82, 85, 160, 161, 163, 178, 214, 255, 291, 325,
  403, 405, 432, 486, 632, 724, 767, 1115, 1135, 1136, 1214, 1225, 1272, 1344, 1383, 1398, 1402,
  1483, 1827 and 2008) gave are pinned by tests and combine. With the fixes for 1827 and 2008, the
  survey's own seed 2, seeds 20 to 259 and seeds 1445 to 2500 all combine but 1963, and none
  hangs; seeds 260 to 1114 all combined when last surveyed, those from 61 to 985 before an
  earlier marching change and all of them before the fixes for 1398 and 1483, and seeds 1115 to
  1444 combined with those two fixes, and neither range was run again. Seeds beyond 2500 are
  unsurveyed. Seed 1963 fails `Unfollowable` in every operation: a sphere of radius 4 and a torus
  (6, 2) turned 2.6184948483327046 radians about (0.8850402778235873, 0.5468644663968192,
  -0.568044893273137) and moved by (-2.855303481999147e-5, -7.514472743545222e-5,
  -3.223061658147419e-5). The sphere lies along the torus's inner equator, of the same radius, all
  round, its centre 8.6e-5 along the torus's axis and 1.3e-5 across it, so the surfaces overlap by
  at most 1.3e-5 in a lens about 0.006 wide along half the equator, too far off coaxial for
  `coaxial.rs`. A section seeded at one end of the lens, 90 degrees round the equator from its
  deepest point, where the normals turn parallel, collapses on its first step (6.5e-9) with no
  boundary ahead.
  Tori that nearly coincide are surveyed only down to turns of 1e-5 radians: a torus and its
  copy turned 1e-4 about nine chosen axes, X and Y, and 1e-5 about six more,
  combine (the four `a_torus_and_its_copy_turned_…_combine` tests), as
  they do about all ten axes `Random::new(7)` draws (components in -1 to 1) at 1e-4 and the next
  ten at 1e-5. At 1e-6, seven of the ten axes `Random::new(11)` draws fail (`Split`, `Open`), and
  the other three combine into wrong volumes: their union plus intersection falls 4.6e-5 to 6.8e-5
  short of both tori, the union ending about 1e-7 beyond one torus where the difference says 3e-5,
  which the turned tori test's bound (1e-6 of a torus) does not see. Sines there stay below
  `STOP_SINE` nearly everywhere, so most sections are never seeded (`TOUCH_SINE`); three of the
  seven (the second, third and seventh axes) combined into such wrong volumes before branches
  stalling far from a touch marched on to `FAR_STOP_SINE`, and now fail `Split` beside the
  u seam on the inner equator, at (4, 0, ±1e-6).
  A torus and its copy shifted 1e-5 along X or Y combine
  (`a_torus_and_its_copy_shifted_a_hundred_thousandth_along_x_combine_exactly`, `_along_y_combine`),
  as they do along Z and along all three axes at once, and along Y by 3e-5 and 1e-3; by 2e-5 along Y
  they are `Split` in every operation at (-0.083, 5.9994, 2), on the top circle. Shifts of 1e-4 and
  3e-6 along Y did not finish all three operations within a minute, so are unjudged.
  Lowering `STOP_SINE` itself to 1e-7 or 1e-8 fails
  the X shift again (`Invalid`: an edge used twice at one place in a face's domain) or a 1e-5 turn
  (`Split`), and polishing near-tangent contacts on one surface by the other's signed distance
  changed none of these.
- [medium · hard] Near-tangent booleans can turn on the last bit of a result. The kernel and the
  geometry crate now take their maths from `libm`, so Linux and Windows agree bit for bit, but a
  plug 2e-6 off its bore's axis at 45 degrees and 3e-6 wider still fails as `Open` at the plate's
  start face in the intersection (it combined with the C library's results moved one ulp up or at
  random), and the turned tori tests (`a_torus_and_its_turned_copies_combine`)
  fail `Open` with every result moved one ulp up or down, so such tori still turn on the last
  bit. The combine tests of a failing place cut a plug 1.5e-6 off at 112.5 degrees and 1e-6 wider
  from the plate, its wall 5e-7 inside the bore's over a wide arc on the near side (`Split` at the
  plate's start face), whose outcome survived every result moved one ulp up, down or at random; the
  plug they used before (8.03e-6 off, 7.76e-6 wider, its rim dipping 2.7e-7 through the bore's wall)
  combines now that such a dip is two crossings. Plugs 8e-6 to 2e-5 off with their wall dipping
  2.7e-7 to 9e-7 inside the bore's failed `Split` in every operation at 9.6 to 80 degrees; they now
  combine, but at 45 degrees their intersection is `NonManifold` at mid-height, as it already was for
  plugs 5e-6 off at every angle and for walls just clear of the bore's. A
  combine failing `NonManifold` names no place, whatever the reason, since
  `CombineCause::Edgewise` drops the kernel's `BooleanSite`.
- [medium · hard] Shell cannot split a corner whose offsets do not meet when its convex and concave
  edges alternate (two ridges of different slopes crossing) or one convex edge meets concave ones (a
  cavity whose ridge runs over its inside corner): the offset there joins faces the body keeps
  apart, or runs an edge between another pair of faces, which splitting the corner into several
  cannot give. Corners of more than `MAX_SPLIT_FACES` (24) faces are not split either. All are
  refused as `Corner` (`shell::tests::a_corner_where_ridges_and_valleys_alternate_is_named`,
  `a_cavity_whose_ridge_runs_over_its_inside_corner_is_named`). In the cavity, roofs sloping 3/5
  and 3/7 put the offset ridge 0.076 of the thickness to the front of the wall below it, so the
  ridge runs on straight past the corner and that wall's top edge now meets the back roof: the
  edge between the front roof and the wall has to become a ridge piece plus a new edge between the
  wall and the back roof. A mirrored `ConvexButFold` (concave but one convex edge, residual signs
  flipped) was tried and finds no triangulation of that cycle, as expected. Between the crossing
  ridges, the lower one stops short on the higher roof, and the two faces of that roof's plane
  that the lower roof kept apart (touching only at the vertex) become one face. Both need the
  corner rebuilt from the arrangement of its offset planes under its own union and intersection
  formula (convex edges intersect, concave ones unite), with an edge allowed to change its pair of
  faces and faces of one plane allowed to merge; the plan builds one face per original face.
- [low · hard] Fills that use up a whole face lose the edges on it: a 4 mm pocket's twelve edges
  chamfered 2 mm (`fills_using_up_a_pocket_s_walls_lose_its_rim`) is `Lost` for the rim, since the
  corner edges' bevels meet exactly across each wall and take it away; cutting first instead leaves
  the corner edges ending at the rim chamfers' mitre, the concave mirror of a boss's upright, which
  `carried_round` handles only for concave edges after convex ones. The rim would have to be
  carried onto the edges between the top and the corner bevels.
- [medium · hard] An edge ending at a corner earlier fillets made meets them along a crease, or is
  refused. The uprights of a 40×30×20 box whose top edges were filleted 2 mm take any fillet or
  chamfer, but only a 2 mm fillet ends in a smooth ball corner (`corner::after_blends`); a 1 mm one
  meets the top fillets along their intersection, a crease of about 50° where the three faces meet
  (`out_through_blends`), where other modellers roll the ball on along the seam between the top
  fillets (a pipe surface pinched to a point at the top face; the seam is smooth there, so
  `blend_chain` never follows it and the lofted path cannot end on it). The seam, the two
  cylinders' intersection in the bisecting plane, leaves the upright's top vertex along the
  upright (so the chain would continue tangentially) and turns from 90° there to tangent at the
  top face; the missing pieces are a sharpness test that does not judge the seam by its smooth end
  and a lofted run whose sections narrow to nothing at its end. A pocket's corners over its
  rounded floor likewise crease at any size but the floor's radius, where they end in a concave
  ball corner (`a_pocket_s_corners_filleted_as_its_floor_was_end_in_the_ball_corners_of_a_pocket_filleted_at_once`).
  The upright of a boss whose foot an earlier fillet rounded is still `UnsupportedEnd` at
  its foot, the refusal naming the earlier blend and saying to move the feature above it
  (`an_upright_of_a_boss_whose_foot_was_rounded_is_refused_at_its_foot`,
  `an_upright_rising_from_a_foot_an_earlier_blend_rounded_names_it_and_says_to_move_above_it`):
  the foot's fills meet in a concave seam under a convex edge, and extending the edge's tool
  down would cut into the plate.
- [medium · hard] The top rim of a box tapered to widen upwards, whose slanted corner edges were
  filleted barely larger than the rim fillet, is refused as `TooLarge`: under a 2 mm rim fillet a
  40×40×30 box tapered -4° needs corners of 2.5 mm (2.4 mm is refused), at -8° 2.7 mm (2.5 mm is
  refused), at -2° 2.4 mm (2.2 mm is refused)
  (`a_rim_fillet_whose_ball_pivots_inside_the_corners_of_a_box_widening_upwards_is_too_large`).
  The ball's side contact lies below the top, where the leaning corner cylinder's axis has moved
  in by about the drop times the lean, so the ball centre, `R − r` from that axis, stands almost
  over the rim arc's centre of curvature: the top contact barely moves (or turns back) round the
  corner, `Reach::unfolded` cuts the chord's overhang beyond it below `LEAST_OVERHANG`, and the
  lofted tool's sections would cross there. Rounding these needs a tool whose sections are not
  fanned about the rim's curvature axis, or the fillet surface trimmed where it folds.
- [low · medium] Two edges alike in every other part of their names are told apart by
  `occurrence` (`naming/mod.rs`), their midpoints rounded to a 1e-4 grid and ordered by x, then y,
  then z: a midpoint within an ulp of a cell boundary can round either way after an upstream edit,
  and turning the sketch plane changes which world axis orders them, so a fillet's reference can
  resolve to the other edge without `Ambiguous` (`kernel-naming.md` calls the grid coarse enough
  never to swap them). Unreproduced; an order from the feature itself (the profile entity, the
  parameter along the generating curve) would not depend on placement.
- [low · hard] `hidden_lines` draws edges and silhouettes only, so two bodies overlapping without a
  boolean show no line where their surfaces cross (each hides its part inside the other, and the
  stretches are right). A traced silhouette on a revolution or spline face misses a closed loop
  smaller than a cell of its grid (`hidden_lines/trace.rs`, `MAX_CELLS` a side), and a contour
  folding in the gap between two samples of a bounding edge can run past the edge there
  (`roots_among` brackets by sign changes at sampled points).

## Sketch solver

- [medium · hard] A sketch with a closed centripetal spline tangent to a line at one of its fit
  points held on that line moves by 0.14 mm when solved again from its own result (seed 306204 of
  `a_solved_sketch_does_not_move_when_solved_again`, kept as the ignored
  `a_closed_spline_tangent_to_a_line_at_a_fit_point_held_on_it_stays_put_when_solved_again`). The
  spline passes through a point coincident with the origin and is tangent to the vertical axis, so
  the contact falls on that fit point and the tangency is a double root: at the solution the
  contact's row is the sum of the point's interpolation row and its coincidence. Each spacing round
  starts a few 1e-7 mm off, Gauss-Newton stops with a residual of 2e-7 shared among those three
  rows, and the retry perturbed by 1e-3 lands 0.01 mm away. Holding the tangent at the fit point
  itself, as a spline's end on a line already is (`Joints::spline_end_on`), would remove the double
  root, but needs the origin counted as a point on the axes and a form at a fixed parameter. The
  spline also passes one point twice; the cause found does not involve it.
- [low · hard] A point held on an open centripetal fit spline through three points, whose tangent
  nearly vanishes at its start, moves by 2.6e-3 mm when solved again from its own result (seed
  55494 of `a_solved_sketch_does_not_move_when_solved_again`, beyond the cases the stress job runs,
  kept as the ignored
  `a_point_held_on_an_open_centripetal_spline_through_three_points_stays_put_when_solved_again`):
  the first solve holds the point at parameter 0.6994 and the second starts it at the sample
  0.6875, so the point slides along the curve and drags the fit points with it. The cause was not
  worked out further.
- [low · hard] A sketch with a closed degree-one spline (a polygon of its control points) and a
  `Distance` dimension from it to another curve moves by 0.4 mm when solved again from its own
  result (seed 1555 of `a_solved_sketch_of_splines_of_chosen_degrees_does_not_move_when_solved_again`
  with `regraded` allowed degree one for closed splines, which it no longer does: `regraded`
  gives a closed spline at least degree two). The cause was not worked out.
- [low · hard] An open centripetal fit spline through three coincident points and a fourth, held a
  distance from a line, still moves by up to 1e-4 mm when solved again from its own result for
  about one rounding in fifty of its inputs by up to three ulps
  (`a_spline_with_three_of_four_points_coincident_stays_within_a_micrometre_however_its_inputs_round`
  holds it within a micrometre over 64 roundings). The spline is a straight segment, the dimension
  sets the line parallel to it and every point of it is a stationary point of the distance. The
  hidden controls beside the coincident points weigh about 1e-3 in their interpolation rows, so the
  end tangent there is known only to about 1e-6 rad: the first solve holds the contact at that end
  with the line parallel to it, the far end comes nearer by up to `DISTANCE_SETTLING` of the value,
  which the re-measurement accepts, and the second solve starts its contact there. In the worst
  cases the first solve also stops with the interpolation rows at the tolerance (1.5e-10), which
  leaves the solved curve 1e-7 mm from the drawn one.

## Sketching and expressions

- [low · medium] Two splines sharing a stretch shorter than two sample steps of both (about 1/128
  of each) are still searched as crossings by trim and break (`intersect::spline_spline`), so they
  are cut where their polylines cross: two cubics with equal knots sharing the four control points
  of one short span, and parting from it without crossing, are not cut there at all. A stretch
  long on either spline, such as a short copy of a piece of a long one, is met at its ends.
- [low · medium] A sketch's entity adders (`add_point` and the other infallible ones) still count
  into the unstorable range once the id counter reaches `FIRST_UNSTORABLE_ID`: only
  `add_constraint`, `add_text` and `add_parametric_gear` refuse as `SketchError::IdsExhausted`, and
  an entity at a reserved id is refused only when the document applies it (`ReservedId`). Making
  the entity adders fallible touches about 1,500 call sites.

## STEP import

- [high · hard] A body whose faces meet only within the file's declared precision (CATIA and
  Autodesk exports whose tangent fillet splines sit micrometres apart) is bent where it can be
  (`step-read.md`, "Bent faces") and otherwise imports as flat facets. On a CATIA V5 export of 11
  bodies at 0.01 mm, bending makes one of the seven loose bodies exact. Two bend every side but
  leave one traced edge off its face at validation (3.6e-6 and 8.2e-5 mm); one fails tracing an
  end-to-end join of two fillets slanted on both (2.6e-3 rad apart); one has a side that is a pole
  or seam; one bends a face whose boundary then crosses itself in uv. In the VZ330 assembly 63
  bodies import as facets. The bent build refuses them for these reasons:
  - 25 have a corner of faces with no common point within the resolution. "Pulley (1)", "Pulley
    (2)", "Pulley (1) (1)", "Pulley(Mirror) (1)" and "X motor mount part A" and its mirror (3.4e-6
    mm) join two equal tooth-root cylinders 9.6e-6 mm apart, each exactly tangent to its own fillet,
    so snapping holds both (`round_faces`); moving one needs its fillet and the flank beyond to
    follow, or a rigid face turned into a spline it can bend. "Part 9 (1)" to "Part 14 (1)" miss a
    spline they lie on no line of by 2e-5 to 5e-5 mm; the Raspberry Pi connectors, "top (1)" and
    "powerswitch bracket" miss planes and cylinders by 1e-4 to 5e-4 mm.
  - 13 keep a traced edge that misses its vertex, tracing having failed across a near tangency:
    "gantry bottom" and "CPAP duct 15mm" (0.006 mm), "bottom (1)" (2.5e-5 mm).
  - 5 (the knobs) fail validation by 2e-6 to 2e-4 mm after every side bends.
  - 7 of the "PCB (1)" and "Blades (1)" splines have a target that jumps along a side (up to 0.0035
    mm) or a corner off the face it bends onto, as has "CPAP duct 18mm (1)" within 3.8e-7 mm.
  - 4 ("front handle1", "front handle2", "X motor mount part B (top)" and its mirror) have a vertex
    whose common point lies farther along the tangency than the precision.
  - "Micro Switch" and its copy settle the two vertices of each of four edges onto one point;
    merging them (`step-read.md`) leaves a face whose boundary encloses no area, which would have
    to be left out with them. "Lead Screw lower bracket" has two edges overlapping along a side.
- [medium · hard] The VZ330 assembly (an Autodesk export at 0.01 mm, 3,982 copies of 1,204
  bodies) imports every body, but "front handle1", "Component25", "Lead Screw lower bracket" and
  "goliath heatsink fan duct (1)" only as flat facets; each needs the bent build (entry above) to
  hold where the exact build cannot. Reproduce by reading the file with `STEP_CORPUS` in release
  (about 90 s).
  - "front handle1": the exact build leaves a vertex 0.00003 mm off its edge where faces meet
    tangentially further apart than the resolution; bending would move a face past the precision.
  - "Component25" (a thread): the exact build leaves a vertex 0.00008 mm off a helical edge it
    cannot follow; bent, a corner lies 0.0005 mm off the faces bent to meet it.
  - "Lead Screw lower bracket": a triangular spline corner patch 0.003 mm across, one side a pole,
    meets the large spline beside it nearly tangentially along a circle edge. Settling slides the
    circle's two vertices 0.00037 and 0.0003 mm along that tangency, to 0.0001 mm apart, and the
    trace between them fails; extended, the 0.057 mm edge of a thin strip beside it is traced 0.34
    mm long, zigzagging, and its pcurve needs more than 65,536 samples; bent, two of the patch's
    edges overlap along one side.
  - "goliath heatsink fan duct (1)": a spline and a cylinder 2e-4 rad from tangent lie 2.7e-6 mm
    apart where they meet a plane, so that corner settles 1.4e-6 mm off both, beyond the
    0.75e-6 mm the trace of its plane edge pins an end within, and the edge ends 1.0e-6 mm short;
    bent, three splines at another corner have no common point within 0.0006 mm.
- [low · hard] A face whose surface cannot be read is left out (`step-read.md`, "Unreadable
  faces"), but from the outer shell that turns the whole body into flat facets, since the kernel
  holds only closed solids, and a lost face with holes is not closed at all; an edge whose curve
  cannot be read still loses the whole body.

## Viewer

- [low · medium · blocked by: wgpu's GL backend] On GL and other devices without texture view
  formats the multisample resolve still averages in gamma space. A resolve of its own (a pass
  reading the samples through a `texture_multisampled_2d` and averaging them in linear light) was
  tried: it matches the view-format resolve on Vulkan, but wgpu 30's GL backend binds a
  multisampled texture as `TEXTURE_2D` (`gles::Texture::get_info_from_desc` never chooses
  `TEXTURE_2D_MULTISAMPLE`), so every sample reads as zero there and the frame comes out black. It
  needs that fixed in wgpu, or a GL-only blit resolve into an sRGB texture.
