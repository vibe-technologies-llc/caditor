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
  - "crates/caditor/src/body_snap.rs"
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
  - "crates/caditor/src/blend_curving.rs"
  - "crates/caditor/src/tidying.rs"
  - "crates/caditor/src/tidy_panel.rs"
  - "crates/caditor/src/gearing.rs"
  - "crates/caditor/src/gear_panel.rs"
---

# Sketch editing in the app

## Editing context

- A context, not a mode: `editing.rs` holds the edited sketch and the active `Tool`, changed by
  `Action::Editing` commands that `app::perform` routes after the UI pass. It ends by itself when
  the sketch disappears or another document opens.
- Entering faces the camera to the plane, fits it, dims other features (unpickable) and keeps only
  that sketch selected.
- Look at sketch (`Command::LookAtSketch`, Alt+Shift+V, the palette and the button beside Finish
  in the sketch bar) animates back to the same facing view at any time while editing, and turns it
  a quarter turn when the view already faces the sketch.
- The edited sketch, its origin and axes and the drawing preview go on `Layer::Front`
  (`render.md`), so a body never hides or z-fights with them wherever the sketch lies.
- Slice the bodies at the sketch plane (`SectionCommand::SliceSketch`, Sketch menu, palette, no
  default key; kept for the session in `ViewportState::sketch_slice`, not saved) cuts away what
  lies on the side of the sketch plane the normal points to, the side the sketch is faced from,
  while a sketch is edited: `ViewportState::shown_section` puts the edited plane, hatched, first
  among the section planes (`app.md`), so a sketch inside a body is drawn on a section of it.
  Faces lying on the plane are kept (`section_slack`), and the edited sketch on the front layer is
  never cut.
- Clicks and primary drags select with the Select tool; a drawing tool (`Tool::draws`) draws, and
  the modify tools (`Tool::modifies`) act on what is under the pointer. Escape backs out one step
  at a time in the order of `ViewportState::escape`, counting only presses that are not key
  repeats (`pressed_without_repeat`), so a held Escape takes one step and never walks on to
  finishing the sketch. The Select and Finish tooltips (`SELECT_KEY`, `FINISH_KEYS`) say when
  Escape reaches them.
- Default keys come from `Command::default_shortcuts`, not from here.

## Dragging and box selection

- With Select, a primary drag starting on a non-reference entity of the edited sketch is a `Grab`:
  a point, line, arc or spline moves its points by the pointer's offset, a circle alone changes
  radius, and a selected grabbed entity moves every selected entity.
- Each pointer move sends `DragCommand::Move`; `SketchDragging` (in `Display`) gives the newest to
  its own worker thread, which drops older unstarted ones and solves each from the previous
  solution of the same drag with `solve_from`. Results are shown through
  `DisplayedSketches::show_dragged`, which every consumer of a displayed sketch sees.
- A grab snaps while snapping is on or Alt is held, and Ctrl is not held
  (`ViewportState::grab_snapping`, `Grab::follow`); with Alt it lands as a held snap does
  (`tracking::land` with a `Hold`, see Snapping). Its handle, the grabbed point or, for a curve or selection, the moving point
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
- A primary drag starting on projected geometry or on the origin or axes of the edited sketch
  moves nothing and draws no box: it is `PrimaryDrag::Unmovable`, which says why beside the
  pointer where `DRAG_BLOCKED` goes (`PROJECTED_STAYS`, `REFERENCE_STAYS`) until release.
- A primary drag elsewhere draws a box: left to right a window taking what lies inside (curves
  faceted as drawn, `app.md`), right to left a crossing box taking what it touches; with Select
  with a lasso on, a freehand outline taking what lies inside it (`app-input.md`). It replaces
  the selection (Shift or Ctrl adds); a point is left out when a curve it belongs to was taken.
- Split the selected curve at the selected point (`Command::SplitCurve`, Sketch menu, palette;
  `sketch_tools::SplitChange`) takes one selected point and the line, arc or elliptical arc
  selected with it, or the one such curve the point lies on by `Coincident`, and splits it there
  in one transaction;
  anything else is refused with `NOTHING_TO_SPLIT` or the sketch's reason. A point placed on a
  curve first (the Point tool snaps onto it) gives a split anywhere.
- Respace the selected fit-point splines by their points (`Command::RespaceFitSplines`, Sketch
  menu, palette, no default key; `sketch_tools::RespaceChange`) turns the selected evenly spaced
  fit-point splines (from files saved before centripetal spacing) into centripetal ones in one
  transaction (`Sketch::respace_fit_spline` through `reshape_sketch`), keeping their points;
  without such a spline selected it is unavailable with `NOTHING_TO_RESPACE`.
- Break the selected curves at every crossing (`Command::BreakCurves`, Sketch menu, palette;
  `sketch_tools::BreakChange`) takes the selected lines, arcs and elliptical arcs and breaks each
  at every
  crossing with the other curves and the axes in one transaction (`sketch.md`, Break), so the
  pieces of a profile come in one command rather than one point at a time. Nothing selected, or
  nothing selected that crosses another curve, is refused with `NOTHING_TO_BREAK` or the
  sketch's reason in a notice.
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
  reference geometry or a constraint reaching outside the copy. Copying writes it to the system
  clipboard as text (`caditor_file::sketch_clipboard_text`, `file-format.md`) with the parameters
  its dimensions use, so another caditor pastes it too; the app keeps nothing else, the last text
  copied standing in only when the system clipboard holds no text. Paste reads the clipboard's text
  (`read_sketch_clipboard`): foreign, damaged or newer text, or features, are refused in a notice
  saying so. It is one transaction (`Sketch::paste` on the working copy under fresh ids, a `Fix`
  moving with the copy) centred under the pointer when it is on the sketch, else in place in
  another sketch or shifted by a quarter of the copy's size when pasted into the sketch it came
  from in the same model, and selects what it pasted. Dimensions keep parameters of the same name
  in the target model, else take the copied value, and a notice says how many values became
  numbers.
- Double-clicking a curve (no tool active) selects its chain, the lines and arcs joined end to end
  that Offset would take (`Sketch::offset_chain_through`).
- Select what is still free (`Command::SelectFree`, Sketch menu, palette, no default key;
  `sketch_drag::select_free`) replaces the selection with the edited sketch's points and curves
  that are not `EntityState::FullyConstrained` in the settled solution (`Model::settled_solution`),
  projected and reference geometry left out and a point dropped when a curve using it is taken.
  Without a settled solution (recompute pending, the sketch failing) it is refused with
  `NOT_SOLVED`, and with nothing free with `NOTHING_FREE`. The degrees-of-freedom pill runs it
  (Status pills, below). Select all and Select free state their availability each frame with
  early-exit checks (`can_select_all`, `can_select_free`) and build their lists only when run.

## Drawing tools

- `drawing.rs` holds the tools' state and `shapes.rs` their geometry. The drawing is keyed by a
  `Shape` (tool plus way of drawing). Changing either carries the points placed so far when they
  mean the same in the new shape (`carries_points`): a lone first point that is a centre
  (rectangle by centre, circle by centre, arc, slot by centre, arc slot, polygon by corner or side
  middle, ellipse, elliptical arc), a corner (rectangle by corners or three points, polygon by
  side) or a point on a circle (circle by two or three points) in any shape whose first point is
  the same; a three-point arc's and a conic's start and end; and every spline point between ways
  that both place control points or both fit points (a lone first point between the two open
  ways). Anything else drops a shape in progress on a tool change, while a change of way within
  the same tool (`Drawing::refuses_mode`, checked in `app::perform` before `SetMode` applies) is
  refused in a notice telling to finish or cancel the shape first, keeping it. Each finished
  shape is one transaction, settled first like any sketch transaction; inferred constraints are
  checked with `Sketch::check_constraint` on a shadow sketch and skipped if refused.
- A shape with no size gets a `Refusal` (a notice for a click, the field error for a typed point).
- A press that drags or is held too long to be a click still places a point where it is released,
  since egui reports it as a drag (`ViewportState::click`). When nothing of the shape is placed
  yet and the release is at least `DRAG_DRAWS_FROM_PRESS` from the press, the press position is
  placed first (`place_press_point`), so one press-drag-release draws a line, rectangle or circle;
  in a line chain a drag from its last point draws a tangent arc instead (below).
- What is being drawn shows its size so far below the snap label (`Drawing::readout`): a line, a
  rectangle, a circle, a polygon (radius, or side, and its sides), an arc (radius, and the sweep once
  its end is chosen), a tangent arc (radius and sweep), a three-point arc or circle (radius), a
  three-point rectangle (both sides), a slot (length and end diameter), an arc slot (radius, sweep,
  then width) and a spline (length and angle of the leg from its last control point).
- Holding Ctrl places the point exactly under the pointer: no snapping and no alignment guides
  (`Drawing::place_freely`). It also stops a tangent arc from starting, which needs a snapped
  point. Snapping on or off for good (`Command::ToggleSnapping`, View menu and palette, kept for the
  session in `ViewportState::snapping`) has the same effect as holding Ctrl all the time, except
  that holding Alt still snaps; while it is off the drawing prompt says "Snapping off" before the
  Alt hint instead of naming Ctrl. Ctrl wins when both are held (AltGr arrives as Ctrl+Alt on
  Windows).
- The pointer is on the sketch only within `MAX_LENGTH` of the origin, so an edge-on view cannot
  place a point at an enormous distance.
- Lines and tangent arcs chain, each joined to the last end by `Coincident`, until Enter,
  Escape, a click on the last point, or a line closing the outline on the chain's first point
  (`Drawing::finish` gives `Ended::Stopped`). Splines finish on Enter or a click on the last
  point, and close (periodic, `SplineKind`) on a click on the first once three are placed ("Close
  the spline"; a typed point there does the same); Enter with fewer points than the spline's kind
  needs keeps them and is refused in a notice (`Refusal::SplinePoints`,
  `ClosedSplinePoints`). Backspace or Delete (both kept from commands mid-shape,
  `KEPT_WHILE_DRAWING`) takes back the last point, and `ViewportState::prompt` adds one hint
  for it to every shape in progress (`TAKE_BACK_HINT`). Take back the last point, Finish the shape
  (a line, tangent arc or spline chain only, `Drawing::finishable`, as Enter) and Cancel the shape
  (as Escape; `Command::TakeBackPoint`, `FinishShape`, `CancelShape`, palette and the view's
  context menu, no default key) do the same without the keys. Each continuing segment records its
  anchor (`ChainStep`); taking back undoes the last segment when it is the newest undo step, and
  when the anchor's point is gone (undo, a deletion) the chain steps back to the newest anchor
  still there rather than ending.
- An arc runs the way the pointer swept round its centre; a typed end goes the shorter way
  (`Sweep::aim`), and Reverse the arc sends either the other way. The end is projected onto the
  circle through the start and keeps its snap only if the target lies on that circle.
- An ellipse is its centre, the end of its major axis (levelled from the centre like an arc's
  start, kept by `HorizontalPoints` or `VerticalPoints`) and a width point that never snaps, its
  distance from the major axis the minor radius (`shapes::ellipse_through`). An elliptical arc's
  third point sets the minor radius the same way and starts the arc where the ray from the centre
  through it meets the ellipse (`shapes::toward_on_ellipse`); its end is the same projection of
  the pointer and runs the way the pointer swept round the centre (`Shape::sweeps_from`), Reverse
  the arc included. Neither has a default key (every free one is taken); the Curve button, the
  Sketch menu and the palette reach them.
- A conic (`Tool::Conic`, in the Curve group, no default key) is its start, its end and its apex,
  where the tangents at its ends meet (`Draft::conic`, a `SplineKind::Conic`), previewed with
  the pointer as apex; an end on its start (`Refusal::ConicEnds`) or an apex on the line through
  the ends (`ConicApex`) is refused. Its rho, 0.5 until set and kept while caditor runs like the
  polygon's sides, is typed in the point field as `0.3 rho` (`typed_point::rho`, any
  dimensionless expression; outside 0.01 to 0.99 refused in words) and shown in the prompt and
  the readout. An expression using parameters also gives the new conic a `Rho` dimension holding
  it (`Draft::conic`), so the parameter keeps driving its shape.
- A tangent arc starts on a point ending a line, arc or spline (the newest if several) and leaves
  along that curve's direction with a `Tangent`.
  Switching between the Line and Tangent arc tools with a segment started (`Drawing::sync`) keeps
  the chain and its anchors: the arc leaves along the line just drawn, and the lines go on from the
  arc's end. Stepping back onto a line step while arcing finds its tangent again from the sketch.
- The Line tool turns its next segment into a tangent arc without a tool change (`Drawing::arcing`,
  `Arcing`; `Drawing::current` then draws, previews, labels and prompts as `Shape::TangentArc`
  from the chain's last point, with the same `Tangent` and `Coincident`), and the chain carries on
  with lines after that one arc, as Fusion's line tool does. A press on the last point dragged
  away is `Arcing::Dragged` (`ViewportState::arc_from_press` hovers the press position again at
  the drag's start, as `place_press_point` does, and `Drawing::arc_from_press` takes it when the
  hover is the pending last point and the point ends a line, arc or spline): the release places
  the arc's end, and a release back on the last point draws no arc and keeps the chain. The arc
  bends to the side of the line the pointer is on (`shapes::tangent_from`), so dragging back
  across the line's direction switches it to the other side. From the keyboard, Next segment:
  tangent arc or line (`Command::ChainArc`, palette, Sketch menu and the view's context menu, no
  key of its own) toggles `Arcing::Toggled`, and the Tangent arc command (T) while the Line tool
  has a point placed runs it instead of switching tools (`ViewportState::arc_on_tangent_arc_key`,
  before the sketch bar takes the command); the ribbon's Tangent arc button still switches tools.
  The prompt's keys name it ("T: a tangent arc next", "T: a line instead") while it applies
  (`Drawing::chain_arc`: the Line tool with its last point on a point; refused with
  `NOTHING_TO_ARC_FROM` when no line, arc or spline ends there). A drawn arc, a dragged arc's
  release, taking back or stepping back go back to lines.
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
  grouped on the ribbon under one Arc button, and so do Spline, Ellipse and Elliptical arc under
  one Curve button (`app-look.md`), since an ellipse is a curve of its own, not a way to draw
  another.
- Spline has four ways (`SplineMode`): by control points, through fit points (the curve passes
  each placed point, which stays a point to constrain and dimension, spaced centripetally,
  `SplineKind::fit`), and both closed, where Enter closes the loop. Clicking the first point closes either open way too. The preview is the
  curve of that kind through the placed points and the pointer.
- Blend curve keeps its two ways the same way: tangent (G1) and curvature-continuous (G2)
  (`ShapeMode::Blend`, `ShapeModes::blend`), listed with the shapes under Sketch › Ways to draw
  shapes; the modify tool reads the way each frame and its prompt leads with it.
- Running the tool's command while it is active steps to its next way, round to the first;
  clicking its ribbon button only chooses the tool. Off-ribbon tools with ways (Blend curve) step
  the same way from their command (`sketch_toolbar::off_ribbon_tool`). Every way is also its own command
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
- Intersect (`Tool::Intersect`, Alt+I, Sketch menu, palette; no button of its own on the sketch
  bar, whose Edit group would grow a row, but in Project's corner menu,
  `sketch_toolbar::OFF_RIBBON`, `MODEL_GEOMETRY_TOOLS`) draws where the model crosses the sketch
  plane. It is a projecting tool (`Tool::projects`), so bodies are pickable as for Project, and
  `Context::intersecting` also draws and offers the shown datum planes above the sketch and the
  shown principal planes that cross it (a parallel one is not drawn). A click on a face adds the
  cut through that face (`section_curves`, filtered by the face's name), a Shift-click or
  `Command::IntersectBody` (Shift+Space, on the highlighted or hovered face) the cut through its
  whole body, and a click on a datum or principal plane the line where it crosses the sketch as a
  construction line, spanning the shown bodies (`datum_reach`). Each is one undoable transaction
  from the body's state at the sketch; a plane missing the face or body, a parallel plane, a later
  datum and cuts already drawn are refused in words.
- The drawing tools project body corners and edges themselves when a point snaps to one
  (Snapping), so drawing from a body needs no Project first.
- Projected geometry is drawn in the `PROJECTED` palette, is never grabbed or dragged and follows
  its source on every recompute (`document.md`).

## Construction geometry

- Construction makes the selected curves construction in one transaction
  (`sketch_tools::ConstructionChange`), ordinary when all already are; with no curve selected it
  switches drawing (`ActiveSketch::construction`): new curves are construction, points stay
  points. Construction curves are dashed (`scene::curve_segments`) and coloured by constraint
  state like any curve, so dashes never carry a constraint state.

- The edited sketch's control-point splines (open or closed) and conics show their control
  polygon (`scene::control_polygon`, `Sketch::spline_control_points`): dashed like construction,
  in the curve's state colour at the regular width, never picked, so the points that shape the
  curve read as its handles. A fit-point spline shows none, its fit points being its handles.
  Show or hide spline control polygons (`Command::ToggleControlPolygons`, View menu, palette, no
  default key; `ViewAids::control_polygons_hidden`, shown by default) hides them for the session.

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

## Trim, extend, offset, mirror, patterns and sketch fillet

- `trimming.rs` and `modifying.rs` (a facade over `offsetting.rs`, `mirroring.rs`, `patterning.rs`, `filleting.rs`)
  hold the tools' UI state; geometry and constraint rules are the sketch's (`sketch.md`). Each
  frame the tool aims through the displayed sketch, previews the result and puts its words, or
  why not, where hover descriptions go. Trim and Extend aim at splines too, so they can be refused
  in words. Ellipses and elliptical arcs are Trim's targets like lines, circles and arcs
  (`trimming::is_trimmable`, the keyboard's steps and the trim path included); Extend takes the
  ends of elliptical arcs (its targets skip their centre and axis points) and refuses whole
  ellipses in words. The sketch axes and collinear or co-circular overlaps cut and bound them like any
  curve (`sketch.md`): the words name "Vertical axis" or the overlapping curve, the preview marks
  the cuts and the axis or overlapping curve is highlighted; the axes themselves are never aimed
  at.
- A click acts at once, without waiting for a GPU pick: one transaction built by `reshape_sketch`
  from a working copy (the definition with the displayed positions), settled first like any sketch
  transaction. A refusal is a notice, or for a typed value the field's error, and the tool stays
  active.
- Trim's primary drag is a trim path: every piece the path crosses is collected and trimmed on
  release in crossing order as one transaction, each found again by its middle on whichever curve
  now carries it after earlier splits. Extend ignores drags.
- Offset works on the selected chain; with none, a click on a curve selects its chain (an
  ellipse or elliptical arc alone). The pointer's side and distance choose side and distance,
  previewed live; an ellipse's words end `offsetting::FREE_SPLINE`, since its offset spline does
  not follow it (`sketch.md`). Mirror copies the
  selection about the line or axis under the pointer (sketch lines win a tie with an axis).
- Mirror and both patterns select inside the tool, as Smart dimension picks: started with nothing
  selected (or once the selection empties) they gather (`Modifying::gathers`), where a click on a
  sketch item toggles it in the selection (`ViewportState::gather_click`), a primary drag draws a
  box or lasso that adds what it takes, hover describes what a click takes, and the highlight
  commands step through the scene's pickables with Activate toggling. Enter moves Mirror on to
  its line and Circular pattern to its centre (`finish`, refused in a notice while nothing is
  selected); Rectangular pattern, which has no click step, gathers throughout and reads its
  numbers whenever typed. Escape goes back from the line or centre to gathering, then lets go of
  what was gathered (`Modifying::lets_go_of_selection`), then leaves the tool; started with a
  selection they go straight to the line or centre and Escape leaves the tool keeping it.
- Rectangular pattern and Circular pattern (`Tool::RectangularPattern`, `Tool::CircularPattern`,
  `patterning.rs`; Sketch menu, palette and the corner menu of Mirror on the sketch bar, no
  default key since every free one is taken, `sketch_toolbar::OFF_RIBBON`) repeat the selection
  (`sketch.md`, Patterns). Both take their numbers in the typed-point field ("Repeat"), which opens
  on a digit or `=` and previews the copies live while the text parses; Enter makes one undoable
  "Pattern geometry" transaction and an error (a refusal or text it cannot read) keeps the field
  open. Rectangular reads `count x spacing`, optionally `< angle` for a slanted direction, and a
  second such term after a comma for rows, square to the first unless it has its own angle:
  `4 x 10`, `4 x 10, 3 x 15`, `3 x 12 < 45`. Circular reads `count` to space the copies over a
  full turn or `count over angle` to spread them across it. Counts are whole numbers; spacings
  and angles are expressions with parameters, kept as typed. Circular first needs its centre: the
  one selected point no selected curve uses, else a point clicked or highlighted (the origin
  included, Highlight the next item steps through points and Space or Enter chooses), shown
  highlighted until Escape lets it go; the preview and Enter use it.
- Tangent circle (`Tool::TangentCircle`, `tangent_circling.rs`; Sketch menu, palette and
  Offset's corner menu, no default key, `sketch_toolbar::OFF_RIBBON`) takes the lines, circles
  and arcs the new circle touches (`sketch.md`, Tangent circles): the one or two of them selected
  when it starts, then each one clicked or highlighted and chosen with Space or Enter (an axis
  included; clicking a chosen curve lets it go, Escape lets go of the last). A third curve
  finishes it at once, previewed live while the pointer is over it, as the circle whose centre is
  nearest the pointer, so inside or outside a triangle's sides picks its incircle or an
  excircle. With two chosen the typed-point field ("Radius", opens on a digit or `=`) draws the circle
  of that radius nearest the pointer instead, previewed while the text parses and kept as typed
  with parameters. Each draw is one undoable "Draw tangent circle" transaction.
- Spur gear (`Tool::Gear`, `gearing.rs`; the Polygon button's corner menu, the Sketch menu among
  the drawing tools and the palette, no default key) draws an involute gear outline (`sketch.md`,
  Spur gears). While it is active the Spur gear panel (`gear_panel.rs`, a right-hand panel
  sharing the side panels' room, closing back to Select) holds Module, Teeth, Pressure angle,
  Profile shift, Root fillet and Bore as text fields read each frame like any value
  (`patterning::quantity`: units and expressions with parameters; teeth a whole number, bore
  empty for none), kept in `Modifying` (`GearSettings`, from 2 mm, 20 teeth, 20°, no shift,
  0.75 mm and no bore) while caditor runs, across tools and sketches. Under them it lists the
  pitch, tip, root and base diameters, or the gear's refusal in an error callout; a field that
  does not read says so under itself. The pointer previews the gear centred on the point under
  it (the origin included) or where it is, the outline as curves, its four circles as dashed
  guides and the words saying its teeth, module and tip diameter, or why not; a click draws it
  there. Enter in the view, or the panel's primary button ("Draw at Point 7", "Draw at the
  origin"), draws it at the one selected point, else the origin; the highlight commands step
  through the points and the origin and Space or Enter draws on the highlighted one. Each is one
  undoable "Draw spur gear" transaction, refused in a notice naming the tool otherwise.
- Blend curve (`Tool::BlendCurve`, `blend_curving.rs`; Alt+Shift+B, Sketch menu, palette and
  Offset's corner menu, `sketch_toolbar::OFF_RIBBON`) joins two curve ends with a spline (`sketch.md`, Blend
  curves). It takes the end of a line, arc or spline (projected ones included) nearest the
  pointer on the curve under it, or a point selected when it starts that ends exactly one curve;
  the highlight commands step through every such end and Space or Enter chooses. Clicking the
  chosen end again lets it go, Escape lets go of the highlight and then the chosen end. With one
  end chosen the second under the pointer or highlighted previews the spline and its control
  points live and says in words what it would draw or why not; a click or Activate draws it in one
  undoable "Draw blend curve" transaction, refused in a notice otherwise.
- Sketch fillet is named so, to keep it apart from the model's Fillet. It first takes its corners:
  started with a selection, every selected point that is a corner (`Sketch::corner_at`) and every
  corner where two selected lines, arcs or elliptical arcs meet (`Sketch::fillet_corners`),
  duplicates dropped
  (`filleting::gathered`), the words saying "Round 4 corners" and how many selected items were
  left out with the first one's reason (a point that is no corner, a curve meeting no other
  selected one); two selected curves meeting at no corner make one corner where they cross or
  would meet (`filleting::Aim::Crossing`, picks from `Sketch::crossing_picks`); else the curve end
  under the pointer, refused in words when it is no corner. A click on a line, arc or circle away
  from a corner picks it as the first curve (highlighted, "Click the second line, arc or circle",
  Escape lets it go) and a click on a second one chooses the corner where they cross or would meet
  (`Sketch::join_at_crossing` on a copy for the hover, the preview and the fillet, refused in words
  when they cannot be joined). A click, or Space on a highlighted corner, adds further corners. Then the pointer sets the radius
  through the first corner (`radius_through`), previewed on all of them, and a click, a typed
  value or Enter rounds them all in one transaction ("Fillet corners" when several, one undo
  step), each corner found again by its point on the working copy as the ones before change it.
  The chosen corners are cleared after each fillet. The last value committed (typed as typed, or
  the pointer's as a length) is kept per cut while caditor runs (`Modifying` keeps it across tool
  changes, `LastSizes`): it is the field's placeholder, Enter on an empty field or with the field
  closed and corners chosen reuses it, and the prompt's keys say "Enter: round it with
  4, as last time"; with none kept, Enter rounds at the pointer as a click does. Sketch chamfer (`Tool::Chamfer`) is the same tool cutting the corner
  instead (`filleting::CornerCut`): the pointer sets one distance for both sides
  (`distance_through`) and the field is "Chamfer", which reads `5` (the same on both curves),
  `5, 3` (a distance on each, in the order of the corner's curves) or `5 < 45` (a distance on the
  first curve and the angle of the cut from it) as a `ChamferSize` (`sketch.md`), previewed
  while it parses and kept as typed, parameters included. It has no button of its own on the
  sketch bar, whose Modify group would widen past one row (`sketch_toolbar::OFF_RIBBON`, its
  command still offered there); Sketch fillet's corner menu, the Sketch menu, the palette and its
  key reach it.
- The tools without a button of their own sit in the corner menu of a partner's compact button
  (`sketch_toolbar::Partners`, `widgets::compact_corner_menu_button`, a notch in the button's
  lower right that adds no width): Sketch fillet with Sketch chamfer (`CORNER_TOOLS`), Offset with
  Tangent circle and Blend curve (`CURVE_FROM_GEOMETRY_TOOLS`), Mirror with both patterns
  (`COPYING_TOOLS`), Project with Intersect (`MODEL_GEOMETRY_TOOLS`). The menu lists the partner
  first, each with its keys, and the notch takes the accent colour while any of them is active.
- Offset and Sketch fillet take a typed value in the typed-point field ("Offset
  by", "Fillet radius"), and Sketch chamfer its text above: the preview follows the text while it parses, Enter commits the expression as typed
  (parameters included) and an error keeps the field open. A negative offset goes to the other
  side, so the keyboard alone does it: select, Offset, the distance, Enter.
- With any of them active the highlight commands step through that tool's targets instead of the
  scene's pickables (`app-input.md`), and Activate or Enter acts on the highlighted one.

## Snapping

- `snap.rs` runs on the UI thread against the displayed sketch in screen space. Priority: the
  shape's pending point; points and the origin within `POINT_TOLERANCE`; then, only where any snap
  is accepted and within the same tolerance, the middle of a line, arc or elliptical arc
  (`Target::Midpoint`, an elliptical arc's halfway round its parameter), the
  centre of a closed outline (`Target::Centre`), the crossing of two of the up to
  `MAX_CROSSING_CURVES` curves nearest the pointer, axes included (`Target::Intersection`; a
  spline crosses lines, circles, arcs, axes and other splines, and an ellipse lines, circles, arcs,
  axes, splines and other ellipses, through `Sketch::curve_crossings`), the right, top, left and
  bottom of a circle or of an arc sweeping through them (`Target::Quadrant`) and the far end of an
  ellipse's major axis and both ends of its minor axis, those an elliptical arc sweeps through
  (`Target::AxisEnd`); lines, circles, arcs, splines and axes within
  `CURVE_TOLERANCE`, projecting onto the curve. A snapped point gets a `Coincident` with its
  target (with both curves at a crossing), a `Midpoint` constraint for a middle, a `Symmetric`
  about it of two opposite corners for a centre, or for a side a `Coincident` with the curve and
  a horizontal or vertical points constraint with its centre. The far end of a major axis is held
  by a `Symmetric` of the axis point and the new point about the centre; a minor axis end by a
  `Coincident` with the ellipse and, when the major axis is level or upright, a vertical or
  horizontal points constraint with the centre (`AxisEnd::MinorUpright`, `MinorLevel`), on a
  slanted ellipse (`MinorSlanted`) an `OnMinorAxis` with it, so the point stays at the end
  however the ellipse turns.
- A line's end within `POINT_TOLERANCE` of where a line from its start would touch a circle, arc,
  ellipse or elliptical arc (`snap::tangents_from`, the two tangent points from outside it, within
  an arc's sweep; an ellipse's found on the unit circle it scales to) lands
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
  `ALIGNED_CROSSING_TOLERANCE` of the pointer. Typed points never align, and a locked heading
  (`app-input.md`) takes the place of every snap.
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
  stays free, joined by no constraint, and shows the snap marker labelled "On the grid"
  (`Snap::Grid`).
- Holding Alt while drawing or dragging holds the snap (`snap::held`, `Hold`, `Drawing::hold_snap`,
  ahead of directions, tracks and the toggles, which stay as they are): the point always lands on
  the nearest crossing of the grid's minor lines, whether or not grid snapping is on, unless a
  snappable entity lies no further than it on screen. Point-like targets (pending point, points,
  middles, centres, crossings, quadrants) pull from `HELD_POINT_PULL`, three times the hover's
  tolerance, nearest first; failing those, curves and extensions from `HELD_CURVE_PULL`, and a
  curve snap moves to the grid crossing nearest its foot when that crossing lies on the curve, so
  Alt along a grid-aligned line or an axis steps by the grid and stays joined to it. Targets keep
  their constraints and labels; a grid crossing is free, labelled "On the grid", and, for the
  points that align or level from a start (`aligned_from`, `levelled_from`), exactly level or
  upright with that start takes Horizontal or Vertical as an aligned point would ("On the grid,
  horizontal"). Width points still never snap, typed points are untouched, and with the grid hidden
  only entities are taken. The drawing tools' key hints name Alt beside Ctrl.
- While a drawing tool draws, the shown bodies standing at the sketch
  (`Evaluation::body_state_seen_by`; bodies made later in the tree or hidden are left out) are
  snapped to as well (`body_snap.rs`, `Target::Body`): their corners, the middle of an edge that
  projects to a line or arc, the centre of a round edge that projects to a circle, arc, ellipse or
  elliptical arc, and any point on an edge, each matched where it projects onto the sketch plane,
  as Project would draw it, so in a view facing the sketch they sit where the body's own are seen.
  Every item of a shown body is offered, hidden or not: projected onto the plane, a hidden corner
  lies where the one hiding it is, or is reached only through it, so leaving it out would hide
  nothing but cost an occlusion test per hover. Where two project to one place the one nearer the
  sketch plane wins (`BodySnaps` keeps its candidates sorted by depth, and a tie keeps the
  first), so a corner on the plane is taken before the one behind it. Point-like sketch targets
  win, then body corners, middles and centres, then sketch curves, then body edges. `Accept::Points`
  takes body corners and centres only, `OnCircle` none, nor does a tangent arc's start; the held
  snap (Alt), directions' and tracks' crossings and acquired points take none, as a body item is
  not in the sketch until a point lands on it. The label names the body: "Corner of Base",
  "Middle of an edge of Base", "Centre of a round edge of Base", "On an edge of Base".
- A point landing on a body item projects that vertex or edge in the shape's own transaction
  (`Draft::projection` through `TransactionBuilder::add_projected`: the same fixed geometry and
  stable `ProjectionSource` Project makes, or the existing projection of that source, once per
  shape) and joins it: a `Coincident` with the projected corner or round edge's centre, a
  `Midpoint` on the projected edge, a `Coincident` on it; where a shape uses a target point
  itself (a circle through it, a polygon's side middle) it uses the projected point. One undo
  takes the drawing and its projections away together. The candidates come from `BodySnaps`, a
  cache per sketch session of the corners (one sharing its name with another vertex is left out,
  as Project refuses it) and non-seam edges, held in `Drawing` behind an `Arc` and rebuilt by
  `Drawing::track_bodies` only when the edited sketch, its plane or a shown body's result `Arc`
  changes (`Basis`), so a hover walks only its points and edges, as for the sketch's own. A
  placement made from an older cache (its `generation`) lands free.
- Preview curves are faceted like the sketch's (`Drawing::preview` and `Trimming::preview` take
  the scene's `Faceting`); the snap target and a direction's reference line
  (`Drawing::snap_entities`) replace the GPU hover while a drawing tool is active.

## Constraint tools

- `sketch_tools.rs` turns the selection into candidates checked by `Sketch::check_constraint`;
  `sketch_toolbar.rs` offers them as buttons and commands, disabled with what to select
  (`app-look.md`).
- Curvature (`ConstraintTool::Curvature`) takes a spline and the line, arc or spline at one of its
  ends and adds `Tangent` (left out when already there) with `Curvature`. It is not on the sketch
  bar, whose Constrain group would grow a third row (`sketch_toolbar::OFF_RIBBON_CONSTRAINTS`, its
  command still offered there); Sketch › Constraints, the palette and its key reach it.
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
- Scale the whole sketch on its first dimension (`Command::ToggleFirstDimensionScales`, Sketch
  menu, palette, no default key; off by default, kept for the session in
  `ViewportState::first_dimension_scales`) makes a value typed in a dimension's field on the
  canvas (`sketch_tools::dimension_change`) scale every point and circle of the settled sketch
  about its origin by the new value over the measured one, in the same transaction as the value
  ("Scale <sketch> to its first dimension"), so a traced or imported outline is sized in one
  step. It applies only to an active length dimension that is the sketch's only active dimension,
  with no active `Fix` and no projected geometry and the sketch settled; otherwise the value is
  set as usual.
- A new dimension whose every entity is `EntityState::FullyConstrained` in the settled solution
  (`Model::settled_solution`, none while a recompute is pending) is already determined, so it is
  added inactive in the same transaction, labelled "Add reference ...", with a notice saying it
  shows the measured value and can be enabled to drive. It takes no focus for typing a value. The
  test is conservative: a circle counts only when its centre and radius are both determined.
- Smart dimension (`Tool::Dimension`, `dimensioning.rs`, first in the sketch bar's Dimension group,
  Sketch › Dimensions and the palette) takes the geometry clicked after it, not the selection: the
  selection is cleared when it starts, then holds its picks, and is cleared again when it ends
  (another tool or sketch, or leaving the sketch), so no pick stays selected. A click on a sketch item picks it (a
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
  radius. A point or circle and a slanted line, or two circles, also wait for placement when their
  horizontal and vertical offsets are both above zero (`dimensioning::offset_ends`, the ends of
  `Sketch::axis_offset_ends`): within the horizontal one's span the horizontal distance, within
  the vertical one's the vertical, elsewhere the distance as before. A lone ellipse or elliptical arc adds its major and minor radii (`MajorRadius`,
  `MinorRadius`, the one already held left out), as the Radius tool does with one selected; an
  ellipse picked with a point or any curve takes the distance between them (drawn between
  `Sketch::ellipse_gap`'s points), an elliptical arc and a line sharing an end the angle between
  them as for an arc; with more picks it is refused in words, its centre and axis points
  dimensioning the rest. A lone conic takes its rho (`Constraint::Rho`, through the Radius candidates,
  labelled `rho` and laid out from the middle of its chord to the point it passes there), never
  added as a reference, since rho is no freedom the other constraints could take up
  (`sketch_tools::determines`). A point and a construction line also wait for placement (`dimensioning::about_axis`):
  across the line the diameter (`Constraint::AxisDiameter`, drawn from the point to its mirror
  image across the line and labelled Ø), on the point's side the distance. Enter always adds the
  aligned distance, the length or the radius. The Diameter tool adds the same diameter for a point
  and any line selected. A spline waits for a
  point, line, circle, arc or ellipse, whose distance from it is the dimension (from a curve, the
  gap where the spline bulges toward it, drawn between `Sketch::spline_gap`'s points); another
  spline is refused in words, and a lone point waits. A line and an arc sharing an end take the angle
  between them there instead of their distance, measured inside the corner as two lines' is
  (`sketch_tools::angle_to_arc`), drawn from the arc's tangent at the joint. The dimension goes
  through the same candidates, checks, reference rule and inline field as the dimension buttons
  (`ConstraintTool::candidates_among`, `add_constraints`), the field taking the typed value. The
  prompt says what Enter would add and the hover what a click would; with the tool active labels
  are not interactive, Activate picks the highlighted item as a click would, and Escape lets go of
  the picks before leaving the tool.
- Constraint states, degrees of freedom and redundancies come from the last evaluation
  (`sketch_status.rs`, `scene.rs`), never from solving on the UI thread (drags solve on their own
  worker).

## Constraining automatically

- Add the relations the drawing shows (`Command::FindRelations`), Dimension fully from a datum
  point (`DimensionFromDatum`, taking the one selected point as the datum) and Check the sketch
  for flaws (`CheckSketch`) open the Constrain automatically panel (`tidy_panel.rs`, a right-hand
  panel sharing the side panels' room) at its Relations, Dimensions or Check task; a segmented
  control switches between them. They are in the Sketch menu and the palette, not on the sketch
  bar, whose Dimension group would push it past one row at 1400 points.
- `Tidying` (`tidying.rs`, in the `Workspace`) works on the settled sketch only
  (`Model::settled_sketch`; "Waiting for the sketch to solve…" otherwise) on a thread of its own,
  since finding relations and dimensions solves (`sketch.md`): a job per basis (feature,
  revision, evaluation, task, tolerance, chosen kinds, datum), the one in flight cancelled when
  the basis changes, so the panel is redone after every change, an applied proposal included,
  and then says nothing is left. It closes when sketch editing ends or another sketch is edited,
  and with a new session.
- Relations and Check take a tolerance (`Looseness`: Tight, Normal, Loose, shares of the
  sketch's extent and angles, shown in the document's unit), Relations a checkbox per
  `RelationKind`, Dimensions the datum (Use the selected point, Use the origin). The proposal is
  listed by kind with what adding it leaves free, a dimension with its measured value, a flaw
  in words with its remedy; at most `MAX_LISTED` rows a group. Its entities are highlighted in
  the view as hovered geometry is (`ViewportState::preview_entities`, under the pointer's own
  hover), a hovered row's alone, so the set is seen before it is added.
- The primary action sits in a footer below the list (Add N relations, Add N dimensions, Fix N
  flaws) and applies the whole proposal as one undoable transaction settled first like any
  sketch transaction; dimensions go into the document's unit (`sketch_tools::in_unit`). A
  flaw's own button applies its fix alone; a fix set drops added constraints touching removed
  geometry. A sketch that does not solve is refused in words (`tidying::UNSOLVED`).

## Status pills

- `sketch_status::show` (the sketch's card in the tree) draws the status pill and, in words, a
  pill each for redundant constraints, open ends and points beyond their curves; the sketch bar's
  header keeps one height whatever the status: the title on its first line with the counts at its
  right end as `widgets::count_pill`s (an icon per kind, `icons::REDUNDANT`, `OPEN_ENDS`,
  `BEYOND`, and the number; the words in the hover and the accessible name;
  `sketch_status::counts_from_the_right`), the title truncating to leave them room and the line
  reserving a count's height, and the status pill alone on the second line, the header at least
  as wide as the widest status (`sketch_status::widest_status`), so the bar and the view below it
  never move as the status changes.
- Pills that act are `widgets::PillRole::Button`s (outlined, an outline in their colour on hover,
  the pointing hand, a focus ring, named "<words>: <what a click does>") and return a
  `sketch_status::StatusRequest`: a failed or conflicting status shows the problem in the card
  (`ShowProblem`), the degrees-of-freedom status runs Select what is still free, the open ends
  count Select the open ends (`Command::SelectOpenEnds`: selects the open ends' points and frames
  them, `ViewportState::fit_requested`) and the redundant count Select the redundant constraints
  (`Command::SelectRedundant`: selects each `Redundancy::constraint`, ready for Delete); both are
  sketch-scope commands in the Sketch menu and palette with no default key, refused in words
  without a solved sketch or with none (`sketch_status::NOT_SOLVED`, `NO_OPEN_ENDS`,
  `NOTHING_REDUNDANT`). The other statuses and the points-beyond count stay plain. In the tree
  card the selecting pills act only for the edited sketch, through `PanelState::sketch_command`,
  which the app triggers before the viewport runs its commands.

## Open ends

- The edited sketch's open ends (`SketchResult::open_ends` of its up-to-date result) are ringed in
  `canvas::WARNING` over the view (`annotations.rs`, only those in view, one ring per
  `RING_MERGE` square of the screen, so ends closer than that share a ring), counted beside the
  sketch's status (Status pills, its hover saying to join them) and in the 3D view's
  description, so an outline that will not close is seen while drawing rather than when the
  extrusion fails. Where more than `MOST_RINGS_PER_CELL` rings would fall in one
  `RING_CLUSTER_CELL` square (a zoomed-out sketch of thousands of unjoined curves), they become
  one double ring (`CLUSTER_RADIUS` round the ordinary one, so it is told by shape, not colour)
  at their ends' centroid (`annotations::open_end_marks`, over flat screen grids rather than a
  set, `ScreenGrid`, `ScreenCells`). Its hover (`Hover::OpenEnds`) says how many ends it stands
  for and to zoom in to tell them apart; it senses hover only, so clicks and the scene's hover
  reach the geometry beneath, and the open ends stay reachable through the keyboard highlight and
  Select the open ends.
- Points held past the drawn ends of their line or arc (`SketchResult::beyond`) are marked the same
  way in `canvas::MUTED`: a dashed extension from the curve's nearer end to the point and a ring,
  counted as information ("1 point beyond its curve", its hover `sketch_status::BEYOND_HELP`)
  and in the 3D view's description. It is information, not a warning, since a point on a line's
  extension is often meant (`Target::Extension`).
- The edited sketch's closed regions (its result's display regions) are tinted `closed_region` on
  the front layer (`scene::Builder::closed_regions`), while the shown geometry is the result's
  (`same_geometry`), so a drag never shows regions it left behind. With the Select tool active
  (`Context::selecting`) each is `Pickable::SketchRegion`, the area between crossings rather than
  whole curves: hover tints it, a click selects it, Shift or Ctrl adds more, and curves and points
  win near them (surface pick priority); a drag starting on one draws a box. Other tools leave
  them unpicked, so drawing, modifying and dimensioning never take a region. Outside sketch
  editing, with no feature open and nothing chosen in the view (`Context::picks_shown_regions`),
  every shown sketch's regions are drawn and picked the same way, like faces, on `Layer::Model`
  (so a body in front hides them, while one lying on a face draws over it). Extrude and Revolve
  take the selected regions (`app-modelling.md`), and Measure reads their area and section
  (`app.md`). A sketch too large for
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

- Drawn with the egui painter (`annotations.rs`, placement in `annotation_layout.rs` over the
  unit-free part shared with drawing export, `caditor_sketch::annotation`: what each dimension
  measures, lanes, label obstacles) from the
  displayed geometry through the current view, with offsets and sizes in screen points. Unplaced
  dimensions sit away from the sketch's centre, and linear ones measured along one line on one
  side whose spans overlap stack into lanes (`annotation::lanes`, shortest nearest,
  `LANE_SPACING` apart), so an overall dimension clears the chain beneath it; other constraints
  are glyphs stacked beside each constrained entity on the opposite side.
- Dragging a dimension's label places it: the offset is stored with the dimension (`sketch.md`)
  in the dimension's `LabelFrame` (the middle of a distance along it, a circle's centre, an
  angle's vertex along its first ray, an arc's centre along its start), so the label keeps its
  place relative to the geometry as it moves or turns, and the drop is one undoable "Move
  dimension label" transaction. A placed distance runs its dimension line through the label
  (extension lines to it, the line extended when the label is past an end); a placed radius or
  diameter points its leader at the label; a placed angle, arc length or sweep takes the label's
  distance as its arc's radius and extends the arc round to it; the others draw a leader from
  their usual label place. Placed dimensions take no lane. From the keyboard, Move selected
  geometry with one dimension alone selected (`annotations::label_to_move`) opens the same "Move
  to" field at the label (`@` for an offset) and moves it there.
- Glyphs keep clear of dimension labels and of each other (`annotation_layout::place_glyphs` over
  `Obstacles`); when nothing is free the least covered place wins, but only while it is covered by
  at most `MOST_GLYPH_COVER` of its area, and a group whose unshifted places (both sides, or a
  point's four quadrants) lie in cells already `FULL_CELL_SHARE` covered is left out without
  trying any (`Thinning::WhenCrowded`), so a zoomed-out dense sketch shows as many glyphs as fit
  and never walks crowded places. A thinned group also skips each place lying wholly in such
  cells before measuring its overlap, and stops measuring a place once it is covered more than
  the best so far or than `MOST_GLYPH_COVER` allows (`overlap_within`), so a partly crowded
  neighbourhood costs a cell lookup per place. Where each group's glyphs sit in the sketch
  (`annotation_layout::GlyphSite`: a line's ends, a curve's point and centre, a spline's middle
  leg) is found once with the measures and only projected per layout. A thinned group anchored on a line under `COLLAPSED_BELOW`
  points long on screen (`annotation_layout::collapses`) is left out unless one of its constraints
  conflicts or is redundant. A group anchored on a selected entity of the sketch, or holding
  a selected or keyboard-highlighted constraint, is placed first and always (`Thinning::Never`,
  `ViewKey::kept` and `forced`), so what the user chose is still found. An entity's anchor is clipped to
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
  (`annotations::LabelTexts`), and their measured sizes by text across revisions until the pixel
  density or the loaded fonts change (`SizesKey`, at most `MAX_MEASURED_TEXTS`), and a mark's description is formatted only for its hover
  (`annotations::Hover`), so a still frame formats and evaluates nothing.
- Marks are worked out in two cached stages. `annotations::Measures` (what each dimension
  measures, its lane, label frame and `annotation_layout::Reach`, the glyph groups with their
  standings, open ends and points beyond) is kept until the feature, revision, evaluation,
  displayed sketches or the dragged label change (`SketchKey`). `annotations::Marks` (laid-out
  dimensions with their label rectangles, placed glyphs, projected open ends and points beyond)
  is kept until any of those, the `SketchScreen` (camera, view size, pixel density), the view
  rectangle, units, the dragged label's offset, the dimension being edited, the forced
  dimensions or Show glyphs change (`ViewKey`), so a still frame only paints and interacts with
  what is already laid out. A layout projects through `SketchScreen::projector`
  (`caditor_render::PlaneProjection`, the view's projection of the sketch plane worked out once
  per layout, a few multiplications a point), since it projects every dimension's reach, glyph
  site and open end in view. Only a dimension whose reach (its measured geometry and label frame
  origin, and with a placed label the label and the square it swings an arc through) projects
  within `DIMENSION_OFFSET`, its lanes, `ANGLE_RADIUS` and `LABEL_REACH` of the view is laid out
  (`Reach::on_screen`, `ScreenReach::near_view`; a corner that does not project counts as near),
  so obstacle avoidance sees every label that can reach a shown glyph. Selected dimensions of the
  sketch, the keyboard-highlighted one, the one dragged, the one edited inline and the one waiting
  for its field are laid out first and wherever they are, so Move to and Focus::Dimension still
  reach an off-screen label. Any other dimension is tested before it is laid out: one whose reach
  spans under `COLLAPSED_BELOW` points on screen while measuring something in the sketch
  (`ScreenReach::collapses`; a zero-length dimension never collapses) and that neither conflicts
  nor is redundant becomes a collapsed mark, a `COLLAPSED_RADIUS` dot at the reach's centre, one
  per `COLLAPSED_SPACING` cell, painted under the labels, hoverable, clickable to select (which
  lays it out) and double-clickable to edit; one whose label neighbourhood (the reach grown by its
  largest label offset and the label's cached size, `ScreenReach::label_neighbourhood`) lies in
  cells labels laid out before it already cover by `FULL_CELL_SHARE` is left out without being
  laid out (`annotation_layout::crowded`), as glyphs are; and a label they cover by more than
  `MOST_LABEL_COVER` of its area once laid out is left out with its dimension
  (`annotation_layout::mostly_covered`), so a pile of labels zoomed out thins to the readable ones
  without laying most of them out. Collapsed marks are spaced through a flat grid of taken cells
  over the view (`ScreenCells`). Drawing export (`caditor-file`'s `export/annotation.rs`)
  places every dimension and shares none of this thinning or collapsing. The ignored
  `frame_costs_on_a_large_sketch_and_a_large_model` (`viewport.rs`) times annotations idle and
  with the camera moving, zoomed out over `large_sketch` (5,000 dimensions, 3,000 glyph
  constraints) and zoomed in on a corner, and `sketch_card_costs_on_a_large_sketch`
  (`feature_tree.rs`) its sketch card.
- Labels and glyphs are `Pickable::SketchConstraint`: hover highlights the entities, click selects
  (Shift or Ctrl toggles), Delete removes selected constraints and entities in one transaction.
  They are painted but not interactive while a drawing tool is active. Only the marks within
  `POINTER_REACH` of the pointer (hover, press and interaction positions), the label being dragged
  and a mark holding focus are handed to egui each frame (`PointerReach`); egui hit-tests against
  the previous frame's widgets, so the reach is wide enough that a mark is registered before the
  press that lands on it. Keyboard users reach marks through the highlight commands, never Tab.
- Double-clicking a label opens an inline `commit_field` with the value selected, as does a new
  dimension or any `Focus::Dimension` of the edited sketch, which the app takes from the panels
  and hands to the viewport, waiting until the dimension can be drawn.
