---
paths:
  - "crates/caditor/src/editing.rs"
  - "crates/caditor/src/sketch_drag.rs"
  - "crates/caditor/src/drag_solver.rs"
  - "crates/caditor/src/drawing.rs"
  - "crates/caditor/src/trimming.rs"
  - "crates/caditor/src/modifying.rs"
  - "crates/caditor/src/offsetting.rs"
  - "crates/caditor/src/mirroring.rs"
  - "crates/caditor/src/patterning.rs"
  - "crates/caditor/src/filleting.rs"
  - "crates/caditor/src/shapes.rs"
  - "crates/caditor/src/shape_modes.rs"
  - "crates/caditor/src/snap.rs"
  - "crates/caditor/src/inference.rs"
  - "crates/caditor/src/body_snap.rs"
  - "crates/caditor/src/body_picks.rs"
  - "crates/caditor/src/tracking.rs"
  - "crates/caditor/src/sketch_tools.rs"
  - "crates/caditor/src/sketch_toolbar.rs"
  - "crates/caditor/src/sketch_status.rs"
  - "crates/caditor/src/display.rs"
  - "crates/caditor/src/annotations.rs"
  - "crates/caditor/src/annotation_layout.rs"
  - "crates/caditor/src/model_dimensions.rs"
  - "crates/caditor/src/feature_values.rs"
  - "crates/caditor/src/value_shapes.rs"
  - "crates/caditor/src/typed_point.rs"
  - "crates/caditor/src/viewport.rs"
  - "crates/caditor/src/dimensioning.rs"
  - "crates/caditor/src/blend_curving.rs"
  - "crates/caditor/src/tidying.rs"
  - "crates/caditor/src/tidy_panel.rs"
  - "crates/caditor/src/gearing.rs"
  - "crates/caditor/src/gear_panel.rs"
  - "crates/caditor/src/constraint_trial.rs"
  - "crates/caditor/src/symmetric_drawing.rs"
  - "crates/caditor/src/tangent_circling.rs"
---


# Sketch editing in the app

Default keys come from `Command::default_shortcuts`, not from here. A tool or command listed
without a key has none (every free one is taken) and is reached from the Sketch menu, the palette,
the sketch ribbon's group menus (`app-look.md`) and, for off-ribbon tools, a partner's corner menu.

## Editing context

- A context, not a mode: `editing.rs` holds the edited sketch and the active `Tool`, changed by
  `Action::Editing` commands that `app::perform` routes after the UI pass. It ends by itself when
  the sketch disappears or another document opens.
- Entering records `editing::Entered` (the sketch, the `UndoMark` from when the batch that entered
  it began, and whether a step of that batch added it), as an open feature records `Opened`.
  Cancel sketch (`Command::CancelSketch`, `EditingCommand::CancelSketch`; palette, Sketch menu, the
  compact cross left of Finish sketch on the ribbon, over Look at sketch) takes back every step since that mark as one
  new undoable step: the steps' inverses (`Model::undo_steps_since`) concatenated into one
  `Transaction` labelled by `cancel_title`, so Undo brings the whole session back at once; when the
  only step is the sketch's own creation it is undone instead, so it leaves no trace and Redo makes
  it again. It refuses as an open feature's cancel does (`changes_since`, shared with
  `SketchEditing::cancelling`): `HistoryChanged` once Undo went below the mark, `ChangedElsewhere`
  for a step touching more than the sketch and parameters it owns. Undo inside the session moves
  steps to redo, so they are not among the steps since the mark and stay undone; the cancel step,
  like any apply, clears redo. A new sketch left with no entities or constraints is removed by
  Finish and Escape (`SketchEditing::finish`, `remove_empty`) the same way ("Remove the empty
  Sketch N", or undoing its creation), with an info notice saying Undo or Redo brings it back.
- Opening a feature, entering another sketch, or New sketch closing what was open
  (`editing::switches`) is a switch: `SketchEditing::note_switch` keeps what closed and
  `announce_switch`, run by `app::perform` after the batch, posts "Finished <closed> to edit
  <next>" (or "to start a new sketch"), unless the batch posted a warning or error or the feature
  opened is built on the one closed (`Document::dependents_of`): extruding the edited sketch is the
  expected way on and says nothing, while a switch to unrelated work is told. When the batch
  also changed the document the notice carries a `SwitchBack`: its Undo button
  (`EditingCommand::SwitchBack`) undoes the batch's steps and restores the closed sketch's
  `Entered` or feature's `Opened`. A feature closed for another feature says nothing.
- Entering faces the camera to the plane, fits it, dims other features (unpickable) and keeps only
  that sketch selected; with Preferences › Sketching › Opening a sketch off
  (`Sketching::look_at_on_entering`, handed over by `ViewportState::set_sketching`) the view stays
  where it is. `Command::LookAtSketch` (Alt+Shift+V) returns to the facing view, and turns it a
  quarter turn when already facing.
- The edited sketch, its origin and axes and the drawing preview go on `Layer::Front`
  (`render.md`), so a body never hides or z-fights with them.
- Slice the bodies at the sketch plane (`SectionCommand::SliceSketch`, session-only in
  `ViewportState::sketch_slice`) cuts away the side the normal points to while a sketch is edited:
  `ViewportState::shown_section` puts the edited plane, hatched, first among the section planes
  (`app.md`). Faces on the plane are kept (`section_slack`); the edited sketch is never cut.
- Clicks and primary drags select with Select; a drawing tool (`Tool::draws`) draws and the modify
  tools (`Tool::modifies`) act on what is under the pointer. Escape backs out one step in the order
  of `ViewportState::escape`, ignoring key repeats (`pressed_without_repeat`), so a held Escape never
  walks on to finishing the sketch. Its last step, finishing the sketch, is taken only while
  `Sketching::escape_finishes` (Preferences › Sketching › Escape, on by default); off, Escape stops
  at clearing the selection and the Select prompt names only the Finish keys. The Select and Finish
  tooltips (`SELECT_KEY`, `FINISH_KEYS`) say when Escape reaches them.

## Dragging, box selection and selection commands

- With Select, a primary drag starting on a non-reference entity of the edited sketch is a `Grab`: a
  point, line, arc or spline moves its points by the pointer's offset, a circle alone changes radius,
  and a selected grabbed entity moves every selected entity.
- Each pointer move sends `DragCommand::Move`; `SketchDragging` (in `Display`) gives the newest to its
  worker thread, which drops older unstarted ones and solves each from the previous solution of the
  drag (`solve_from`). Results show through `DisplayedSketches::show_dragged`.
- A grab snaps while snapping is on or Alt is held, and Ctrl is not (`ViewportState::grab_snapping`,
  `Grab::follow`). Its handle (the grabbed point or, for a curve or selection, the moving point
  nearest the press) goes through `tracking::land` on the displayed sketch, ignoring every moving
  point and curve using one; Alt lands as a held snap does (`Hold`). A circle dragged alone changes
  its radius and lands its rim on the sketch point near the pointer only (`Grab::follow_rim`, its centre
  and itself ignored, never a point nearer the centre than `SMALLEST_SNAPPED_RADIUS`, no body items,
  tracks or guides); release adds `Coincident` of that point with the circle (`Join::rim_of`, which
  `reached` also checks against the solved radius). Tracks and extensions work as while drawing, from the grab's own `Acquired`. It snaps to body
  items too (`Grab::snapping_to_bodies`, handed to `tracking::land`, whose `onto_body` takes them):
  where no point-like target was found, a body corner, middle or centre, else an edge; with Alt they
  pull as in the held snap (Snapping). Release sends the
  target's and tracks' constraints (`Landing::joins`, worked out on the displayed sketch each time
  the landing is found and kept in `Grab::joins`; `Join::body` as `BodyJoin`) with
  `DragCommand::Finish`; `Model::commit_drag` adds them (projecting body items, or reusing the
  projection) to the settle transaction when the solved point reached the landing, leaving out any
  the sketch refuses, already has or contradicts.
- Release commits one `settle_sketch` transaction and the dragged shape stays until an evaluation of
  that revision reaches the sketch, so it never jumps back. Escape, another edit or document, or a
  drag begun on an older revision drops it. A drag still solving at release carries its revision in
  `Finished`; `Model::commit_drag` drops a solution arriving after the revision moved on (an undo
  meanwhile) with a notice, so the undo and its redo stand.
- When the worker cannot solve its newest frame (`Polled::blocked`, `Model::drag_blocked`), the
  viewport says `DRAG_BLOCKED` beside the pointer (polite live region) and the geometry stays at the
  last good frame; with conflicting constraints `DRAG_CONFLICT` names them (`Model::sketch_conflict`),
  as does the notice when the drag ends. A drag starting on projected geometry or the edited sketch's
  origin or axes is `PrimaryDrag::Unmovable`: nothing moves, no box, and it says why until release
  (`PROJECTED_STAYS`, `REFERENCE_STAYS`).
- A primary drag elsewhere draws a box: left to right a window taking what lies inside (curves
  faceted as drawn, `app.md`), right to left a crossing box; with Select and a lasso on, a freehand
  outline (`app-input.md`). It replaces the selection (Shift or Ctrl adds); a point is left out when a
  curve it belongs to was taken. Double-clicking a curve (no tool) selects its chain, the lines and
  arcs joined end to end that Offset would take (`Sketch::offset_chain_through`).
- Select what is still free (`Command::SelectFree`, `sketch_drag::select_free`) selects points and
  curves not `EntityState::FullyConstrained` in the settled solution (`Model::settled_solution`),
  leaving out projected and reference geometry and points of taken curves; refused with `NOT_SOLVED`
  or `NOTHING_FREE`. The degrees-of-freedom pill runs it. Select all and Select free check
  availability each frame with early exits (`can_select_all`, `can_select_free`).
- Split (`Command::SplitCurve`, `sketch_tools::SplitChange`) takes one selected point and the line,
  arc or elliptical arc selected with it or the one such curve it lies on by `Coincident`; else
  `NOTHING_TO_SPLIT`. A point placed on a curve first (the Point tool snaps onto it) gives a split
  anywhere. Break (`Command::BreakCurves`, `BreakChange`) breaks each selected line, arc and
  elliptical arc at every crossing with other curves and the axes in one transaction (`sketch.md`,
  Break); else `NOTHING_TO_BREAK`. Respace (`Command::RespaceFitSplines`, `RespaceChange`) turns
  selected evenly spaced fit-point splines (files saved before centripetal spacing) into centripetal
  ones (`Sketch::respace_fit_spline` through `reshape_sketch`); else `NOTHING_TO_RESPACE`.
- Move selected geometry opens the typed-point field (`app-input.md`) as "Move to", solved like a
  drag; button and command share availability (`Moving::offered`). Positions and the relative anchor
  are taken afresh from the displayed sketch at Enter (`Moving::refreshed`, `Transforming::refreshed`),
  so a drag or undo made while the field waited is not undone; gone geometry or a recalculating
  sketch leaves the field open with the reason; a recalculating one also queues the entry
  (`queued::Rerun::Typed`, `ViewportState::resubmit_typed`), entered again once the sketch is shown,
  and closing the field drops it. Rotate and Scale (`Command::RotateGeometry`,
  `ScaleGeometry`; `sketch_drag::Transforming`) open the same field as "Rotate by" (degrees unless a
  unit is named, counter-clockwise) or "Scale by" (a factor above zero), acting about the one selected
  point no selected curve uses (a lone point or the origin), else the centre of the selection's
  extent (circle rims and whole ellipses included, `ellipse_half_extent`); a scale also scales
  selected circles' radii. A lone point with nothing to turn about is refused in words.
- Copy, Cut and Paste (`Command::CopyGeometry`, `CutGeometry`, `PasteGeometry`) work through a
  `SketchClip` (`Sketch::clip`): selected curves with their points, lone points, construction flags
  and every constraint among them (inactive staying inactive), never reference geometry or a
  constraint reaching outside. Copy writes system clipboard text
  (`caditor_file::sketch_clipboard_text`, `file-format.md`) with the parameters its dimensions use, so
  another caditor pastes it; the last text copied stands in only when the clipboard holds no text.
  Paste (`read_sketch_clipboard`) refuses foreign, damaged or newer text, or features, in a notice.
  It is one transaction (`Sketch::paste` on the working copy under fresh ids, a `Fix` moving with the
  copy), centred under the pointer when on the sketch, else in place in another sketch or shifted by a
  quarter of the copy's size into the sketch it came from, and selects the result. Dimensions keep
  same-named parameters of the target model, else take the copied value, and a notice says how many
  became numbers.

## Drawing tools

- `drawing.rs` holds the tools' state and `shapes.rs` their geometry (per-shape constraints and
  degrees of freedom are in `shapes.rs` and its tests). The drawing is keyed by a `Shape` (tool plus
  way). Changing either carries the points placed so far when they mean the same in the new shape
  (`carries_points`); otherwise a tool change drops the shape in progress, while a change of way
  within a tool (`Drawing::refuses_mode`, checked in `app::perform` before `SetMode`) is refused in a
  notice telling to finish or cancel first. Each finished shape is one transaction, settled first;
  inferred constraints are checked with `Sketch::check_constraint` on a shadow sketch and skipped if
  refused. A shape with no size gets a `Refusal` (a notice for a click, the field error for a typed
  point).
- A press that drags or is held too long to be a click still places a point at its release (egui
  reports a drag, `ViewportState::click`). With nothing placed and the release at least
  `DRAG_DRAWS_FROM_PRESS` from the press, the press position is placed first (`place_press_point`),
  so one press-drag-release draws a line, rectangle or circle; in a line chain a drag from its last
  point draws a tangent arc. `Drawing::readout` shows the size so far below the snap label.
- Ctrl places the point exactly under the pointer: no snapping, no guides (`Drawing::place_freely`),
  and no tangent arc start. Snapping off (`Command::ToggleSnapping`, session-only in
  `ViewportState::snapping`, starting as Preferences › Sketching says, `StartingToggles`) equals holding Ctrl always, except Alt still snaps; the prompt then says
  "Snapping off" before the Alt hint. Ctrl wins over Alt (AltGr arrives as Ctrl+Alt on Windows). The
  pointer is on the sketch only within `MAX_LENGTH` of the origin, so an edge-on view cannot place a
  point at an enormous distance.
- Lines and tangent arcs chain, each joined to the last end by `Coincident`, until Enter, Escape, a
  click on the last point, or a line closing on the chain's first point (`Drawing::finish` gives
  `Ended::Stopped`). Splines finish on Enter or a click on the last point and close (periodic,
  `SplineKind`) on a click on the first once three are placed (a typed point there too); Enter with
  too few points keeps them and is refused (`Refusal::SplinePoints`, `ClosedSplinePoints`). Backspace
  or Delete (`KEPT_WHILE_DRAWING`) takes back the last point; `Command::TakeBackPoint`, `FinishShape`
  (chains and splines only, `Drawing::finishable`) and `CancelShape` do the same without keys. Each
  continuing segment records its anchor (`ChainStep`); taking back undoes the last segment when it is
  the newest undo step, and when the anchor's point is gone the chain steps back to the newest anchor
  still there rather than ending.
- An arc runs the way the pointer swept round its centre; a typed end goes the shorter way
  (`Sweep::aim`), and Reverse sends either the other way. The end is projected onto the circle through
  the start and keeps its snap only if the target lies on that circle.
- An ellipse is its centre, the major axis end (levelled from the centre like an arc's start, kept by
  `HorizontalPoints` or `VerticalPoints`) and a width point that never snaps, its distance from the
  major axis the minor radius (`shapes::ellipse_through`). An elliptical arc's third point sets the
  minor radius likewise and starts the arc where the ray from the centre through it meets the ellipse
  (`shapes::toward_on_ellipse`); its end is the same projection, running the way the pointer swept
  (`Shape::sweeps_from`).
- A conic (`Tool::Conic`, Curve group) is its start, end and apex, where the end tangents meet
  (`Draft::conic`, a `SplineKind::Conic`); an end on its start (`Refusal::ConicEnds`) or an apex on
  the line through the ends (`ConicApex`) is refused. Its rho (0.5 until set, kept while caditor
  runs) is typed as `0.3 rho` (`typed_point::rho`, dimensionless, 0.01 to 0.99); an expression using
  parameters also gives the conic a `Rho` dimension holding it.
- A tangent arc starts on a point ending a line, arc or spline (the newest if several) and leaves
  along that curve's direction with a `Tangent`. Switching between Line and Tangent arc with a
  segment started (`Drawing::sync`) keeps the chain and anchors.
- The Line tool turns its next segment into a tangent arc without a tool change (`Drawing::arcing`,
  `Arcing`; `Drawing::current` then draws, previews and prompts as `Shape::TangentArc` from the
  chain's last point), then carries on with lines. A press on the last point dragged away is
  `Arcing::Dragged` (`ViewportState::arc_from_press` hovers the press position again at the drag's
  start, `Drawing::arc_from_press` takes it when the hover is the pending last point ending a line,
  arc or spline): the release places the arc's end, and a release back on the last point draws no arc
  and keeps the chain. The arc bends to the pointer's side of the line (`shapes::tangent_from`).
  `Command::ChainArc` toggles `Arcing::Toggled`, and the Tangent arc command (T) with the Line tool
  holding a point runs it instead of switching tools (`ViewportState::arc_on_tangent_arc_key`, before
  the sketch bar takes the command); `Drawing::chain_arc` gates it, refused with
  `NOTHING_TO_ARC_FROM`. A drawn arc, a dragged arc's release, taking back or stepping back return to
  lines.
- Draw symmetrically about the selected line (`Command::DrawSymmetrically`, `symmetric_drawing.rs`)
  takes the one line or axis selected (`symmetric_drawing::chosen_line`, `NO_LINE_SELECTED`
  otherwise), keeps it in `ViewportState::symmetry` for that sketch; run again it turns off, and it
  turns off itself when the sketch is no longer edited or the line no longer resolves (`Mirror::of`,
  worked out each frame in `track_drawing`). While on, `Drawing::draw_about` makes `Drawing::click` and
  `finish` pass each shape's transaction through `symmetric_drawing::mirrored`: `Sketch::mirror` of the
  new non-projected entities gives the image, appended to the same transaction labelled "Draw line
  symmetrically", so one undo takes both halves (`Drawing::click` stores that label in the chain step
  so taking back finds its segment as the newest undo step). The preview gains the reflection
  (`Mirror::add_image`). A shape that is its own image is drawn once.
- What a placed point may snap to is `Drawing::accept`; width points of slots and three-point
  rectangles never snap.
- The polygon side count (`MIN_SIDES` to `MAX_SIDES`) is kept until another document opens
  (`ViewportState::forget_document`); More sides and Fewer sides show only with Polygon. Typing
  `N sides` (`typed_point::sides`) sets it. Holding Shift with a polygon started freezes the preview
  and scrubs the count by one per `SCRUB_POINTS_PER_SIDE` of sideways travel (`Drawing::scrub_sides`).

## Ways of drawing a shape

- Rectangle, circle, polygon and slot each keep one tool with several ways (`ShapeMode`), so the
  ribbon and keymap stay small and a shape is found under its one key. Arcs stay separate tools under
  one Arc button, and Spline, Ellipse and Elliptical arc under one Curve button (`app-look.md`),
  since an ellipse is a curve of its own, not a way to draw another.
- Spline has four ways (`SplineMode`): control points, fit points (the curve passes each placed
  point, which stays a point to constrain and dimension, spaced centripetally, `SplineKind::fit`),
  and both closed, where Enter closes the loop. Clicking the first point closes either open way.
- Blend curve (`ShapeMode::Blend`, `ShapeModes::blend`: tangent G1 or curvature-continuous G2) and
  Offset (`ShapeMode::Offset`, `ShapeModes::offset`: three ways) keep theirs likewise; the modify
  tool reads the way each frame and its prompt leads with it.
- Running a tool's command while it is active steps to its next way, round to the first; clicking
  its ribbon button only chooses the tool (`sketch_toolbar::partnered_tool_button` for compact
  buttons, `off_ribbon_tool` for off-ribbon tools). Every way is also its own command
  (`Command::ShapeMode`, id `sketch.<shape>.<way>`) in the palette, the corner menu and Sketch ›
  Ways to draw shapes.
- `SketchEditing` remembers the last way per shape (`ShapeModes`) while caditor runs, across
  sketches and documents; it is not a preference, as no tool state is.

## Projecting model geometry

- Project (`Tool::Project`, Alt+P, sketch bar Edit group) brings model geometry in: while active,
  `Context::projecting` draws bodies and other sketches as pickable background
  (`Presence::Projectable`) and `Pickable::is_available` lets their faces, edges, corners and curves
  be hovered; it also draws the shown principal axes and origin and the shown datum axes and points
  above the sketch (`Builder::projectable_references`). A click or Activate runs
  `projecting::project` over `projections_of` (shared with a hole's Place by): an edge, a corner,
  every boundary edge of a face not seen end-on, a curve or point of a sketch above it, a datum or
  principal axis (a line spanning the shown bodies, `axis_reach`, or a point where it stands square
  to the sketch), a datum point or the origin, each one undoable transaction from the body's state at
  the sketch (`body_result_seen_by`). Geometry made later in the tree, a datum plane (Intersect takes
  it), a corner shared by two vertices of one name and anything already projected
  (`ProjectionSource::is_same_source`) are refused in words. The highlight commands step only
  through projectable items.
- Intersect (`Tool::Intersect`, Alt+I; in Project's corner menu, `sketch_toolbar::OFF_RIBBON`,
  `MODEL_GEOMETRY_TOOLS`) draws where the model crosses the sketch plane. It is a projecting tool
  (`Tool::projects`); `Context::intersecting` also offers shown datum planes above the sketch and
  shown principal planes crossing it. A click on a face adds the cut through that face
  (`section_curves`, filtered by the face's name), Shift-click or `Command::IntersectBody`
  (Shift+Space) the cut through its whole body, a click on a datum or principal plane the line where
  it crosses the sketch as a construction line spanning the shown bodies (`datum_reach`). Each is one
  undoable transaction; a plane missing the face or body, a parallel plane, a later datum and cuts
  already drawn are refused in words.
- Drawing tools project body corners and edges themselves when a point snaps to one (Snapping).
  Projected geometry is drawn in `ScenePalette::projected`, is never grabbed or dragged and follows
  its source on every recompute (`document.md`).

## Construction geometry and control polygons

- Construction makes the selected curves construction in one transaction
  (`sketch_tools::ConstructionChange`), ordinary when all already are; with no curve selected it
  switches drawing (`ActiveSketch::construction`): new curves are construction, points stay points.
  Construction curves are dashed (`scene::curve_segments`) and coloured by constraint state like any
  curve, so dashes never carry a constraint state.
- Control-point splines (open or closed) and conics of the edited sketch show their control polygon
  (`scene::control_polygon`, `Sketch::spline_control_points`): dashed, in the curve's state colour,
  never picked. Fit-point splines show none. `Command::ToggleControlPolygons`
  (`ViewAids::control_polygons_hidden`, shown by default) hides them for the session.

## Constraint states without colour

- Each sketch entity has a `SketchState` (under or fully constrained, conflicting, redundant,
  failed, projected, background) drawn by the palette's `Look`. In high contrast the form says
  whether it can move: fully constrained, projected, redundant and conflicting curves are heavy
  (`heavy_curve_width`) with solid points; under-constrained curves are regular with hollow points
  (the point marker plus a smaller canvas-coloured `hole` marker). The standard palette draws every
  state in one form.
- The status pill names the sketch's problem (conflict and redundancy never share a sketch: a
  conflict fails the solve); in high contrast the involved constraint marks also get a frame
  (`annotations::paint_frame`): solid for conflicting, dashed for redundant.

## Trim, extend, offset, mirror, patterns, gear, blend and sketch fillet

- `trimming.rs` and `modifying.rs` (a facade over `offsetting.rs`, `mirroring.rs`, `patterning.rs`,
  `filleting.rs`) hold the tools' UI state; geometry and constraint rules are the sketch's
  (`sketch.md`). Each frame the tool aims through the displayed sketch, previews the result and puts
  its words, or why not, where hover descriptions go. Every curve is Trim's target
  (`trimming::is_trimmable`); Extend takes the ends of lines, arcs, elliptical arcs and open splines
  (`trimming::ends_of`) and refuses whole ellipses and closed splines in words. The axes and collinear or co-circular overlaps
  cut and bound curves like any curve (`sketch.md`): the words name them and the preview highlights
  the overlapping curve; the axes themselves are never aimed at.
- A click acts at once, without a GPU pick: one transaction built by `reshape_sketch` from a working
  copy (the definition with the displayed positions), settled first. A refusal is a notice (or for a
  typed value the field's error) and the tool stays active.
- Trim's primary drag is a trim path: every piece the path crosses is trimmed on release in crossing
  order as one transaction, each found again by its middle on whichever curve now carries it after
  earlier splits. Extend ignores drags.
- Offset works on the selected chain; with none, a click on a curve selects its chain. The pointer's
  side and distance choose side and distance, previewed live; an ellipse's words end
  `offsetting::FREE_SPLINE`, since its offset spline does not follow it (`sketch.md`). `OffsetMode`
  (one side, both sides with round ends, both with flat ends): the both ways put the distance on each
  side (pointer side irrelevant, a negative typed value its magnitude) as one "Offset curves to both
  sides" transaction (`Sketch::offset_both_sides`). Mirror copies the selection about the line or axis
  under the pointer (sketch lines win a tie with an axis).
- Mirror and both patterns select inside the tool, as Smart dimension picks: started with nothing
  selected (or once it empties) they gather (`Modifying::gathers`): a click on a sketch item toggles it
  (`ViewportState::gather_click`), a primary drag draws a box or lasso that adds, and the highlight
  commands step through the scene's pickables with Activate toggling. Enter moves Mirror on to its line
  and Circular pattern to its centre (`finish`, refused while nothing is selected); Rectangular
  pattern gathers throughout. Escape goes back from the line or centre to gathering, then lets go of
  what was gathered (`Modifying::lets_go_of_selection`), then leaves the tool; started with a
  selection they go straight to the line or centre and Escape leaves the tool keeping it.
- Rectangular and Circular pattern (`Tool::RectangularPattern`, `CircularPattern`, `patterning.rs`;
  Mirror's corner menu) repeat the selection (`sketch.md`, Patterns). Numbers go in the typed-point
  field ("Repeat"), opened by a digit or `=`, previewed live while the text parses; Enter makes one
  "Pattern geometry" transaction and an error keeps the field open. Rectangular reads
  `count x spacing`, optionally `< angle`, and a second term after a comma for rows (square to the
  first unless it has its own angle): `4 x 10`, `4 x 10, 3 x 15`, `3 x 12 < 45`. Circular reads
  `count` (a full turn) or `count over angle`. Counts are whole numbers; spacings and angles are
  expressions with parameters, kept as typed. Circular first needs its centre: the one selected point
  no selected curve uses, else a point clicked or highlighted (origin included), shown highlighted
  until Escape lets it go.
- Tangent circle (`Tool::TangentCircle`, `tangent_circling.rs`; Offset's corner menu) takes the
  lines, circles and arcs the new circle touches (`sketch.md`, Tangent circles): the one or two
  selected at start, then each one clicked or highlighted and chosen with Space or Enter (axes
  included; clicking a chosen curve lets it go, Escape lets go of the last). A third curve finishes it
  at once, as the circle whose centre is nearest the pointer (so the pointer inside or outside a
  triangle's sides picks its incircle or an excircle). With two chosen the typed-point field
  ("Radius") draws the circle of that radius nearest the pointer. One "Draw tangent circle"
  transaction.
- Spur gear (`Tool::Gear`, `gearing.rs`; Polygon's corner menu) draws an involute gear outline, a
  pair of meshing gears or a roller chain sprocket (`sketch.md`, Spur gears). While active, the Spur
  gear panel (`gear_panel.rs`, a right-hand panel closing back to Select) opens with a segmented
  control for the `GearKind` (Single gear, Gear pair, Sprocket) and holds that kind's fields
  (`GearKind::fields`): Module, Teeth, Pressure angle, Profile shift, Root fillet and Bore for a gear,
  with Meshing teeth and Meshing bore for a pair, and a Chain combo (`ChainSize::ALL`) with Teeth and
  Bore for a sprocket (teeth from `MIN_SPROCKET_TEETH`). They are text fields
  (`patterning::quantity`: units and expressions with parameters), kept in `Modifying`
  (`GearSettings`, which evaluates the shown fields into a `Part`) while caditor runs, and the panel
  lists the derived diameters or the refusal. The pointer previews the wheels (`Wheel`) on the point
  under it (origin included) or where it is, a pair's second gear to the right of the first; a click
  draws them there; Enter or the panel's primary button draws at the one selected point, else the
  origin; Space or Enter on a highlighted point draws there. One "Draw spur gear", "Draw gear pair" or
  "Draw sprocket" transaction (`Part::transaction`).
- Blend curve (`Tool::BlendCurve`, `blend_curving.rs`; Alt+Shift+B, Offset's corner menu) joins two
  curve ends with a spline (`sketch.md`, Blend curves). It takes the end of a line, arc or spline
  (projected included) nearest the pointer on the curve under it, or a selected point ending exactly
  one curve; the highlight commands step through every such end and Space or Enter chooses. Clicking
  the chosen end again lets it go, Escape lets go of the highlight and then the end. With one end
  chosen, the second previews the spline live; a click or Activate draws it in one "Draw blend curve"
  transaction.
- Sketch fillet is named so to keep it apart from the model's Fillet. It first takes its corners:
  started with a selection, every selected point that is a corner (`Sketch::corner_at`) and every
  corner where two selected lines, arcs or elliptical arcs meet (`Sketch::fillet_corners`),
  duplicates dropped (`filleting::gathered`), the words saying how many were left out with the first
  one's reason; two selected curves meeting at no corner make one where they cross or would meet
  (`filleting::Aim::Crossing`, `Sketch::crossing_picks`); else the curve end under the pointer. A
  click on a line, arc, circle or elliptical arc away from a corner picks it as the first curve and a click on a
  second chooses the corner where they cross or would meet (`Sketch::join_at_crossing` on a copy,
  kept in `Filleting::joined` by the `SketchVersion` of the displayed sketch (feature, document
  revision, evaluation and displayed-sketch generations) and by the aims, a crossing aim by its
  `CrossingPlan`: which crossing and which ends move, so a pointer moving along the second curve
  copies the sketch again only when the kept side changes). A click, or Space on a
  highlighted corner, adds corners. Then the pointer sets the radius through the first corner
  (`radius_through`), previewed on all, and a click, typed value or Enter rounds all in
  one transaction ("Fillet corners" when several), each corner found again by its point on the working
  copy as earlier ones change it; chosen corners clear after each fillet. The last value committed is
  kept per cut while caditor runs (`Modifying`, `LastSizes`): it is the field's placeholder and Enter
  on an empty field, or with the field closed and corners chosen, reuses it; with none kept, Enter
  rounds at the pointer.
- Sketch chamfer (`Tool::Chamfer`) is the same tool cutting the corner (`filleting::CornerCut`): the
  pointer sets one distance for both sides (`distance_through`) and the field "Chamfer" reads `5`
  (both curves), `5, 3` (a distance on each, in the corner's curve order) or `5 < 45` (a distance on
  the first curve and the cut's angle from it) as a `ChamferSize` (`sketch.md`). Its compact button
  sits beside Sketch fillet's in the Modify group.
- Tools without a button (`sketch_toolbar::OFF_RIBBON`) sit in the corner menu of a partner's
  compact button (`sketch_toolbar::Partners`, `widgets::compact_corner_menu_button`): Offset with
  Tangent circle and Blend curve (`CURVE_FROM_GEOMETRY_TOOLS`), Mirror with both patterns
  (`COPYING_TOOLS`), Project with Intersect (`MODEL_GEOMETRY_TOOLS`). The notch takes the accent
  colour while any is active, and its hover lists the tools inside (`app-look.md`).
- Offset, Sketch fillet and Sketch chamfer take a typed value in the point field: the preview follows
  while it parses, Enter commits the expression as typed (parameters included) and an error keeps the
  field open. A negative offset goes to the other side, so the keyboard alone does it.
- With any of these tools active the highlight commands step through that tool's targets instead of
  the scene's pickables (`app-input.md`), and Activate or Enter acts on the highlighted one.

## Snapping

- `snap.rs` runs on the UI thread against the displayed sketch in screen space. Priority: the shape's
  pending point; points and the origin within `POINT_TOLERANCE`; then, where any snap is accepted and
  within the same tolerance, the middle of a line, arc or elliptical arc (`Target::Midpoint`), the
  centre of a closed outline (`Target::Centre`), the crossing of two of the up to
  `MAX_CROSSING_CURVES` curves nearest the pointer, axes included (`Target::Intersection`,
  `Sketch::curve_crossings`), quadrants of a circle or arc (`Target::Quadrant`) and ellipse axis ends
  (`Target::AxisEnd`); then lines, circles, arcs, splines and axes within `CURVE_TOLERANCE`,
  projecting onto the curve. A snapped point gets a `Coincident` with its target (both curves at a
  crossing), a `Midpoint` for a middle, a `Symmetric` of two opposite corners about it for a centre
  (a `Centroid` of the corners for an odd outline's), and for a quadrant a `Coincident` plus a horizontal or vertical points constraint with the centre.
  A major axis end is held by a `Symmetric` of the axis point and the new point about the centre; a
  minor axis end by a `Coincident` with the ellipse plus a vertical or horizontal points constraint
  with the centre when the major axis is level or upright (`AxisEnd::MinorUpright`, `MinorLevel`), else
  an `OnMinorAxis` (`MinorSlanted`).
- A line's end within `POINT_TOLERANCE` of where a line from its start would touch a circle, arc,
  ellipse or elliptical arc (`snap::tangents_from`) lands there (`Target::Tangent`): a `Coincident` and
  a `Tangent`. It wins over curve snaps, not over point-like ones (`Target::is_point_like`). A line
  starting on a circle or arc, and the first side of a three-point rectangle or a polygon drawn by its
  side, is offered the tangent direction there (`Direction::Tangent`).
- An outline has a centre when its lines and arcs join end to end into one closed loop (ends within
  `JOINED_CORNER_TOLERANCE` of the sketch's extent, each corner meeting exactly one other piece, at
  most `MAX_OUTLINE_LINES` pieces) of an even count whose opposite corners share one midpoint and
  whose opposite pieces are both lines or both arcs of one radius with centres mirrored about it
  (rectangle, parallelogram, even regular polygon, slot, rounded rectangle); a third piece at a corner
  breaks the loop. An odd loop of lines has the centre of its corners (their mean; none for a loop
  of no area) as centre (`Target::Centroid`), held by a `Centroid` of the corner points the outline
  search finds again from the target's piece (`snap::outline_corners`, one point per corner, so
  `Target` stays `Copy`). A loop with a piece or corner moving in a drag offers no centre.
- `Accept` keeps every shown snap a constraint that already holds: a circle's rim takes points only;
  an arc's end takes points on its circle and where it crosses other curves
  (`Sketch::circle_crossings`) and the axes.
- A line end (and the second point of either straight slot, of a three-point rectangle's first side
  and of a polygon's side) within `ALIGN_ANGLE_DEGREES` or `ALIGN_TOLERANCE` of a direction from its
  start takes it exactly (`Snap::Aligned`). Horizontal and vertical win whenever either applies;
  otherwise parallel or perpendicular to one of the `NEARBY_LINES` lines nearest on screen
  (construction included, `drawing::guides`). A line is never inferred parallel to a line through its
  start point; perpendicular is offered. An arc's start (and an arc slot's ends), a polygon's corner
  from its centre and each spline control point from the one before take horizontal or vertical the
  same way (`Drawing::levelled_from`, `LEVEL_AND_UPRIGHT` only, `Direction::between`).
- Geometry snaps win over a direction, which joins only where compatible (`Snap::AlignedOn`): a point
  snap keeps a direction that already holds (`HELD_TOLERANCE`) and never moves the point; a curve snap
  moves to where the direction's ray crosses the curve, within `ALIGNED_CROSSING_TOLERANCE`. Typed
  points never align, and a locked heading (`app-input.md`) replaces every snap.
- Every point-like sketch target the hover snapped to is acquired (`tracking::Acquired`, newest
  first, at most `MAX_ACQUIRED_POINTS`): a point or the origin, the middle of a line, arc or
  elliptical arc, a crossing of two curves, an outline's centre (`Centre`, `Centroid`), a quadrant
  and an ellipse's axis end, each kept as its `Target` with where it lay (`Anchor`), which tells
  the two crossings of one pair or the two minor axis ends apart (`Target::has_several_places`).
  So are the centre of a circle, arc or ellipse whose rim, middle, quadrant, axis end or centre was
  hovered and the lines ending at a hovered point (`MAX_ACQUIRED_LINES`). Kept across tools within
  one sketch; `Acquired::retain` drops an anchor once an entity defining it is gone
  (`Target::involved`) and moves a several-placed one to where it lies now. Body items are never
  acquired.
- Each frame an anchor is found again where it lies now (`snap::position_of`: the crossing or minor
  axis end nearest where it was; an odd outline's centroid by the outline search from its first
  piece). One moving in a drag (touching the ignored entities, or an odd outline with a moving
  piece) or placed by the shape in progress (`own`) draws no track.
  Where any snap is accepted, a placed point within `TRACK_TOLERANCE` of the horizontal or vertical
  through an anchor (at least `MIN_TRACK_LENGTH` away) lands on it (`Tracks`), both at once where
  they cross. A track joins a curve snap where it crosses the curve (never along a line it runs on)
  and a line's direction where they cross, within `ALIGNED_CROSSING_TOLERANCE` (`tracking.rs`);
  point snaps never take one.
- Every track holds the placed point at its anchor (`Track::constraint`): the anchor's
  `Target::point_target` (`sketch.md`, point targets: a point, a middle, a crossing, the two
  opposite corners of a `Centre` or the corners of a `Centroid`, a quadrant, an ellipse's far
  major or nearer minor end) in `HorizontalPoints` or `VerticalPoints`, so the point follows the
  middle, crossing or centre when its geometry moves, with no construction point left in the
  sketch. The label names held tracks first and ends `NOT_KEPT_THERE` when a track's anchor has no
  target (none does today). Where two tracks of one kind coincide (a right quadrant level with its
  centre), a held one wins, then one through a sketch point, the simpler constraint (`prefers`,
  within `TIED_OFFSET`). The canvas marks them H or V on the point (and on a target that is a
  sketch point); `OnDirection` is `GlyphKind::Through` (a slanted stroke through a ring) and
  `Centroid` `GlyphKind::Centre` (a triangle round a dot).
- An anchor also tracks along each acquired line's direction and square to it (`Axis::Slanted`),
  never along a line through the anchor itself nor where the direction is level or upright, and the
  middle of a line tracks square to that line (`bisector`, labelled "on the perpendicular bisector
  of"; for a level or upright line it is the upright or level track). A slanted track holds the
  point by `OnDirection` along or square to its line through the anchor's target; crossing a
  horizontal or vertical track, both hold. An acquired line extends past its ends: after curves, the pointer within
  `CURVE_TOLERANCE` of its infinite carrier snaps to it (`Target::Extension`, a `Coincident` on the
  line, which the solver treats as infinite); directions and tracks join it as they do a curve.
- Each kind of constraint drawing infers can be turned off in Preferences › Sketching
  (`inference.rs`: `Inference`, a set `Inferences` that is on for every kind unless stored off;
  `Sketching::inferred`, set on the `Drawing` every frame, `Drawing::infer`). A kind is the
  direction guides horizontal, vertical, parallel, perpendicular and tangent
  (`Inferences::allows_direction`; tangent also covers a line end touching a round), the targets a
  point snaps to (`Inferences::allows_target`: coincident for a point and a body corner, on a curve
  for curves, axes, extensions and body edges, midpoint, centre for an outline's centre and a round
  edge's centre, intersection, quadrant with ellipse axis ends) and the tracks. A disabled kind is
  filtered where candidates are chosen, never afterwards, so the next candidate takes over
  (`snap::nearest_within`, `BodyPart::takes`, `drawing::guides`, `level_and_upright`, the tangent
  touch and the tracks in `Drawing::place`, `level_on_grid`); the preview, the label and the
  constraints committed all read the one placement, so they agree. A shape's own structure (a
  chain's joins, a rectangle's level sides, a typed value, Alt's held snap beyond these filters) is
  not an inference. Drags and grabs infer everything (`Inferences::default` in `tracking::land`),
  and the dimension-from-datum and Constrain automatically panels are unaffected.
- Snap to the grid (`Command::ToggleGridSnapping`, off by default, session-only in
  `ViewportState::grid_snapping`, starting as Preferences say) places a point no target, direction or track took on the nearest
  crossing of the grid's current level (`ViewportState::snap_grid_spacing`, handed to
  `Drawing::snap_to_grid`) within `POINT_TOLERANCE`; the point stays free, labelled "On the grid"
  (`Snap::Grid`). The grid draws three levels, 1, 10 and 100 times `grid_minor_spacing`, each
  fading in as its cells grow past 4 points and drawn in full from 16; the current level
  (`caditor_render::grid_snap_spacing`) is the finest drawn in full, else the coarsest, and cells
  are measured on screen at the view's target. The finer lines still fading in are never snapped
  to; coarser levels lie on the current one's crossings. The canvas label ("Grid 10 mm",
  `LengthUnit::grid_text`) names the same spacing.
- Holding Alt while drawing or dragging holds the snap (`snap::held`, `Hold`, `Drawing::hold_snap`,
  ahead of directions, tracks and the toggles): the point lands on the nearest crossing of the same
  current level, grid snapping on or not, unless a snappable entity lies no further on screen. Point-like targets pull
  from `HELD_POINT_PULL` (three times the hover tolerance), then curves and extensions from
  `HELD_CURVE_PULL`, a curve snap moving to the grid crossing nearest its foot when that lies on the
  curve. Targets keep their constraints; a grid crossing is free and, for points that align or level
  from a start (`aligned_from`, `levelled_from`), exactly level or upright with it takes Horizontal or
  Vertical. Width points never snap, typed points are untouched, and with the grid hidden only
  entities are taken. Body items pull too (`Lookup::bodies`, none for a tangent arc's start or a
  circle's rim drag): corners, middles and centres after point-like sketch targets
  (`BodySnaps::point_within`), body edges after sketch curves (`BodySnaps::edge_within`), an edge
  moving to the grid crossing nearest its foot when that lies on it (`BodySnaps::foot`).
- While a drawing tool draws, the shown bodies standing at the sketch (`Evaluation::body_state_seen_by`;
  later or hidden bodies left out) are snapped to too (`body_snap.rs`, `Target::Body`): corners, edge
  middles projecting to a line or arc, round edge centres, and any point on an edge, each matched where
  it projects onto the sketch plane as Project would draw it. Every item of a shown body is offered,
  hidden or not (a hidden corner projects where its hider is, and an occlusion test per hover costs
  more than it saves). Where two project to one place the one nearer the sketch plane wins
  (`BodySnaps` keeps candidates sorted by depth). Priority: point-like sketch targets, body corners,
  middles and centres, sketch curves, body edges. `Accept::Points` takes body corners and centres
  only, `OnCircle` none, nor does a tangent arc's start; directions' and tracks' crossings and
  acquired points take none, as a body item is not in the sketch until a point lands on it.
- A point landing on a body item projects that vertex or edge in the shape's own transaction
  (`Draft::projection` through `TransactionBuilder::add_projected`: the same fixed geometry and stable
  `ProjectionSource` Project makes, or the existing projection, once per shape) and joins it:
  `Coincident` with the projected corner or round edge's centre, `Midpoint` on the projected edge,
  `Coincident` on it; where a shape uses a target point itself (a circle through it, a polygon's side
  middle) it uses the projected point. One undo takes the drawing and its projections away together.
  Candidates come from `BodySnaps`, a cache per sketch session of corners (one sharing its name with
  another vertex is left out, as Project refuses it) and non-seam edges, held in `ViewportState`
  behind an `Arc`, rebuilt only when the edited sketch, its plane or a shown body's result `Arc`
  changes (`Basis`), and shared with `Drawing` (`Drawing::use_bodies`), a grab and the constraint
  offers. A placement made from an older cache (its `generation`) lands free.
- With Select or Smart dimension, a body item is picked as a sketch item is
  (`ViewportState::body_hover`, `Pickable::BodyItem` with the edited sketch, the body and a stable
  `BodyItem`: a corner by its `VertexName`, a round edge's centre or an edge by its `EdgeName`, so a
  pick survives a cache rebuild): where the GPU hover is no sketch point, the nearest body corner or
  round-edge centre within `POINT_TOLERANCE` is hovered, else, where no sketch curve either, the
  nearest body edge; drawn by `ViewportState::body_preview`, available only while that sketch is
  edited. The highlight commands reach them after the scene's pickables
  (`ViewportState::steppable_body_items` from `BodySnaps::steppable`: corners and round-edge
  centres, then edges, each place or path once, the one nearer the sketch plane kept), the
  highlighted one drawn by `body_preview` as a hovered one and kept while the cache holds it;
  Activate picks it as a click would and Show the context menu opens at it
  (`ViewportState::body_item_on_screen`). `body_picks::Staged` turns picked sketch entities and body items into one sketch: each body
  item is projected through a throwaway transaction (`add_projected`, reusing the sketch's existing
  projection of that source or one already staged), its entities inserted into clones of the
  definition and displayed sketch as projected; staged ids are those the real transaction allocates,
  since a sketch's ids come from its counter in order. Smart dimension (`dimensioning::dimension`,
  `Staged::names`) and the constraint offers (`ConstraintOffers`, whose basis holds the body picks and
  the cache's generation) work on that sketch, and `sketch_tools::add_constraints` adds the staged
  `Projections` first in the constraint's own transaction, so one undo removes both. Staged projected
  entities count as determined for the reference rule. A body item whose source left the cache is
  refused naming its body (`Unstaged::Gone`).
- Preview curves are faceted like the sketch's (`Drawing::preview` and `Trimming::preview` take the
  scene's `Faceting`); the snap target, a direction's reference line and each track's anchor (its
  point, or the curves its middle, crossing or quadrant lies on) (`Drawing::snap_entities`) replace
  the GPU hover while a drawing tool is active. An outline's centre, hovered or a track's anchor,
  shows every piece of its loop, and for `Centre` the two corners its `Symmetric` holds, since
  `Target` names only the piece the outline search started from: `ShownOutlines` finds the loop
  again from that piece (`snap::outline_through`) when the centre starts being shown and keeps it
  while it stays shown, so `Target` stays `Copy` and no outline search runs per frame
  (`tracking::ShownOutlines`, shared with the grab). A grab highlights its landing the same way
  (`Grab::snap_entities`: the target's and track anchors' entities, an outline's centre showing the
  whole loop, never one that moves with it), a body edge it lands on drawn as guides along the edge
  (`ViewportState::grab_preview`).

## Constraint tools

- `sketch_tools.rs` turns the selection into candidates checked by `Sketch::check_constraint`;
  `sketch_toolbar.rs` offers them as buttons and commands, disabled with what to select
  (`app-look.md`). Candidates are kept in `PanelState::constraint_offers` and recomputed only when the
  selection, revision, evaluation, displayed sketches or units change
  (`sketch_toolbar::ConstraintOffers`).
- Curvature (`ConstraintTool::Curvature`) takes a spline and the line, arc or spline at one of its
  ends and adds `Tangent` (left out when already there) with `Curvature`. It has no button on the
  sketch bar, whose Constrain group would grow a third row (`sketch_toolbar::OFF_RIBBON_CONSTRAINTS`),
  and is in the Constrain group's menu.
- Chaining constraints (parallel, equal, collinear, concentric, perpendicular, coincident points,
  horizontal or vertical points) relate every selected item to the first in one transaction. Coincident
  also puts every selected point on the one curve selected with them, and Tangent makes the one line
  selected (else the first item) touch each other curve. Symmetric takes the one axis selected, else
  whichever of the three items mirrors the other two best; curves are mirrored by point symmetries
  (two lines: each end with the other's end it mirrors best; two circles: centres and `Equal`; two
  arcs: ends and `Equal`, centres left to those since mirroring them too would repeat the radius), so
  no constraint repeats another.
- A candidate the sketch already has (`Relations::restating`, from `Sketch::relations` built once per
  refresh of the offers) is left out of a batch and, when nothing is left, refused as already in the
  sketch; one that contradicts a constraint (`Sketch::contradicting`) is refused naming it. These
  checks are structural, since the UI thread never solves; a geometric (not dimension) constraint added
  to a sketch that solved is then tried off the UI thread (`Action::Trial`, `constraint_trial.rs`): the
  document with the transaction applied is solved on its own thread, and the transaction applies once
  it solves, or is refused in a notice naming the constraints it conflicts with. A trial past
  `PATIENCE` (or any other failure) applies it as before, so a slow sketch is never held up; a second
  geometric constraint while one is checked waits as a queued action (`queued::CONSTRAINT_CHECK`,
  `app-input.md`) and runs again from the selection once the verdict is in; `Queue::follow` carries
  it over the revision the verdict applied. The trial keeps the revision it started on
  (`Running`), and a verdict reaching a later revision is `Verdict::Outdated`: nothing applies and a
  notice says so, so an undo pressed meanwhile stands with its redo.
- Disable (`Command::ToggleConstraintActive`, `sketch_tools::ActivityChange`) switches the selected
  constraints off, or on when none is on. A disabled dimension is a reference: its label shows the
  measured value in parentheses in the muted `Standing::Inactive` colour.
- Dimensions start at the displayed geometry's measured value. Every sketch transaction first settles
  the sketch to the last result when up to date (`Model::settled_sketch`).
- Scale the whole sketch on its first dimension (`Command::ToggleFirstDimensionScales`, off by
  default, session-only in `ViewportState::first_dimension_scales`) makes a value typed in a
  dimension's canvas field (`sketch_tools::dimension_change`) scale every point and circle of the
  settled sketch about its origin by the new value over the measured one, in the same transaction. It
  applies only to an active length dimension that is the sketch's only active dimension, with no
  active `Fix`, no projected geometry and the sketch settled.
- A new dimension whose every entity is `EntityState::FullyConstrained` in the settled solution
  (`Model::settled_solution`, none while a recompute is pending) is already determined, so it is added
  inactive in the same transaction ("Add reference ...") with a notice, and takes no focus. The test is
  conservative: a circle counts only when its centre and radius are both determined.
- Smart dimension (`Tool::Dimension`, `dimensioning.rs`) takes the geometry clicked after it, not the
  selection: the selection is cleared when it starts, holds its picks, and is cleared when it ends. A
  click on a sketch item picks it (again lets it go); `dimensioning::fitting` chooses the dimension: a
  line's length, circle's diameter or arc's radius for one pick, the angle between two non-parallel
  lines, otherwise the distance between two picks (origin and axes included). A second pick adds its
  dimension at once, except two points (`dimensioning::awaits_placement`), which wait for a click
  placing it; a single pick waits for a second, and Enter or a click on empty space adds the single
  one. Enter always adds the aligned distance, the length or the radius. The placing click chooses
  (`dimensioning::placed`): for two points or a lone line, within their horizontal span above or below
  the horizontal distance, within their vertical span beside them the vertical one, elsewhere (and
  always for level or upright ones) the aligned one; for a lone arc, beyond it within its sweep its
  length, inside it its sweep, outside its sweep its radius. A point or circle and a slanted line, or
  two circles, also wait when their horizontal and vertical offsets are both above zero
  (`dimensioning::offset_ends`, from `Sketch::axis_offset_ends`). A lone ellipse or elliptical arc adds
  its major and minor radii (`MajorRadius`, `MinorRadius`); an ellipse picked with a point or any curve
  takes the distance between them (`Sketch::ellipse_gap`), an elliptical arc and a line sharing an end
  the angle; more picks are refused in words, its centre and axis points dimensioning the rest. A lone
  conic takes its rho (`Constraint::Rho`, through the Radius candidates), never added as a reference,
  as rho is no freedom other constraints could take up (`sketch_tools::determines`). A point and a
  construction line wait for placement (`dimensioning::about_axis`): across the line the diameter
  (`Constraint::AxisDiameter`, labelled Ø), on the point's side the distance. A spline waits for a
  point, line, circle, arc or ellipse, whose distance is the dimension (from a curve, the gap where
  the spline bulges toward it, `Sketch::spline_gap`); another spline is refused. A line and an arc
  sharing an end take the angle between them there (`sketch_tools::angle_to_arc`), never waiting for
  placement (`Sketch::angle_vertex`). It goes through the same candidates, checks, reference rule and
  inline field as the dimension buttons (`ConstraintTool::candidates_among`, `add_constraints`). With
  the tool active labels are not interactive, Activate picks the highlighted item as a click would, and
  Escape lets go of the picks before leaving the tool. Body corners, round-edge centres and edges are
  picked too and projected with the dimension (Snapping).
- Constraint states, degrees of freedom and redundancies come from the last evaluation
  (`sketch_status.rs`, `scene.rs`), never from solving on the UI thread (drags solve on their own
  worker).

## Constraining automatically

- Add the relations the drawing shows (`Command::FindRelations`), Dimension fully from a datum point
  (`DimensionFromDatum`, taking the one selected point) and Check the sketch for flaws (`CheckSketch`)
  open the Constrain automatically panel (`tidy_panel.rs`, a right-hand panel sharing the side
  panels' room) at its Relations, Dimensions or Check task; a segmented control switches between
  them. They are in the Sketch menu, the palette and the sketch bar's Constrain and Dimension group
  menus, with no button there, as the Dimension group would push the bar past one row at 1400
  points.
- `Tidying` (`tidying.rs`, in the `Workspace`) works on the settled sketch only
  (`Model::settled_sketch`; "Waiting for the sketch to solve…" otherwise) on a thread of its own, since
  finding relations and dimensions solves (`sketch.md`): a job per basis (feature, revision,
  evaluation, task, tolerance, chosen kinds, datum), the one in flight cancelled when the basis
  changes, so the panel is redone after every change, an applied proposal included, and then says
  nothing is left. It closes when sketch editing ends or another sketch is edited, and with a new
  session.
- Relations and Check take a tolerance (`Looseness`: Tight, Normal, Loose, shares of the sketch's
  extent and angles, in the document's unit), Relations a checkbox per `RelationKind`, Dimensions the
  datum (selected point or origin). The proposal is listed by kind with what adding it leaves free, a
  dimension with its measured value, a flaw in words with its remedy; at most `MAX_LISTED` rows a
  group. Its entities are highlighted in the view as hovered geometry is
  (`ViewportState::preview_entities`, under the pointer's own hover), a hovered row's alone.
- The primary action sits in a footer (Add N relations, Add N dimensions, Fix N flaws) and applies the
  whole proposal as one undoable transaction settled first; dimensions go into the document's unit
  (`sketch_tools::in_unit`). A flaw's own button applies its fix alone; a fix set drops added
  constraints touching removed geometry. A sketch that does not solve is refused in words
  (`tidying::UNSOLVED`).

## Status pills

- `sketch_status::show` (the sketch's card in the tree) draws the status pill and, in words, a pill
  each for redundant constraints, open ends and points beyond their curves. The sketch bar's header
  keeps one height whatever the status: the title on its first line with the counts at the right end
  as `widgets::count_pill`s (an icon per kind, `icons::REDUNDANT`, `OPEN_ENDS`, `BEYOND`, and the
  number; words in the hover and accessible name; `sketch_status::counts_from_the_right`), the title
  truncating to leave room, and the status pill alone on the second line, the header at least as wide
  as the widest status (`sketch_status::widest_status`), so the bar and the view below never move as
  the status changes.
- Pills that act are `widgets::PillRole::Button`s (named "<words>: <what a click does>) and return a
  `sketch_status::StatusRequest`: a failed or conflicting status shows the problem in the card
  (`ShowProblem`), degrees-of-freedom runs Select what is still free, open ends Select the open ends
  (`Command::SelectOpenEnds`: selects their points and frames them, `ViewportState::fit_requested`)
  and redundant Select the redundant constraints (`Command::SelectRedundant`: selects each
  `Redundancy::constraint`, ready for Delete); both are sketch-scope commands refused in words without
  a solved sketch or with none (`sketch_status::NOT_SOLVED`, `NO_OPEN_ENDS`, `NOTHING_REDUNDANT`).
  Other statuses and the points-beyond count stay plain. In the tree card the selecting pills act only
  for the edited sketch, through `PanelState::sketch_command`, which the app triggers before the
  viewport runs its commands.

## Open ends, beyond points and regions

- The edited sketch's open ends (`SketchResult::open_ends` of its up-to-date result) are ringed in
  `Chrome::warning` (`annotations.rs`, only those in view, one ring per `RING_MERGE` square), counted
  beside the status and in the 3D view's description, so an outline that will not close is seen while
  drawing rather than when the extrusion fails. Where more than `MOST_RINGS_PER_CELL` rings fall in
  one `RING_CLUSTER_CELL` square they become one double ring (`CLUSTER_RADIUS` round the ordinary one,
  told by shape, not colour) at their centroid (`annotations::open_end_marks`, over flat screen grids,
  `ScreenGrid`, `ScreenCells`). Its hover (`Hover::OpenEnds`) says how many ends it stands for and to
  zoom in; it senses hover only, so clicks and the scene's hover reach the geometry beneath, and open
  ends stay reachable through the keyboard highlight and Select the open ends.
- Points held past the drawn ends of their line or arc (`SketchResult::beyond`) are marked likewise in
  `Chrome::muted`: a dashed extension from the curve's nearer end to the point and a ring, counted as
  information (hover `sketch_status::BEYOND_HELP`) and in the 3D view's description, not a warning,
  since a point on a line's extension is often meant (`Target::Extension`).
- The edited sketch's closed regions (its result's display regions) are tinted `closed_region` on the
  front layer (`scene::Builder::closed_regions`), while the shown geometry is the result's
  (`same_geometry`), so a drag never shows regions it left behind. With Select active
  (`Context::selecting`) each is `Pickable::SketchRegion`, the area between crossings: hover tints it,
  a click selects it, Shift or Ctrl adds, curves and points win near them, a drag starting on one
  draws a box. Other tools leave them unpicked. Outside sketch editing, with no feature open and
  nothing chosen in the view, or while a loft chooses its profiles (`Context::picks_shown_regions`),
  every shown sketch's regions are drawn and picked likewise, like faces, on `Layer::Model` (a body in front hides them, one lying on a face
  draws over it). Extrude and Revolve take the selected regions (`app-modelling.md`), and Measure
  reads their area and section (`app.md`). A sketch too large for recompute to find them unasked gets
  them asked for each frame it is edited (`Model::request_regions`, one request per result), and
  their arrival moves the displayed sketches' generation so the cached scene picks them up.

## Displayed sketches

- `Model::displayed_sketch` (`display.rs`) is the definition with solved positions wherever the last
  result has the same entity. `DisplayedSketches`, owned by `Model` and handed to the scene through
  `Sources`, works it out once per sketch and keeps it, with the bounds of its points that fitting and
  the reference size use, until the document or evaluation changes (`forget`). Its `generation` moves
  on `forget` and when a dragged sketch is shown or dropped, which the cached scene watches (`app.md`).

## Tracing reference images

- Reference images (`app-modelling.md`) stay drawn while a sketch is edited and are never picked
  then, so drawing tools place points, snap to sketch geometry and show their previews over a
  picture as anywhere else, and the edited sketch's curves draw over it (`render.md`). Insert
  reference image… while a sketch is edited puts the picture on that sketch's plane and keeps the
  sketch open (`ui_tests/reference_images.rs`).

## Annotations

- Drawn with the egui painter (`annotations.rs`; placement in `annotation_layout.rs` over the
  unit-free part shared with drawing export, `caditor_sketch::annotation`: what each dimension
  measures, lanes, label obstacles) from the displayed geometry through the current view. Unplaced
  dimensions sit away from the sketch's centre, and linear ones measured along one line on one side
  with overlapping spans stack into lanes (`annotation::lanes`, shortest nearest, `LANE_SPACING`
  apart); other constraints are glyphs stacked beside each constrained entity on the opposite side.
- Dragging a dimension's label places it: the offset is stored with the dimension (`sketch.md`) in its
  `LabelFrame` (relative to the geometry, so it keeps its place as the geometry moves), and the drop is
  one undoable "Move dimension label" transaction. A placed distance runs its dimension line through
  the label; a placed radius or diameter points its leader at it; a placed angle, arc length or sweep
  takes the label's distance as its arc's radius; the others draw a leader. Placed dimensions take no
  lane. From the keyboard, Move selected geometry with one dimension alone selected
  (`annotations::label_to_move`) opens "Move to" at the label (`@` for an offset).
- Glyphs keep clear of dimension labels and each other (`annotation_layout::place_glyphs` over
  `Obstacles`); when nothing is free the least covered place wins, only while covered by at most
  `MOST_GLYPH_COVER`, and a group whose unshifted places lie in cells already `FULL_CELL_SHARE`
  covered is left out without trying any (`Thinning::WhenCrowded`), so a zoomed-out dense sketch shows
  as many glyphs as fit. A thinned group anchored on a line under `COLLAPSED_BELOW` points on screen
  (`annotation_layout::collapses`) is left out unless one of its constraints conflicts or is
  redundant. A group anchored on a selected entity, or holding a selected or keyboard-highlighted
  constraint, is placed first and always (`Thinning::Never`, `ViewKey::kept` and `forced`). An
  entity's anchor is clipped to the view first (`within_view`; off-screen entities get no glyphs) and
  places are generated lazily from the middle out, at most `MAX_GLYPH_SHIFTS` each way, so zooming in
  never makes a frame walk a line's full length. An entity shows at most `MAX_STACKED` glyphs: past
  that the last slot is a "+N" mark whose hover lists the rest and whose click selects the first.
  Show or hide constraint glyphs (Alt+G, `ViewportState::glyphs_shown`, shown at start unless
  Preferences › Sketching says otherwise) hides them all, dimensions staying. Labels use `canvas::body` on the canvas backdrop, glyph letters `canvas::emphasis`.
- Labels show the expression in the document's naming, followed by its value when not a literal;
  conflicting and redundant constraints take the error and warning colours. Label texts are cached per
  sketch (`annotations::LabelTexts`), their measured sizes by text across revisions (`SizesKey`, at
  most `MAX_MEASURED_TEXTS`), and a mark's description is formatted only for its hover
  (`annotations::Hover`), so a still frame formats and evaluates nothing.
- Marks are worked out in two cached stages: `annotations::Measures` (what each dimension measures,
  lane, label frame, `annotation_layout::Reach`, glyph groups, open ends, points beyond; keyed by
  `SketchKey`: feature, revision, evaluation, displayed sketches, dragged label) and
  `annotations::Marks` (laid-out dimensions, placed glyphs, projected ends; also keyed by the
  `SketchScreen`, view rectangle, units, dragged offset, edited and forced dimensions and Show glyphs:
  `ViewKey`), so a still frame only paints and interacts. Layout projects through
  `SketchScreen::projector` (`caditor_render::PlaneProjection`). Only a dimension whose reach is near
  the view is laid out (`Reach::on_screen`, `ScreenReach::near_view`, within `DIMENSION_OFFSET`,
  `ANGLE_RADIUS` and `LABEL_REACH`), so obstacle avoidance sees every label that can reach a shown
  glyph; selected, highlighted, dragged, inline-edited and field-waiting dimensions are laid out first
  and wherever they are, so Move to and `Focus::Dimension` reach an off-screen label. Any other whose
  reach spans under `COLLAPSED_BELOW` points (`ScreenReach::collapses`; never a zero-length one) and
  neither conflicts nor is redundant becomes a collapsed mark (a `COLLAPSED_RADIUS` dot per
  `COLLAPSED_SPACING` cell, hoverable, clickable to select and lay out, double-clickable to edit); one
  in cells already `FULL_CELL_SHARE` covered is dropped (`annotation_layout::crowded`), and a laid-out
  label covered by more than `MOST_LABEL_COVER` is dropped with its dimension
  (`annotation_layout::mostly_covered`). Drawing export (`caditor-file`'s `export/annotation.rs`)
  places every dimension and shares none of this thinning. The ignored
  `frame_costs_on_a_large_sketch_and_a_large_model` (`viewport.rs`) and
  `sketch_card_costs_on_a_large_sketch` (`feature_tree.rs`) time it on `large_sketch`.
- Labels and glyphs are `Pickable::SketchConstraint`: hover highlights the entities, click selects
  (Shift or Ctrl toggles), Delete removes selected constraints and entities in one transaction. They
  are painted but not interactive while a drawing tool is active. Only marks within `POINTER_REACH` of
  the pointer, the dragged label and a mark holding focus are handed to egui each frame
  (`PointerReach`); egui hit-tests against the previous frame's widgets, so the reach is wide enough
  that a mark is registered before the press that lands on it. Keyboard users reach marks through the
  highlight commands, never Tab.
- Double-clicking a label opens an inline `commit_field` with the value selected, as does a new
  dimension or any `Focus::Dimension` of the edited sketch, which the app takes from the panels and
  hands to the viewport, waiting until the dimension can be drawn.

## Dimensions on the model

- Show or hide dimensions on the model (`Command::ToggleDimensions`) is a session-only view toggle
  (`ViewportState::dimensions_shown`, off at first unless Preferences › Sketching turns it on). While on and no sketch is edited,
  `ViewportState::annotate` hands `model_dimensions::ModelDimensions` the view and the subjects: the
  feature of the highlighted or hovered item (`viewport::feature_of`), the rows chosen or hovered in
  the tree, then the features of the first `SUBJECT_ITEMS_READ` selected items, at most `MAX_SUBJECTS`
  and never the open feature, whose handles already show its values. The feature of an open field and
  of a hovered value label stay subjects (`ModelDimensions::held`).
- Sketch dimensions: every active sketch that is not hidden, plus the sketch a subject sweeps
  (`feature_values::sketch_of`) or a subject sketch itself even when hidden; a sketch several features
  sweep shows once. Each has its own `Annotations` laid out on its solved plane with
  `Surface::outside`: the same cached `Measures` and `Marks`, lanes, thinning and collapsing as the
  edited sketch, but no glyphs, open ends or points beyond, labels that do not drag or select, and an
  inline field opened by double-click or Enter.
- Labels are thinned across all sketches and features through one obstacle grid a frame: feature
  values are placed first (those of the first subject with values always, any other left out when
  labels placed before it cover it by more than `MOST_LABEL_COVER`), and their rectangles seed each
  sketch's layout in turn (`Surface::taken`, part of `ViewKey`), each sketch's labels joining the grid
  for the next (`Annotations::label_rects`), so labels of two sketches never overlap.
- Feature values (`feature_values.rs`) cover extrusion distances (`ValueSlot::Extrude`), revolve
  angles, fillet radius and chamfer distances, shell thickness, hole diameter and depth, offset face
  distance, primitive sizes (`ValueSlot::Primitive`), non-zero move offsets (`ValueSlot::MoveOffset`),
  thread depth, and pattern counts, spacings and angle, each with the panel row's caption, dimension
  and `feature_fields::Rule`, worked out when the revision, evaluation, units or subjects change. Each
  is drawn as a `value_shapes::Drawn`: dimension lines along the reach (`reach_handles::reach_lines`),
  arcs about the axis (`turn_handles::turn_arcs`), paths with end ticks for hole and offset face, a
  leader with a dot on the largest face the feature made for fillet, chamfer and shell, else a stack
  under the centre of the faces the feature made (`FaceOrigin`, `origin_feature`) or of its body.
  Labels read `<caption> <value>` or `<caption> <expression> = <value>` in `canvas::body` on the canvas
  backdrop, announced as buttons "<feature> <caption>: <text>".
- A label whose value is a parameter, owned (`ParameterOwner::Feature` with the caption, or a
  dimension's) or a bare reference to a model parameter, reads `name = value`
  (`field::driving_parameter`), and its field holds `name = expression` (`field::driven_text`): typing
  `name = expression` renames that parameter and sets its expression (`NamedField::through_reference`,
  refused in words for a measured parameter), typing a value alone replaces the reference. Otherwise
  the field follows the feature panel exactly (`NamedField::transaction`, `feature_values::change` a
  checked "Edit <feature>" `SetFeatureKind`; for a sketch dimension `field::dimension_edit` with
  `Reference::Followed`), each commit one undoable transaction.
- Keyboard: the shown labels (laid out or collapsed sketch dimensions as `Pickable::SketchConstraint`,
  feature values as `Pickable::FeatureValue`, which nothing selects or picks) follow the scene's
  pickables in the highlight order, stay highlighted while shown (`ModelDimensions::holds`), and Enter
  or Activate opens the highlighted one's field (`ModelDimensions::open`).
