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
  result: a spline with four coincident control points, tangent to a zero-size arc on one of them,
  with a zero distance from that arc to the spline's first point. `sketch_solve` finds such cases
  within minutes once it requires `solve_from` of a solved geometry to succeed; it does not yet (it
  discards that result), so the property is unchecked.
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
- [medium · medium] The VZ330 assembly (an Autodesk export at 0.01 mm, 3,894 copies) leaves 14
  parts out. Thirteen are refused as "a vertex 0.00001 to 0.0015 mm away from the end of its
  edge", the exact build's refusal at faces meeting tangentially further apart than the
  resolution, where the faceted fallback then fails on its own: in "Part 9 (1)" to "Part 14 (1)" a
  corner patch's edge curve ends 0.011 to 0.017 mm from its vertex (its x mirrored), in
  "Component25" and "M3 30mm Adjustement screw (1)" an edge lies 2.3 and 3.8 mm off a face,
  "Goliath Air cooled" meshes into a face boundary crossing itself in uv, "front handle1" and
  "front handle2" leave an edge of no length, "User Library-USB - C" has faces 0.011 mm apart and
  "Goliath long duct" keeps a vertex 0.012 mm off its edge. "Lead Screw lower bracket" has "an
  edge that could not be followed across its face": a thin spline strip meets its neighbours
  nearly tangentially, and tracing its edges across them zigzags (a 0.057 mm edge traced 0.34 mm
  long, whose pcurve then needs more than 65,536 samples). Refusing such traces (longer than the
  file's curve by more than the precision) only moves the refusal on: settling two vertices
  0.003 mm apart slides each about 0.002 mm along the tangency, their order along the 1 mm circle
  between them flips, and that edge runs almost a full turn and misses its vertex by 0.0004 mm.
  Settling should keep a vertex's order along the edges it ends, or move it no farther than the
  precision, as the faceted build does. Four bodies refused before for other reasons import only
  as facets: "M3 Nut" (three faces meeting so nearly tangentially that their common point lies
  0.0115 mm along, on the next vertex, so the ellipse edge between them has no length; bending
  gets within 2e-6 mm), "Top plate MGN9H (1)" and "Top plate MGN9C (1)" (a helical thread face
  whose boundary backtracks at a junction in uv, so it crosses itself), and "goliath heatsink fan
  duct (1)" (the lead screw bracket's tangent strip). Reproduce by reading the file with
  `read_step_copies` in release (about 80 s); `STEP_CORPUS` reads through `read_step`, which
  places only the first 1000 copies.
- [low · hard] A face whose surface cannot be read is left out (`step-read.md`, "Unreadable
  faces"), but from the outer shell that turns the whole body into flat facets, since the kernel
  holds only closed solids, and a lost face with holes is not closed at all; an edge whose curve
  cannot be read still loses the whole body. An offset that folds anywhere in its basis's domain
  is refused even where the face's own region is clear: fit over the region the face uses.

## Interface

- [medium · medium] The angle arrows of a revolve (`turn_handles.rs`) cannot be dragged when the
  view is square to the sketch, which is edge-on to their plane: `Ray::intersect_plane` returns
  `None`, `TurnDrag::begin` refuses, and nothing gates the arrows off as the move rings are. Near
  edge-on in perspective the angle is wildly sensitive. `ValueHandles` `Follow::Turn` has the same
  weakness. Typing the angle or orbiting away works; an angle taken from the pointer's motion round
  the axis on screen would work edge-on.
- [medium · medium] The end-on gate for arrows (`reach_handles.rs`, `value_handles.rs`,
  `handle_snap.rs`, `move_manipulator.rs`) compares the arrow's direction with the camera's
  `forward`, not with the ray from the eye to the arrow, so in perspective an off-centre arrow
  pointing at the eye passes the gate while `Ray::closest_along_line` divides by an `apart` near
  zero and the dragged value amplifies the pointer's motion many times over. Gating on the eye ray,
  or dragging in the plane facing the camera through the arrow, bounds it.
- [low · medium] Undoing past a sketch's entry mark makes Cancel sketch refuse with a warning
  (`changes_since` returns `CancelRefusal::HistoryChanged`, `editing.rs`), since the undo stack is
  the document's; Redo back to the mark restores it, and Finish, Escape, Undo and Redo keep working.
  Scoping Undo to the session, or a Cancel that says to redo first, would save the surprise.

## Viewer

- [low · medium · blocked by: wgpu's GL backend] On GL and other devices without texture view
  formats the multisample resolve still averages in gamma space. A resolve of its own (a pass
  reading the samples through a `texture_multisampled_2d` and averaging them in linear light) was
  tried: it matches the view-format resolve on Vulkan, but wgpu 30's GL backend binds a
  multisampled texture as `TEXTURE_2D` (`gles::Texture::get_info_from_desc` never chooses
  `TEXTURE_2D_MULTISAMPLE`), so every sample reads as zero there and the frame comes out black. It
  needs that fixed in wgpu, or a GL-only blit resolve into an sRGB texture.
