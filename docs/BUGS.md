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

- [high · hard] An edge reference keeps only the piece that kept its curve's id when an upstream
  sketch edit splits the edge, silently: the 10×8×4 block of `blend_tests` with its front and left
  top edges filleted 1 mm, whose front sketch line is then notched (lines from (4, 0) to (4, 2),
  (6, 2) and (6, 0), the front line trimmed between them), recomputes with no failure and only one
  piece of the front edge rounded (301.52 against 300.66 with both). Trim gives the split-off piece
  fresh ids, so its side face and the edge along it have new names, and `EdgeReference` resolves
  to the edge on the original curve's face alone (`pieces.rs` gathers the pieces of an edge split
  within its faces, not an edge whose face became two faces of sibling curves). The UX rule that
  an early sketch edit never silently rewires later features needs the reference widened to every
  edge along the same curve between the same cap and the faces of its `Collinear` pieces, or the
  feature marked as having lost part of its choice with a fix
  (`blend_tests::a_notch_trimmed_into_a_filleted_edge_keeps_both_pieces_rounded`, ignored).
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
- [medium · hard] An edge ending at a corner that two earlier blends share cannot be blended at all:
  a 40×30×20 box whose four top edges were chamfered 1 mm (mitred corners) or filleted 2 mm
  (spherical corners) refuses every fillet or chamfer of its vertical edges, of any size, as
  `UnsupportedEnd` at the top vertex, worded "The fillet cannot be closed off where the edge
  between Base side from Line 2 and Base side from Line 5 ends. Also choose the edges that
  continue from it, or leave it out", though the edges continuing from it belong to the earlier
  feature and cannot be chosen. With one top edge blended the verticals at its ends take a chamfer
  or a smaller fillet, but a fillet larger than the top one (3 mm under a 2 mm fillet) is
  `TooLarge`, its foot on the side face crossing the end of the top fillet. Top edges first, then
  the verticals, is an order other modellers take routinely. The end needs closing against the
  earlier blend's corner faces (`blend/mod.rs`, ends at a vertex whose faces are neither one flat
  face nor a round face to clip by); until then the remedy should say to move the feature above
  the one that blended the corner
  (`blend::tests::vertical_edges_are_rounded_after_the_top_rim_is_chamfered`, ignored).

## Sketching

- [medium · hard] Offset keeps one offset curve per original, so an offset that would drop a curve
  is refused: a 30×20 rounded rectangle (2 mm tangent corner arcs) offset inward by 2 mm or more is
  `Collapses` on its first arc, where an arc shrinking to nothing should leave a sharp corner and
  the lines meet beyond it, and an outline with a notch narrower than twice the distance (a 30×20
  rectangle with a 2 mm wide, 2 mm deep notch in its top, offset outward 1.5 mm) is `UsedUp` on a
  notch wall, where the notch should close up. The inner wall of a moulded or printed box drawn as
  its outline's offset by the wall thickness meets both. `Chain::outline` (`offset.rs`) would drop
  a vanishing arc, or the run of curves a closing notch uses up, and re-meet the neighbours,
  leaving out the dropped curves' constraints as trim does.

## Sketch solver

- [medium · hard] Before naming a conflict, diagnosis descends on it from the drawn shape and from
  the closest witness, and damped Gauss–Newton crawls on a set that cannot hold: near its
  least-squares point the Jacobian is nearly singular, so the line search cuts each step back and
  it gains about 1%, too much to count as stalled. On a chain of 300 lines with its far end fixed
  out of reach those two descents (three attempts each, most running all 100 iterations) take about
  370,000 of the 500,000 units of `DIAGNOSIS_WORK`, against about 90,000 for the search and 26,000
  for trimming, leaving about 14,000, so a longer chain would run out there and be reported as not
  solving. A step regularised along the near-null direction (Levenberg–Marquardt, or the
  factorisation trimming already makes) would reach the minimum in a few steps.
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
  bends a face whose boundary then crosses itself in uv. The bent path has no STEP test yet (a
  fixture of a spline tangent to its neighbour a few micrometres off), and that file now reads in
  8.7 s against 7.6 s. Healing still traces an edge only between exactly two distinct faces, so an
  edge used twice by one face (a cylinder seam) with a vertex a few micrometres off is refused
  outright when the file declares no precision.
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
