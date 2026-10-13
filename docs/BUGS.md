# caditor bugs

Wrong results, silent losses and refusals of shapes and operations that should work today, each with
its reproduction or pinning test where one exists.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Kernel

- [high · hard] Booleans between the fixture solids in random placements all succeed on the
  survey's seed (`boolean::tests::random_placements_of_every_fixture`, ignored), but another seed
  still fails an extruded spline against a torus with `Invalid(EdgeOffSurface)` (an edge 1.3e-6 off
  its face). Tori that nearly coincide still fail when turned rather than shifted: a torus and its
  copy turned by 1e-5 to 1e-3 radians are `Ambiguous` or `Split`, each after about 0.7 s of
  seeding. Intersection curves crossing at a tangent point (tori touching along their equators, a
  face touching a torus's inner equator) cannot be split
  (`tori_touching_along_their_equators_cannot_be_split`), and a lump too thin for the validation
  mesh is refused as invalid: the difference of a torus and its copy shifted 1e-5 along each axis
  is `Invalid(VoidOutside)`, with no test pinning it.
- [medium · hard] Offsets within `LINEAR_RESOLUTION` compound past it: a block whose back and
  bottom are each within the resolution of a plate's faces (8.3e-7 and 6.2e-7) has its corner
  1.03e-6 off the plate's edge, so the corner is neither pooled with the edge nor apart from it,
  and about 0.4% of aligned contacts with offsets between 1e-7 and 1e-4 fail as `Open`, `Split` or
  `Invalid(VertexOffCurve)` (offsets of 1e-6 to 1e-4 alone, as in
  `boolean::tests::aligned_contacts_a_micrometre_or_so_apart`, all combine), and an extruded plug
  flush with a plate 1.1e-6 to 2e-6 off its bore's axis is `Open` (the UI test of a failing
  combine's place uses it), though 3e-6 and more combine. Pooling points within the resolution of each other
  transitively, or snapping faces within the resolution onto each other before imprinting, would
  close it; whichever is chosen, the band where offsets are snapped or refused is still to be
  documented.
- [medium · hard] Shell cannot split a corner whose offsets do not meet when its convex and concave
  edges alternate (two ridges of different slopes crossing) or one convex edge meets concave ones (a
  cavity whose ridge runs over its inside corner): the offset there joins faces the body keeps
  apart, or runs an edge between another pair of faces, which splitting the corner into several
  cannot give. Corners of more than eight faces are not split either. All are refused as `Corner`
  (`shell::tests::a_corner_where_ridges_and_valleys_alternate_is_named`,
  `a_cavity_whose_ridge_runs_over_its_inside_corner_is_named`).
- [medium · hard] `select::classify` lets inside or outside samples win over coincident ones in a
  partly coincident fragment (only coincident samples of opposite senses make it `Ambiguous`). A
  sample now counts as coincident only on a face of the same elementary surface, which removed the
  tangent-line noise that made every partly coincident fragment `Ambiguous` fail
  `stress_cylinders_on_a_grid`, and none occurs in the boolean tests or the aligned-contact survey;
  making a mixed fragment `Ambiguous` is unchecked against the random-placement survey, and
  splitting fragments exactly at coincident boundaries would replace it.
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

- [medium · easy] Typing a plain value over a feature's named value (`depth = 12 mm`, then `14 mm`),
  which `app-modelling.md` says unnames it, is refused and nothing changes: the field shows "That
  parameter no longer exists". Reproduced on an extrusion's Distance through the UI harness; the
  blend, hole and primitive panels build their change the same way. `solid_tools::edit`,
  `blend_tools::edit`, `hole_tools::edit` and `primitive_tools::edit` pass the change through
  `Document::carrying_owners`, which already releases a named value the new kind no longer holds
  and appends the `RemoveParameter` of one nothing else uses. `NamedField::transaction`
  (`field.rs`) then calls `Document::releasing` on that result for the same parameter:
  `releasing` takes the id into `pending` before applying the transaction, finds it unused, its
  second `RemoveParameter` is refused, so it stays pending and ends as an
  `Edit::SetParameterOwner` for a parameter that is already gone, which `field::checked` refuses
  as `UnknownParameter`. The `releasing` test in `model_parameter_tests.rs` hands it a bare
  `SetFeatureKind`, so it passes. A value another expression still uses is not affected, and sketch
  dimensions do not go through `carrying_owners` (not run). Keeping in `pending` only the ids the
  applied transaction left in the document, or leaving the field's release out where the panel's
  change has done it, closes it; a UI test should name a value, type a plain number and expect the
  number in the field and the parameter gone.
- [medium · medium] Reported by the user: a Remove extrusion typed far past the body (100000 mm, to
  cut through without measuring) does not cut, so the exact depth must be found. Not reproduced:
  in `caditor-document`, a box, a cylinder, a thin block and a filleted block cut by rectangles
  and circles inside, over, at the edge of and below the body, one side both ways, symmetric and
  two sides, give the same volume and no failure at 100, 1000, 100000 and 999999 mm; through the
  UI harness (`ui_tests/far_cuts.rs`), a cut sketched on a plate's top face and typed as 20, 1000,
  100000 and 999999 mm, committed or only previewed, leaves 14000 mm³ with no failed feature, the
  open cut's tool (the whole 100 m column, not only what it removes) is in the overlay and the
  scene's bounds stay those of the model. A distance more than ten times the bodies' size now
  offers Cut through all instead under the field. Still open: a body of another kind (a revolve,
  a shelled or imported one), a cut that removes from several bodies, the drag arrow, the camera
  with the 100 m tool shown, or what the report meant by "does not cut" (the preview, the closed
  result).
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
- [medium · hard] An edge ending at a corner that two earlier fillets share cannot be blended at
  all: a 40×30×20 box whose four top edges were filleted 2 mm refuses every fillet or chamfer of
  its vertical edges, of any size, as `UnsupportedEnd` at the top vertex, where the vertical edge
  meets the fillets along their tangent line; the refusal names the earlier blend and says to move
  the feature above it (`Context::earlier_blend_at`,
  `an_edge_ending_at_a_corner_an_earlier_blend_cut_names_it_and_says_to_move_above_it`). Corners
  two earlier chamfers made work (`out_through_corner`). With one top edge filleted the verticals
  at its ends take a chamfer or a smaller fillet, but a fillet larger than the top one (3 mm under
  a 2 mm fillet) is `TooLarge`, its foot on the side face crossing the end of the top fillet. Top
  edges first, then the verticals, is an order other modellers take routinely. The end needs
  closing against the earlier fillets' corner faces (`blend/mod.rs`, ends at a vertex whose faces
  are curved and neither one round face to clip by nor planes the tool can run out through).
- [medium · hard] The top rim of a box tapered to widen upwards, whose slanted corner edges were
  filleted less than about twice the rim fillet, is refused: a 40×40×30 box tapered -4° with
  3 mm corners refuses a 2 mm rim fillet as `Rounding(Untraced)` and with 2.2 mm corners as
  `Intricate` (sections meeting at a joint too far apart), and at -8° with 2.2 mm or 2.5 mm
  corners as `TooLarge`; 4 mm corners at -4° work. The ball's side contact there moves from a
  corner's cylinder onto the side plane partway along the straight rim, a switch the run
  splitting and the rounding's retraced edges do not yet follow. Tapering the other way works down
  to corners a little larger than the fillet, and corners no larger than it are refused as
  `TooLarge`.
- [low · easy] Cancel feature and Cancel sketch are refused once a named value made during the
  session has been unnamed or dropped: "Name depth", made while Extrude 1 was open, changed more
  than Extrude 1, and the session stays applied. Reproduced through the UI harness by naming an
  extrusion's distance and then applying the change `solid_tools::edit` builds for a plain
  distance; the same holds for a mode switch that drops the value, and for a sketch dimension
  named and then typed over with a plain number (not run). `changes_since` in `editing.rs` judges
  each undo step alone (`changes_only`, `owned_by`) and accepts a parameter only while the live
  document still gives it to the feature, or when that very step inserts or reassigns it to the
  feature. The inverse of a naming is a bare `RemoveParameter { id }` that carries no owner, so
  once a later step has removed the parameter, `owned_by` is false for the earlier step even
  though the later step's own inverse (an `InsertParameter` owned by the feature) shows whose it
  was. Judging the steps since the mark together, taking a parameter as the feature's when any of
  them inserts it with the feature as owner or sets that owner, fixes it without letting Cancel
  take a parameter another feature owns. A UI test should name a value, replace it with a number,
  cancel and find the feature as it was opened (`ui_tests/cancel_and_parameters.rs` cancels only
  while the name is still on the feature).
- [low · medium] A model file whose feature uses one stored after it (only a damaged or hand-made
  file; caditor never saves one) loads with the user above what it uses, since loading appends
  features and an insert checks only the inserted feature's own references. Undoing a deletion or
  kind change of that user, or undoing a move that lifts the used feature above it, is then
  refused (`MissingFeature`, `BelowDependent`): the undo would restore an order no edit allows. An
  insert refusing to land below a feature using it would close it, but needs a cheap way to find
  the users of an ID, since loading and `transaction_to` insert every feature.


## Sketching

- [low · medium] An offset that closes a notch between edges running the same way at different
  heights (a 30×20 outline whose top steps down to 19 mm at a 2 mm wide notch, offset outward
  1.5 mm) is refused as `UsedUp` on the notch floor: across a dropped run `bridge` (`offset.rs`)
  re-meets lines that cross and merges lines along one line, but two parallel lines side by side
  need a piece joining them (a line square to both through the higher edge's offset end, or the
  round about its corner) that the outline does not make.

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
- [medium · medium] Only planes are snapped (`step-read.md`, "Snapped faces"): curved rigid faces
  that meet within the file's declared precision but not within `LINEAR_RESOLUTION` (two
  cylinders, or a cylinder and a torus, tangent a few micrometres off; coaxial faces whose axes
  differ by a hair) still turn the whole body into flat facets, since bending moves only splines.
  Snapping them would move the face with fewer neighbours onto its partner's axis or radius, each
  move bounded by the precision and noted like a plane's.
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


## Viewer

- [low · medium · blocked by: wgpu's GL backend] On GL and other devices without texture view
  formats the multisample resolve still averages in gamma space. A resolve of its own (a pass
  reading the samples through a `texture_multisampled_2d` and averaging them in linear light) was
  tried: it matches the view-format resolve on Vulkan, but wgpu 30's GL backend binds a
  multisampled texture as `TEXTURE_2D` (`gles::Texture::get_info_from_desc` never chooses
  `TEXTURE_2D_MULTISAMPLE`), so every sample reads as zero there and the frame comes out black. It
  needs that fixed in wgpu, or a GL-only blit resolve into an sRGB texture.

