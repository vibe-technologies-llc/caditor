---
paths:
  - "crates/caditor/src/editing.rs"
  - "crates/caditor/src/sketch_drag.rs"
  - "crates/caditor/src/drag_solver.rs"
  - "crates/caditor/src/drawing.rs"
  - "crates/caditor/src/trimming.rs"
  - "crates/caditor/src/shapes.rs"
  - "crates/caditor/src/snap.rs"
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

- A context, not a mode: `editing.rs` holds the edited sketch and active `Tool`, changed by
  `Action::Editing` commands that `app::perform` routes after the UI pass; it ends by itself when
  the sketch disappears or another document is opened. The viewport watches it.
- Entering faces the camera to the plane and fits it, moves the grid there, makes the origin and
  axes pickable, dims other features (unpickable) and keeps only that sketch selected.
- The edited sketch, its origin and axes and the drawing preview go on `Layer::Front` (`render.md`),
  so a body never hides or z-fights with them: a sketch on a face, inside or behind a body draws
  and picks over it with its hover and selection highlights, while the dimmed body stays in view
  for context. Other sketches, bodies and datum geometry stay on their usual layers.
- Clicks and primary drags select with the Select tool; a drawing tool (`Tool::draws`) draws, and
  Trim and Extend (`Tool::modifies`) act on the curve under the pointer. Escape backs out one step
  at a time: a drag or trim path in progress, plane choice, shape in progress, keyboard highlight,
  tool, selection, then editing.

## Dragging and box selection

- `sketch_drag.rs` and `drag_solver.rs`.

- With Select, a primary drag starting on a non-reference entity of the edited sketch (the press's
  hover, once its pick arrived) is a `Grab`: a point, line, arc or spline moves its points by the
  pointer's offset; a circle alone changes radius; a selected grabbed entity moves all selected
  entities' points.
- Each frame the pointer moves sends `Action::Drag(DragCommand::Move)` with the `Drag`s;
  `SketchDragging` (in `Display`, owned by `Model`) gives the newest to its own worker thread, which
  drops older unstarted ones and solves each from the previous solution of the same drag (the first
  from the displayed sketch) with `solve_from` and its memo, and hands it back to be shown
  (`DisplayedSketches::show_dragged`, which every consumer of a displayed sketch sees).
- `Finish` (release) waits for the last solution and commits one `settle_sketch` transaction `Drag
  <what>` (nothing if unchanged; a notice if unsolved); the dragged shape stays until an evaluation
  of that revision reaches the sketch, so it never jumps back. Escape (`Cancel`), another edit or
  document, or a drag begun on an older revision drops it and shows the sketch as it was.
- A primary drag elsewhere draws a box: left to right a window taking the points and curves whose
  outline lies inside, right to left a crossing box taking what it touches; it replaces the
  selection (Shift or Ctrl adds to it); a point is left out when a curve it belongs to was taken.
- Move selected sketch geometry (M) opens the typed-point field (`app-input.md`) as "Move to": the
  first selected point goes there (`@` offsets from it), the rest follows, committed like a drag
  (`Move <what>`), solved like a drag. Select all sketch geometry (Ctrl+A) selects what a box around
  everything would. The sketch ribbon's Move and Select all buttons trigger these commands, which
  the viewport carries out; both share their availability through `Moving::offered` and
  `sketch_drag::can_select_all`/`select_all`.

## Drawing tools

- `drawing.rs` (point, line, rectangle, circle, arc, three-point arc, tangent arc, slot, polygon,
  spline), geometry in `shapes.rs`; clicked points, hover and arc sweep are viewport UI state; each
  finished shape is one transaction (`Draw line`, …, `Draw hexagon` for a polygon), settled first
  like any sketch transaction; inferred constraints are checked with `Sketch::check_constraint` on a
  shadow sketch and skipped if refused.
- A shape with no size gets a `Refusal` (notice for a click, field error for a typed point): flat
  rectangle; line, circle or arc ending where it starts; slot without width; three collinear points;
  tangent arc without a curve to continue, or ending on its line.
- Keys: P, L, R, C, A, T (tangent arc), U (slot), G (polygon), S (spline), Alt+A (three-point arc).
- Lines chain, each joined to the last end by `Coincident`, until Escape, a click on the last point,
  or a line ending on the chain's first point (or the point it snapped to), closing the outline.
  Splines finish on Enter or a click on the last control point.
- Arc: runs the way the pointer swept round its centre; a typed end goes the shorter way
  (counter-clockwise at exactly half a turn, `Sweep::aim`); Reverse the arc (X, a sketch command
  offered once centre and start exist, named in the prompt) sends either the other way. Its end is
  projected onto the circle through its start, keeping its snap only if the target lies on that
  circle (a typed end on a point off it lands free).
- Three-point arc: start, through, end; the third takes only points, kept on it.
- Tangent arc: starts on a point ending a line, arc or spline (the newest if several, named in the
  snap label as "Continue …"), leaves along that curve's direction with a `Tangent`, chains like
  lines, each arc tangent to the one before.
- Slot: two centres (the second aligns like a line end) and a never-snapping width point; two
  semicircular arcs and two lines joined by `Coincident`, lines `Tangent` to both arcs, arcs
  `Equal`; five degrees of freedom.
- Polygon: centre and first corner; sides joined corner to corner, corners on a construction circle
  about the centre, sides `Equal` to the first; four degrees of freedom. 3 to 64 sides, six at
  first, kept until another document opens; `]` (another side) and `[` (one fewer) change it,
  offered only with the Polygon tool and named in its prompt.

## Construction geometry

- Construction (Q, `Command::Construction`) makes the selected curves construction in one
  transaction (`sketch_tools::ConstructionChange`), ordinary when all already are; with no curve
  selected it switches drawing (`ActiveSketch::construction`): new curves are construction, points
  stay points (the button shows pressed). Construction curves and the preview while drawing them are
  dashed (`scene::curve_lines`), coloured by constraint state like any curve. Its button leads the
  ribbon's Modify group, after the drawing tools.

## Trim and extend

- `trimming.rs` holds the tool's UI state; the geometry and constraint rules are the sketch's
  (`sketch.md`). Each frame the curve under the pointer (nearest within the curve snap distance on
  screen, splines included so they can be refused in words) is aimed at through the displayed
  sketch: Trim's piece, drawn over the curve in a red preview with its cut points and its cutters
  highlighted; Extend's reach from the nearer end, previewed with a marker on its target, which is
  highlighted. The aim's words ("Trim Line 3 back to Line 5 and Circle 2", "Extend Line 3 to
  Arc 4", or why not) stand where hover descriptions go; the prompt names the keys.
- A click acts at once, without waiting for a GPU pick: one transaction `Trim <curve>` or
  `Extend <curve>` built by `reshape_sketch` from a working copy (the definition with the displayed
  positions) to the trimmed copy, settled first like any sketch transaction; a refusal is a notice
  (`Trim: …`, `Extend: …`). The tool stays active.
- With Trim, a primary drag is a trim path: every piece of a line, circle or arc the pointer's path
  crosses is collected (shown red) and on release trimmed in turn, in crossing order, as one
  transaction (`Trim 3 pieces`); each is found again by its middle, on whichever curve now carries
  it after earlier splits. Escape drops the path. Extend ignores drags.
- Keyboard: Trim (K) and Extend (J) are sketch tools with palette commands and compact buttons after
  Construction in the ribbon's Modify group. While either is active, N and Shift+N step through its
  targets instead of the scene's pickables (every piece of every line, circle and arc; every line
  or arc end that can reach something), and Enter or Space acts on the highlighted target, or on
  the one under the pointer.

## Snapping

- `snap.rs` (UI thread, displayed sketch, screen space). Priority: the shape's pending point; points
  and the origin within 8 logical pixels; lines, circles, arcs and axes within 6, projecting onto
  the curve. A snapped point gets a `Coincident` with its target.
- `Accept` keeps every snap shown a constraint that already holds: a circle's rim takes points only
  (a rim on a curve would add no constraint); an arc's end, points on its circle and where it
  crosses lines, circles, arcs and the axes.
- A line end (and a slot's second centre) within 3° or 6 pixels of a direction from its start
  takes it exactly, with that constraint (`Snap::Aligned`). Horizontal and vertical win whenever
  either applies, so a line within 3° of level is never inferred parallel to; otherwise parallel
  or perpendicular to one of the six lines of the edited sketch (construction lines included)
  nearest on screen to the pointer or the start (`drawing::guides`), the smallest offset winning,
  a tie going to the nearer line. A line is never inferred parallel to a line through its start
  point (that only continues it straight), though perpendicular to it, as to the chain's last
  line, is offered. The label names the reference ("Parallel to Line 3").
- Snapping to geometry wins over a direction, which joins it only where compatible
  (`Snap::AlignedOn`, labelled "On Line 2, vertical"): a point snap keeps a direction that already
  holds exactly (relative 1e-9), never moving the point; a curve snap moves to where the ray of an
  applying direction crosses that curve, if within 12 pixels of the pointer. Typed points never
  align.
- The preview, snap marker and snap label are drawn from this state; the snap target and a
  direction's reference line (`Drawing::snap_entities`) replace the GPU hover while a drawing tool
  is active, so the reference is highlighted.

## Constraint tools

- `sketch_tools.rs` turns the selection into candidates checked by `Sketch::check_constraint`;
  `sketch_toolbar.rs` offers them as compact buttons, geometric then dimensional
  (`app-look.md`), and commands (Shift+letter by default), disabled with what to select.
- Parallel, equal, collinear, concentric and horizontal or vertical points chain every selected item
  to the first in one transaction. Fix locks selected points where shown. Symmetric takes two points
  or lines and the mirror: the one axis selected, else whichever of the three mirrors the other two
  best, pairing line ends by the reflection.
- Dimensions start at the displayed geometry's measured value. Every sketch transaction first
  settles the sketch to the last result, when up to date (`Model::settled_sketch`).
- The UI thread never solves (drags solve on their own worker): constraint states, degrees of
  freedom and redundancies come from the last evaluation (`sketch_status.rs`, `scene.rs`).

## Displayed sketches

- `Model::displayed_sketch` (`display.rs`) is the definition with solved positions wherever the last
  result has the same entity: the solved sketch itself when every entity matches, else a merged
  copy. `DisplayedSketches`, owned by `Model` with the body meshes in `Display` and handed to the
  scene through `Sources`, works it out once per sketch and keeps it, with the bounds of its points
  that fitting and the reference size use, until the document or evaluation changes (`forget`), so a
  frame compares, copies and polylines no sketch for these.

## Annotations

- Drawn with the egui painter (`annotations.rs`, placement in `annotation_layout.rs`) from the
  displayed geometry through the current view; offsets and sizes in screen points, no stored
  positions.
- Distances between points are parallel dimension lines with extension lines; point–line distances
  are perpendicular (a line–line spacing from the second line's middle); point–circle distances run
  along the radius; horizontal and vertical distances are level or upright dimension lines beyond
  the farther point; angles are arcs at the lines' intersection (between their closest ends when
  nearly parallel); radii are leaders with an `R` prefix; diameters span the circle with an `Ø`
  prefix. Dimensions sit away from the sketch's centre.
- Other constraints are glyphs stacked beside each constrained entity on the opposite side
  (horizontal or vertical points, concentric, collinear and symmetric on each item, midpoint and fix
  on the point), painted as shapes or as letters the default fonts carry.
- Labels show the expression in the document's naming, followed by its value when not a literal;
  conflicting and redundant constraints take the error and warning colours. Labels and glyphs are
  `Pickable::SketchConstraint`: hover highlights the entities, click selects (Shift/Ctrl toggles),
  Delete removes selected constraints and entities in one transaction; painted but not interactive
  while a drawing tool is active.
- Double-clicking a label opens an inline `commit_field` with the value selected, as does a new
  dimension or any `Focus::Dimension` of the edited sketch, which the app takes from the panels and
  hands to the viewport, waiting until the dimension can be drawn.
