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
  - "crates/caditor/src/filleting.rs"
  - "crates/caditor/src/shapes.rs"
  - "crates/caditor/src/shape_modes.rs"
  - "crates/caditor/src/snap.rs"
  - "crates/caditor/src/tracking.rs"
  - "crates/caditor/src/sketch_tools.rs"
  - "crates/caditor/src/sketch_toolbar.rs"
  - "crates/caditor/src/sketch_status.rs"
  - "crates/caditor/src/display.rs"
  - "crates/caditor/src/annotations.rs"
  - "crates/caditor/src/annotation_layout.rs"
  - "crates/caditor/src/typed_point.rs"
  - "crates/caditor/src/viewport.rs"
  - "crates/caditor/src/dimensioning.rs"
---

# Sketch editing in the app

## Editing context

- A context, not a mode: `editing.rs` holds the edited sketch and the active `Tool`, changed by
  `Action::Editing` commands that `app::perform` routes after the UI pass. It ends by itself when
  the sketch disappears or another document opens.
- Entering faces the camera to the plane, fits it, dims other features (unpickable) and keeps only
  that sketch selected.
- Look at sketch (`Command::LookAtSketch`, Alt+Shift+V, the palette and the button beside Finish
  in the sketch bar) animates back to the same facing view at any time while editing.
- The edited sketch, its origin and axes and the drawing preview go on `Layer::Front`
  (`render.md`), so a body never hides or z-fights with them wherever the sketch lies.
- Clicks and primary drags select with the Select tool; a drawing tool (`Tool::draws`) draws, and
  the modify tools (`Tool::modifies`) act on what is under the pointer. Escape backs out one step
  at a time in the order of `ViewportState::escape`.
- Default keys come from `Command::default_shortcuts`, not from here.

## Dragging and box selection

- With Select, a primary drag starting on a non-reference entity of the edited sketch is a `Grab`:
  a point, line, arc or spline moves its points by the pointer's offset, a circle alone changes
  radius, and a selected grabbed entity moves every selected entity.
- Each pointer move sends `DragCommand::Move`; `SketchDragging` (in `Display`) gives the newest to
  its own worker thread, which drops older unstarted ones and solves each from the previous
  solution of the same drag with `solve_from`. Results are shown through
  `DisplayedSketches::show_dragged`, which every consumer of a displayed sketch sees.
- A grab snaps while snapping is on and Ctrl is not held (`ViewportState::grab_snapping`,
  `Grab::follow`). Its handle, the grabbed point or, for a curve or selection, the moving point
  nearest the press, goes through `tracking::land` on the displayed sketch, ignoring every moving
  point and every curve using one, so a dragged line or selection lands that point on the target
  and moves the rest by the same offset. A circle dragged alone changes its radius and never snaps.
  Tracks and extensions work as while drawing, from the grab's own `Acquired`: at the start the
  other points of the curves using the handle (so a line end dragged level with its start is kept
  horizontal), then whatever the drag snaps to. The landing shows the snap marker, its label and
  dashed guides. Release sends the target's and tracks' constraints (`Landing::joins`) with
  `DragCommand::Finish`, and `Model::commit_drag` adds them to the settle transaction when the
  solved point reached the landing, leaving out any the sketch refuses, already has or
  contradicts.
- Release commits one `settle_sketch` transaction and the dragged shape stays until an evaluation
  of that revision reaches the sketch, so it never jumps back. Escape, another edit or document,
  or a drag begun on an older revision drops it.
- When the worker cannot solve its newest frame (`Polled::blocked`, `Model::drag_blocked`), the
  viewport says `DRAG_BLOCKED` beside the pointer as a polite live region and the geometry stays
  where the last good frame put it. In a sketch whose constraints conflict it says `DRAG_CONFLICT`
  with the conflict named (`Model::sketch_conflict`, the first two constraints and how many
  more), and the notice when the drag ends names them too.
- A primary drag elsewhere draws a box: left to right a window taking what lies inside (curves
  faceted as drawn, `app.md`), right to left a crossing box taking what it touches; with Select
  with a lasso on, a freehand outline taking what lies inside it (`app-input.md`). It replaces
  the selection (Shift or Ctrl adds); a point is left out when a curve it belongs to was taken.
- Split the selected curve at the selected point (`Command::SplitCurve`, Sketch menu, palette;
  `sketch_tools::SplitChange`) takes one selected point and the line or arc selected with it, or
  the one line or arc the point lies on by `Coincident`, and splits it there in one transaction;
  anything else is refused with `NOTHING_TO_SPLIT` or the sketch's reason. A point placed on a
  curve first (the Point tool snaps onto it) gives a split anywhere.
- Move selected geometry opens the typed-point field (`app-input.md`) as "Move to", committed and
  solved like a drag; button and command share availability (`Moving::offered`).
- Rotate and Scale selected geometry (`Command::RotateGeometry`, `ScaleGeometry`, Sketch menu,
  palette; `sketch_drag::Transforming`) open the same field as "Rotate by" (an angle, degrees
  unless a unit is named, counter-clockwise) or "Scale by" (a factor above zero), committed and
  solved like a drag, so constraints still hold where they disagree. They turn or scale about the
  one selected point no selected curve uses (a lone point or the origin), which stays put, else
  about the centre of the selection's extent (circles' rims included); a scale also scales the
  selected circles' radii. A lone point with nothing to turn it about is refused in words.
- Copy, Cut and Paste (`Command::CopyGeometry`, `CutGeometry`, `PasteGeometry`, Ctrl+C, Ctrl+X,
  Ctrl+V, the Sketch menu and the palette) work on the edited sketch's selected geometry through a
  `SketchClip` (`Sketch::clip`): the selected curves with their points and the lone points, their
  construction flags and every constraint among them (inactive ones staying inactive), never
  reference geometry or a constraint reaching outside the copy. It lives in
  `ViewportState::clipboard` with the sketch it came from, forgotten with the document. Paste is
  one transaction (`Sketch::paste` on the working copy under fresh ids, a `Fix` moving with the
  copy) centred under the pointer when it is on the sketch, else in place in another sketch or
  shifted by a quarter of the copy's size in the same one, and selects what it pasted. Copying
  also puts a line of text on the system clipboard, since egui-winit sends Ctrl+V as a paste
  event only when the system clipboard holds text.
- Double-clicking a curve (no tool active) selects its chain, the lines and arcs joined end to end
  that Offset would take (`Sketch::offset_chain_through`).

## Drawing tools

- `drawing.rs` holds the tools' state and `shapes.rs` their geometry. The drawing is keyed by a
  `Shape` (tool plus way of drawing), so changing either drops a shape in progress. Each finished
  shape is one transaction, settled first like any sketch transaction; inferred constraints are
  checked with `Sketch::check_constraint` on a shadow sketch and skipped if refused.
- A shape with no size gets a `Refusal` (a notice for a click, the field error for a typed point).
- A press that drags or is held too long to be a click still places a point where it is released,
  since egui reports it as a drag (`ViewportState::click`). When nothing of the shape is placed
  yet and the release is at least `DRAG_DRAWS_FROM_PRESS` from the press, the press position is
  placed first (`place_press_point`), so one press-drag-release draws a line, rectangle or circle.
- What is being drawn shows its size so far below the snap label (`Drawing::readout`): a line, a
  rectangle, a circle, a polygon (radius, or side, and its sides), an arc (radius, and the sweep once
  its end is chosen), a tangent arc (radius and sweep), a three-point arc or circle (radius), a
  three-point rectangle (both sides), a slot (length and end diameter), an arc slot (radius, sweep,
  then width) and a spline (length and angle of the leg from its last control point).
- Holding Ctrl places the point exactly under the pointer: no snapping and no alignment guides
  (`Drawing::place_freely`). It also stops a tangent arc from starting, which needs a snapped
  point. Snapping on or off for good (`Command::ToggleSnapping`, View menu and palette, kept for the
  session in `ViewportState::snapping`) has the same effect as holding Ctrl all the time.
- The pointer is on the sketch only within `MAX_LENGTH` of the origin, so an edge-on view cannot
  place a point at an enormous distance.
- Lines chain, each joined to the last end by `Coincident`, until Escape, a click on the last
  point, or a line closing the outline on the chain's first point. Splines finish on Enter or a
  click on the last control point. Each continuing segment records its anchor (`ChainStep`);
  Backspace undoes the last segment when it is the newest undo step, and when the anchor's point
  is gone (undo, a deletion) the chain steps back to the newest anchor still there rather than
  ending.
- An arc runs the way the pointer swept round its centre; a typed end goes the shorter way
  (`Sweep::aim`), and Reverse the arc sends either the other way. The end is projected onto the
  circle through the start and keeps its snap only if the target lies on that circle.
- A tangent arc starts on a point ending a line, arc or spline (the newest if several) and leaves
  along that curve's direction with a `Tangent`.
  Switching between the Line and Tangent arc tools with a segment started (`Drawing::sync`) keeps
  the chain and its anchors: the arc leaves along the line just drawn, and the lines go on from the
  arc's end. Stepping back onto a line step while arcing finds its tangent again from the sketch.
- What a placed point may snap to is `Drawing::accept` (points only where a curve would add no
  constraint); width points of slots and three-point rectangles never snap. Per-shape constraints
  and degrees of freedom are in `shapes.rs` and its tests, not here.
- The polygon side count (`MIN_SIDES` to `MAX_SIDES`) is kept until another document opens
  (`ViewportState::forget_document`); More sides and Fewer sides are offered only with the Polygon
  tool. Typing `N sides` in the point field (`typed_point::sides`) sets the count, refused outside
  `MIN_SIDES..=MAX_SIDES` in words. Holding Shift with a polygon started freezes the preview where
  it was and scrubs the count by one per `SCRUB_POINTS_PER_SIDE` of sideways pointer travel from
  where Shift went down (`Drawing::scrub_sides`); releasing Shift resumes following the pointer.

## Ways of drawing a shape

- Rectangle, circle, polygon and slot each keep one tool with several ways of drawing
  (`ShapeMode`), a switch within the tool rather than a tool and key per way, so the ribbon and
  keymap stay small and a shape is always found under its one key. Arcs stay separate tools,
  grouped on the ribbon under one Arc button (`app-look.md`).
- Running the tool's command while it is active steps to its next way, round to the first;
  clicking its ribbon button only chooses the tool. Every way is also its own command
  (`Command::ShapeMode`, id `sketch.<shape>.<way>`, no default key), offered in the palette, the
  button's corner menu and Sketch › Ways to draw shapes.
- `SketchEditing` remembers the last way per shape (`ShapeModes`) while caditor runs, across
  sketches and documents; it is not a preference, as no tool state is. The prompt's key line leads
  with the way in use and what the key does next.

## Projecting model geometry

- Project (`Tool::Project`, Alt+P, the sketch bar's Edit group, palette) brings model geometry into
  the edited sketch: while it is active `Context::projecting` draws bodies and other sketches as
  background but pickable (`Presence::Projectable`) and `Pickable::is_available` lets their faces,
  edges, corners and curves be hovered. A click or Activate on one runs `projecting::project`: an
  edge, a corner, every boundary edge of a face not seen end-on, or a curve or point of a sketch
  above it, each as one undoable transaction from the body's state at the sketch
  (`body_result_seen_by`). Geometry made later in the tree, a corner shared by two vertices of one
  name and anything already projected are refused in words. The highlight commands step only
  through projectable items, and the hover says what a click projects.
- Projected geometry is drawn in the `PROJECTED` palette, is never grabbed or dragged and follows
  its source on every recompute (`document.md`).

## Construction geometry

- Construction makes the selected curves construction in one transaction
  (`sketch_tools::ConstructionChange`), ordinary when all already are; with no curve selected it
  switches drawing (`ActiveSketch::construction`): new curves are construction, points stay
  points. Construction curves are dashed (`scene::curve_segments`) and coloured by constraint
  state like any curve, so dashes never carry a constraint state.

## Constraint states without colour

- Each sketch entity has a `SketchState` (under or fully constrained, conflicting, redundant,
  failed, projected, background) drawn by the palette's `Look`. In high contrast the form says
  whether it can move: fully constrained, projected, redundant and conflicting curves are heavy
  (`heavy_curve_width`) and their points solid; under-constrained curves are regular and their
  points hollow (a ring: the point marker with a smaller unpicked marker in the canvas colour,
  `hole`, drawn after it at the same place). The standard palette draws every state in one form.
- Which problem a sketch has is named by its status pill, and conflict and redundancy never share
  a sketch (a conflict fails the solve); in high contrast the constraint marks involved also get a
  frame (`annotations::paint_frame`): solid for conflicting, dashed for redundant, in the mark's
  colour.

## Trim, extend, offset, mirror and sketch fillet

- `trimming.rs` and `modifying.rs` (a facade over `offsetting.rs`, `mirroring.rs`, `filleting.rs`)
  hold the tools' UI state; geometry and constraint rules are the sketch's (`sketch.md`). Each
  frame the tool aims through the displayed sketch, previews the result and puts its words, or
  why not, where hover descriptions go. Trim and Extend aim at splines too, so they can be refused
  in words.
- A click acts at once, without waiting for a GPU pick: one transaction built by `reshape_sketch`
  from a working copy (the definition with the displayed positions), settled first like any sketch
  transaction. A refusal is a notice, or for a typed value the field's error, and the tool stays
  active.
- Trim's primary drag is a trim path: every piece the path crosses is collected and trimmed on
  release in crossing order as one transaction, each found again by its middle on whichever curve
  now carries it after earlier splits. Extend ignores drags.
- Offset works on the selected chain; with none, a click on a curve selects its chain. The
  pointer's side and distance choose side and distance, previewed live. Mirror copies the
  selection about the line or axis under the pointer (sketch lines win a tie with an axis) and
  asks for a selection first.
- Sketch fillet is named so, to keep it apart from the model's Fillet. It first takes a corner (a
  selected one, else the curve end under the pointer, `Sketch::corner_at`, refused in words when
  it is no corner); then the pointer sets the radius (`radius_through`). The chosen corner is
  cleared after each fillet. Sketch chamfer (`Tool::Chamfer`) is the same tool cutting the corner
  instead (`filleting::CornerCut`): the pointer sets the distance (`distance_through`) and the
  field is "Chamfer distance". It is not on the sketch bar, whose Modify group would widen past
  one row (`sketch_toolbar::OFF_RIBBON`, its command still offered there); the Sketch menu, the
  palette and its key reach it.
- Offset, Sketch fillet and Sketch chamfer take a typed value in the typed-point field ("Offset
  by", "Fillet radius", "Chamfer distance"): the preview follows the text while it parses, Enter commits the expression as typed
  (parameters included) and an error keeps the field open. A negative offset goes to the other
  side, so the keyboard alone does it: select, Offset, the distance, Enter.
- With any of them active the highlight commands step through that tool's targets instead of the
  scene's pickables (`app-input.md`), and Activate or Enter acts on the highlighted one.

## Snapping

- `snap.rs` runs on the UI thread against the displayed sketch in screen space. Priority: the
  shape's pending point; points and the origin within `POINT_TOLERANCE`; then, only where any snap
  is accepted and within the same tolerance, the middle of a line or arc (`Target::Midpoint`), the
  centre of a closed outline (`Target::Centre`), the crossing of two of the up to
  `MAX_CROSSING_CURVES` curves nearest the pointer, axes included (`Target::Intersection`; a
  spline crosses lines, circles, arcs, axes and other splines through `Sketch::spline_crossings`) and the right, top, left and bottom of a circle or of an arc
  sweeping through them (`Target::Quadrant`); lines, circles, arcs, splines and axes within
  `CURVE_TOLERANCE`, projecting onto the curve. A snapped point gets a `Coincident` with its
  target (with both curves at a crossing), a `Midpoint` constraint for a middle, a `Symmetric`
  about it of two opposite corners for a centre, or for a side a `Coincident` with the curve and
  a horizontal or vertical points constraint with its centre.
- A line's end within `POINT_TOLERANCE` of where a line from its start would touch a circle or arc
  (`snap::tangents_from`, the two tangent points from outside it, within an arc's sweep) lands
  there (`Target::Tangent`): a `Coincident` with the curve and a `Tangent` between it and the line.
  It wins over curve snaps, not over point-like ones (`Target::is_point_like`).
- A line starting on a circle or arc (on its rim, a side, a tangent point or a point ending an
  arc), and the first side of a three-point rectangle or a polygon drawn by its side, is offered
  the direction tangent to it there (`Direction::Tangent`, a `Tangent` constraint), ranked with
  parallel and perpendicular.
- An outline has a centre when its lines and arcs join end to end into one closed loop (ends
  within `JOINED_CORNER_TOLERANCE` of the sketch's extent, each corner meeting exactly one other
  piece, at most `MAX_OUTLINE_LINES` pieces) of an even count whose opposite corners all share one
  midpoint and whose opposite pieces are both lines or both arcs of one radius with centres mirrored
  about it: a rectangle, a parallelogram, an even regular polygon, a slot, a rounded rectangle. A
  corner where a third piece meets breaks the loop (a rectangle with its diagonals snaps to their
  crossing instead). A loop of an odd count of lines (a triangle, a pentagon) has the centroid of
  its area as centre (`Target::Centroid`): the point lands there exactly but no constraint holds it,
  since the sketch has no constraint for a centroid, and the label ends "not kept there". A loop
  with a piece or corner that is moving in a drag offers no centre.
- `Accept` keeps every shown snap a constraint that already holds: a circle's rim takes points
  only (a rim on a curve would add no constraint); an arc's end takes points on its circle and
  where it crosses other curves (splines through `Sketch::circle_crossings`) and the axes.
- A line end (and the second point of either straight slot, of a three-point rectangle's first
  side and of a polygon's side) within `ALIGN_ANGLE_DEGREES` or `ALIGN_TOLERANCE` of a direction
  from its start takes it exactly (`Snap::Aligned`). Horizontal and vertical win whenever either
  applies, so a line near level is never inferred parallel to; otherwise parallel or perpendicular
  to one of the `NEARBY_LINES` lines of the edited sketch (construction lines included) nearest on
  screen (`drawing::guides`), the smallest offset winning. A line is never inferred parallel to a
  line through its start point (that only continues it), though perpendicular to it is offered.
- An arc's start (and an arc slot's ends) from its centre, a polygon's corner from its centre and
  each spline control point from the one before take horizontal or vertical the same way
  (`Drawing::levelled_from`, `LEVEL_AND_UPRIGHT` only), kept by a horizontal or vertical points
  constraint between the two points (`Direction::between`).
- Snapping to geometry wins over a direction, which joins it only where compatible
  (`Snap::AlignedOn`): a point snap keeps a direction that already holds (`HELD_TOLERANCE`) and
  never moves the point; a curve snap moves to where the direction's ray crosses the curve, within
  `ALIGNED_CROSSING_TOLERANCE` of the pointer. Typed points never align.
- Points the hover snapped to are acquired (`tracking::Acquired`, newest first, at most
  `MAX_ACQUIRED_POINTS`), as are the centre of a circle or arc whose rim, middle or centre was
  hovered and the lines ending at a hovered point (`MAX_ACQUIRED_LINES`); they are kept across
  tools within one sketch and dropped when gone. Where any snap is accepted, a placed point within
  `TRACK_TOLERANCE` of the horizontal or vertical through an acquired point (other than the
  shape's own placed points, at least `MIN_TRACK_LENGTH` from it) lands on it with a
  `HorizontalPoints`/`VerticalPoints` constraint (`Tracks`), on both at once where a horizontal
  and a vertical from two points cross. A track joins a curve snap where it crosses the curve
  (never along a line it runs on) and a line's direction where they cross, within
  `ALIGNED_CROSSING_TOLERANCE` (`tracking.rs`); point snaps never take one. The preview draws each
  track as a dashed guide from its point (`Preview::guides`) and highlights the point.
- An acquired point also tracks along each acquired line's direction and square to it
  (`Axis::Slanted`, "parallel to" or "perpendicular to" the line in the label), never along a line
  through the point itself (its extension covers that) nor where the direction is level or upright.
  The point lands on a slanted track exactly but no constraint holds it, as the sketch has none for
  a point on a slanted line through another; a slanted track crosses a horizontal or vertical one
  from another point as those cross each other, keeping that one's constraint.
- An acquired line extends past its ends: after curves, the pointer within `CURVE_TOLERANCE` of its
  infinite carrier snaps to it (`Target::Extension`, a `Coincident` on the line, which the solver
  treats as the infinite line), with a dashed guide from the nearer end; directions and tracks
  join it as they do a curve.
- Snap to the grid (`Command::ToggleGridSnapping`, View menu and palette, off by default, kept for
  the session in `ViewportState::grid_snapping`) places a point no target, direction or track took
  on the nearest crossing of the grid's minor lines (`grid_minor_spacing` of the view, handed to
  `Drawing::snap_to_grid` each frame) when it lies within `POINT_TOLERANCE` on screen; the point
  stays free, joined by no constraint.
- Preview curves are faceted like the sketch's (`Drawing::preview` and `Trimming::preview` take
  the scene's `Faceting`); the snap target and a direction's reference line
  (`Drawing::snap_entities`) replace the GPU hover while a drawing tool is active.

## Constraint tools

- `sketch_tools.rs` turns the selection into candidates checked by `Sketch::check_constraint`;
  `sketch_toolbar.rs` offers them as buttons and commands, disabled with what to select
  (`app-look.md`).
- Chaining constraints (parallel, equal, collinear, concentric, perpendicular, coincident points,
  horizontal or vertical points) relate every selected item to the first in one transaction.
  Coincident also puts every selected point on the one curve selected with them, and Tangent
  makes the one line selected (else the first item) touch each other curve; two items keep their
  order. Symmetric takes the one axis
  selected, else whichever of the three items mirrors the other two best. It mirrors two points,
  two lines (each end with the other line's end it mirrors best), two circles (their centres and
  `Equal`) or two arcs (each end with the other arc's end it mirrors best, and `Equal`: the
  centres are left to those, since mirroring them as well would repeat the radius), so curves
  are mirrored by point symmetries without any constraint repeating another.
- A candidate the sketch already has (`Relations::restating`, from `Sketch::relations` built once
  per refresh of the offers) is left out of a batch and, when
  nothing is left, refused as already in the sketch; one that contradicts a constraint
  (`Sketch::contradicting`) is refused naming it. These checks are structural, since the UI thread
  never solves; a geometric (not dimension) constraint added to a sketch that solved is then tried
  off the UI thread (`Action::Trial`, `constraint_trial.rs`): the document with the transaction
  applied is solved on a thread of its own, and the transaction is applied once it solves, or
  refused in a notice naming the constraints it conflicts with when the solve finds a conflict.
  A trial past `PATIENCE` (or any other failure) applies it as before, so a slow sketch is never
  held up and a conflict found later is reported afterwards as usual; a second constraint while one
  is checked is refused saying so. Dimensions start at the measured value and are applied at once.
- The candidates of every tool are kept in `PanelState::constraint_offers` and worked out again
  only when the selection, the revision, the evaluation, the displayed sketches or the units
  change (`sketch_toolbar::ConstraintOffers`).
- Disable (`Command::ToggleConstraintActive`, `sketch_tools::ActivityChange`, the Dimension group of
  the sketch bar and the palette) switches the selected constraints off, or on again when none of them
  is on. A disabled dimension is a reference: its label shows the measured value in parentheses in
  the muted `Standing::Inactive` colour, and a disabled constraint's description ends in
  "(disabled)".
- Dimensions start at the displayed geometry's measured value. Every sketch transaction first
  settles the sketch to the last result when up to date (`Model::settled_sketch`).
- A new dimension whose every entity is `EntityState::FullyConstrained` in the settled solution
  (`Model::settled_solution`, none while a recompute is pending) is already determined, so it is
  added inactive in the same transaction, labelled "Add reference ...", with a notice saying it
  shows the measured value and can be enabled to drive. It takes no focus for typing a value. The
  test is conservative: a circle counts only when its centre and radius are both determined.
- Smart dimension (`Tool::Dimension`, `dimensioning.rs`, first in the sketch bar's Dimension group,
  Sketch › Dimensions and the palette) takes the geometry clicked after it, not the selection: the
  selection is cleared when it starts and then holds its picks. A click on a sketch item picks it (a
  picked one again lets it go); `dimensioning::fitting` chooses the dimension: a line's length, a
  circle's diameter and an arc's radius for one pick, the angle between two lines that are not
  parallel, and otherwise the distance between the two picks (origin and axes included). A second
  pick adds its dimension at once, except two points (`dimensioning::awaits_placement`), which wait
  for a click placing it; a single pick waits for a second, and Enter or a click on empty space
  adds the single one. Where the placing click lands chooses the dimension
  (`dimensioning::placed`, from the sketch pointer): for two points or a lone line, within their
  horizontal span and above or below them the horizontal distance, within their vertical span and
  beside them the vertical one, elsewhere (and always for level or upright ones) the aligned one;
  for a lone arc, beyond it within its sweep its length, inside it its sweep, outside its sweep its
  radius. Enter always adds the aligned distance, the length or the radius. A spline is refused in
  words and a lone point waits. The dimension goes
  through the same candidates, checks, reference rule and inline field as the dimension buttons
  (`ConstraintTool::candidates_among`, `add_constraints`), the field taking the typed value. The
  prompt says what Enter would add and the hover what a click would; with the tool active labels
  are not interactive, Activate picks the highlighted item as a click would, and Escape lets go of
  the picks before leaving the tool.
- Constraint states, degrees of freedom and redundancies come from the last evaluation
  (`sketch_status.rs`, `scene.rs`), never from solving on the UI thread (drags solve on their own
  worker).

## Open ends

- The edited sketch's open ends (`SketchResult::open_ends` of its up-to-date result) are ringed in
  `canvas::WARNING` over the view (`annotations.rs`, only those in view), counted in a warning
  pill beside the sketch's status in the sketch bar ("2 open ends", its hover saying to join them)
  and in the 3D view's description, so an outline that will not close is seen while drawing
  rather than when the extrusion fails.
- Points held past the drawn ends of their line or arc (`SketchResult::beyond`) are marked the same
  way in `canvas::MUTED`: a dashed extension from the curve's nearer end to the point and a ring,
  counted in an info pill ("1 point beyond its curve", its hover `sketch_status::BEYOND_HELP`)
  and in the 3D view's description. It is information, not a warning, since a point on a line's
  extension is often meant (`Target::Extension`).
- The edited sketch's closed regions (its result's display regions) are tinted `CLOSED_REGION` on
  the front layer, unpicked (`scene::Builder::closed_regions`), while the shown geometry is the
  result's (`same_geometry`), so a drag never shows regions it left behind. A sketch too large for
  recompute to find them unasked gets them asked for each frame it is edited
  (`Model::request_regions`, one request per result), and their arrival moves the displayed
  sketches' generation so the cached scene picks them up.

## Displayed sketches

- `Model::displayed_sketch` (`display.rs`) is the definition with solved positions wherever the
  last result has the same entity. `DisplayedSketches`, owned by `Model` and handed to the scene
  through `Sources`, works it out once per sketch and keeps it, with the bounds of its points that
  fitting and the reference size use, until the document or evaluation changes (`forget`), so a
  frame compares, copies and polylines no sketch for these. Its `generation` moves on `forget` and
  when a dragged sketch is shown or dropped, which the cached scene watches (`app.md`).

## Annotations

- Drawn with the egui painter (`annotations.rs`, placement in `annotation_layout.rs`) from the
  displayed geometry through the current view, with offsets and sizes in screen points and no
  stored positions. Dimensions sit away from the sketch's centre, and linear ones measured along
  one line on one side whose spans overlap stack into lanes (`annotation_layout::lanes`, shortest
  nearest, `LANE_SPACING` apart), so an overall dimension clears the chain beneath it; other constraints are glyphs
  stacked beside each constrained entity on the opposite side.
- Glyphs keep clear of dimension labels and of each other (`annotation_layout::place_glyphs` over
  `Obstacles`); when nothing is free the least covered place wins. An entity's anchor is clipped to
  the view first (`within_view`; off-screen entities get no glyphs, a line is anchored at the middle
  of its visible part), and the places tried are generated lazily from the middle out, at most
  `MAX_GLYPH_SHIFTS` steps each way, so zooming in never makes a frame walk a line's full length.
  An entity shows at most `MAX_STACKED` glyphs: past that the last slot is a "+N" mark whose
  hover lists the rest and whose click selects the first of them. Show or hide constraint glyphs
  (Alt+G, View menu, palette; `ViewportState::glyphs_shown`) hides them all, dimensions staying.
  Labels use `canvas::body` on
  the canvas backdrop, glyph letters `canvas::emphasis`.
- Labels show the expression in the document's naming, followed by its value when not a literal;
  conflicting and redundant constraints take the error and warning colours. Label texts are kept
  per sketch until the revision, evaluation, displayed sketches or units change
  (`annotations::LabelTexts`), and a mark's description is formatted only for its hover
  (`annotations::Hover`), so a still frame formats and evaluates nothing.
- Labels and glyphs are `Pickable::SketchConstraint`: hover highlights the entities, click selects
  (Shift or Ctrl toggles), Delete removes selected constraints and entities in one transaction.
  They are painted but not interactive while a drawing tool is active.
- Double-clicking a label opens an inline `commit_field` with the value selected, as does a new
  dimension or any `Focus::Dimension` of the edited sketch, which the app takes from the panels
  and hands to the viewport, waiting until the dimension can be drawn.
