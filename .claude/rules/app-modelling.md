---
paths:
  - "crates/caditor/src/solid_tools.rs"
  - "crates/caditor/src/body_reach.rs"
  - "crates/caditor/src/solid_panel.rs"
  - "crates/caditor/src/sweep_tools.rs"
  - "crates/caditor/src/blend_tools.rs"
  - "crates/caditor/src/blend_panel.rs"
  - "crates/caditor/src/shell_tools.rs"
  - "crates/caditor/src/shell_panel.rs"
  - "crates/caditor/src/offset_face_tools.rs"
  - "crates/caditor/src/offset_face_panel.rs"
  - "crates/caditor/src/split_face_tools.rs"
  - "crates/caditor/src/split_face_panel.rs"
  - "crates/caditor/src/primitive_tools.rs"
  - "crates/caditor/src/primitive_panel.rs"
  - "crates/caditor/src/combine_tools.rs"
  - "crates/caditor/src/combine_panel.rs"
  - "crates/caditor/src/move_tools.rs"
  - "crates/caditor/src/hole_tools.rs"
  - "crates/caditor/src/hole_panel.rs"
  - "crates/caditor/src/hole_placement.rs"
  - "crates/caditor/src/thread_tools.rs"
  - "crates/caditor/src/thread_panel.rs"
  - "crates/caditor/src/move_panel.rs"
  - "crates/caditor/src/move_manipulator.rs"
  - "crates/caditor/src/manipulator.rs"
  - "crates/caditor/src/reach_handles.rs"
  - "crates/caditor/src/mirror_tools.rs"
  - "crates/caditor/src/mirror_panel.rs"
  - "crates/caditor/src/scale_tools.rs"
  - "crates/caditor/src/scale_panel.rs"
  - "crates/caditor/src/scale_model.rs"
  - "crates/caditor/src/configurations.rs"
  - "crates/caditor/src/datum_tools.rs"
  - "crates/caditor/src/datum_panel.rs"
  - "crates/caditor/src/pattern_tools.rs"
  - "crates/caditor/src/pattern_panel.rs"
  - "crates/caditor/src/sketch_pattern_tools.rs"
  - "crates/caditor/src/feature_fields.rs"
  - "crates/caditor/src/reference_picking.rs"
  - "crates/caditor/src/visibility.rs"
  - "crates/caditor/src/sketch_placement.rs"
  - "crates/caditor/src/new_sketch_panel.rs"
  - "crates/caditor/src/plane_selector.rs"
  - "crates/caditor/src/reference_rows.rs"
  - "crates/caditor/src/principal_tree.rs"
  - "crates/caditor/src/feature_tree.rs"
  - "crates/caditor/src/reversing.rs"
  - "crates/caditor/src/body_selection.rs"
  - "crates/caditor/src/handle_snap.rs"
  - "crates/caditor/src/hole_handles.rs"
  - "crates/caditor/src/hole_on_curve.rs"
  - "crates/caditor/src/hole_placement/**"
  - "crates/caditor/src/last_values.rs"
  - "crates/caditor/src/mate_panel.rs"
  - "crates/caditor/src/mate_tools.rs"
  - "crates/caditor/src/place_handles.rs"
  - "crates/caditor/src/removal.rs"
  - "crates/caditor/src/split_panel.rs"
  - "crates/caditor/src/split_tools.rs"
  - "crates/caditor/src/turn_handles.rs"
  - "crates/caditor/src/value_gauges.rs"
  - "crates/caditor/src/value_handles.rs"
  - "crates/caditor/src/projecting.rs"
  - "crates/caditor/src/similar.rs"
  - "crates/caditor/src/image_tools.rs"
  - "crates/caditor/src/image_panel.rs"
  - "crates/caditor/src/image_textures.rs"
---

# Modelling tools in the app

Tools without a ribbon button (it stays one row) live in the Model menu and the palette: Offset face,
Split face, the primitives, Thread, Copy body, Split body, Mate body, Mirror faces, Curve and Point
pattern, Coordinate system, Scale model and Configurations. Sweep has a button in the Solid group
and a Model menu row, and is in the right-click menu of selected sketch curves
(`view_menu::SKETCH_TOOLS`). Panel rows, keys and wordings are in the
`*_panel.rs` files; this file holds the behaviour behind them.

## Preview while open

- A feature that makes or changes a body, while open, draws that body see-through at `PREVIEW_ALPHA`
  (`scene::previewed_bodies`), faces still picked; solid when closed or while choosing in the view. A
  feature choosing nothing on the state before it (move, combine) draws its result solid and the body
  before it at `GHOST_ALPHA`, never picked (`scene::OpenView::Ghost`).
- An open fillet or chamfer that computed (`FeatureState::UpToDate`) draws its result with unpicked
  faces and edges, and over it the edges of the body before it, still `Pickable::BlendEdge`
  (`OpenView::Result`, `Builder::open_result`); hidden ones stay hidden since unpicked faces write
  depth in the pick pass. An open shell that computed draws its hollow result likewise, faces carried
  from the body before as `Pickable::ShellFace`, opened ones in the selected colour, and over the
  opening the opened faces of the body before, see-through and still picked (`Builder::opened_faces`),
  so a click on the opening closes it. A failed or pending blend or shell shows the body before it
  (`OpenView::Before`).
- Material-removing features (extrusion or revolve removing, hole) show the body solid as cut and only
  their tools (`Evaluation::cuts`) in `ScenePalette::cut_preview` over everything
  (`Scene::overlay_meshes`, edges on the front layer), never picked.
- Typing in any open feature's value field previews before Enter: every value row is
  `feature_fields::expression_row_drafting`; valid text becomes `Action::Preview` with the transaction
  it would commit, never applied. `Model` applies it to a copy of the document (`DraftPreview`, dropped
  when the feature closes, the text turns invalid, Escape or the field is left unchanged; replaced
  whenever the feature's kind or the transaction differs). The copy keeps its parameter values
  (`Model::shown_parameters`), so a value named while typed (`h = 30`) moves arrows and a move's
  preview too.
  - The copy is computed as a draft (`document-recompute.md`); the draft's state of the body the
    feature makes or changes (`Feature::body`, `Model::draft_body_result`) is drawn in that body's
    place once meshed (`BodyMeshes::draft`, `OpenDraft`). A draft replacing one of the same feature and
    revision keeps the evaluation it replaces until its own arrives (`Model::preview`), so a drag never
    falls back to the committed body between steps. A removal's tools are the draft's
    (`Model::draft_cuts`).
  - A feature making no body (open datum `scene::drafted`, open thread `placed_threads`) is drawn from
    `Model::draft_evaluation_of`, handed to the scene as `Sources::draft` with `Revisions::draft` and
    `Model::draft_generation` in its key.
  - A move is not recomputed: `Model::draft_placement` places the drawn body
    (`MeshInstance::placement`, edges and vertices moved on the CPU).
  - Entering the value commits, and the draft stays shown (held) until the model has recomputed and
    meshed. A draft whose feature fails shows the body before it (a datum or thread its committed
    result), and the focused row shows the failure as an error callout
    (`feature_fields::draft_failure_row`, `Model::draft_failure`).

## What a command acts on

- A command takes the selection's target or refuses in words; it never swaps in a body, plane, sketch
  or axis the selection does not name. `Selection` keeps pick order beside its sorted set
  (`Selection::in_pick_order`; a bulk pick enters sorted).
- A body is named by selected faces, edges or vertices (`body_selection::bodies_in`) or by rows chosen
  in the tree (`body_selection::tree_bodies`: a body row, or a feature row's body). The later of the
  two wins, since choosing in the view clears the tree's (`move_tools::chosen_body`). Move, Copy,
  Mirror, Split, Scale, Pattern, Combine (two tree bodies), Remove body, Rename body and Body colour
  take it so; Measure and Interference take every chosen row's body. Offers carry the tree's bodies in
  their basis.
- Several candidates for one slot (two planes, two axes, curves of two sketches, faces of two bodies)
  are refused naming the conflict (`datum_tools::only_plane`, `only_axis`, `chosen_plane`, `SEVERAL_*`),
  except where pick order decides: a revolve turns about the line or axis picked last
  (`solid_tools::with_model_axis`), and a pattern's direction ignores edges and faces of the patterned
  body when an axis outside it is selected.
- What a command drops from a mixed selection is said in a notice (`body_selection::left_out_words`;
  the body card: `body_appearance::other_faces_note`).
- The tree's selection-taking commands (Place sketch, Use selected axis, Up to selected, Start at
  selected, Mirror across selected, Split along selected, datum and pattern Use selected) act on the
  open feature, else the tree's row (`feature_tree::feature_commands`); the rest act on
  `feature_tree::current_feature`.

## Where new features go

- Every tool creates its feature through `TransactionBuilder::add_feature`, so with the rollback bar
  above the end new features land right above it (`document.md`). Defaults come from the model as of
  the bar: last sketch and body are the last active ones (`Document::active_features`); faces, edges
  and datums are captured where the bar stands (`Document::bar_index`).
- A suppressed or rolled-back feature cannot be opened or edited: `SketchEditing` refuses it,
  `SketchEditing::sync` closes an edited sketch or open feature that becomes one, and the tree's Edit
  says why.
- Starting values are the `DEFAULT_*` constants of each `*_tools.rs`, except those the user has set
  before: `last_values::Remembered` names the slots (fillet and chamfer size, chamfer form and second
  distance or angle, shell thickness, hole diameter and blind depth, extrusion distance, linear and
  circular pattern counts). `app::perform` hands every applied transaction to `last_values::changes`,
  which keeps slots a `SetFeatureKind` changed (a tool's insert changes nothing, so open-and-cancel
  never overwrites), and `PreferencesCommand::RememberLast` stores their text with the document's
  parameter names then (a feature's own named parameter stores its expression). The creators
  (`blend_tools::create`, `shell_tools::create`, `solid_tools::create`, `hole_tools::new_hole`,
  `pattern_tools::kind_for`) take a `last_values::Starts` from `Model::last_values` against the current
  document (`Starts::of`): a text that no longer parses, evaluates, has the slot's dimension or is
  above zero gives way to the default for that slot alone.

## Cancelling an open feature

- Escape, Enter and the checkmark keep an open feature's changes and close it. Cancel
  (`Command::CancelFeature`, no default key; palette, Model › Features, the open row's cross, its row
  menu) closes it and takes back what was done since it opened, through `EditingCommand::CancelSolid`.
- Opening records `editing::Opened`: the feature, the `UndoMark` from when the batch of actions that
  opened it began (`app::perform` hands it to `SketchEditing::perform_after`, so a tool's `Apply`
  before `OpenSolid`, and a Choose in the view adding selected edges, count as the opening), and
  whether a step of that batch inserted the feature (`added`: Cancel removes it, worded "Cancel the new
  <name>"). Opening the already open feature keeps the record.
- `SketchEditing::cancelling` checks before undoing: `Model::undo_steps_since` the mark must still lead
  back to it (`Editor::undo_steps_since`, `document.md`), else `CancelRefusal::HistoryChanged`; every
  step since must touch only the feature and parameters it owns (`Transaction::touched`; the adding
  step excepted), else `ChangedElsewhere` naming the step. A refusal is a notice leaving feature and
  model as they are. Otherwise it performs that many ordinary journaled `Action::Undo`s (Redo brings
  them back) and closes; with nothing changed it only closes.
- Opening a feature while a sketch is edited, or entering a sketch or starting a new one while a
  feature is open, closes it with a notice and, when the opening batch changed the document, an
  Undo that returns to it (`app-sketching.md`, Editing context). Escape on an open feature still
  keeps its changes and closes it.
- A tool whose body has no shape yet while a recompute runs (Move, Mirror, Combine, Fillet, Chamfer,
  Shell, Offset faces, Split face, Thread, Sweep) is queued, not refused (`app-input.md`, Queued
  actions), and runs when the shape arrives.

## Feature panels

- Every panel runs in one order: its shape switch (Extent, Shape) or, for a shell or datum, a muted
  description (a panel with both, a hole, puts the description first); then references, values and
  options; then Body. Small exclusive choices are `segmented`; longer lists stay combo boxes. A hole
  runs Shape, Sketch, Placed on before its sizes (a slot's Length and Angle with the values), and a
  move's Make a copy comes before Body.
- Every value row (`feature_fields::Quantity` carries the feature) and sketch dimension is a
  `field::NamedField`: typing `name = expression` names the value (`document.md`, Model parameters). A
  new name is one transaction inserting a parameter owned by the field (`ParameterOwner::Feature` with
  the row's caption, or `Dimension`) holding what was typed (a plain number taking the field's unit),
  with the field set to it: the panel's change is built with a marker (the value negated twice) and
  `Transaction::substituting` puts the reference where the marker went, so no panel knows about naming;
  a change that drops the marker refuses as `NOT_NAMEABLE`. A field holding its own named value shows
  `name = expression` and previews that expression; `name = …` typed there renames or edits the
  parameter (one "Edit <name>" change), a plain expression unnames it (`Document::releasing`).
- A mode switch keeps named values with their fields: `solid_tools::edit`, `blend_tools::edit`,
  `hole_tools::edit` and `primitive_tools::edit` pass every change through `Document::carrying_owners`
  with the new kind's captioned fields (`solid_panel::captioned`, `blend_panel::captioned` with
  `size_caption`; none for holes and primitives, whose switches only drop values). A switch copying a
  value into a second field (Two sides, Two distances) copies it unnamed (`field::copied_value`), so
  only one field holds the name.
- `feature_fields.rs` holds the shared rows and `Rule` wordings; panels use them rather than their own
  spacing or text. An unset reference reads None chosen; a missing body or sketch reads Missing with
  the warning colour and icon; a refused change leaves the notice "<feature name> was not changed:
  <reason>".
- A reference row offers Use selected when the selection gives a change, else Choose in the view, which
  opens the feature and starts choosing that slot (`reference_picking::Picking`, held by
  `SketchEditing` and published to panels through egui temp data each frame, since the tree's call
  signatures carry no editing state). While choosing, a click (or the activated keyboard highlight) is
  tried as the selection for that slot: it applies and stops, or leaves a warning notice with the
  reason. A datum axis keeps a first plane pending until a second crosses it. Escape stops choosing
  before anything else; opening another feature or closing this one stops it too. While a slot is
  being chosen its row shows the prompt and Stop choosing (`feature_fields::choosing_prompt`, shared
  by every reference row, the shell's Own thickness and the hole's anchors and Add another hole).
  Lists of regions, blend edges and shell, offset and split faces use `feature_fields::list_in_view`:
  Choose in the view (`widgets::choose_in_view`, the shared `CHOOSE_IN_VIEW` wording) opens the
  feature so clicks toggle entries, and while it is open the row shows the prompt and Stop choosing,
  which closes the feature keeping its changes, as Escape does.
- A slot that goes on choosing after a click (`Slot::keeps_choosing`: the sweep's path, the shell's
  own thickness faces, Add another hole) and every list row while open show an info pill
  `KEEPS_CHOOSING` ("Keeps choosing until Esc") beside Stop; a single-click slot has none.
- Axis references are one row, `feature_fields::axis_row` (Revolve, Pattern, Mate, Move): a combo of
  what the feature can use, with the picker under it, reading None chosen when unset.
  `model_axis_choices` lists the principal, datum and coordinate system axes above the feature
  (`pattern_tools::listed_axes`) plus the current one; a revolve adds its sketch's axes and lines and
  offers a model axis off the sketch plane disabled with the reason (`caditor_document::along_plane`);
  a mate lists them only for the axis it mates onto (`mate_tools::target_axis_change`), the moving
  axis being one of its body's, picked only.
- Plane references are one row, `plane_selector::row` (Extrude and Revolve's Up to and Starts at, a
  hole's Up to, Datum plane's Starts from, a coordinate system's XY plane, a primitive's Placed on, a
  mate's Onto or Rests on a plane, Mirror's Mirror across): a combo of every plane the feature can
  use (`datum_tools::listed_planes`: principal planes, then datum planes, then coordinate system
  planes made before it and not suppressed) with the picker under it, so a plane is chosen from the
  list, selected or clicked. The combo shows the current plane, a face included although a face is
  never listed (faces are chosen in the view), and reads Choose a plane while a slot is being chosen
  with nothing set. Choosing from the list is the same one change Use selected makes (a panel passes
  `plane_selector::row` a closure building its `Transaction`, so every refusal is the panel's own)
  and stops choosing the slot. `plane_selector::combo` is the bare list, shared with the New sketch
  card and the section view.
- A feature waiting for an input (`FeatureState::Waiting`, `document-recompute.md`) is no failure:
  its tree row shows `icons::AWAITING_INPUT` in the info tone with `MissingInput::waiting_words` as
  hint, and an info callout with the words, `MissingInput::prompt` and, while closed, Choose in the
  view starting the slot (`Slot::PatternDirection`, `Slot::SweepPath`) or opening the feature for a
  list (`feature_tree::waiting`). State words take "waiting" for the filter.

## Extrude and Revolve

- Extrude with flat faces of one body in one plane selected (no sketch edited, no sketch geometry
  selected) extrudes them: one transaction creates a hidden sketch on the first face picked holding all
  their boundary edges projected, each shared edge once (`projecting::face_projections`), and an
  extrusion of it added to that body outward, with a notice naming the sketch
  (`solid_tools::create_on_faces`). Revolve does the same with the axis, straight edge or round face
  picked last (`FaceProfile::axis`), a full turn as `RevolveAxis::Model`, refusing faces without one
  (`NO_AXIS_FOR_FACE`). A curved face, faces of two bodies or two planes are refused
  (`solid_tools::faces_to_sweep`, via `Offers::extrude_faces` and `revolve_faces`). A selected face
  never falls through to the last sketch.
- Otherwise they take the edited sketch, else the selection's sketch (curves of two sketches refused,
  `SEVERAL_SKETCHES`); only with nothing selected (for Revolve, nothing but the axis) do they guess the
  open extrusion's sketch, else the last shown sketch, and only while it has an even-depth region no
  feature sweeps (`solid_tools::has_unswept_regions`; a Hole guesses a sketch no hole drills,
  `Guess::Drill`). A guessed sketch some of whose regions are swept chooses only the regions left
  (`solid_tools::regions_left`), so parts of one sketch extrude as separate features. A selection
  holding no sketch is refused (`NOTHING_TO_EXTRUDE`, `NOTHING_TO_REVOLVE`). The revolve axis is the
  line, sketch axis, principal axis, datum axis, straight edge or round face picked last; the panel's
  axis picker refuses several.
- A new feature adds to the body whose faces are selected with its sketch (`SweepSource::body`); an
  extrusion of a sketch on a face of a body still standing (`solid_tools::face_body`) instead cuts that
  body, reversed into it, unless faces of another body are selected. Otherwise it adds to the body it
  reaches, else makes a new body (`solid_tools::default_operation`, `body_reach::reached_body`).
  - Reaching is decided once, at creation, on the UI thread from the evaluation shown: only shown
    bodies standing at the bar (`Document::bodies_standing`, so never one a Combine consumed) whose
    `SolidResult::bounding_box` meets a bound of the sweep (outline widened by a wall's thickness and a
    slack, swept by the distance or turned about the axis) are looked at, so with nothing near no
    kernel work is done. For those the tool is swept as the feature will be and checked with
    `caditor_kernel::interference` under a `REACH_BUDGET` deadline through the kernel interrupt. An
    overlap ranks above a touch, several overlaps go to the largest volume, ties (within `SAME_VOLUME`)
    and touches to the later body in the tree; a check that fails or times out counts as a touch.
    Deciding before the feature exists keeps creation one undoable step and the preview in its final
    colour from the first frame.
  - The panel's Body list is `Document::bodies_before`. `SketchEditing` holds at most one open solid
    feature, never with an edited sketch; `editing::Context` carries both to the scene and
    availability checks.
- With regions selected (`Pickable::SketchRegion`, `app-sketching.md`), a new extrusion or revolve
  chooses them (`solid_tools::selected_regions`). With curves of the sketch selected that close up, it
  chooses the regions whose anchor lies inside them, even-odd, adding them to any selected regions
  (`solid_tools::with_selected_outline`, a revolve leaving its axis line out), so a rectangle
  overlapping a circle sweeps the rectangle and their overlap. Curves that do not close and no selected
  region sweep every region.
- An open feature's tree row is its property panel; every change is one `SetFeatureKind`, checked
  before it is offered. Regions are fills on the front layer, so the see-through preview never covers
  them; `Pickable::Region` clicks add or leave out, turning `RegionChoice::All` into explicit keys.
  `Command::ClearChosenRegions` (Alt+Shift+R, the panel's Clear, palette; `solid_tools::clear_regions`,
  on the open feature else the tree's row) chooses none in one undoable change; the feature then waits
  for regions (`MissingInput::Regions`). Double-clicking a face opens the feature that made it.
- Each extrusion side has an end kind; one that cannot apply is offered disabled with the reason on
  hover. Up to face takes the selected face or plane captured where the extrusion sits in the tree
  (`solid_panel::selected_target`, through `datum_tools::plane_reference`), or one selected curved face
  as an `UpToSurface` end (`sketch_placement::surface_at`), each refusal saying why; with none usable
  it starts choosing in the view.
  - A removing or intersecting extrusion whose end distance reaches past `FAR_PAST_THE_BODY_TIMES` the
    box diagonal of the bodies it cuts, as seen at the extrusion, gets an info callout
    (`solid_panel::FAR_PAST_THE_BODY`) whose Cut through all (`CUT_THROUGH_ALL`) switches that end to
    Through all in one change.
  - An end Up to next or Up to face has an offset field (`solid_panel::END_OFFSET`, key
    `<side>-offset`; any sign, negative stopping short, zero clearing), kept when the end switches
    between the two or takes another face.
- Direction row (`solid_panel::SQUARE`, `ALONG`): Along takes the one selected edge, axis, round face or
  sketch line (`datum_tools::only_axis`), else chooses one in the view (`Slot::ExtrudeDirection`), with
  a button back to square; an extrusion along a direction has no reach arrows.
- An extrusion's and revolve's Start row (`solid_panel::start_rows`): the sketch plane with a Start
  offset field (key `start`, zero clears), or Face or plane, taken like an end
  (`solid_panel::start_change`, `Slot::StartPlane`). An extrusion then has a Taper field (key `taper`,
  below `MAX_TAPER_DEGREES` either way, `Rule::Taper`; zero clears).
- A Fill switch (Solid, Thin wall) under Regions turns the sweep into a wall of the sketch's curves
  (`solid_tools::with_wall`, `DEFAULT_WALL_THICKNESS` centred; a revolve's Profile switch is hidden and
  its side cleared), with Thickness (key `wall-thickness`) and Wall (Inside, Outside, Centred); Regions
  then only says the wall follows every curve. A new extrusion or revolve of every region of a sketch
  whose curves enclose nothing starts as a thin wall, and such an unused sketch is one Extrude guesses
  (`solid_tools::is_open_profile`).
- While an extrusion is open (nothing being chosen in the view), each end set by a distance has a reach
  arrow (`reach_handles.rs`, `Handle::Reach`) at that end over the middle of its sketch's bounds,
  measured from where the extrusion starts (`reach_handles::start_offset`; for a start face or plane
  `displayed_start_offset`, resolved on the body as the feature sees it), pointing the way that side
  runs, in `ScenePalette::handle` at the move arrows' size, left out when pointing at the eye. A
  symmetric extrusion has one, on its forward side, whose drag grows the whole distance twice as fast.
  It drags like a move arrow (`manipulator.rs` holds both behind `Manipulator` and `Manipulating`): in
  steps, snapping to geometry (Ctrl drags freely; Value handles below), previewed with the distance
  beside the pointer, committed on release as one edit setting a measured distance of at least one
  step, dragging back through the start flips it (below). Arrows follow a typed distance's preview
  (`Model::draft_kind`, `Model::shown_parameters`).
- While a revolve turning by angles is open, each end set by an angle has an arrow (`turn_handles.rs`,
  `Handle::Revolve`, `TurnEnd`) at the middle of its sketch's bounds turned about the axis to that end:
  positive angles sweep toward the sketch normal, a reversed one-sided turn the other way, a symmetric
  turn shows its forward half and drags the total twice as fast, two angles an arrow each. A drag
  follows the pointer's angle about the axis in the plane square to it, unwrapped past half a turn, in
  steps of `STEP_DEGREES` (Ctrl free) or snapping onto a point (Value handles below) between one step
  and what the full turn leaves beside the other angle. A revolve starting off its sketch plane turns profile and axis shifted along the normal by the
  start, so its arrows stand and turn there (`start_offset`).
- A negative one-sided extrusion distance flips the extrusion instead of being refused
  (`Rule::FlipWhenNegative`, which refuses only zero; every other extrusion distance keeps
  `Rule::AboveZero`, so symmetric and two-sided sides still say to enter a value above zero). The
  field's value is stored as its magnitude (`reversing::unsigned`: a leading minus is dropped, `-5` to
  `5`, `-depth` to `depth`, any other expression wrapped as `-(width - 30 mm)`, never baked to a
  number, so parameters and units are kept) and the extrusion reversed (`reversing::flipped_extrude`):
  an Add becomes a Remove of the same body and a Remove an Add, a New body or an Intersect only reverses.
  It is one "Edit <feature>" change with an info notice naming what happened
  (`reversing::announcement`, "<feature>: reversed and switched to Remove; a negative distance cuts the
  other way"), and the typed preview shows the same before Enter since the draft is that transaction.
  Three places commit a typed distance and all flip: the panel's Distance field
  (`solid_panel::end_rows`), the value label on the model (`feature_values::change`) and a value typed
  over the reach arrow (`reach_handles::typed`), the last also writing the magnitude into a named
  distance's parameter. The flip happens once, when the value is typed; a parameter later turning the
  distance negative does not flip again but fails the feature with the kernel's message to enter a
  distance above zero or flip the direction. The reach arrow dragged back through its start flips the
  same way (`ReachDrag::limited` keeps the sign of a one-sided drag, the readout adds "reversed and
  switched to Remove"); the other ends stay at least one step. A revolve's angle has no such flip:
  its rules (`Rule::Turn`) refuse a negative angle and Reverse direction turns it the other way.
- A drag never drops a name a field holds (`manipulator::Held`, read from the field's caption as its
  `ParameterOwner`, captions shared with the panels: `solid_panel::DISTANCE` and others,
  `move_panel::distance_caption`, `turn_caption`, `ANGLE`). A plain number is replaced by the measured
  value; the field's own named value (`h = 20`) keeps the reference and the drag edits the parameter's
  expression (`Edit::SetParameterExpression`, same "Edit <feature>" change,
  `manipulator::keeping_names`); a value using any other parameter, directly or through its named value
  (`h = depth * 2`), is driven: its handle does not drag and its hover says what drives it
  (`Manipulator::driven`, `words`). A ring that rewrites all three turns is driven when any is.
- Reverse the direction (`Command::ReverseDirection`, palette, no default key; `reversing::reverse_change`,
  in `feature_tree::feature_commands`' changes) acts on the open feature, else the tree's current one,
  doing what its panel's reverse control does (an extrusion's one-sided end, a revolve's one-sided or
  up-to turn, a hole, a primitive not centred on its place, a pattern's first direction, a mate's
  facing or side, a split's kept side) as one "Reverse <feature>" change. An extrusion or revolve
  running both ways and a full turn are refused saying which Extent to choose, an angle or point mate
  saying it has no side (`MATE_HAS_NO_SIDE`); other kinds `NOTHING_TO_REVERSE`.
- A revolve can also take two angles (refusing a pair turning more than a full turn), or turn Up to
  face (`solid_panel::TURN_UP_TO`): the selected flat face or plane, taken like an extrusion's end, else
  choosing in the view (`Slot::RevolveTarget`). Its Profile switch (Whole, One side) keeps one side of
  the axis, starting from the side holding more of the chosen regions' area
  (`solid_panel::larger_side`), with Keep the other side of the axis to swap.
- A removal's panel lists under Body the other bodies it Also cuts, each with a stop button, and an Add
  another body list of `bodies_before` not yet cut; switching away from Remove from body drops them
  (`solid_tools::with_operation`).

## Sweep

- `Command::Sweep` (`sweep_tools.rs`, `Offers::sweep`) makes a `SolidFeature::Sweep`, so it opens,
  chooses regions, previews, cancels and takes its Body rows as an extrusion does.
- `sweep_tools::source` reads the selection in pick order: the profile is the edited sketch, else
  a selected region's sketch, else the first sketch of selected curves that has regions when
  another sketch's curves or edges are selected too, else the one sketch of selected curves when it
  has regions, else the last shown sketch with regions no feature sweeps (`guessed_profile`); its
  selected regions or closing curves choose regions as an extrusion's do
  (`solid_tools::with_selected_outline`). The path is the selected curves of one other sketch
  (`is_path_curve`: any curve but a reference, construction ones too) or the selected edges of one
  body; mixing them, curves of two other sketches or edges of two bodies is refused in words.
- A new sweep adds to the path edges' body, else the body of selected faces, else is a new body;
  it hides the profile's sketch but not the path's, whose curves stay clickable. With no path it
  opens choosing one (`Slot::SweepPath`).
- Panel: Sketch, Regions, Path (`path_words`, "2 curves of Path"; Use selected replaces it,
  `selected_change`; Choose in the view; the cross clears it), Orientation (`SQUARE_TO_PATH`,
  `PARALLEL`, a segmented choice that becomes a list when narrow) and Body. While choosing the
  path each click toggles a curve or edge (`toggled_path`: the same sketch or body toggles, another
  starts a new path) and choosing goes on until Escape (`reference_picking::click` keeps every
  `Slot::keeps_choosing` slot). With no path it waits (`MissingInput::Path`). Edges are captured in the body as the sweep sees it
  (`Evaluation::body_result_seen_by`, else as shown).
- It has no arrows or values; Reverse direction refuses it (it follows its path from the profile)
  and `body_reach` decides nothing for it.

## Fillets, chamfers and shells

- Fillet, Chamfer and Shell take the selected edges (or faces) of one body and create a feature
  that opens. Fillet and Chamfer also take selected faces, each standing for its boundary edges, and a
  lone body chosen in the tree for every edge of it (`blend_tools::selected_edges`, `Offers::blend`, an
  `EdgeSource` of edges picked one by one and `GroupSource`s; seams left out, each edge once, an edge on
  a selected face left to the face). The face (`FaceReference`) or body is stored as an `EdgeGroup` and
  expanded on every recompute (`document.md`), so an edge a later change adds is rounded too; the panel
  lists it as "All edges of <face>" or "All edges of <body>" (`blend_panel::group_row`). A click on an
  edge an `EdgeGroup` stands for (`blend_tools::toggle_edge`) replaces the group with its current edges
  less the clicked one's chain. Edges and openable faces (flat, cylinder, cone, sphere, torus:
  `openable_face`, `BodyFace::openable`) of the body before the feature (`BodyMeshes::body_before`)
  are `Pickable::BlendEdge` and `Pickable::ShellFace`, drawn as in Preview while open; a click
  toggles one, and opening a face drops its own thickness (`shell_tools::without_own_thickness`).
- Choose in the view on a fillet or chamfer first adds the edges selected in the view of its body and
  its selected faces as `EdgeGroup`s (`blend_tools::with_selected_edges`, one undoable change, edges
  already in a chosen chain and faces whose edges are all chosen skipped), then opens it. A shell's and
  an offset face's do the same with the selected faces of their body
  (`shell_tools::with_selected_faces`, openable faces only; `offset_face_tools::with_selected_faces`;
  already chosen skipped), the panels taking the selection through their `FacesRow`.
- A chamfer's Distances (Equal, Two, Angle; `blend_tools::FormChoice`) follow Shape: Two names the size
  First distance and adds Second distance (starting as the first), Angle adds an angle field (45 deg to
  start, refused at or past 180°, `Rule::ChamferAngle`), each previewed while typed, and either adds
  Measure from the other face (`flipped`). The palette has the same on the open feature or the tree's
  current one (`blend_tools::form_change`, `flip_change`), refused in words on anything but a chamfer
  and the flip on an equal one.
- A shell's panel runs Open faces, Thickness, Own thickness, Walls, Body. Walls is a `segmented`
  Inside, Outside or Both sides (`shell_tools::side_change`, "Set the walls of <shell> to <side>").
  Own thickness lists each face given a thickness of its own as a value row captioned with the face
  (`feature_fields::expression_row_removable`, owned as "Thickness of <face>", a remove button giving
  it the shell's thickness again); Choose faces starts `Slot::ShellOwnThickness`, which keeps
  choosing: each click on a face of the body (`shell_tools::own_thickness_click`, routed before other
  picking like `HoleAdd`) adds it at the shell's thickness or removes it, one undoable change each,
  an opened or unoffsettable face refused in a notice. The thickness arrow stands on the first opened
  flat face's rim pointing into the wall: inward for Inside, outward for Outside, outward at half
  scale for Both sides.
- Hovering a row of the edges, opened, moved or split faces in an open fillet's, chamfer's, shell's,
  offset face's or split face's panel lights that edge or face (its pieces when split) as hovered
  (`widgets::removable_row_hovered`, `RowCache::preview`, handed to `ViewportState::preview_entities`
  each frame).
- Chains and opened faces stay in `BodyBefore::choice` until the references change; each panel's list
  stays in `PanelState::reference_rows` (`reference_rows.rs`) until the state before it or the revision
  changes.

## Offset face and Split face

- Offset face (Alt+Q) takes the selected faces of one body, any shape (`offset_face_tools::selected_faces`),
  and creates an `OffsetFace` of 1 mm that opens (faces in words with Leave this face out, a signed
  Distance, Move the faces tangent to these too as `tangent`). While open and computed, the result is
  drawn and its faces are `Pickable::ShellFace`s, moved ones in the selected colour
  (`FaceChoice::Moving`, `Builder::choosable_faces`, tangent chain included); a click moves a face or
  leaves it out; faces keep their keys across the feature. A failed or pending one shows the body before
  it with every face pickable.
- Split face (Alt+Shift+K) takes the selected faces of one body (`split_face_tools::source`) and, from
  the rest of the selection, a principal or datum plane or the curves of one earlier sketch
  (`split_tools::chosen_along`, faces left aside), else the YZ plane, and creates a `SplitFace` that
  opens; what it drops is said in a notice. Split along has Use selected and Choose in the view on
  `Slot::SplitPlane`, as the palette's Split along selected does. For a sketch, Carried is
  `split_face_panel::SQUARE` or `ALONG` (as an extrusion's Direction row, through
  `split_face_tools::direction_change`, `Slot::SplitDirection`) or `WRAPPED` round the faces (through
  `split_face_tools::wrapped_change`, disabled with the reason unless the chosen faces lie on one
  cylinder or cone as the body before it stands, `on_one_unrolled_surface`; also
  `Command::WrapSplitCurves` on the open feature else the tree's row).
- A split face's pieces carry new names, so the result's faces are not the keys of the state before it:
  while open it always shows the body before it (`OpenView::Before`, `FaceChoice::Splitting`) with every
  face a `Pickable::ShellFace` and chosen ones in the selected colour, a click splitting a face or
  leaving it out, and over it the result's edges the body before lacks in the selected colour
  (`Builder::split_edges`), so the split lines preview.

## Primitives

- Box (Alt+B), Cylinder (Alt+Y), Sphere (Alt+U), Torus (Alt+Shift+U), Cone, Wedge and Prism (Model
  menu's Primitives group) each create a `Primitive` of the `DEFAULT_*` sizes of `primitive_tools.rs`
  (a cone to a point, a wedge sloping to an edge, a six-sided prism), a sphere and torus starting at
  their centre and the rest at their base's centre, and open it, previewed like an extrusion.
- With one plane, datum plane or flat face selected (`datum_tools::chosen_plane`; several refused), it
  stands there: on a face at its middle (`hole_tools::face_middle`) added to that face's body, on a
  plane at its origin as a new body. With none it stands on XY at the origin as a new body and starts
  choosing its place (`Slot::PrimitivePlace`, prompt saying Escape leaves it there): a click on a plane,
  datum plane or flat face places it where the pointer's ray meets it (`primitive_tools::place_click`,
  the ray handed to `pick_action`); the keyboard highlight places it at the face's middle or the
  plane's origin.
  A click or Activate on a face, edge or corner of the primitive itself can hold no place, so it
  leaves it where it is and selects as it would anywhere else (`ViewportState::leaves_primitive_place`,
  `primitive_tools::is_part_of`) instead of being refused as made after the primitive.
- Panel specifics: Shape switching takes the new shape's default sizes; Position X and Y (key
  `primitive-field`, `("at", index)`); sizes (`("size", index)`) each checked by its `SizeRule`
  (`Rule::AboveZero`, `ZeroOrMore`, or `Sides` for a count); Reverse direction is left out for Centre.
  Switching Result between joining (New body, Add) and cutting (Remove, Intersect) on a face reverses
  the direction to match (`primitive_tools::with_operation`); placing it again never changes the
  operation.
- While a primitive is open (and its place not being chosen), a hole's placement handles stand at its
  position on its plane (`place_handles.rs`, `displayed_plane`): the square drags it anywhere on the
  plane, the arrows along its X and Y, committed on release as one "Edit <shape>" change setting only
  moved positions to measured values, a named one through its parameter and a driven one not dragging
  (`manipulator::Held`).

## Hole

- Hole (Alt+O) with one flat face selected (no sketch edited, no sketch geometry selected, which wins)
  creates in one transaction a hidden sketch on that face holding one point and the hole: the point
  inside the face farthest from its edges and holes (`deepest_point`, a refined grid search over the
  sampled loops, ties to the middle of its bounds), with a notice saying how to move it
  (`hole_tools::create_on_face`).
- With one curved face selected (`HoleStart::CurvedFace`, `hole_on_curve.rs`) the transaction also makes
  a hidden datum point at a spot on the face (its boundary's mean projected onto it) and a hidden datum
  plane touching the face nearest that point (`PlaneThrough::TangentAt`); the hidden sketch lies on that
  plane with its point at the plane's origin, so the hole drills square into the face; it opens choosing
  its place (`Slot::HolePlace`). Such a hole (`hole_on_curve::mount`) moves by rewriting the datum point
  to where the pointer's ray first meets the body (`SolidClassifier::first_crossing`), projected onto
  the clicked face, and the tangent plane to that face; its Placed on names the face and has no
  Position rows. Its placement handles (`place_handles.rs`, `Subject::Curved`) stand on the tangent
  plane at its foot (from the draft's evaluation while one is shown); a drag's point is projected onto
  the face (`hole_on_curve::moved_to`) and only the datum point moves.
- Otherwise it takes the sketch as Extrude does (edited, selected, or with nothing selected the opened
  or last one; else `NOTHING_TO_DRILL`) and needs at least one free point or circle in it
  (`hole_centres`); its body is the one the sketch is attached to, else the one whose faces are
  selected, else the last body standing. It creates a plain blind hole of 6 mm by 10 mm, hides the
  sketch and opens the panel.
- Panel specifics: a size sets exact millimetre values, and typing any hole, counterbore or
  countersink size makes it Custom. Fit is the size's `fits` (Close, Normal, Loose, Tapped, Fine,
  Insert from M2 to M8); tapped ones add Pitch (when the size has several fine pitches), Thread form
  (Modelled, Cosmetic; `hole_tools::new_hole` makes new holes Modelled, the note under Fit says
  whether the bore is cut or drawn), Thread class, hand and length (Whole bore, To a depth, key `thread-depth`), an insert a note with its length and
  least wall. Style (Plain, Counterbore, Countersink, Stepped) takes the size's head dimensions when it
  has them, else defaults; Stepped starts from the counterbore and a step midway to the hole. Sized by
  Circles sets `CirclesAndHeads` and shows while the sketch has circles or the hole is sized by them.
  Depth (Blind, Through all, Up to next, Up to face) takes the face like an extrusion's end through
  `solid_panel::selected_target`, else chooses with `Slot::HoleTarget`; either up-to kind has the
  extrusion's Past the face field, key `end-offset`. A blind round hole has Drill point (a 118° cone,
  key `drill-point-angle`). The Sketch row's Edit the sketch enters it, hidden or not.
- The panel places the holes while the sketch reads as a placement (`hole_placement::read`): only free
  points, each held by nothing (free, optionally by a `HorizontalDistance` and `VerticalDistance` from
  the sketch origin, `PlacedHole::along`), by `Distance` constraints to at most `MAX_EDGES` projected
  straight edges, or by a `Coincident` with the centre of one projected round edge, every projection an
  edge of a body (`ProjectionSource::Edge`, an `EdgeReference`) used by exactly one of those
  constraints. The placement lives wholly in the hidden sketch (no record added, the file format is the
  sketch's) and the hole follows those edges as any projection does. A sketch drawn otherwise is placed
  by editing it, Placed on then only naming its face.
- Placed on names the face the sketch lies on; while the sketch holds one point and no constraint
  (`hole_tools::lone_point`) it also has Use selected or Choose in the view (`Slot::HolePlace`): a click
  on a flat face while choosing (`hole_tools::place_click`, routed by `pick_action` with the pointer's
  ray) puts the point where the ray meets the face's plane, the keyboard highlight or Use selected at
  the face's middle; a face other than the sketch's re-attaches the sketch there (`SetSketchPlacement`)
  and moves the hole to that face's body, all in one change. Add another hole (`hole_panel::ADD_HOLE`)
  chooses in the view (`Slot::HoleAdd`, staying on until Escape or Stop adding holes): a click on a face
  in the sketch plane (`solid_tools::same_plane`) adds a free point where the ray meets the plane, the
  keyboard highlight the spot of that face farthest from its edges and the other holes
  (`hole_tools::clear_spot`), each one "Add a hole to <hole>" change sharing size and depth
  (`hole_placement::added_at`).
- Several holes are each listed ("Hole n", with Remove hole n, `hole_placement::removed`, taking its
  point and anchors in one change and refusing the last). A free one has Position X and Y (key
  `("hole-position", feature, point, axis)`, previewed, one "Move <hole>" change,
  `hole_placement::positioned`): the point is set where the value puts it (`SetSketchEntity`), and a
  value using parameters is also held by a `HorizontalDistance` or `VerticalDistance` from the sketch
  origin holding the expression as a magnitude (negated on the negative side,
  `hole_placement::signed`, which also gives the field the expression back); a plain value typed over
  it drops that dimension. The square's typed "x, y" goes the same way.
- A hole held by edges or Concentric has no Use selected under Placed on (that takes only
  `hole_tools::lone_point` sketches) and no place arrows along x and y (see the last bullet of this
  section for its handles). One measured from edges has a Measured from row
  per edge (`edge_words`, Stop measuring from this edge, `released`) and its Distance (key
  `hole-edge-distance`, previewed, one "Move <hole>" `SetDimension`). One centred on an edge has
  Concentric with and a Stop centring row. Place by offers From an edge (`Slot::HoleEdge(point)`, while
  fewer than two edges hold it and it is not centred) and Concentric (`Slot::HoleConcentric(point)`),
  each applying at once when the one selected edge gives a change, else choosing in the view.
  - From an edge (`hole_placement::edge_change`) projects a straight edge of a body seen by the sketch
    as construction (`add_projected`) and adds a `Distance` from the point at its measured value in the
    model's unit, refusing in words an edge made after the sketch, a curved one, one square to the face,
    the same edge, one parallel to the first, a centre lying on the edge, a third edge and a centred
    hole.
  - Concentric (`concentric_change`) projects a circle or arc as construction (drilling and sizing
    nothing), moves the point to its centre and adds the `Coincident`, replacing whatever held the
    point, in one "Make <hole> concentric with an edge" change.
- While a hole of one free point is open (nothing being chosen), its centre carries handles on the face
  (`place_handles.rs`, `Manipulator::Place`): a square to drag it anywhere on the face and an arrow
  along the sketch plane's x and y (`PlaceGrip`), in `ScenePalette::handle` at `ARROW_SHARE` of the move
  arrows' length. A drag follows where the pointer's ray meets the sketch plane, in steps (Ctrl free),
  previewed with Position X and Y beside the pointer and committed on release as one "Move <hole>"
  change; the handles follow the preview (`Model::draft_transaction`).
- Every other placement (`hole_placement::read`) has its handles in `hole_handles.rs`
  (`Manipulator::Holes`, `Handle::Hole(HoleGrip)`, standing where the place handles do and drawn,
  hit and cleared of value arrows the same way): a square on each free point
  (`HoleGrip::Square(point)`) and, on a point measured from edges, an arrow per `Distance`
  (`HoleGrip::Distance(constraint)`) standing at the point and pointing along the edge's normal
  away from the edge (`leaving`, read off the solved sketch of the draft's evaluation while one is
  shown); a point centred on an edge has none. A drag is a `place_handles::Slide` (shared with
  `PlaceDrag`: a plane or a line, steps, Ctrl free, snapping to corners, edge middles and round
  centres, the arrow also kept at least one step from the edge) committed on release as one "Move
  <hole>" change: the square through `hole_placement::positioned` with the moved axes as measured
  values, the arrow as a `SetDimension` of that constraint's value or, for a named one, an
  `Edit::SetParameterExpression` (`manipulator::Held`, read from the panel's `EDGE_DISTANCE`
  caption). A point whose position is held by a dimension from the origin, and a distance using
  other parameters, are driven: the handle does not drag, its hover names what drives it and typing
  opens no field. Typing over a square takes "x, y" and over an arrow a length at least zero, as the
  panel's fields do (`hole_handles::typed`). The hover names the hole ("hole 2") once there are
  several.

## Value handles

- A value arrow (`value_handles.rs`, `Handle::Value`, `value_gauges::Measured`) drags one value of an
  open feature as a reach arrow does (steps, Ctrl free, readout, one "Edit <feature>" change keeping a
  named value and refusing a driven one through `manipulator::Held`, each `Measured` naming its field's
  caption, shared with its panel through a constant). A `Gauge` holds where it stands: a `Track` (a
  line from a base along a direction, or a `turn_handles::Swing` about an axis) and a `Mapping` from
  value to distance or angle (`Linear` with scale and offset, so a diameter's rim moves half as fast;
  `Tangent` for a taper; `Reciprocal` for what a turn divides, a prism's sides and a spread curve
  pattern's count; `Cotangent` for a countersink's angle read off how deep its cone ends).
  `value_gauges::Builder` makes them from the shown kind (the draft's while previewed) and the body as
  the feature sees it (`handle_snap::before`), at most `MOST_GAUGES` per feature, for:
  - A blind hole's Depth (bottom of its first hole) and Diameter (along the sketch diagonal between
    the place arrows, at the level the bore starts under a counterbore, countersink or steps, across
    the side of a slot; not when sized by circles; dragging makes the size Custom). A counterbore's
    Diameter (its mouth) and Depth (down its wall, opposite the bore's arrow); a countersink's Diameter
    (its mouth) and Angle (an arrow down the bore wall at the foot of the cone, so dragging the foot
    deeper narrows the cone); a slot's Length (its tip) and Angle (a turn arrow at the other tip); a
    tapped hole's Thread depth (down the bore wall from where the bore starts, only with a thread
    length); and an up-to hole's Past the face, standing where its axis meets the face (an up-to-next
    hole's from the plane the recompute recorded, `ReachedEnd`). All of these but Depth and Past the
    face are left out for a hole sized by circles and the countersink's for a slot (which cannot be
    countersunk); dragging a counterbore or countersink value makes the size Custom as the diameter
    does, the thread depth and slot values keep it.
  - An offset face's Distance; a datum plane's Offset and Angle (an arrow along the turn at an arm of
    `ARROW_POINTS` from the axis).
  - A fillet's or chamfer's size (along the bisector of its faces into the material, foot on the
    rounded or cut face: `1/cos(a/2) - 1` or `sin(a/2)` of the size for faces turned by `a`, from the
    faces' normals at the edge's middle, which holds for convex and concave edges alike). A chamfer
    of two distances (First and Second distance) or of a distance and an angle stands each foot where
    the cut meets the face: the arrow of the first distance along the face the kernel measures it on
    (`caditor_kernel::measured_face`, so Measure from the other face moves it), the second distance's
    along the other face, and the angle's as a turn arrow about the first foot standing where the cut
    meets the other face (`Corner::cut_length`, the swing's radius being that point's distance from
    the first foot, or an arm of `ARROW_POINTS` when the cut misses). Distances run along the faces' tangent lines at the
    edge's middle, which lie on a plane face and only touch a curved one.
  - A shell's and a thin wall's Thickness; an extrusion's Taper (shown with no taper too, zero clearing
    it) and an end Up to face's or Up to next's Past the face (an Up to next end stands on the first
    face's plane the recompute recorded, `ReachedEnd::level_over`; none when that face is slanted or
    curved).
  - A primitive's sizes (`primitive_arrows`, the mapping worked out from the footprint and anchor),
    including a wedge's Top length at the top of its slope and a prism's number of sides as a turn
    arrow on its second top corner (`PrismSides`, the corner's angle from the flat edge being half
    the angle one side subtends).
  - A linear pattern's Spacing (or Total length) per direction, at its last copy and only with two
    copies or more (a Count arrow on the same line read as a duplicate, so the count is typed); a
    circular pattern's Total angle and Count (halfway round to the original for a full turn) as turn
    arrows; a curve pattern's Spacing at its last copy when spaced by distance, and its Count at the
    second copy when spread evenly (the arrow stands at the copy `PatternPath::copy_at` places, with
    its base set so the foot is exact now and slides along the curve's tangent while dragged). A
    point pattern has no count or spacing: its copies are its sketch's points.
  Counts always step by one.
- `Manipulator` holds them beside the primary handles as `Reach`, `Revolve`, `Place`'s and `Holes`'
  second part, or alone as `Manipulator::Value`, hit before the primary handles. Since value arrows stand at model
  positions while primary handles keep their size on screen, one coming within `CLEARANCE_POINTS` on
  screen of a primary handle's arrows or square, or of an earlier value arrow, is left out
  (`ValueHandles::clear_of`, `move_manipulator::segments_apart`) until zooming in parts them; its panel
  field stays and typing reaches it.
- While a handle is dragged (or its value typed) the open feature's panel is drawn from the draft's
  kind (`manipulator::publish_dragged`, `dragged`, read in `feature_tree::body`).
- Every handle drag snaps to the shown geometry under the pointer, ahead of the steps (Ctrl drags
  freely; `handle_snap.rs`): `Probe::snap` takes the CPU hits under the pointer
  (`Scene::hits_through`, `PickTable::within_reach`), keeps those no deeper than the nearest face that
  resolves (so the open feature's see-through preview, whose new faces the body before it lacks, hides
  nothing), and resolves them on the body as the feature sees it (its own body's state before it, a
  move's own body never): a corner first, then a round edge's centre or an edge's middle within
  `MIDDLE_REACH_POINTS`, then a face. What a face gives depends on the drag's `Guide`
  (`Manipulating::guide`): only a `Guide::Line` (a reach arrow, a move's arrow, a value arrow on a
  line) takes faces, a flat one by its plane and a curved one where the handle's line meets its surface
  (`meeting_curved`: `intersect_curve_surface` over the line across the body's box, hits outside the
  face dropped, the one nearest where the pointer's ray met the face kept; none is no snap). A line
  handle lands level with the point along its line, or on the face, a face only within `MAGNET_STEPS`
  steps of where the drag would be (`Snap::holds`); a plane handle (`Guide::Points`) lands on the
  point's projection, an arrow along one axis on that coordinate. A turning handle (`Guide::Points`)
  turns to the angle bringing the point into its turned plane, the half-plane through its axis and the
  dragged foot (`Swing::snapped_near`, taking the turn of that angle nearest where the drag would be):
  a revolve's arrow and a value arrow on a `Swing` (a datum plane's angle, a circular pattern's angle
  and count, a slot's or chamfer's angle, a prism's sides) put their foot's half-plane there, refused
  when the value it gives falls outside the drag's range or `Measured::accepts` would change it (so a
  count snaps only where a whole count lands); a move's ring turns by the angle from the spot grabbed
  to the point about the ring's axis (`MoveDrag::turned_to`). The value set is measured, never a
  reference, and the readout ends with what it snapped to ("to a corner").
- Typing a digit while a handle is hovered or held (outside a sketch) opens the typed-point field
  (`app-input.md`) labelled with that handle's field (`Manipulator::typing`, `manipulator::Typing`; a
  square takes "x, y"), dropping a drag in progress; the text previews as typed and Enter commits the
  expression as typed, parameters included, into the slot a drag writes (`Typing::transaction`, through
  `field::checked`). Each value is held to the `feature_fields::Rule` its panel field holds
  (`Typing::parse`, rules from `Measured::rule`, `Reach::field_rule` and `turn_handles::field_rule`,
  none for positions and moves), since the trial apply checks no ranges: a refusal stays in the field in
  the panel's words, Escape drops the preview. A driven value opens no field.

## Thread

- Thread (Alt+Shift+O) takes the one selected cylindrical face of a shown body
  (`thread_tools::selected_face`; a flat face, several faces or anything else selected with it is
  refused in words) and creates a `Thread` named "Thread on <the face in words>" (numbered when taken),
  of the ISO metric coarse size nearest the face (minor diameter for a bore, major for a shaft), the
  side's first class, right-handed, the whole face long and `ThreadForm::Modelled`, then opens it.
- Form (Modelled, Cosmetic, `thread_panel::form_rows`, a segmented row: one undoable change per
  click, keyboard reachable like every segmented row) sits under the face; a taper pipe thread shows
  `DRAWN_ONLY` instead. While modelled threads are shown as cosmetic, a modelled thread's panel (and
  a modelled tapped hole's) says so (`thread_panel::SHOWN_COSMETIC`).
- The panel reads face and side from the body as the thread sees it (`thread_tools::threaded_bore`).
  Switching Standard takes the standard's size nearest the face and its first class for the side; Class
  is hidden when the side has only one. The Face row takes another face with Use selected or Choose in
  the view (`Slot::ThreadFace`, `thread_tools::face_change`): the one round face `selected_face`
  accepts, existing where the thread sits, keeping the size while it fits the new face (else the
  standard's nearest) and the class while the side offers it.
- Show modelled threads as cosmetic (`Command::ToggleCosmeticThreads`, View menu and palette, a
  toggle shown On/Off through `ToggleStates::cosmetic_threads`; invoked in `status_bar`) is session
  state on `Model` (`threads_shown_cosmetic`), not content: while on, `Model::recompute` and draft
  previews submit `Document::threads_as_cosmetic` (`Model::as_shown`), so the evaluation shown has
  every modelled thread evaluated as cosmetic, and switching back reuses the cached cuts. Toggling
  resubmits an open draft and says what changed (`THREADS_COSMETIC_ON`, `THREADS_COSMETIC_OFF`).
- Only cosmetic threads are drawn (`PlacedThread::modelled` is skipped: the cut shows itself).
  They are drawn on shown bodies (`scene::Builder::threads`, from `placed_threads`), outside sketch
  editing: the thread's circle (major diameter for a bore, minor for a shaft) solid at its start, the
  same circle dashed at its end and four dashed lines along it, each `Stroke::DashedWhereHidden`, in
  `ScenePalette::thread` (3:1 on a body in high contrast), the open thread in the selected colour and
  wider. A hidden thread is drawn only while open. A tapped hole's threads are drawn the same way, and
  the Hole panel says so.

## Combine

- Combine (Alt+J) is offered when the selection touches faces, edges or vertices of exactly two shown
  bodies, or two bodies are chosen in the tree. Picked in the view, the first picked is the target and
  the other the tool (`Selection::in_pick_order` through `body_selection::bodies_in`); in the tree, the
  earlier body is the target. It creates a Join and opens the panel: Operation (Join, Cut, Intersect),
  Target and Tool lists (`Document::bodies_before`, each leaving out bodies the other rows hold), Swap
  target and tool (`combine_panel::SWAP`, `swapped`) in one edit, Also with further tool bodies, and
  Keep tool. Nothing is chosen in the view while it is open.

## Move

- Move body (Alt+M) takes the one body whose faces, edges or vertices are selected and creates a `Move`
  of zero turns and distances, then opens it. Copy body (Alt+Shift+C, also a body's right-click menu)
  does the same with `copy` set, named Copy N. New moves turn about the body's centre; Turn about
  (Centre, Origin, Axis) changes it. Fields are expressions (key `move-field`, `offset` or `turn`, axis
  index). Switching to Axis (`move_tools::about_axis`) takes the one axis the selection names, else Z,
  keeping any angle set; its Axis row is a combo of principal and earlier datum axes with Use selected
  and Choose in the view (slot `MoveAxis`, the palette's Turn moved body about selected axis,
  `move_tools::axis_change`), followed by Angle (key `angle`); the turns about X, Y and Z are then shown
  only while not zero. Nothing else is chosen in the view while it is open.
- While a move is open (nothing being chosen) a manipulator stands at the centre of the moved body's
  box, following its preview (`move_manipulator.rs`): an arrow along each world axis in the axis colours
  (`ScenePalette::axis`) and a square per plane, on the front layer in the overlay batch at a constant
  size in points (`ARROW_POINTS`); an arrow pointing at the eye or a square seen edge-on is left out.
  Hit-testing is on the UI thread in screen space (`MoveHandles::hit`, arrows within `HIT_POINTS`
  winning over squares), needing no pick; the hovered or dragged handle turns
  `ScenePalette::handle_highlighted`, the hover says what a drag does, and a click on a handle selects
  nothing.
- A primary drag on a handle (`PrimaryDrag::Manipulate`, `Manipulating`) follows the point of its axis
  nearest the pointer's ray (`Ray::closest_along_line`) or where the ray meets its plane, from where it
  was grabbed, in steps of 1, 2 or 5 of a power of ten about `STEP_POINTS` on screen (Ctrl free). Each
  change is an `Action::Preview` with a readout of the distances beside the pointer; release commits
  one edit setting the dragged distances to measured values (a named one through its parameter), Escape
  or leaving the feature drops it.
- A move turning about its body's centre stands its manipulator at that centre (as moved) and adds a
  ring about each axis (`Handle::Turn`, `RING_REACH` beyond the arrows, left out when edge-on). A drag
  turns the body by the angle swept round the centre in the ring's plane, in steps of
  `TURN_STEP_DEGREES` (Ctrl free) or onto a point under the pointer (Value handles below), previewed
  and committed like an arrow: a turn about an axis applied
  after the others (Z, or Y with no Z turn, or X alone) adds to its own field, any other is composed
  with the turns there and written back as three turns (`turns_about_axes`), changing only fields whose
  value changed. A move turning about the origin has no rings. A move turning about an axis stands its
  manipulator on that axis, at the foot of the body's centre shifted by the distances, with one ring
  (`Handle::TurnAbout`, `ScenePalette::handle`) whose drag adds the swept angle to the Angle field the
  same way.

## Mirror, split, mate and scale

- Mirror body (Alt+Shift+M) takes the body of the selection like Move and creates a `Mirror` that keeps
  the original, across the one principal or datum plane selected with it, else the one selected flat
  face (of that body or another), else the YZ plane (`datum_tools::chosen_plane`); a selected datum axis
  or point, a datum after the bar or several planes are refused. The panel chooses a principal, datum or
  coordinate system plane from a combo, or any earlier plane or flat face with Use selected, Choose in the view (slot
  `MirrorPlane`) or the palette's Mirror across selected.
- With rows chosen in the tree that a pattern would repeat (`pattern_tools::repeatable`), Mirror body
  mirrors those features instead (`MirrorSource::mirrored`), named "Mirror features N"; its Mirrors rows
  list them and Mirror the chosen features takes the rows chosen now; Keep the original is hidden. The
  open mirror shows its body and reflected cuts like a pattern of holes.
- Mirror faces (`Command::MirrorFaces`, Bodies group) takes the selected faces of one body
  (`mirror_tools::faces_source`) and, from the rest of the selection, a principal, datum or coordinate
  system plane, else the YZ plane, and creates "Mirror faces N" mirroring the region they bound, then
  opens it. Its panel lists the faces in words (`offset_face_panel::face_row`, read on the body as the
  mirror finds it, `body_result_seen_by`), each with Stop mirroring this face (the last leaves the whole
  body mirrored); Mirror the selected faces (also `Command::MirrorSelectedFaces` on the open feature or
  the tree's row) takes the faces selected in the view now (`mirror_tools::faces_change`), as Mirror the
  chosen features takes tree rows, each clearing the other. Keep the original shows only while the whole
  body is mirrored.
- Split body (Alt+K, also a body's right-click menu) takes the body the same way and creates a `Split`
  along what is selected (`split_tools::source`, `source_for` from the Bodies group): a plane or flat
  face chosen as Mirror's is, else curves or regions of one earlier sketch, else exactly one selected
  curved face (`chosen_surface`, through `sketch_placement::surface_at`) as a `SplitAlong::Surface` with
  the body beside it split, else, with faces or edges of exactly two bodies and no tree rows, the first
  picked is split along the second; nothing chosen splits along YZ. Split along lists principal planes,
  every other body standing before the split and every earlier sketch that is one open chain
  (`is_open_chain`), worded by `split_tools::describe`; Use selected, Choose in the view (slot
  `SplitPlane`) and the palette's Split along selected take a plane or flat face, a sketch curve, one
  curved face, or an edge of another earlier body (`along_change`); Split face takes the same through
  `chosen_along`. While open both bodies show as previews.
- Mate body takes picks in pick order (`mate_tools::source`), resolved at the bar, the first on the
  body to move (which must be shown). Two picks make, tried in this order: a faces mate (a flat face of
  the body, then a flat face of another body or a plane, `datum_tools::plane_reference`); an axes mate
  (an axis of the body, a straight edge or round face, then any axis, `axis_reference`); a tangent mate
  (a cylindrical or spherical face of the body, `moving_round`, then a plane); a flat face resting on a
  round one (flat face of the body, then a cylindrical or spherical face of another body,
  `round_target`); a point mate (a corner, round edge or sphere of the body, then a point,
  `point_reference`, or else a plane). Four picks, two on the body (a flat face and an axis) and two off
  it (a plane and an axis), in any order within each pair, make a flush and concentric mate. Mate body
  at an angle takes the same two flat faces or axes and makes an angle mate at 90 deg (`angle_source`).
  Either creates "Mate N" (faces flush at distance 0) and opens it. A mate keeps the kind it was made
  with.
- The mate panel (`mate_panel.rs`) names each reference with Use selected and Choose in the view (slots
  `MateMoving(MatePart)`, taking only the moving body's references, and `MateTarget(MatePart)`, only
  references off it; `MatePart::Axis` is the axis pair of a flush and concentric mate); an angle mate's
  Angle runs 0 to 180 deg.
- Scale body (Alt+Shift+S) takes the body the same way and creates a `Scale` by 1 about the origin. The
  factor is a plain number above zero; Centre in (World, or a coordinate system above it, through
  `feature_fields::frame_row`, shown once the model has one) measures the centre from that origin along
  its axes; fields are expressions (key `scale-field`, `("factor", 0)` or `("center", axis index)`).
  Centre at: Use selected or Choose in the view (`Slot::ScaleCentre`, `scale_tools::centre_change`)
  writes the coordinates of the one selected point (`datum_tools::point_reference`: a corner, round edge
  or sphere centre placed on its body as it stands before the scale, `PointReference::on_body`, else a
  sketch or datum point or the origin as shown, `measure::point_of`), and Body centre
  (`scale_panel::BODY_CENTRE`, `body_centre_change`) the middle of the body's box before the scale; both
  as measured values in the Centre in system.

## Scale model

- Scale model… (`Command::ScaleModel`, Bodies group, no default key, refused while a sketch is edited)
  opens a modal (`Workspace::scale_model`, dropped with the session) with the Factor (an expression
  giving a plain number; Enter scales), the Centre (the origin, a datum point or a coordinate system's
  origin, read from the evaluation) and what to Scale (Typed values, or Values and parameters,
  `ScaledValues`). It builds `Document::scaled` (`scale_model::scaling`) and applies it as one undoable
  change with a notice summing up what changed, what was left to follow parameters, which holes lost
  their standard size and which threads to check; a refusal (no factor, a length, zero or less, 1)
  stays in the dialog as an error callout. While the centre is off principal geometry some features
  use, an info callout (`moving_words`, from `displaced_by_scale` cached per revision and centre) names
  each feature and geometry before anything is applied, and the notice says which datums they now use.
  The transaction also rewrites the length cells of the inactive configuration rows with the same
  rescaler as the parameters they set, in one `SetConfigurations` (the active row follows the live
  values).

## Configurations

- Configurations… (`Command::Configurations`, Bodies group, no default key) opens a modal
  (`Workspace::configurations`, dropped with the session) holding the model's configurations
  (`document.md`) as a table: a row per configuration (switch, Rename, Duplicate, Delete), a column per
  configured value still in the model (named by `value_name`: the parameter, "<feature> suppressed",
  "<body> colour"). A parameter's cell is a `commit_field` taking an expression; a suppression a
  checkbox; a colour a combo of the body colour swatches. Editing a cell of the active row edits the
  model itself. Add configuration copies the live values (the first becomes active). Every change is
  one undoable transaction checked first; a refusal stays in the dialog as an error callout.
- A configured column and any expression in a row's cell count as uses of a parameter
  (`Document::used_parameters`, `parameter_users`, naming "<row> configuration" or "the
  configurations"), so Delete on it is `inline_parameter`: the other rows' cells get its value (the
  row's own cell when it is the column) and its column and cells go, in one `SetConfigurations` before
  the removal; `Edit::RemoveParameter` refuses one the configurations still name, and `transaction_to`
  empties the table first.
- The palette lists each configuration as "Switch to <name>" (`Choice::Configuration`,
  `Palette::take_configuration`), the active one unavailable; Enter applies
  `configurations::switching`.

## Patterns

- With rows chosen in the tree that are all extrusions, revolves, holes or primitives adding to or
  removing from one body (`pattern_tools::repeatable`, from the tree's raw rows, which the offers carry
  beside its bodies), Linear and Circular pattern repeat those features instead of the body, with a
  default spacing of twice their tools' extent along the direction. The Repeats rows list them (the
  last Stop repeating leaves the whole body repeated), and Repeat the chosen features takes the rows
  chosen now (Ctrl+click while the pattern is open).
- Otherwise Linear and Circular pattern take the body of the selected faces, edges or vertices or of the
  tree (`move_tools::chosen_body`), the last body standing only with nothing selected, and the one
  selected axis, straight edge, round face or sketch line as direction or axis (edges and faces of the
  body itself count only when nothing else is selected), else for a linear pattern the X axis
  (`pattern_tools::source`). A circular pattern never takes an axis it was not given: with none
  selected it opens with no axis and the Axis slot waiting for a click in the view
  (`Slot::PatternDirection`, as a sweep waits for its path), waiting (not failing) until one is chosen;
  switching another kind to Circular keeps a linear pattern's first direction and otherwise leaves
  the axis unchosen. While open the patterned body is shown with its directions or axis drawn
  like a revolve's axis. Direction, Second direction (also None) and Axis are `axis_row`s listing the
  principal axes and datum axes made before the pattern (`pattern_tools::listed_axes`, set through
  `with_axis`), an unset one reading None chosen.
- Each linear direction has a Measured switch (Each, Overall), captioned Spacing or Total length.
  Switching keeps the copies in place when the count is a plain number (`pattern_panel::respaced`: a
  literal multiplied or divided, else the expression times or over the steps), otherwise it keeps the
  expression.
- Instances is a grid of `widgets::instance_toggle`s, one per instance at its step along each
  direction, checked when made; clicking one leaves it out or brings it back as one undoable change, the
  original disabled. The grid is laid out like the copies in a view along the pattern's directions
  taken as +X and +Y on screen (the top and front views): columns run left to right with the first
  direction, rows run bottom to top with the second, so the original is at the bottom left; a reversed
  direction (`Pattern::reversed_directions`) flips its axis, keeping the original on the side it sits
  relative to its copies. The grid does not follow the camera, so it is not re-laid while orbiting.
  Rows are built from the top down (`grid_order`), which is also the keyboard order. Switching between
  linear and circular clears the left-out instances.
- Curve pattern and Point pattern (`Command::CurvePattern`, `PointPattern`; Model › Patterns) take the
  body as Linear and Circular do, except that a selection of sketch geometry alone, like an empty one,
  leaves the body to the last one standing (anything else naming no body, such as an axis, plane or
  centre of mass, is refused), plus a sketch (`sketch_pattern_tools::source`): the one whose curves,
  points or regions are selected (several sketches, one made after the bar, or one that does not fit
  are refused in words), else the last sketch above the bar that no later feature uses and that fits
  (`guess`): its curves one chain (`is_curve_path`) for a curve pattern, at least one lone point for a
  point pattern. A curve pattern starts with 4 copies spread evenly, kept as they are; a point pattern
  moves its copies from the origin.
- The Shape switch has all four kinds (Linear, Circular, Curve, Points). Switching to Curve or Points
  keeps the pattern's sketch when it fits, else guesses one, else is refused saying what to draw
  (`pattern_tools::reshaped`); a count carries over between Linear, Circular and Curve. A curve
  pattern's Curve row lists fitting sketches above it (`sketch_pattern_tools::listed`, `Slot::PatternPath`);
  a point pattern's Base point takes a point as a datum point does (`Slot::PatternBase`) and, as its
  copies are named by point, it has no instance grid but a line counting the copies left out with Bring
  the copies back. Reverse the direction refuses a point pattern in words.
- While a pattern is open, a click on a face of one of its copies (`FaceOrigin::Copy` of this pattern,
  found through the shown body's result, `pattern_tools::clicked_copy`) leaves that copy out as the
  grid would, and hovering says so; other clicks select as usual. A copy left out is no longer drawn, so
  the grid or Undo brings it back.

## Visibility

- Sketches, datums and body-making features can be hidden (a body through its feature); hidden ones are
  not drawn, picked, kept selected or counted in fitting, except the edited sketch and open datum.
  Extrude and Revolve hide their sketch in the same transaction. Every hide control is also a command.
  Hide selection and Hide everything but the selection take the rows chosen in the tree as well as the
  view's selection (`visibility::hideable_rows`: a row that can be hidden stands for itself, any other
  for the body it changes), and Hide or show feature toggles every chosen row, else the open feature
  (`toggle_rows`: hides all while any is shown). Hide everything but the selection
  (`visibility::hide_others`) hides in one transaction every other shown hideable feature and principal
  item, except the edited sketch.
- Hide or show every sketch, datum or body (`Command::ToggleSketches`, `ToggleDatums`, `ToggleBodies`,
  View menu; `visibility::toggle_kind`) is one transaction per kind: while any of that kind is shown it
  hides every one shown, otherwise shows them all; the edited sketch is never touched, and a model with
  none of the kind refuses in words. Body-making features count as bodies, as `can_hide` judges them.
- Principal planes, axes and origin hide through a group row (`principal_tree.rs`) with an eye for the
  group and one per item. Hidden ones are drawn and offered anyway while a sketch's plane is chosen
  (`Context::choosing_plane`).

## Datums

- Points are taken from the selection by `datum_tools::point_reference`: the origin, a datum point, a
  corner, the centre of a round edge, the centre of a spherical or toroidal face or a sketch point made
  before the datum; a sketch line is an axis (`axis_reference`).
- Plane starts from the selected plane or flat face (XY otherwise), turned about the selected axis,
  straight edge or round face if any, else offset; three points make a plane through them, two planes
  one midway, an axis and a point one through both. Axis runs along the selected axis, straight edge,
  round face or sketch line, where two selected planes or flat faces meet, through two points, or
  square to a plane through a point. Point (Alt+Shift+P) sits at the one selected point, else the
  origin, with zero offsets. The XY plane and origin are the start only for an empty selection; a
  non-empty one holding nothing usable (a sketch region or constraint, a centre of mass, a body item) is
  refused with `PLANE_CHOICES` or `POINT_CHOICES`, as Axis is. A selected face, edge or corner giving
  nothing (curved, made later, not recomputed) refuses with that reason.
- Selections giving the constructed forms (`document.md`): Plane with two axes or straight edges passes
  through both, and with one non-straight edge and nothing else stands square to it at 0 mm along
  (`PlaneThrough::SquareToCurve`); Point with two axes or straight edges sits where they cross, with an
  axis and a plane where they meet, with three planes where they meet, with one edge that is not round
  at 0 mm along it (`PointBy::Along`), with one sphere or torus face at its centre, with any other one
  face at its centre (`PointBy::FaceCentre`). A curved face and a point giving no other form
  (`datum_tools::face_tangent`: for Plane a curved face giving no axis, such as a sphere, spline or
  extruded face; for Axis any curved face but a sphere) make Plane touch the face nearest the point
  (`PlaneThrough::TangentAt`) and Axis stand square to it there (`DatumAxis::SquareToFace`).
  Combinations keeping an older meaning (an axis and a plane turn a plane; one round edge is a centre; a
  round face and a point are a plane through its axis; a sphere and a point an axis through both) keep
  it.
- A plane through references, a point and the new axis forms show a Defined by (or At) row re-chosen
  from the selection, refused while nothing is selected. A plane through an axis and a point switches
  between Contains it and Square to it, and for a round face's axis also Tangent to it
  (`PlaneThrough::Tangent` for a cylinder or cone, the point picking the side, else `TangentAt`), going
  back keeping the face and point; a point at an edge switches between Its centre (round edge), Along it
  and Its middle (`PointBy::EdgeMiddle`). Choosing again from the selection keeps the form a panel was
  switched to and its distance (`keeping_plane_mode`, `keeping_point_mode`). Choosing in the view holds
  up to two clicks (`Picking::pending`) until the selection makes one, the prompt saying what is still
  wanted.
- A datum cannot switch between plane and axis, nor an extrusion and a revolve, since `SetFeatureKind`
  refuses a kind change (`document.md`). Outside sketch editing datums are picked as `Pickable::Datum`
  and tinted when failed; double-clicking one opens it.

## Reference images

- Insert reference image… (`Command::InsertImage`, `model.insert_reference_image`, no default key) is
  in the palette, the Model menu's Datums group and the Reference ribbon group's menu; it has no
  ribbon button, since the ribbon stays one row. Its offer (`Offers::reference_image`,
  `image_tools::target`) is the edited sketch's plane (`ImageTarget::Sketch`), else the one selected
  plane, datum or coordinate system plane or flat face (`datum_tools::chosen_plane`; a face at its
  middle, `primitive_tools::chosen_spot`), else XY at the origin; several planes refuse. It sends
  `FileCommand::InsertImage` (`app-files.md`), whose answer `image_tools::create` turns into one
  "Insert <file stem>" change adding a `ReferenceImage` at the bar, 100 mm wide
  (`DEFAULT_WIDTH`), `DEFAULT_IMAGE_OPACITY_PERCENT`, behind bodies, unlocked, named after the file
  (else "Image N"). It opens the image's panel unless a sketch is edited, which stays open.
- The panel (`image_panel.rs`, shared rows of `feature_fields`): a description of the picture's
  format and pixels with a "Preparing" line while it decodes or an error callout when it cannot be
  shown, for a sketch-plane image the sketch it lies on, Placed on (`plane_selector::row`, slot
  `Slot::ImagePlace`; a click in the view places the middle where the ray meets the plane,
  `image_tools::place_click`), Position X and Y, Angle, Flip left to right, Width or, once
  calibrated, Distance between the points (keys `("image-field", feature, "along" | "across" |
  "turn" | "width" | "distance")`), Calibrate by two points or Calibrate again and Size by width,
  Opacity (`image_panel::OPACITY_STEPS`, a stored value shown at the nearest step), Shows (Behind or
  In front of bodies) and Locked. Each is one "Edit <name>" `SetFeatureKind`.
- Calibrating (`Slot::ImageCalibration`) takes two clicks on the picture, each the ray meeting its
  plane read back as an `ImageSpot` (`image_tools::spot_under`; a click off the picture or
  meeting no plane warns and keeps choosing). The first is held in `SketchEditing`
  (`EditingCommand::CalibrateFrom`, also holding the image in `Picking::pending` so the prompt asks
  for the second); the second sets `ImageSize::Calibrated` with the distance the two points lie apart
  now, so nothing moves until the user types the real distance.
- `image_textures.rs` (`ImageTextures`, owned by `Model`) decodes each picture of the document on a
  thread of its own (`picture`, under `catch_unwind`) into a `PictureTexture` when the revision
  changes (`want`), wakes the app with the result (`poll`, from `Model::poll`) and forgets pictures
  the document no longer holds; the insert hands over the texture its read already made (`keep`), so
  a new picture is not decoded twice. Its `generation` joins the scene key (`Revisions::pictures`).
- The scene (`scene::Builder::reference_image`) draws each active, shown image (the open one even
  when hidden, from the draft while edited) from its `PlacedImage` and texture, with an outline in
  the datum colours while it is open, hovered, selected or has no texture yet. An unlocked image is
  `Pickable::Datum` of its feature outside sketch editing (selecting, opening and hiding it as a
  datum, every datum command refusing it as not a datum); a locked one, and every image while a
  sketch is edited, is never picked, so clicks reach the sketch and what lies behind it. Images count
  in fitting (`model_bounds`, `datum_points`) and hide with the datums (`visibility::Kind::Datums`).

## Coordinate systems

- Coordinate system (`Command::CoordinateSystem`, Datums group, no default key) takes from the
  selection (`datum_tools::frame_from_selection`) at most one point for its origin (else the origin),
  one axis, straight edge, round face, sketch line or another system's axis for its X axis and one
  plane, flat face or another system's plane for its XY plane, two of a kind refused with
  `FRAME_CHOICES`. A missing axis is the principal axis lying flattest in the chosen plane, a missing
  plane the principal plane holding the chosen axis best (directions read through
  `measure::direction_of`), so defaults never stand square to each other. Origin, X axis and XY plane
  rows each have Use selected and Choose in the view (slots `FrameOrigin`, `FrameAxis`, `FramePlane`,
  `datum_panel::frame_change`).
- It is drawn (`scene::Builder::frame`) as a triad at its origin `FRAME_AXIS_SCALE` of the reference
  size long: an origin marker that is `Pickable::Datum` (selecting and opening the feature), an axis
  line per axis in the palette's axis colours (`Pickable::FrameAxis`) and a small square in the datum
  colours per plane (`Pickable::FramePlane`), tinted when failed. Those pickables are axes and planes
  wherever one is taken from the selection (`datum_tools` `plane_reference`, `axis_reference`,
  `chosen_plane`, `sketch_placement::DatumTarget`), so patterns, mirrors, splits, primitives, revolves,
  moves, datums and sketches use them, and they are listed in the pattern and move axis lists
  (`pattern_tools::listed_axes`) and every plane list (`datum_tools::listed_planes`, `plane_selector.rs`) for
  every system above the feature. Its origin marker is a point wherever one is taken from the selection
  (`PointReference::Frame`), so a datum point, plane or axis through a system's origin needs no point of
  its own.
- A move's Directions combo (World, or a coordinate system above it; shown once the model has one) sets
  `Move::frame`; the manipulator's arrows, squares and rings follow the system's axes
  (`Model::move_frame`, the world being `Plane::XY`). The scale's Centre in and the import's Placed in
  combos are the same row (`feature_fields::frame_row`).

## Sketches on faces and planes

- New sketch starts on the one selected principal plane, datum plane, coordinate system plane or flat
  face (`sketch_placement::sketch_target`, several refused with `SEVERAL_TO_SKETCH_ON`, as Place on the
  selection does); while choosing a plane a click on any of them does the same. Choosing also shows a
  New sketch card above the tree (`new_sketch_panel.rs`): a Plane combo (`plane_selector::combo`) of
  the principal planes and every datum and coordinate system plane before the bar
  (`datum_tools::listed_planes_at_the_bar`), each disabled with the reason `new_sketch_on_datum` gives
  when it cannot take a sketch, choosing one sending the same `EditingCommand` a click does; Cancel sends `CancelNewSketch`, as Escape does, so a plane is
  chosen without the pointer.
- The attachment is captured from the body's state where the sketch sits in the tree: a face made
  further down is refused with the reason, and one whose body is not recomputed that far says so
  (`body_state_before` tells the two apart). A sketch's row says what it lies on and offers Detach and
  Place on the selection.
- Every refused New sketch (including a click on a curved face while choosing) and every Fillet,
  Chamfer, Shell or datum that cannot be created leaves a notice with the reason.
- What draws or maps onto a sketch takes its plane from the solved result (`scene::sketch_plane`,
  `Model::displayed_sketch`), since an attached sketch's stored plane is where it was placed.
