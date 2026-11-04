# caditor bugs

Wrong results, silent losses and refusals of shapes and operations that should work today, each with
its reproduction or pinning test where one exists.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Kernel

- [high · hard] Booleans between the fixture solids in random placements all succeed on the survey's
  seed (`boolean::tests::random_placements_of_every_fixture`, ignored), and the failures other seeds
  (`Random::new(n)`, 7 to 19, 48, 52, 57, 76, 79, 82, 85, 160, 161, 163, 178, 214, 255, 291, 325,
  403, 405, 432 and 486) gave are pinned by tests and combine. Seeds 20 to 700 all combine but
  632: a frustum (4, 2, 5) and its copy turned about (0.0731, -0.3603, -0.8102) by 5.2258 and moved
  by (0.0303, 0.0442, -0.0480) fail every operation as `Unrefined` (`Unsolved`) near
  (-1.9899, 2.8863, 1.2357), where the walls cross at sines of 2.7e-4 to 4e-4: the traced nodes
  there alternate in heading (neighbours 3e-6 apart with opposite tangents), as if the march
  stepped between two arms passing within a step of each other, and the Hermite segment over that
  stretch has no point on both surfaces across its middle. It fails the same without the march
  going on past far stalls. Seeds beyond 700 are unsurveyed.
  Tori that nearly coincide are surveyed only down to turns of 1e-5 radians: a torus and its
  copy turned 1e-4 about nine chosen axes, X and Y, and 1e-5 about six more,
  combine (`a_torus_and_its_copy_turned_a_hundred_thousandth_of_a_radian_or_more_combine`), as
  they do about all ten axes `Random::new(7)` draws (components in -1 to 1) at 1e-4 and the next
  ten at 1e-5. At 1e-6, seven of the ten axes `Random::new(11)` draws fail (`Split`, `Open`), and
  the other three combine into wrong volumes: their union plus intersection falls 4.6e-5 to 6.8e-5
  short of both tori, the union ending about 1e-7 beyond one torus where the difference says 3e-5,
  which the turned tori test's bound (1e-6 of a torus) does not see. Sines there stay below
  `STOP_SINE` nearly everywhere, so most sections are never seeded (`TOUCH_SINE`); three of the
  seven (the second, third and seventh axes) combined into such wrong volumes before branches
  stalling far from a touch marched on to `FAR_STOP_SINE`, and now fail `Split` beside the
  u seam on the inner equator, at (4, 0, ±1e-6).
  A torus and its copy shifted 1e-5 along X combine exactly
  (`a_torus_and_its_copy_shifted_a_hundred_thousandth_along_x_combine_exactly`); along Y they are
  `Split` in every operation at (-8, 5e-6, 0), where the section in the plane y = 5e-6 crosses
  both outer equator seams, which lie within the resolution of each other for about 0.8 either
  side, the section also running 5e-6 from both tori's meridian seams at x = 8. Along Z, and
  along all three axes at once, they combine. Lowering `STOP_SINE` itself to 1e-7 or 1e-8 fails
  the X shift again (`Invalid`: an edge used twice at one place in a face's domain) or a 1e-5 turn
  (`Split`), and polishing near-tangent contacts on one surface by the other's signed distance
  changed none of these.
- [medium · hard] Near-tangent booleans can turn on the last bit of the C library's `sin`, `cos`
  and `atan2`, which glibc and the Windows UCRT round differently: a plug 2e-6 off its bore's axis
  at 247.5 degrees and 1e-6 narrower fails as `Open` on glibc (the bore's rim at the start face is
  used by the plate's side and start face and the peg's start face but not the peg's side) and as
  edge contact on Windows and whenever every transcendental result is moved one ulp either way.
  The combine test of a failing place now uses a plug 2.5e-6 off at 292.5 degrees (`Split` at the
  plate's start face), whose outcome survived the pure-Rust `libm` and every result moved one ulp
  up, down or at random. Taking the kernel's maths from `libm` would make Linux and Windows agree
  bit for bit, but with it `boolean::tests::a_marched_boolean_cancelled_anywhere_stops_with_cancelled`
  fails: the union of a radius-1 cylinder and a radius-1.5 sphere centred on its wall is
  `Unfollowable` once `cos` rounds one ulp differently, a marching fragility to fix first.
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

## Sketch solver

- [medium · hard] A sketch with a closed centripetal spline through one point twice can move when
  solved again from its own result, by up to 0.6 mm, or be refused: the spacing rounds do not
  settle within `SPACING_ROUNDS` (seeds 269470 and 306204, a distance to or a tangent with such a
  spline), the result is left a few micrometres off its own layout (173746), or the second solve
  reports a conflict (257788, `a_solved_sketch_solves_again_from_its_own_result`). The knots near
  the repeated point sit `SHORTEST_GAP_SHARE` apart, where the curve swings with the smallest move
  of its fit points; `a_solved_sketch_does_not_move_when_solved_again` skips these splines.

- [medium · hard] An open fit spline through three coincident points and a fourth, held a distance
  from a line (`a_spline_with_three_of_four_points_coincident_solves_again_from_its_own_result`),
  moves by 0.05 mm when solved again from its own result for about one rounding in ten of an
  input by up to three ulps (the Windows CI run fails it where Linux passes). The hidden controls
  beside the coincident points sit 1e-6 mm apart, so the end tangent's direction is finer than a
  coordinate of 10 mm can hold: the first attempt stalls at a residual of 2e-9 against a tolerance
  of 1.6e-9 and a perturbed retry lands elsewhere. Weighting the tangent form by the tangent's
  length, or the interpolation rows by 1e3, each removed the jump and left a drift of 2e-6 mm or
  moved it to the closed spline; neither is kept.

## STEP import

- [high · hard] A body whose faces meet only within the file's declared precision (CATIA and
  Autodesk exports whose tangent fillet splines sit micrometres apart) is bent where it can be
  (`step-read.md`, "Bent faces") and otherwise imports as flat facets. On a CATIA V5 export of 11
  bodies at 0.01 mm, bending makes one of the seven loose bodies exact. Two bend every side but
  leave one traced edge off its face at validation (3.6e-6 and 8.2e-5 mm); one fails tracing an
  end-to-end join of two fillets slanted on both (2.6e-3 rad apart); one has a side that is a pole
  or seam; one has an iso-line edge whose neighbours' feet fall on both sides of it within the
  file's noise (the bound test needs a tolerance from the precision rather than the domain); one
  bends a face whose boundary then crosses itself in uv. The bent path's one STEP test bends a
  spline side onto a plane it meets across; a fixture of a spline tangent to its neighbour a few
  micrometres off is still missing, and that file now reads in 8.7 s against 7.6 s. Healing still
  traces an edge only between exactly two distinct faces, so an edge used twice by one face (a
  cylinder seam) with a vertex a few micrometres off is refused outright when the file declares no
  precision. In the VZ330 assembly 67 bodies import as facets this way, among them six of the ten
  M4 T-nuts (a thread flank whose iso-line edge has the face on both sides of it further along, as
  the flank runs out), "Body6" and its copies (a side closed around a periodic spline, then the
  same L-shaped face), "Body7" and "front handle1" (the bound test
  above), "CPAP duct 15mm" (a side bent to 2.504e-8 mm of its targets against `BEND_TOLERANCE`'s
  2.5e-8). Fixing these blockers of the bent path (closed sides of periodic splines, the bound
  test's tolerance, `BEND_TOLERANCE` at 2.5e-8 mm, thread flanks running out) would cover most of
  the 67. Two others, "Pulley (1)" (a plane 3.4e-6 mm off the cylinder it is tangent to) and
  "Micro Switch" (two pairs of parallel planes 1e-4 mm apart meeting at one corner), are what
  snapping (`step-read.md`, "Snapped faces") now makes exact in fixtures built after them; reading
  the file again should confirm they import exact.
- [medium · medium] The VZ330 assembly (an Autodesk export at 0.01 mm, 3,981 copies) leaves 1
  part out, refused as "a vertex 0.00003 to 0.0004 mm away from the end of its edge", the exact
  build's refusal at faces meeting tangentially further apart than the resolution, where the
  faceted fallback then fails on its own: "front handle1" meshes a
  sliver 1.2e-6 mm wide whose edges three triangles share (welding wider only opens more). "Lead Screw lower bracket" imports
  only as facets: exact, it has "an edge that could not be followed across its face": a thin spline strip meets its neighbours
  nearly tangentially, and tracing its edges across them zigzags (a 0.057 mm edge traced 0.34 mm
  long, whose pcurve then needs more than 65,536 samples). Refusing such traces (longer than the
  file's curve by more than the precision) only moves the refusal on: settling two vertices
  0.003 mm apart slides each about 0.002 mm along the tangency, their order along the 1 mm circle
  between them flips, and that edge runs almost a full turn and misses its vertex by 0.0004 mm.
  Settling should keep a vertex's order along the edges it ends, or move it no farther than the
  precision, as the faceted build does. Five bodies refused before for other reasons import only
  as facets: "Component25" (a thread whose helical edges its exact build cannot follow), "M3 Nut" (three faces meeting so nearly tangentially that their common point lies
  0.0115 mm along, on the next vertex, so the ellipse edge between them has no length; bending
  gets within 2e-6 mm), "Top plate MGN9H (1)" and "Top plate MGN9C (1)" (a helical thread face
  whose boundary backtracks at a junction in uv, so it crosses itself), and "goliath heatsink fan
  duct (1)" (the lead screw bracket's tangent strip). Reproduce by reading the file with
  `STEP_CORPUS` in release (about 65 s).
- [low · hard] A face whose surface cannot be read is left out (`step-read.md`, "Unreadable
  faces"), but from the outer shell that turns the whole body into flat facets, since the kernel
  holds only closed solids, and a lost face with holes is not closed at all; an edge whose curve
  cannot be read still loses the whole body. An offset that folds anywhere in its basis's domain
  is refused even where the face's own region is clear: fit over the region the face uses.

## Viewer

- [low · medium · blocked by: wgpu's GL backend] On GL and other devices without texture view
  formats the multisample resolve still averages in gamma space. A resolve of its own (a pass
  reading the samples through a `texture_multisampled_2d` and averaging them in linear light) was
  tried: it matches the view-format resolve on Vulkan, but wgpu 30's GL backend binds a
  multisampled texture as `TEXTURE_2D` (`gles::Texture::get_info_from_desc` never chooses
  `TEXTURE_2D_MULTISAMPLE`), so every sample reads as zero there and the frame comes out black. It
  needs that fixed in wgpu, or a GL-only blit resolve into an sRGB texture.
