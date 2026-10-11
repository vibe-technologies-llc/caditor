# caditor bugs

Wrong results, silent losses and refusals of shapes and operations that should work today, each with
its reproduction or pinning test where one exists.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Files and recovery

- [high · easy] The "changed by another program" prompt outlives a switch to another model, and
  its Replace then saves the model open now over the old file, keeping neither the outside
  contents nor the old file's versions: `FileEvent::ChangedOnDisk(path)` sets
  `Files::changed_on_disk` with no session (the one prompt the `Since` checks left untagged),
  nothing clears it when another model opens, and `OutsideChange(Replace)` calls
  `Model::save_replacing_outside_changes(path)` on whatever model is open, whose storage worker
  reads versions from its own file (none when untitled). Open A, let another program save over
  it, edit and press Ctrl+S, then Ctrl+N before the save reports: the save's report waits behind
  "Save changes to “A”?" (the guard is drawn first), Continue without saving opens the new model,
  "“A” was changed by another program" follows, and Replace writes the empty model over
  A.caditor. Keep the session with the prompt and drop a stale one with
  `not_written_for_another_model`, as Save As and exports do; pin it beside
  `saving_over_a_file_another_program_changed_asks_before_replacing_it`.
- [medium · easy] Closing the window while a file dialog is open makes the unsaved-changes
  prompt's Save As… do nothing, and the next ordinary save then quits caditor: the title bar's
  Close is drawn over dialogs (`window_frame::over_dialogs`) and `FileCommand::Quit` is refused
  only while closing or waiting for exports, not while `picking`; `GuardChoice::Save` sets
  `after_save = Some(Quit)` and an untitled model's `save` calls `pick(SaveAs)`, which returns at
  once while another pick is pending, and a cancelled Import or Export pick never clears
  `after_save` (only Save As and Open do, `picked`), so the save the user makes later runs
  `request(Quit)` and the window closes. Refuse the prompt's Save while a dialog is open (or stop
  the pending pick first), and clear `after_save` when a pick it did not start ends; pin it in
  `ui_tests/import_jobs.rs` with `hold_dialogs`.
- [low · easy] Restore this version on the preview card ignores the one-restore-at-a-time rule:
  the card's button (`version_preview.rs`) is always enabled while the rows' are disabled when not
  `idle`, and `VersionHistory::start_restoring` refuses only while a Keep runs, so a restore from
  the card while one from a row is loading starts a second, `finish_restoring` applies whichever
  result arrives under the index started last (marking the wrong version Damaged when the first
  fails as `VersionUnavailable`), and both land as "Restore earlier version" steps, the last to
  finish winning. During a Keep the card's button silently does nothing. Disable it as the rows
  are and refuse a restore while one runs.
- [low · easy] Discarding recovered changes whose journal is already gone fails for good, worded
  as the operating system's text: `recovery::discard` returns `NotFound` when the journal no
  longer exists (another window discarded or restored it), the notice prints the `io::Error` as
  is ("No such file or directory (os error 2)") and keeps the card, and every retry fails the same
  way. `PreferencesNotSaved` and `RecentNotSaved` print raw `io::Error` text the same way
  ("Permission denied (os error 13)"), where saves and loads word theirs through `WriteFailure`
  and `ReadFailure`. Count a journal already gone as discarded and word the three errors; pin it beside
  `a_discard_that_fails_keeps_the_recovery_offer_and_says_the_changes_can_still_be_restored`.
- [low · medium] The startup recovery scan is not reconciled with what the window did while it
  ran: `Event::Started` offers every journal it found again (`Files::offer` drops only those
  still listed), so work restored or discarded from the card opening a model showed before the
  scan finished is offered again, and restoring that stale copy replaces the edits made since; and
  the scan holds each journal's lock (`try_lock`) through replay and a whole load, so an open
  reaching `journal_for` meanwhile is refused as "already open in another caditor window", as is
  a Save As target check. Remember the journals restored or discarded this session and skip them
  and the open model's own journal when the scan reports, and retry an open briefly on
  `InUse`; pin it near `unsaved_work_from_a_crash_is_offered_and_restored_with_its_history` with
  `hold_jobs` delaying `Started`.


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
- [medium · easy] Switching a panel's mode moves a named value into a field of another caption, and
  the value is then no longer the field's own: a parameter named in Distance (`h = 20`) and carried
  to Total distance by Extent › Symmetric (`solid_panel::reshaped`), to Total angle or Forward by a
  revolve's Extent (`turn_row`), or to First distance by a chamfer's Distances › Two
  (`blend_tools::FormChoice::applied_to`) keeps its `ParameterOwner::Feature` with the old
  caption, and `Document::owned_parameter` matches the caption, so the field shows `h` instead of
  `h = 20 mm`, typing `h = 30` there is refused as a name already taken, the reach or turn arrow
  is driven ("Total distance follows h", `manipulator::Held::of`), and a plain number typed over it
  leaves `h` unused but still listed as "Extrude 1 · Distance". Choosing a hole Size, a primitive
  Shape or a chamfer form that drops the angle replaces a named value without `releasing` it
  either. Carry the owner to the new caption (`Edit::SetParameterOwner`) in the switch's
  transaction, and release what a switch drops; pin it beside
  `values_named_in_their_fields_are_listed_as_model_parameters_and_used_by_name`.
- [medium · easy] Typing a value over a hovered handle skips the rule its panel field holds:
  `manipulator::Typing::parse` checks only dimension and units, and `Typing::transaction` builds
  the change through `reach_handles::typed`, `turn_handles::typed` or `value_handles::written`
  and checks it with `field::checked`, a trial apply that tests no ranges, so `0` over a fillet's
  size, `2.5` over a pattern count, `-10` over a one-sided distance, `60 deg` over a taper or
  `400 deg` over a revolve's angle is committed as an "Edit …" change and the feature, and those
  after it, fail, where the panel row (`Rule`) and a drag (`Measured::accepts`) both keep it out.
  Give `Typing` the slot's `feature_fields::Rule` and refuse in the field, as `app-modelling.md`
  says a refusal does; pin it beside
  `hovering_an_arrow_and_typing_a_value_sets_it_and_previews_it_first`
  (`ui_tests/feature_handles.rs`).
- [medium · medium] Reported by the user: a Remove extrusion typed far past the body (100000 mm, to
  cut through without measuring) does not cut, so the exact depth must be found. Not reproduced:
  in `caditor-document`, a box, a cylinder, a thin block and a filleted block cut by rectangles
  and circles inside, over, at the edge of and below the body, one side both ways, symmetric and
  two sides, give the same volume and no failure at 100, 1000, 100000 and 999999 mm; through the
  UI harness (`ui_tests/far_cuts.rs`), a cut sketched on a plate's top face and typed as 20, 1000,
  100000 and 999999 mm, committed or only previewed, leaves 14000 mm³ with no failed feature, the
  open cut's tool (the whole 100 m column, not only what it removes) is in the overlay and the
  scene's bounds stay those of the model. Still open: a body of another kind (a revolve, a
  shelled or imported one), a cut that removes from several bodies, the drag arrow, the camera
  with the 100 m tool shown, or what the report meant by "does not cut" (the preview, the closed
  result). Through all already covers the intent and should be offered when a distance reaches far
  past the body.
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
- [medium · hard] The top rim of a box tapered to widen upwards, whose slanted corner edges were
  filleted less than about twice the rim fillet, is refused: a 40×40×30 box tapered -4° with
  3 mm corners refuses a 2 mm rim fillet as `Rounding(Untraced)` and with 2.2 mm corners as
  `Intricate` (sections meeting at a joint too far apart), and at -8° with 2.2 mm or 2.5 mm
  corners as `TooLarge`; 4 mm corners at -4° work. The ball's side contact there moves from a
  corner's cylinder onto the side plane partway along the straight rim, a switch the run
  splitting and the rounding's retraced edges do not yet follow. Tapering the other way works down
  to corners a little larger than the fillet; corners no larger than the fillet are refused as
  `Intricate`, which should read as `TooLarge`.
- [low · easy] Deleting a feature or typing a plain value over a named value leaves a model
  parameter that the configurations still name owned by the feature that no longer uses it:
  `releasing` (`model_parameters.rs`) judges a parameter unused by `parameter_user_ids`, which
  reads parameters and features but not configurations (`parameter_users` beside it does), and
  takes it out of `pending` before trying `RemoveParameter`, so when that is refused the
  `SetParameterOwner { owner: None }` fallback never covers it, and the parameter table keeps
  saying it is the value of the old feature. The table's "Used by" menu (`parameter_table.rs`)
  omits configuration users through the same call. Count configuration users there and keep the
  parameter pending until its removal succeeds.
- [low · easy] Update references is offered from an evaluation of an older revision: the tree's
  callout, its row menu and the palette (`feature_tree.rs`, `reference_update`) take
  `status.healing` from `Model::evaluation`, which until the new revision reports is still the
  last one's (features a running recompute has not reached keep theirs too), and
  `Healing::update` (`healing.rs`) refuses only when the feature's own kind changed. Undo the
  hole drawn in `a_region_matched_again_after_a_hole_is_drawn_warns_until_its_references_are_updated`
  and click Update references before the recompute reports: "Update references of Base" writes
  the region captured on the sketch with the hole into a model without it, giving a new healing
  warning (for a fillet, "A chosen edge is no longer part of the body"). Keep the revision the
  held evaluation is of and offer the update only while it is the current one and the feature
  is not pending.
- [low · medium] A hole's Position X and Y keep a number where an expression was typed:
  `hole_panel::position_rows` previews `width / 2` with its value and then stores the evaluated
  millimetres in the hidden sketch's point (`hole_placement::moved`), as `place_handles::typed`
  does for "x, y" typed over the square, so the hole stays put when `width` changes, against the
  `ux.md` rule that numeric fields take expressions of named parameters; a primitive's Position
  fields keep the expression (`primitive_panel.rs`). Refuse an expression using parameters there
  with a pointer to Place by, or hold the point with horizontal and vertical distances from the
  sketch origin (which `hole_placement::read` would then have to accept); pin it beside the test
  typing into `("hole-position", …)` in `ui_tests.rs`.


## Application

- [medium · easy] A cell of an inactive configuration takes an expression that makes a parameter
  depend on itself, and that configuration can then never be switched to:
  `Document::setting_configuration` writes an inactive row's cell straight into the table
  (`SetConfigurations`, whose checks cover names, counts and ids, not cycles), and the app's cell
  (`configurations.rs`, `field::parameter_expression`) only parses and evaluates it against the
  live values, so with `a = 5 mm`, `b = a * 2` both configured, `b / 2` typed into another row's
  `a` cell is accepted, and switching to that row, from its radio, the palette or an export of
  every configuration, is refused as "This would make b depend on itself" with nothing pointing
  at the cell. The active row's cell goes through `SetParameterExpression` and is refused at once.
  Check the row's merged graph (the live expressions with that row's cells in place) for a cycle
  when a cell is set; pin it in `configurations_tests.rs` beside
  `values_using_each_other_swap_without_a_passing_cycle`.
- [medium · easy] Measure's mass cards give the centroid in world coordinates while Relative to
  names a coordinate system: `measure_panel::mass_card` and `total_card` format
  `properties.centroid` as it is (`MeasureContext` carries no `Relative`), while the "Centre of
  mass of <body>" reading card beside it goes through `Relative::reading`, so one panel shows two
  coordinates for one point, and the mass card's Keep this measurement along X, Y and Z
  (`Keepable::of_mass_row`, `positions(None)`) keeps a reading along the world's axes, against
  `app.md`'s "reads every position … in the chosen coordinate system". Map the centroid through
  the panel's `Relative` and pass the frame to `of_mass_row` (inertia stays along the model axes as
  the rule says); pin it beside
  `measure_reads_positions_and_offsets_in_the_chosen_coordinate_system`
  (`ui_tests/coordinate_systems.rs`).
- [low · easy] A configuration added while a configured body colour's feature makes no body can
  never be switched to once it makes one again: `live_settings_of` leaves out a column whose live
  setting is none, so `adding_configuration` and `duplicating_configuration` store the row
  without it, and `out_of_step` counts a row with no setting for a column as out of step
  (`row.setting(v) != Some(&live)`), so `SetActiveConfiguration` refuses it with
  `ConfigurationOutOfStep` and every-configuration export fails for it, while the cell reads
  Default and choosing Default is dropped as no change. Configure Extrude 2's colour, switch it to
  Add, add a configuration, switch it back to New body and switch to that configuration. Ignore
  columns a row holds no setting for, or fill them from the live values when the table is set.
- [low · easy] Deleting the active configuration switches to the first other row, not the next one
  `document.md` names: `deleting_configuration` takes `rows.iter().find(|other| other.id != id)`,
  so with M4, M6 and M8 and M6 active the model becomes M4. The one test deletes the first row,
  where the two agree. Take the row after the deleted one, else the one before.
- [low · easy] A feature created at a rollback bar inside a folder splits the folder in two of the
  same name: `TransactionBuilder::add_feature` inserts `Feature::new` (no group) at the bar, and
  nothing applies the membership rule moves follow (`grouping::membership_edits`), so with A and B
  grouped as "Group 1" and rolled back to A, a new sketch lands between them ungrouped, the tree
  draws two "Group 1" folders, and Rename group and Ungroup (`group_run`) reach one half. Give an
  inserted feature the group its neighbours at the bar share; pin it beside
  `moving_into_the_middle_of_a_group_joins_it_and_moving_out_leaves_it` (`grouping_tests.rs`).
- [low · easy] The tree's choice keeps features an undo removed: `PanelState::chosen` and
  `toggle_chosen` (`panels.rs`) never drop ids missing from the document, cleared only by a new
  session and by deletions made from the tree, so after pasting a feature (which chooses the copy)
  and pressing Ctrl+Z, Ctrl+clicking two rows and dragging one shows "That feature no longer
  exists" (`move_features`, `MissingFeature`) and moves nothing. Drop missing ids when the
  revision changes; pin it in `ui_tests/feature_tree_choice.rs`.
- [low · easy] Undoing back to the saved state of a model with a kept measurement leaves it marked
  unsaved: `Model::changed` judges `dirty` right after the undo, while the measured parameter
  still holds the reading `follow_readings` wrote for the undone revision, and the next
  evaluation writes the old reading back without judging it again (`dirty` is set only in
  `changed`, after a save and on load), so the title keeps its Unsaved pill and Close or Open asks
  to save a model equal to the saved one. Clear `dirty` when a follow brings the document back to
  the saved one (never set it there, as following never makes a model unsaved); pin it beside
  `a_measured_distance_is_kept_in_the_model_and_follows_upstream_edits`
  (`ui_tests/cancel_and_parameters.rs`).
- [low · easy] Clear face colours, and the selected faces' body-colour swatch, also drop the
  opacity and finish a STEP import gave faces: `body_appearance.rs` clears with
  `faces: Vec::new()` and `with_face_colours` with no colour filters the selected faces' entries
  out, though a `FaceColour` also carries `opacity` and `finish`, which no control sets, so an
  imported glass face turns opaque and plain and only Undo brings it back; repainting keeps them,
  and `app-look.md` says clearing does. Reset only the colour of entries holding an opacity or
  finish; pin it beside `selected_faces_take_a_colour_of_their_own_from_the_body_card`.
- [low · easy] The theme check never holds muted text on `sunken`, which the menu bar's Search
  commands label and every field's placeholder (egui's weak text on `text_edit_bg_color`) draw:
  `appearance::required_pairs` has `Text` on `Sunken` but not `TextMuted`, so a loaded theme with
  `"sunken": "#3c4250"` passes with those at about 3.9:1, against `ux.md`'s 4.5:1 for every panel
  text colour. Add the pair; pin it with the refusal tests in `themes.rs`.
- [low · easy] The Preferences guide page uses markup the guide does not read: `preferences.md`
  has a `### Your own themes` heading and a fenced JSON example, and `guide::parse` knows only
  `# `, `## ` and `- `, so the heading shows its hashes and the example runs into one paragraph
  starting "json {" that is no longer valid JSON if copied. Use `##` and code spans (or teach the
  parser fences) and test that no page has a line the parser does not read.


## Sketching

- [medium · easy] Mirror, patterns, extend and the sketch fillet treat inactive constraints as
  holding, against `sketch.md`'s rule that an inactive constraint no longer holds:
  `Sketch::constraints_using` returns every constraint, and `mirrored_directly` and
  `symmetric_partner` (`mirror.rs`) take a point held `Symmetric` only by an inactive constraint
  as already mirrored, so after deactivating a `Symmetric` between P and Q and moving Q, mirroring
  a line ending at P joins the copy to Q where it now stands, with no symmetry, while the preview
  (`mirror_image`) showed the true reflection; `held_on` (mirror, and `tie_turns` in
  `pattern.rs`) counts an inactive `Coincident` or `Midpoint` as holding a point on the line or
  centre, so no tie is added and the copy comes apart when dragged; `check_free_end`
  (`trim.rs`) refuses to extend an end held only by an inactive `Fix` or `Coincident`; and
  `merge_point` (`fillet.rs`) drops a moved constraint as a duplicate of an inactive one. Add an
  `active_constraints_using` and use it at each, with tests deactivating a `Symmetric`, a
  `Coincident` and a `Fix`.
- [medium · easy] Taking back the last point while Draw symmetrically is on ends the chain instead
  of undoing the segment: `Drawing::remove_last` undoes a segment only when the newest undo label
  is the one the segment was stored with ("Draw line", `segment_label`), but
  `symmetric_drawing::mirrored` renames the transaction "Draw line symmetrically", so Backspace,
  Delete or Take back the last point after drawing (10, 0) to (20, 10) about the vertical axis
  leaves both halves drawn and ends the chain, against `app-sketching.md`'s "taking back undoes
  the last segment when it is the newest undo step". Store the label the transaction will carry;
  pin it in `ui_tests/sketch_symmetry.rs`.
- [medium · easy] Smart dimension on a slanted line and an arc sharing an end never adds their
  angle: `dimensioning::fitting` gives `Angle` for them (`Sketch::angle_vertex`), but
  `offset_ends` counts `(Line, Arc)` as having horizontal and vertical offsets, so
  `awaits_placement` holds the second click for a placing one, the prompt offers horizontal and
  vertical distances, and `placed`, with no arm for the pair, adds a horizontal, vertical or
  aligned distance to the arc's centre; only Enter gives the angle. Level and upright lines are
  unaffected. Make `offset_ends` give none when the two share an end, as `fitting` decides first;
  pin it beside `a_point_or_circle_off_a_slanted_line_and_two_circles_wait_to_be_placed`.
- [medium · hard] Offset keeps one offset curve per original, so an offset that would drop a curve
  is refused: a 30×20 rounded rectangle (2 mm tangent corner arcs) offset inward by 2 mm or more is
  `Collapses` on its first arc, where an arc shrinking to nothing should leave a sharp corner and
  the lines meet beyond it, and an outline with a notch narrower than twice the distance (a 30×20
  rectangle with a 2 mm wide, 2 mm deep notch in its top, offset outward 1.5 mm) is `UsedUp` on a
  notch wall, where the notch should close up. The inner wall of a moulded or printed box drawn as
  its outline's offset by the wall thickness meets both. `Chain::outline` (`offset.rs`) would drop
  a vanishing arc, or the run of curves a closing notch uses up, and re-meet the neighbours,
  leaving out the dropped curves' constraints as trim does.
- [low · easy] Rotate, Scale and Move selected geometry act on the positions taken when their
  field opened: `sketch_drag::Transforming` and `Moving` hold the points as they stood then, the
  waiting field (focus lost without Enter) keeps them (`ViewportState::typed_owner` follows only
  the sketch and tool), and Enter sends targets worked out from those positions, so a drag of the
  same geometry or an undo made while the field waited is silently undone by the turn, scale or
  move. Keep the revision they were taken at and close the field when it moves on, or take the
  points afresh at Enter; pin it beside
  `the_sketch_bar_selects_everything_and_moves_it_to_a_typed_point`.
- [low · easy] Rotating or scaling a lone ellipse moves it: `sketch_drag::extent_centre` bounds an
  ellipse by its centre point, its one major-axis end and a square of its minor radius, never the
  opposite end of the major axis, so an ellipse centred at the origin with its major end at
  (10, 0) and minor radius 4 turns about (3, 0) instead of its centre, against
  `app-sketching.md`'s "the centre of the selection's extent". Count both ends of the major axis
  (or the true extent) for ellipses and elliptical arcs; pin it beside
  `a_selection_turns_about_a_lone_selected_point_or_else_about_its_centre`.

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
- [medium · easy] A hostile STEP file can exhaust memory in the structure and presentation passes,
  whose bounds are per item rather than per file: `Structure::walk` (`read/structure.rs`) caps
  placements at `MAX_INSTANCES` (10,000) per representation but memoises every walked
  representation's whole `Placements` in `walked`, each placement with its own occurrence name
  `String` and a path `Vec` growing about two ids per level, so layers of representations that all
  have the next layer's as parents leave hundreds of nodes holding 10,000 placements each from a
  file of a few hundred KB (`structure.rs`'s many-copies test asserts only time); and
  `presentation.rs` clones a layer's name for every item it lists and a styling context `Vec` for
  every repeated style reference (`instance`), as `product_names` clones a product's name per
  representation, so one long name listed against many ids multiplies into tens of GB. Share
  paths and names (`Arc<[u64]>`, `Arc<str>`) and spend one file-wide placement budget, as `Work`
  bounds the parts.
- [medium · medium] The VZ330 assembly (an Autodesk export at 0.01 mm, 3,894 copies) leaves 63
  parts out, 43 of them as "a vertex 0.00001 to 0.0015 mm away from the end of its edge" (the M4
  T-nuts, "Part 9 (1)" to "Part 14 (1)", "CPAP duct 18mm (1)"), all well within the file's
  precision; the rest as edges that cannot be followed across a face (8), faces too far apart (6),
  meshes that fail the validity check (4), overlapping shells (1) and an edge of no length (1).
  A vertex that far off its edge's end within the declared precision should move onto the edge's
  end, as vertices off their faces already do (`step-read.md`, "Geometry and healing"). Reproduce
  with `STEP_CORPUS` on the file (about 70 s in release).
- [low · hard] A face whose surface cannot be read is left out (`step-read.md`, "Unreadable
  faces"), but from the outer shell that turns the whole body into flat facets, since the kernel
  holds only closed solids, and a lost face with holes is not closed at all; an edge whose curve
  cannot be read still loses the whole body. An offset that folds anywhere in its basis's domain
  is refused even where the face's own region is clear: fit over the region the face uses.


## Viewer

- [medium · medium] High contrast holds edges and face highlights to 3:1 only on the default body
  colour: `scene::body_color` draws a sound body in its painted colour in every palette and
  `face_looks` keeps face colours, while `scene_palette.rs`'s
  `high_contrast_edges_and_highlighted_faces_stand_out_on_a_body` checks `body_edge`,
  `faces.selected` and `faces.hovered` against `DEFAULT_COLOUR` alone, so on the offered Charcoal
  swatch edges read about 1.9:1 and a selected face about 1.3:1, the selection invisible (a black
  or dark grey STEP import the same), and in dark high contrast selected edges on the White swatch
  about 2:1, against `ux.md`'s "edges and highlighted faces to 3:1 on a body". Draw bodies in the
  tested body colour in high contrast, as finishes already are dropped there, or choose edge and
  highlight colours by the body's luminance, and extend the test to every swatch.
- [low · easy] Fit view with a modifying feature's row chosen frames everything, and hovering or
  choosing that row lights nothing: `BuiltScene::bounds_of_features` (`scene.rs`) looks rows up in
  the body meshes, keyed by the feature that made each body, so a fillet, shell, hole, combine or
  cut extrusion row gives no bounds and Fit falls back to the whole model, against "frames the
  chosen rows' bodies" (`app-look.md`, the Navigation guide page); `Highlight::is_of_chosen_row`
  compares the same raw ids. Map each row through `Feature::body` first, as
  `visibility::hideable_rows` does; pin it beside
  `fitting_the_view_frames_the_rows_chosen_in_the_tree_when_nothing_is_selected`.
- [low · medium · blocked by: wgpu's GL backend] On GL and other devices without texture view
  formats the multisample resolve still averages in gamma space. A resolve of its own (a pass
  reading the samples through a `texture_multisampled_2d` and averaging them in linear light) was
  tried: it matches the view-format resolve on Vulkan, but wgpu 30's GL backend binds a
  multisampled texture as `TEXTURE_2D` (`gles::Texture::get_info_from_desc` never chooses
  `TEXTURE_2D_MULTISAMPLE`), so every sample reads as zero there and the frame comes out black. It
  needs that fixed in wgpu, or a GL-only blit resolve into an sRGB texture.

