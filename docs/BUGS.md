# caditor bugs

Wrong results, silent losses and refusals of shapes and operations that should work today, each with
its reproduction or pinning test where one exists.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Kernel

- [high · hard] Booleans between the fixture solids in random placements all succeed on the survey's
  seed (`boolean::tests::random_placements_of_every_fixture`, ignored), but other seeds
  (`Random::new(n)`) still fail. Two are refused as `Intersection(Unrefined)` because a marched
  branch's contacts leave a segment no refinement can follow: seed 7's two cones at a neck where the
  surfaces' normals are 0.004 apart (no point on both surfaces across the segment's middle, formerly
  `Open`), and seed 16's torus against the hollow cuboid round a turn of 2.5 radians within 0.2
  (formerly `EdgeOffSurface`, 2.4e-3 off); the marcher should step shorter there. The rest are
  invalid results: `SelfIntersectingBoundary` tessellating a sphere and frustum (seed 7), two
  frustums (8), an extruded spline and the hollow cuboid and a cone and cylinder (12) and two
  extruded splines (14), and a cone and the hollow cuboid with a `DanglingEdge` (9). The extruded
  spline against a torus that was `Invalid(EdgeOffSurface)` (seed 3) was a Hermite segment fitting
  its curve at its middle but not its quarters, and now combines. Tori that nearly coincide still
  fail when turned rather than shifted: a torus and its copy turned by 1e-5 to 1e-3 radians are
  `Ambiguous` or `Split`, each after about 0.7 s of seeding. Intersection curves crossing at a
  tangent point (tori touching along their equators, a face touching a torus's inner equator) cannot
  be split (`tori_touching_along_their_equators_cannot_be_split`), and a lump too thin for the
  validation mesh is refused as invalid: the difference of a torus and its copy shifted 1e-5 along
  each axis is `Invalid(VoidOutside)`, with no test pinning it.
- [medium · hard] Cylinders of one radius a few resolutions to tens of micrometres off one axis,
  moved away from both seams (seams at +x, an extruded plug 2.5e-6 to 3e-5 off its bore's axis
  along -x), cannot be joined: the union is `Invalid(LoopOrientation)`, though intersection and
  difference combine and 1e-4 joins. The failure carries no place. The tests of a failing combine's
  place use a plug 2e-6 off its bore's axis along +x and 1e-6 narrower, which fails as `Open` at
  the plate's start face.
- [medium · hard] Shell cannot split a corner whose offsets do not meet when its convex and concave
  edges alternate (two ridges of different slopes crossing) or one convex edge meets concave ones (a
  cavity whose ridge runs over its inside corner): the offset there joins faces the body keeps
  apart, or runs an edge between another pair of faces, which splitting the corner into several
  cannot give. Corners of more than `MAX_SPLIT_FACES` (24) faces are not split either. All are
  refused as `Corner` (`shell::tests::a_corner_where_ridges_and_valleys_alternate_is_named`,
  `a_cavity_whose_ridge_runs_over_its_inside_corner_is_named`).
- [medium · hard] `select::classify` lets inside or outside samples win over coincident ones in a
  partly coincident fragment (only coincident samples of opposite senses make it `Ambiguous`). A
  sample now counts as coincident only on a face of the same elementary surface, which removed the
  tangent-line noise that made every partly coincident fragment `Ambiguous` fail
  `stress_cylinders_on_a_grid`, and none occurs in the boolean tests or the aligned-contact survey;
  making a mixed fragment `Ambiguous` is unchecked against the random-placement survey, and
  splitting fragments exactly at coincident boundaries would replace it.
- [medium · hard] A box with every edge chamfered by about a third of its height, shelled open at
  the top, is still refused as `EdgeCollapses` in narrow bands of thickness although the opening
  stays open (150×150×40 chamfered 15 mm at 14.5, 20.5, 22 and 25 mm; 100×100×30 chamfered 10 mm
  near 25 mm). Each round decides which faces close from vertices solved on the topology before
  it: at 14.5 mm the vertical chamfers are taken as closing between their corner triangles when the
  triangles close and the chamfers survive as small rhombi, and at 20.5 mm the bottom chamfers
  close only once the sides above them have, which no round sees because the corners they touch
  cannot be split first. No test pins them; a sweep of chamfered boxes against the cavity's convex
  planes (as `boxes_with_triangle_corners_shell_open_and_closed_or_refuse_cleanly` builds them)
  finds them.
- [low · hard] Two faces side by side that the shell's thickness both closes up (a narrow chamfer
  cone below a narrow lid cone) are refused as `ClosesBesideClosing`: their joints would need the
  offset of the next face that survives, found across a run of dropped bands whose ridges meet
  each other, so the bands would have to be merged into one band spanning several faces, with
  their seams and joints ordered across all of them.
- [low · hard] Meshes fold where two faces meet at a very small dihedral (lens tips, a plane nearly
  tangent to a torus), giving self-overlapping triangles that `validate` does not see: an extruded
  spline intersected with a frustum leaves two tangent edges at one vertex, the end parting bisects
  them down to 1e-7, and the mesh then uses one edge twice in the same direction
  (`assert_watertight` fails on it).


## Modelling

- [low · medium] The incremental recompute suite fails at seed 1652 of
  `recomputing_after_each_step_equals_recomputing_from_scratch`: a revolve of a sketch ends
  `UpToDate` both ways but its `Healing` differs, two chosen regions matched against the regions the
  fresh pass heals, so an incremental pass and a fresh one disagree about which references the
  feature heals. The sketch may hold free degrees of freedom solved differently warm and fresh, in
  which case the comparison of `healing` should follow the one of sketch geometry; undiagnosed.
  `CADITOR_CASES` stays under 1,652 until it is, so the nightly run of 10,000 does not hold.
- [medium · hard] A concave fillet running out under a rounded rim whose fill reaches nearly to
  the rim's tangent with the top face (a 3 mm fillet on a notch floor 3.5 mm under a puck's top
  with a 3 mm rim) fails as a face that could not be divided; smaller ones and chamfers work
  (`blend::tests::a_notch_fillet_climbing_onto_a_rounded_rim_stays_inside_the_puck`).
- [medium · hard] Blend corners still refused where a blend is possible (`blend/survey.rs` chamfers
  every corner of twenty bodies): a convex edge chosen with the concave edges at its foot (a boss's
  corner edge with its base) ends after the fill where two fill faces meet and is `UnsupportedEnd`
  (`a_convex_edge_rising_from_bevelled_concave_edges_is_refused_at_its_foot`), a missing corner
  case that needs the base blend carried round the corner's own blend; and feet meeting exactly
  across a fill are refused, `TooLarge` when a rim chamfer meets a boss's skirt on the face between
  them and `Lost` when the fills use up a pocket's walls
  (`feet_meeting_exactly_across_a_fill_are_refused`), a tolerance question of whether a face
  narrowed to nothing should vanish. Fillets fail more corners than chamfers: a concave edge with
  the convex edge rising from its end (`AfterFill(TooLarge)`) and a notch's floor edge with its
  wall edges (`Boolean(Invalid(PcurveEnds))`).
- [medium · hard] An edge ending at a corner earlier fillets made meets them along a crease, or is
  refused. The uprights of a 40×30×20 box whose top edges were filleted 2 mm take any fillet or
  chamfer, but only a 2 mm fillet ends in a smooth ball corner (`corner::after_blends`); a 1 mm one
  meets the top fillets along their intersection, a crease of about 50° where the three faces meet
  (`out_through_blends`), where other modellers roll the ball on along the seam between the top
  fillets (a pipe surface pinched to a point at the top face; the seam is smooth there, so
  `blend_chain` never follows it and the lofted path cannot end on it). A pocket's corners over its
  rounded floor are creased at every size, the equal radius included (`after_blends` is convex
  only). The upright of a boss whose foot an earlier fillet rounded is still `UnsupportedEnd` at
  its foot, the refusal naming the earlier blend and saying to move the feature above it
  (`an_upright_of_a_boss_whose_foot_was_rounded_is_refused_at_its_foot`,
  `an_upright_rising_from_a_foot_an_earlier_blend_rounded_names_it_and_says_to_move_above_it`):
  the foot's fills meet in a concave seam under a convex edge, and extending the edge's tool
  down would cut into the plate.
- [medium · hard] The top rim of a box tapered to widen upwards, whose slanted corner edges were
  filleted less than about twice the rim fillet, is refused: a 40×40×30 box tapered -4° with
  3 mm corners refuses a 2 mm rim fillet as `Rounding(Untraced)` and with 2.2 mm corners as
  `Intricate` (sections meeting at a joint too far apart), and at -8° with 2.2 mm or 2.5 mm
  corners as `TooLarge`; 4 mm corners at -4° work. The ball's side contact there moves from a
  corner's cylinder onto the side plane partway along the straight rim, a switch the run
  splitting and the rounding's retraced edges do not yet follow. Tapering the other way works down
  to corners a little larger than the fillet, and corners no larger than it are refused as
  `TooLarge`.

## Sketch solver

- [medium · hard] A conflict through a chain of more than about 1,050 lines with its far end fixed
  out of reach is reported as not solving: the diagnosis's two descents outgrow `DIAGNOSIS_WORK`.
  At 1,000 lines the support's descent from the drawing takes about 225,000 units and the closest
  witness's about 219,000 (trimming 12,000), some 85% of it in the two perturbed retries (23 to 32
  steps each against 2 to 16 for the unperturbed attempt), which untangle a start scattered by a
  fraction of the whole chain's extent before Newton steps settle it.
- [medium · hard] A sketch solved from a degenerate start can fail to solve again from its own
  result: a spline with repeated control points (a closed spline of one point, three of four
  coincident), a dimension of 1e-9 mm. `a_solved_sketch_solves_again_from_its_own_result`
  (`caditor-checks`) skips splines with control points within a micrometre and holds for the first
  50,000 seeds; beyond, seed 53733 (a `Distance` of 1e-9 mm beside a `VerticalDistance` and a
  `Coincident`) is solved once and then `Unsolvable`.
- [low · hard] A constraint holding a point at the crossing of two curves that cross twice
  (`PointTarget::Crossing`) takes, on every solve, the crossing the starting geometry misses
  least, so solving a solved sketch again can pick the other crossing: seed 22192 of
  `a_solved_sketch_does_not_move_when_solved_again` (a circle and a fit spline, `VerticalPoints`
  to their crossing) moves a point by 4.9 mm on the second solve. The test holds for the first
  22,000 seeds, so the nightly run of 10,000 does not reach it.
- [low · hard] A dimension placed on entities that still have other freedom is added as a driving
  dimension although the measured value is already determined by their constraints: `determines`
  (`sketch_tools.rs`) takes a new dimension as a reference only when every entity involved is
  `FullyConstrained`. Such a dimension is reported as redundant, not as a conflict, since its value
  equals the measured one; deciding it from the freedom along the one measured direction would need
  the solver's Jacobian.

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
