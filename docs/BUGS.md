# caditor bugs

Wrong results, silent losses and refusals of shapes and operations that should work today, each with
its reproduction or pinning test where one exists.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Kernel

- [high · hard] Booleans between the fixture solids in random placements all succeed on the survey's
  seed (`boolean::tests::random_placements_of_every_fixture`, ignored), and the failures other seeds
  (`Random::new(n)`, 7 to 19, 48, 52, 57, 76, 79, 82, 85, 160, 161, 163, 178, 214, 255, 291, 325,
  403, 405, 432, 486, 632, 724, 767, 1115, 1135, 1214, 1225, 1272, 1344, 1383 and 1402) gave are
  pinned by tests and combine. Seeds 20 to 1114 all combine, those from 61 to 985 surveyed before
  the last marching change and not again. Of seeds 1115 to 1420, one still fails, as it did on the
  build before that change: 1136 (a torus and its copy turned 0.25 radians and 3e-3 off, draw 890)
  is `Invalid`, a face's boundary crossing itself in its domain. Seeds 1115 to 1420 were not
  surveyed again after these fixes, and seeds beyond 1420 are unsurveyed.
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
  A torus and its copy shifted 1e-5 along X or Y combine
  (`a_torus_and_its_copy_shifted_a_hundred_thousandth_along_x_combine_exactly`, `_along_y_combine`),
  as they do along Z and along all three axes at once, and along Y by 3e-5 and 1e-3; by 2e-5 along Y
  they are `Split` in every operation at (-0.083, 5.9994, 2), on the top circle. Shifts of 1e-4 and
  3e-6 along Y did not finish all three operations within a minute, so are unjudged.
  Lowering `STOP_SINE` itself to 1e-7 or 1e-8 fails
  the X shift again (`Invalid`: an edge used twice at one place in a face's domain) or a 1e-5 turn
  (`Split`), and polishing near-tangent contacts on one surface by the other's signed distance
  changed none of these.
- [low · medium] The exact mass of a body holding a thin lens of a sphere is off by up to about 8e-3
  in 474: `face_area` gives up (`None`) on the sphere's lens of area 0.39 that a sphere 1.6e-4 off
  centre in a torus's inner equator leaves
  (`a_sphere_a_little_off_centre_in_a_torus_inner_equator_combines`, which checks the mesh's volume
  for that reason), so `mass_properties_tessellating` meshes that face at the default tolerance and
  the difference and intersection read 473.7492 and -0.0027 where a fine mesh and the union give
  473.7410 and 2.8e-5. The areas of the torus's two pieces add up exactly.
- [medium · medium] A hole up to the next face with an offset fails where the cut back drill's
  moved copy touches the body tangentially (`move_end`): a hole stopped by a round channel whose top
  is level with the plate's top fails the move. `hole_tests.rs`'s `channelled` keeps that top below
  the plate's, so no test pins it yet; the exact error was not recorded.
- [medium · hard] Near-tangent booleans can turn on the last bit of the C library's `sin`, `cos`
  and `atan2`, which glibc and the Windows UCRT round differently: a plug 2e-6 off its bore's axis
  at 45 degrees and 3e-6 wider fails as `Open` at the plate's start face on glibc and with every
  result moved one ulp down, and combines with every result moved one ulp up or at random. The
  combine tests of a failing place use a plug 8.03e-6 off at about 9.6 degrees and 7.76e-6 wider, so
  its wall dips just inside the bore's near where they touch (`Split` at the plate's start face),
  whose outcome survived every result moved one ulp up, down or at random; the pure-Rust `libm` was
  not tried on it. A combine failing `NonManifold` names no place, whatever the reason, since
  `CombineCause::Edgewise` drops the kernel's `BooleanSite`.
  Taking the kernel's maths from `libm` would make Linux and Windows agree
  bit for bit, and `boolean::tests` all pass with it in the kernel; `glam`'s rotations in the
  geometry crate would have to follow through its `libm` feature, or a turned solid still differs.
  With every result moved one ulp up or down,
  `a_torus_and_its_copy_turned_a_hundred_thousandth_of_a_radian_or_more_combine` fails `Open`, so
  such tori still turn on the last bit.
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
- [medium · hard] Filleting 0.5 mm all 24 straight edges of the survey's boss together, 20 convex
  and 4 concave, fails validation as `PcurveEnds`
  (`blend::survey::every_corner_fillets`, ignored and left out of the stress job until it passes).
  Every other corner of the twenty surveyed bodies fillets.
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

- [medium · hard] A sketch with a closed centripetal spline through one point twice moves when
  solved again from its own result (seeds 269470, 306204 and 173746 of
  `a_solved_sketch_does_not_move_when_solved_again`, by 0.34, 0.14 and 0.004 mm; seed 257788 of
  `a_solved_sketch_solves_again_from_its_own_result` now solves again). In seed 269470 (a distance
  from an axis to a spline through four fit points, one repeated, and nothing else) no spacing
  round's solve with the fit points frozen holds, so all eight rounds and then the
  `PARAMETER_ROUNDS` re-measurements run (16 in the first solve, 6 in the second): the loop through
  the repeated point has two bulges near the dimension and each round holds one while the other
  comes nearer, as in the alternating projection `sketch-solver.md` describes, and the solve
  ends on whichever bulge the last round held, sliding the whole sketch up to 0.3 mm along the
  directions the dimension leaves free. `a_solved_sketch_does_not_move_when_solved_again` skips
  these splines.
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
  - 6 ("M3 Nut", the knobs) fail validation by 2e-6 to 2e-4 mm after every side bends.
  - 7 of the "PCB (1)" and "Blades (1)" splines have a target that jumps along a side (up to 0.0035
    mm) or a corner off the face it bends onto, as has "CPAP duct 18mm (1)" within 3.8e-7 mm.
  - 4 ("front handle1", "front handle2", "X motor mount part B (top)" and its mirror) have a vertex
    whose common point lies farther along the tangency than the precision.
  - "Micro Switch" and its copy settle two vertices onto one point (an edge of no length), and
    "Lead Screw lower bracket" has two edges overlapping along a side.
- [medium · medium] The VZ330 assembly (an Autodesk export at 0.01 mm, 3,980 copies) leaves 2
  parts out, refused as "a vertex 0.00003 to 0.00008 mm away from the end of its edge", the exact
  build's refusal at faces meeting tangentially further apart than the resolution, where the
  faceted fallback then fails on its own: "front handle1" meshes a sliver 1.2e-6 mm wide whose
  edges three triangles share (welding wider only opens more), and "Component25" (a thread whose
  helical edges its exact build cannot follow) would need 20,266 flat faces. "Lead Screw lower
  bracket" and "goliath heatsink fan duct (1)" import only as facets: exact, a thin spline strip
  meets its neighbours nearly tangentially, and tracing its edges across them zigzags (a 0.057 mm
  edge traced 0.34 mm long, whose pcurve then needs more than 65,536 samples). "M3 Nut" also imports
  only as facets (three faces meeting so nearly tangentially that their common point lies 0.0115 mm
  along, on the next vertex, so the ellipse edge between them has no length; bending gets within
  2e-6 mm). Reproduce by reading the file with `STEP_CORPUS` in release (about 65 s).
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
