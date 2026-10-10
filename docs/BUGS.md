# caditor bugs

Wrong results, silent losses and refusals of shapes and operations that should work today, each with
its reproduction or pinning test where one exists.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Files and recovery

- [high · easy] A 3MF import can crash the whole app: `import/mesh.rs` parses the model part and
  `_rels/.rels` with `roxmltree::Document::parse`, whose tokenizer recurses once per nesting level
  (the reason `import/svg/xml.rs` exists, `file-import-export.md`), so a part of some 100,000
  nested elements, a few hundred bytes deflated, overflows the import thread's stack, which
  `catch_unwind` cannot catch. Parse both parts with `xml.rs`, which bounds depth and elements, and
  pin it with a deeply nested part.
- [high · easy] A 3MF's components expand without a total budget: `add_object` recurses through
  `<component>` up to `MAX_COMPONENT_DEPTH` (16) and copies the leaf mesh for every instance, so
  16 objects of 4 components each place about 4·10⁹ meshes from a few kilobytes until the
  allocator aborts, and the loop never polls the cancel token, so Cancel does not stop it; objects
  are also found by a linear scan per item and component. DXF, SVG and STEP charge
  `MAX_EXPANDED_OBJECTS` or `MAX_INSTANCES`; charge placed objects and triangles here the same way,
  index objects in an `UntrustedMap` and poll the cancel.
- [high · easy] A save can replace an outside change without asking: `save_with` compares the
  file's head with `unless_changed_from` only before encoding, and encoding (zstd at level 9,
  thinning, recompressing versions) can take seconds on a large model; a sync client or another
  program writing the file meanwhile is then renamed over, neither kept as a version nor reported
  as `ChangedOnDisk`. `write_sharing` even notices (`changed_since_read`), logs and carries on.
  Check again in the read-back closure, by `changed_since_read` and by the target's `FileKey`
  against the file read (a rename-replace leaves the old handle's stamp unchanged, also on
  Windows), and do the same in `set_version_kept`.
- [high · medium] One damaged byte in a journal's snapshot withholds every change after it:
  `decode_journal` returns `DamagedJournal` when the `Snapshot` chunk fails its checksum, though
  the header and entries are intact, so `journal_for` sets the journal aside, the scan never
  offers it and it is deleted after `SET_ASIDE_KEPT_SECONDS`. A plain snapshot is the last saved
  state by definition: when the model file's head digest equals the header's `on_disk`, load the
  file as the base, replay the entries and report it; a `RebasedSnapshot` keeps today's handling.
- [medium · easy] A recovery offer vanishes when discarding it fails: `Event::Discarded` removes
  the journal from `recoverable` before looking at the error, so the journal stays on disk and the
  notice says so, but the card is gone and Recover unsaved work is unavailable until the next
  launch, the only way back in this session for an untitled journal. Keep the candidate on an
  error and say its changes are still there to restore.
- [medium · easy] Quitting while an export runs drops it silently: `Files::request(Intent::Quit)`
  only abandons an open in flight, `run` closes storage and quits without asking
  `Exporter::is_running` or the image export, and their threads die with the process, leaving
  only a temporary. Ask first ("An export to … is still running") or let the Closing modal wait
  for it as it waits for storage.
- [medium · easy] Three concatenated zstd frames still reach the port's history underflow that
  `SPARE_ROOM` was meant to rule out (`zstd.md`): a frame filling the recorded size, one filling
  the spare byte and a third holding sequences start the third on an empty output, where
  `totalHistorySize` is computed before the port's empty-output guard, so builds with overflow
  checks (tests, fuzzing, `cargo run`) can abort inside `extern "C"` on a crafted `VersionData`
  chunk with valid checksums; release builds wrap and fail as before. Refuse a payload that is not
  exactly one frame (`ZSTD_findFrameCompressedSize`) and pin three frames beside the two-frame
  test.
- [low · easy] Forget recent and the other recent-list edits post success at once, while the write
  runs on the files worker and a failed `RecentFiles::save_changes` is only logged, so the next
  launch lists the models again with nothing said. Report it on the files-worker event path, as a
  failed preferences save already is.
- [low · easy] The read-back before a save's rename checks records only: `binary::reads_back`
  checks chunk checksums and the head and records digests, never decoding the delta written for
  the state the save replaces, nor deltas rewritten by thinning, so a compressor defect there would
  leave the most restored version unrebuildable, found only when a restore is refused. Rebuild
  version 0 (and any rewritten delta) against its digest in `check_reads_back`.

## Kernel

- [high · easy] Offset face renames edges that had to be told apart: the shell's inner solid ends
  with `built.renamed(|name, origin| (name, origin))`, and `Solid::renamed` sets every edge to
  plain `between` or `seam` of its faces with none of the `between_at`/`occurrence`
  disambiguation `derived_edge_names` does, while `offset_faces` (`Naming::Kept`) returns that
  solid as it is. On a block whose top a groove splits into two pieces of one name, offsetting
  any other face leaves both groove edges named `between(top, groove)` with the same ends, so a
  fillet holding either fails as `Ambiguous`, against `kernel-operations.md`'s promise that kept
  names keep their edges. Rename only for `Naming::Shell`, or through `derived_edge_names`, and pin
  edge references across an offset (the offset tests compare face names only).
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

- [high · easy] A parameter derived from a measurement keeps an old result when its other inputs
  change: the global `ParameterValues::evaluate` makes every parameter reading a measurement
  `Unmeasured`, `fingerprint` turns that error into `None`, and `Key::of` fingerprints the
  feature's parameters from those global values with only the measurement in its upstream. With
  Measurement 1 feeding `clearance`, `half = clearance / k` and a datum point offset by `half`,
  changing `k` from 2 to 4 reuses the cached point though Parameters shows the new `half`; tests
  miss it because their `evaluate` helper builds a fresh `Recompute`. Fingerprint from the
  overlaid values of the feature's view, or add every parameter the derived ones reach.
- [high · medium] Scale model leaves every other configuration at the old size: `Document::scaled`
  rewrites parameters, feature lengths and saved views but never reads `configurations`, and a
  parameter edit copies its new expression into the active row only, so switching to another
  configuration afterwards rebuilds the part unscaled. Rewrite the inactive rows' length cells with
  the same rescaler in the scale transaction (one `SetConfigurations`), leaving angles, counts,
  suppression and the active row alone.
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
- [medium · easy] Curve pattern and Point pattern pattern the last body when the selection names
  something else: `sketch_pattern_tools` falls back to the last standing body whenever no tree
  row is chosen and no selected item has a body, and `Pickable::body` is a face, edge or vertex
  only, so a selected datum axis, principal plane or centre of mass patterns the last body (along
  a guessed sketch when none is selected). Linear pattern falls back only on an empty selection;
  do the same here, still falling through for a selection of sketch geometry alone.
- [medium · easy] Plane and Point make a datum on XY or at the origin when the selection holds
  nothing they can use: `datum_tools::why_unusable` returns nothing for a sketch region, a sketch
  constraint, a centre of mass or a body item picked while sketching, and with nothing gathered a
  plane starts on XY and a point at the origin, opened as if built on the selection. Axis already
  refuses that selection; refuse in its words when the selection is not empty.
- [medium · easy] A parameter used only as a body's density reads as unused: `used_parameters`
  collects `feature.kind.parameters()` rather than `Feature::parameters`, so the Parameters panel
  draws it unused and Delete unused parameters includes it; the removal is refused (removal reads
  the density), the atomic apply keeps every other unused parameter too, and the success notice
  posted after it replaces the refusal. Collect `Feature::parameters` and inform only once the
  apply is accepted.
- [medium · easy] A Split, or a Move that copies, cannot be edited once something uses its body:
  `SetFeatureKind` keeps a body only for imports and new-body solids and primitives, so any other
  kind whose id another feature's `bodies_used` holds is refused as `BodyInUse`, though
  `makes_body` is true for both. Changing a split's plane under a fillet on the split-off part is
  refused, and so are Update references (`healing.rs` emits the same edit) and, through one
  refused edit, the whole `complete_origins` transaction on loading, logged only. Decide it as
  `Feature { kind: new, .. }.makes_body()`.
- [medium · easy] Sketch dimensions escape the measurement-order check: `set_dimension` and
  `add_sketch_constraint` check references only, and `check_measurement_order` runs on
  `SetFeatureKind` and `InsertFeature`, which a sketch never takes after insertion. A dimension of
  the sketch an extrusion sweeps, set to the parameter a measurement of that extrusion feeds, is
  accepted, closing a cycle the rules refuse as `MeasurementBelowUser`; recompute then fails the
  sketch blaming the measurement, which cannot be moved above it. Run the check on the edited
  sketch in both edits.
- [medium · easy] A fillet hides the chamfer form it keeps from parameter uses: `Blend::expressions`,
  `expressions_mut`, `parameters` and `uses_parameter` go through `chamfer_form`, which is `Equal`
  for a fillet, so a chamfer by two distances using `d2`, switched to a fillet, lets `d2` be
  deleted (and inlining and pasting miss it), after which switching back fails as
  `MissingParameter`; a pasted fillet keeps the source model's raw id. `document.md` says the extra
  expression counts among the feature's; make the four read `form` whatever the kind.
- [medium · medium] Configuration cells are not uses of a parameter: neither `used_parameters`
  nor `remove_parameter` reads them, and `inline_parameter` rewrites the live model only, so a
  parameter named only in an inactive configuration, or inlined out of the live model while a cell
  still names it, can be deleted, after which switching to that configuration fails as a missing
  parameter and rolls back. Count `Configurations::parameters_named` in both and inline stored
  cells too, dropping a cell whose column is the removed parameter.
- [medium · medium] Reported by the user: a Remove extrusion typed far past the body (100000 mm, to
  cut through without measuring) does not cut, so the exact depth must be found. Not reproduced in
  `caditor-document` on a flat plate (one side, 10 mm to 999999 mm, volumes right), so the cause
  is outside that path: a curved or filleted body, the open cut's tool meshed and shown at a
  100 m length, the drag arrow or the camera. Reproduce through the UI harness first; Through all
  already covers the intent and should be offered when a distance reaches far past the body.
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
- [low · easy] Undo undercounts a sketch's size: `FeatureKind::approximate_size` for a sketch counts
  entities, constraints and projections but not its `uses` map, `inactive`, `labels`,
  `construction` and `projected` sets or the attachment's neighbour names, so deleting a sketch of
  100,000 imported curves keeps several megabytes past `MAX_UNDO_BYTES`. Give `Sketch` a
  `heap_size` covering every map.


## Application

- [medium · easy] A file dialog that dies reads as a cancel: the portal's response stream ending
  and zenity exiting with no status are both returned as a cancel (`portal/xdg.rs`, beside the real
  cancel status 1), and a cancel clears `after_save`, so a save meant to continue into quit, new or
  open is dropped without a word. Return `DialogError` for both, keeping status 1 a cancel.
- [medium · easy] Reload import from its file and Replace from file touch the file on the UI
  thread: `kept_source` calls `is_file` and `replace_import` calls `import::is_model` (which opens
  and reads a file whose extension is unknown) before the import thread starts, so an import whose
  share has gone (NFS, SMB) freezes the window, against `ux.md`; the ordinary import runs the same
  check on its thread. Move both checks into the `start_import` job.
- [medium · easy] A new model keeps two ids from the last one: `Workspace::sync` resets every
  panel holding ids but `MeasureTool::relative_to` and the analysis's `Pull::Picked`, which then
  resolve against the new document's features of the same id (a coordinate system the user never
  chose, or a warning naming an unrelated feature). Clear both in the new-session branch, as the
  comb's and section's `forget` are, with a test like theirs.
- [medium · easy] A sketch drag or a constraint trial lands after an undo: a drag still solving at
  release keeps `finishing` and `poll` hands the solution to `commit_drag` with no revision
  check, and a trial's verdict up to `PATIENCE` later applies the transaction it was built with,
  so Ctrl+Z pressed in that window is overwritten on top and its redo dropped. Carry the revision
  in `Finished` and in `Trials::Running` and drop a stale result with a notice.
- [low · easy] A parameter import that left rows out still shows the success notice: Apply always
  posts `Notice::success`, so a partial import reads as complete beside the refused rows' text.
  Use a warning whenever a row was refused.
- [low · easy] Two caches miss the units: the offers' `Basis` holds only the length unit while
  `Offers::size` writes sweeps and half angles in the angle unit, and `scene_description::Key`
  holds no unit while it speaks lengths, so switching degrees to radians, or the length unit,
  leaves the status bar and the 3D view's description stale until the selection or model
  changes. Key them on `Units`.
- [low · easy] The 3D view's description can read "-0": `scene_description.rs` trims
  `format!("{:.3}")` without the `-0` guard `caditor-expression`'s `format_number` has, so a
  direction component of -0.0002 is spoken "along -0, 0.707, 0.707". Nine copies of that trimming
  have drifted (`hole_standard.rs` lacks the guard too); one shared function would end it.
- [low · easy] A frame that panics mid-drag leaves the drag shown: `after_failed_frame` resets the
  interface but never sends `DragCommand::Cancel` (Escape and `Model::switch_to` do), and
  `Display::evaluated` clears only a released drag, so the sketch stays at its uncommitted
  positions for snapping, Measure and region picks until the next edit, and the worker's drag
  carries on from the old start.

## Sketching

- [medium · easy] Filleting or chamfering a corner held by two coincident points turns a disabled
  dimension back on: `merge_point` removes each constraint on the discarded point and inserts it on
  the kept one, while `remove_constraint` clears the inactive flag and the label offset and the
  insert restores neither, so the solver enforces a value the user had switched off. Keep both
  across the move, and pin a disabled distance on the second corner point.
- [medium · hard] Offset keeps one offset curve per original, so an offset that would drop a curve
  is refused: a 30×20 rounded rectangle (2 mm tangent corner arcs) offset inward by 2 mm or more is
  `Collapses` on its first arc, where an arc shrinking to nothing should leave a sharp corner and
  the lines meet beyond it, and an outline with a notch narrower than twice the distance (a 30×20
  rectangle with a 2 mm wide, 2 mm deep notch in its top, offset outward 1.5 mm) is `UsedUp` on a
  notch wall, where the notch should close up. The inner wall of a moulded or printed box drawn as
  its outline's offset by the wall thickness meets both. `Chain::outline` (`offset.rs`) would drop
  a vanishing arc, or the run of curves a closing notch uses up, and re-meet the neighbours,
  leaving out the dropped curves' constraints as trim does.
- [low · easy] Trim, extend, split, break, fillet and chamfer drop the placed label of every
  dimension they keep: `trim::restructure` restores each kept constraint's inactive flag but not
  its label offset, which `remove_constraint` drops, and `move_constraints` copies the flag onto
  the far piece without the offset, so labels jump back to their automatic place. Carry the offset
  beside the flag.

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
- [medium · easy] A body that cannot be rebuilt is reported with kernel internals:
  `describe_build` (`read/topology.rs`) puts a `ValidationError`'s or `BuildError`'s own text in
  the note ("does not make a closed, valid solid (shell ShellId(0) has Euler characteristic 1,
  …)"), Debug ids and topology words included, which reaches the import report and the failure
  notice, against `ux.md`. Word each validation error plainly in an exhaustive match, log the raw
  text, and pin that no note holds `Id(`.
- [low · hard] A face whose surface cannot be read is left out (`step-read.md`, "Unreadable
  faces"), but from the outer shell that turns the whole body into flat facets, since the kernel
  holds only closed solids, and a lost face with holes is not closed at all; an edge whose curve
  cannot be read still loses the whole body. An offset that folds anywhere in its basis's domain
  is refused even where the face's own region is clear: fit over the region the face uses.


## Viewer

- [medium · easy] A sectioned body with a see-through face caps through it in the pick pass:
  `fs_mesh_sectioned` discards a face whose alpha is zero before drawing a cap, but
  `fs_mesh_pick_sectioned` caps every back face with pick id 0, and a see-through face stays on the
  opaque instance at alpha 0 (`scene.rs`), so the colour pass shows the face through a hole in the
  cap while a click there selects nothing. Discard zero alpha before the cap in the pick shader.
- [medium · easy] The axis triad in the corner of the 3D view draws in fixed colours:
  `view_cube::show_axis_triad` takes `Axis::rgb()` (the dark canvas's red, green and blue) for its
  lines and letters, never `ScenePalette::axis`, so on the light canvas its Y reads at about 1.7:1
  and high contrast never reaches it, and its letters have no backdrop, against `ux.md`. Draw it
  from the palette, put the letters on `canvas::backdrop` and add it to the palette checks.
- [medium · medium] Painting or listing under the pointer on a section cap takes the hidden far
  wall: `through.rs` `hits_through` lists every triangle on the kept side of the plane with no
  back-face or cap test and painting keeps the first, while a click on the same pixel hits the
  cap's id 0 and selects nothing. For a closed mesh, treat a back-facing hit behind the ray's
  section entry as the cap and drop it and what lies behind it; open meshes keep their far side.
- [low · medium · blocked by: wgpu's GL backend] On GL and other devices without texture view
  formats the multisample resolve still averages in gamma space. A resolve of its own (a pass
  reading the samples through a `texture_multisampled_2d` and averaging them in linear light) was
  tried: it matches the view-format resolve on Vulkan, but wgpu 30's GL backend binds a
  multisampled texture as `TEXTURE_2D` (`gles::Texture::get_info_from_desc` never chooses
  `TEXTURE_2D_MULTISAMPLE`), so every sample reads as zero there and the frame comes out black. It
  needs that fixed in wgpu, or a GL-only blit resolve into an sRGB texture.

