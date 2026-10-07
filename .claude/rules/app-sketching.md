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
- Release commits one `settle_sketch` transaction and the dragged shape stays until an evaluation
  of that revision reaches the sketch, so it never jumps back. Escape, another edit or document,
  or a drag begun on an older revision drops it.
- When the worker cannot solve its newest frame (`Polled::blocked`, `Model::drag_blocked`), the
  viewport says `DRAG_BLOCKED` beside the pointer as a polite live region and the geometry stays
  where the last good frame put it. In a sketch whose constraints conflict it says `DRAG_CONFLICT`
  with the conflict named (`Model::sketch_conflict`, the first two constraints and how many
  more), and the notice when the drag ends names them too.
- A primary drag elsewhere draws a box: left to right a window taking what lies inside (curves
  faceted as drawn, `app.md`), right to left a crossing box taking what it touches. It replaces
  the selection (Shift or Ctrl adds); a point is left out when a curve it belongs to was taken.
- Move selected geometry opens the typed-point field (`app-input.md`) as "Move to", committed and
  solved like a drag; button and command share availability (`Moving::offered`).
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
  its end is chosen), a three-point arc or circle (radius), a three-point rectangle (both sides) and
  a slot (length and end diameter). Splines show none.
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
  state like any curve.

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
  cleared after each fillet.
- Offset and Sketch fillet take a typed value in the typed-point field ("Offset by", "Fillet
  radius"): the preview follows the text while it parses, Enter commits the expression as typed
  (parameters included) and an error keeps the field open. A negative offset goes to the other
  side, so the keyboard alone does it: select, Offset, the distance, Enter.
- With any of them active the highlight commands step through that tool's targets instead of the
  scene's pickables (`app-input.md`), and Activate or Enter acts on the highlighted one.

## Snapping

- `snap.rs` runs on the UI thread against the displayed sketch in screen space. Priority: the
  shape's pending point; points and the origin within `POINT_TOLERANCE`; then, only where any snap
  is accepted and within the same tolerance, the middle of a line or arc (`Target::Midpoint`), the
  centre of a closed outline (`Target::Centre`) and the crossing of two of the up to
  `MAX_CROSSING_CURVES` curves nearest the pointer, axes included (`Target::Intersection`; lines,
  circles and arcs, not splines); lines, circles, arcs and axes within `CURVE_TOLERANCE`,
  projecting onto the curve. A snapped point gets a `Coincident` with its target (with both curves
  at a crossing), a `Midpoint` constraint for a middle, or a `Symmetric` about it of two opposite
  corners for a centre.
- An outline has a centre when its lines join end to end into one closed loop (ends within
  `JOINED_CORNER_TOLERANCE` of the sketch's extent, each corner meeting exactly one other line, at
  most `MAX_OUTLINE_LINES` lines) of an even count whose opposite corners all share one midpoint: a
  rectangle, a parallelogram, an even regular polygon. A corner where a third line meets breaks the
  loop (a rectangle with its diagonals snaps to their crossing instead).
- `Accept` keeps every shown snap a constraint that already holds: a circle's rim takes points
  only (a rim on a curve would add no constraint); an arc's end takes points on its circle and
  where it crosses other curves and the axes.
- A line end (and the second point of either straight slot, of a three-point rectangle's first
  side and of a polygon's side) within `ALIGN_ANGLE_DEGREES` or `ALIGN_TOLERANCE` of a direction
  from its start takes it exactly (`Snap::Aligned`). Horizontal and vertical win whenever either
  applies, so a line near level is never inferred parallel to; otherwise parallel or perpendicular
  to one of the `NEARBY_LINES` lines of the edited sketch (construction lines included) nearest on
  screen (`drawing::guides`), the smallest offset winning. A line is never inferred parallel to a
  line through its start point (that only continues it), though perpendicular to it is offered.
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
  `ALIGNED_CROSSING_TOLERANCE`; point snaps never take one. The preview draws each track as a
  dashed guide from its point (`Preview::guides`) and highlights the point.
- An acquired line extends past its ends: after curves, the pointer within `CURVE_TOLERANCE` of its
  infinite carrier snaps to it (`Target::Extension`, a `Coincident` on the line, which the solver
  treats as the infinite line), with a dashed guide from the nearer end; directions and tracks
  join it as they do a curve.
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
  selected, else whichever of the three items mirrors the other two best.
- A candidate the sketch already has (`Sketch::restating`) is left out of a batch and, when
  nothing is left, refused as already in the sketch; one that contradicts a constraint
  (`Sketch::contradicting`) is refused naming it. These checks are structural: the UI thread never
  solves, so a constraint that only fails once solved is reported afterwards as a conflict naming
  its constraints.
- The candidates of every tool are kept in `PanelState::constraint_offers` and worked out again
  only when the selection, the revision, the evaluation, the displayed sketches or the units
  change (`sketch_toolbar::ConstraintOffers`).
- Disable (`Command::ToggleConstraintActive`, `sketch_tools::ActivityChange`, the Edit group of the
  sketch bar and the palette) switches the selected constraints off, or on again when none of them
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
- Constraint states, degrees of freedom and redundancies come from the last evaluation
  (`sketch_status.rs`, `scene.rs`), never from solving on the UI thread (drags solve on their own
  worker).

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
  stored positions. Dimensions sit away from the sketch's centre; other constraints are glyphs
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
  conflicting and redundant constraints take the error and warning colours.
- Labels and glyphs are `Pickable::SketchConstraint`: hover highlights the entities, click selects
  (Shift or Ctrl toggles), Delete removes selected constraints and entities in one transaction.
  They are painted but not interactive while a drawing tool is active.
- Double-clicking a label opens an inline `commit_field` with the value selected, as does a new
  dimension or any `Focus::Dimension` of the edited sketch, which the app takes from the panels
  and hands to the viewport, waiting until the dimension can be drawn.
