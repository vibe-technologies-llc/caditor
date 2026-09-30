# Changelog

Every release of caditor is listed here, newest first. Each section says what a user of that
version gains, loses or has to do differently; the git history has the details.

## [Unreleased]

The first release of caditor.

### Modelling

- Sketches on principal planes, datum planes and flat faces of bodies, with points, lines,
  rectangles, circles, arcs and splines, snapping, inferred constraints and typed coordinates.
- A constraint solver with coincident, horizontal, vertical, parallel, perpendicular, tangent and
  equal constraints and distance, angle and radius dimensions. Sketches always show their
  remaining degrees of freedom and colour what is fully constrained; conflicting and redundant
  constraints are named.
- Midpoint, concentric, collinear, fix and symmetric constraints, horizontal and vertical for two
  points, and horizontal distance, vertical distance and diameter dimensions; a distance can also
  hold a point away from a circle or two lines apart. Parallel, equal, collinear, concentric,
  horizontal and vertical apply to every selected item at once, and fix locks every point of the
  selected curves.
- A point can be put on a spline, and a line, circle or arc made tangent to one, either where
  they touch along it or where the spline ends on them.
- Construction geometry: Q (or the Construction button) turns the selected curves into dashed
  construction curves that guide a sketch, take constraints and can be revolved about, but never
  split or add to its regions; with nothing selected it switches drawing to construction curves.
- New drawing tools: three-point arcs (Alt+A), tangent arcs that continue smoothly from the end
  of a line, arc or spline and chain one after another (T), slots (U), and regular polygons of
  3 to 64 sides (G, with ] and [ for more or fewer sides), each drawn with the constraints that
  keep its shape.
- More ways to draw shapes: rectangles from their centre or from three points at any angle,
  circles through the two ends of a diameter or through three points, polygons from their centre
  and the middle of a side or from one whole side, and slots from their centre or curved along an
  arc. Pressing a shape's key again (R, C, G or U) switches to its next way, the corner menu on
  its ribbon button and the palette offer each one, the prompt names the one in use, and each
  shape remembers the last way used. Each is drawn with the constraints that keep its shape.
- Trim (K) and Extend (J) in the sketch ribbon's Modify group. Trim cuts away the piece of a
  line, circle or arc between the curves crossing it, or every piece a drag passes over: a circle
  opens into an arc, a line or arc cut in the middle splits in two, and a piece nothing crosses is
  deleted. Extend lengthens a line or arc from the clicked end to the next curve in its way.
  Constraints stay with the pieces kept, new ends stay on the curves that cut them, each change
  is one undoable step, and both work from the keyboard with N to highlight and Enter to act.
- A typed point can be a length and an angle, such as `25 < 30` or `@25 < 30` from the last
  point, or a length alone, which goes from the last point toward the pointer. Typing a chain of
  line ends no longer starts a new chain after the first line.
- A line or slot drawn nearly parallel or perpendicular to a nearby line of the sketch snaps to
  it exactly and stays that way, with the label naming the line ("Parallel to Line 3") and the
  line highlighted; level and upright still come first. A line ending on a curve keeps its
  direction where it crosses it, so a vertical line can end exactly on a slanted one.
- Sketch geometry can be dragged with the Select tool: a point, line, arc or spline follows the
  pointer, a circle grows or shrinks, and a selection moves together, each as far as its
  constraints allow and as one undoable change; Escape puts it back. M moves the selection to a
  typed position or by a typed offset instead.
- The sketch being edited always draws over bodies, so its curves and points stay visible and
  clickable on a face of a body or inside or behind it, while the dimmed body stays in view.
- Dragging across empty space in a sketch selects with a box: left to right takes what lies
  inside it, right to left what it touches, and Shift or Ctrl adds to the selection. Ctrl+A
  selects all of the sketch's geometry.
- Angles at a corner of a chain of lines are measured inside the corner, tangents where a line
  meets an arc count fully towards a constrained sketch, and a constraint that would shrink a
  line to nothing is reported as a conflict.
- Conflicting constraints in large sketches are found several times faster and never hold up
  the recompute for long; the constraints named are always a smallest set that cannot hold
  together, and when a conflict cannot be pinned down the message names the geometry that
  failed to solve and offers to go to its newest constraint.
- Extrude and revolve sketch regions into new bodies or add to, remove from or intersect
  existing ones; fillet and chamfer edges; shell bodies with open faces; datum planes and axes.
- Linear and circular patterns (toolbar, Model menu and palette) repeat a body along a direction,
  optionally in a second direction too, or around an axis, joining the copies to it. The
  direction or axis can be a principal or datum axis, a straight edge or a round face; count,
  spacing and total angle take expressions, and a full turn spaces copies evenly. Faces of the
  copies keep their references when the count or earlier features change, and a count that is
  not whole, too large (over 100) or copies that cannot be joined fail the pattern alone with the
  reason.
- An extrusion can now end through all of the body it cuts, up to the next face of its body, or
  up to a flat face or plane, a tilted one too, and each side of a two-sided extrusion takes its
  own end; a revolve can turn by two angles, one each way from its sketch. The face or plane is
  selected in the view and taken with the panel's Use selected or the palette's Extrude up to
  selected face or plane; an end that misses the body, lies behind the sketch or whose face is
  gone fails that feature alone with the reason and what to do. Models using them are saved in
  format 3.
- A fillet or chamfer whose edge was split by an earlier cut lists it as that edge in pieces
  instead of as an edge that is no longer there, and a shell lists a split opening the same way.
- Shells whose walls are thicker than a rounded edge or corner leave that rounding out of the
  cavity, corners where four or more faces meet become a short ridge inside it, and a thickness
  that would shrink a wall past nothing names the edge where it happens.
- Opening a face of a hollow inside a body cuts through the wall behind it, and the walls beside
  that face now reach down to the cavity instead of stopping short.
- Adding, removing and intersecting bodies succeeds in cases that used to fail: where a cut
  passes close to or through the pole of a sphere or the tip of a cone, where it only touches a
  round face's seam or crosses a bore exactly there, and where a small face meets a large
  curved one.
- A region whose hole touches its outline at one point can be extruded and revolved, and such a
  hole, or a circle drawn tangent inside another, is left open without choosing regions.
- When the curves of a sketch cannot be divided into regions, the message names the curves
  involved.
- Sketches with thousands of regions, such as a plate with a large grid of holes or a big
  imported drawing, are divided into regions and swept in seconds instead of stalling.
- References to faces and edges survive edits to earlier sketches and features. A feature that
  fails is marked in the tree with the reason and what to do, and everything that does not
  depend on it keeps working.
- A body whose first feature fails stays in view, tinted, in its last good shape, and edits that
  leave geometry unchanged or only rename something do not rebuild what follows them.
- Features can be suppressed and unsuppressed from their menu in the tree, the Model menu or the
  palette, one or several at once (Ctrl+click and Shift+click choose several rows). A suppressed
  feature is struck through and left out as if it were not there; features that use it fail
  saying so, with a button to unsuppress it.
- A rollback bar in the feature tree shows the model as it was at any point: drag it, choose Roll
  back to here on a feature, press Alt+Up or Alt+Down, or Roll to end. Features below it are
  greyed and not computed, new features go in right above it, and where it stands is undoable
  and saved with the model.
- Features can be dragged to a new place in the tree, or across the rollback bar; a place before
  something a feature uses or after something using it is refused, with the reason shown while
  dragging.
- Deleting a feature that others use first lists them and asks whether to delete them too or to
  keep them, where they fail with the reason until the deletion is undone; either way it is one
  undoable step.
- Editing or growing one part of a large sketch solves only that part again, even when the edit
  makes the sketch larger or moves its outermost point.
- Named parameters and unit-aware expressions in every field, with comparisons and `if`,
  `mod`, `hypot`, `exp`, `ln`, `sign`, `clamp`, rounding to a step, units after parentheses
  and names (`(2 + 3) mm`), areas and volumes such as `mm²`, and a clear message for a
  misspelled unit or a computed angle without deg or rad. A number too large to store, such as
  `1e999`, is refused as it is typed instead of being lost on the next save.
- `cbrt`, `log10`, `log2`, `trunc` and the logical `and`, `or` and `not` in expressions; rounding
  to a step no longer falls one step short (`floor(0.3, 0.1)` is 0.3), and chained comparisons,
  decimal commas and `mm2` are explained rather than reported as a missing operator or name.

### Files

- The `.caditor` model format: compressed, checksummed, and partially readable when damaged.
- Models are saved in format 3, which keeps suppressed features and the rollback bar. Models
  saved by earlier versions open as before.
- Every change can be undone, a recovery journal restores unsaved work after a crash, and each
  file keeps the versions its saves replaced so they can be restored later.
- The undo history keeps its memory bounded when large imports are deleted or replaced over and
  over: once it holds about 256 MiB, its oldest steps are dropped.
- Unsaved work is flushed to the recovery journal when caditor is stopped by logout or a signal,
  and a status bar pill says when it cannot be protected while caditor keeps retrying.
- Unsaved work in a model that crashed is offered at the next start even when the model has
  dropped off the recent files, and two windows no longer erase each other's recent files.
- Unsaved changes that cannot be read, such as ones written by a newer version, are kept in a
  file of their own next to the model instead of being overwritten, and opening the model says
  where.
- Save As adds `.caditor` to names such as "Bracket v1.2", asks before replacing a file the
  dialog did not name, and refuses a model open in another window.
- Saving keeps the model file's group, access control lists and other extended attributes.
- Version History shows the date and time each version was saved alongside how long ago, and
  keeps the capitals of the change's name when a version is restored.
- Earlier versions thin out as they age: the ten newest are always kept, then one per hour for
  a day, one per day for a month, one per week for a year and one per month beyond, so a model
  file no longer grows with every save.
- Saving a large model on Btrfs or XFS shares its unchanged earlier versions with the file it
  replaces instead of writing them again, so such saves are faster and take no extra disk space.
- Models larger than 256 MiB keep saving and keep their earlier versions and unsaved changes,
  instead of saving once and then failing.
- Models whose names are as long as the file system allows can be saved, and saving in a folder
  shared with another computer never removes that computer's save in progress.
- Opening, importing or saving over a device, pipe or file larger than 2 GiB is refused with
  the reason instead of hanging or closing caditor.
- Import of DXF drawings into sketches and of STEP models as bodies; export of bodies as STEP,
  STL and 3MF.
- File › Export Image… (Ctrl+Shift+E) saves the 3D view as a PNG image at the view's size or a
  size of your own up to 8192 pixels a side, at 1×, 2× or 4×, over the view's background or a
  transparent one, with the anti-aliasing and shading in use and without highlights, the grid or
  labels. It is written in the background and can be cancelled from the status bar.
- Dropping a model on the window opens it, and dropping drawings or STEP files imports them one
  after another (under X11).
- Large drawings import in a fraction of the memory and time they took, and one whose blocks
  and entities hold more than 16 million values is refused with that reason.
- Drawings whose blocks repeat heavy splines are refused with the reason before they exhaust
  memory, and a large array of a block that draws nothing, such as labels, no longer refuses the
  drawing.
- Importing a drawing with nothing caditor can draw, such as only text or dimensions, says so
  and lists what was left out.
- Drawings bring in the outlines of their hatches, filled solids, traces, 3D faces and
  multilines, so a profile drawn only as a hatch can be extruded; a hatch tied to the lines it
  fills does not repeat them.
- Names in older drawings written in a Central European, Cyrillic, Greek, Turkish, Hebrew,
  Arabic, Baltic, Vietnamese or Thai code page, such as those of missing blocks in the import
  report, read correctly instead of as garbled letters.
- A drawing's spline given by fit points and end tangents leaves and arrives along those
  tangents, as in the program that drew it.
- A drawing's spline through fit points that repeat, or nearly repeat, is imported instead of
  left out.
- A STEP file with a damaged or unknown entry, or one defined twice, still imports what does not
  depend on it, with a note saying how many entries were left out and where.
- STEP assemblies that place parts with transformation operators put them in place, and a part
  whose placement cannot be read is left out with a note instead of landing at the origin.
- A STEP surface model made of several closed shells is imported as one body per shell instead
  of being left out as one that could not be stored.
- A STEP body whose faces could not all be checked for crossing each other is still imported,
  with a note naming the faces, instead of passing the check unnoticed.
- STEP files whose curves are nested many times over, or whose bodies share shells and
  surfaces, import in proportion to their size, and one too intricate to import is refused with
  that reason instead of never finishing.
- Large STEP files are read in less than half the time and with about half the memory they took
  before.
- STEP bodies on a torus whose tube just touches its axis import. The precision a STEP file
  declares is taken into account: small repairs within it are no longer reported, and a body
  whose faces meet only as closely as that precision is refused with a message that says so.
- STEP faces on offsets of planes, cylinders, spheres, tori and cones import as exact surfaces.

### Interface

- caditor draws its own title bar: the menu bar doubles as it, with the model's name (and the
  sketch being edited) in the middle, a details popover with its location, saved state, Save,
  Save As, Version History and Copy file location, and Minimize, Maximize and Close buttons. Drag
  it to move the window, double-click to maximize, right-click for the window menu, drag any edge
  to resize; it keeps working while a dialog is open. F11 enters and leaves full screen. Prefer
  the window manager's title bar? Choose it in Preferences › Appearance or the window menu.
- The sketch bar is redesigned: a header with the sketch's name and its state (with an icon and
  the degrees of freedom left), then Select, Draw, Modify, Constrain and Dimension groups under
  small captions, with the constraints as compact icon buttons that name themselves, their
  shortcut and what to select on hover, Move and Select all buttons beside Construction and
  Delete, and Finish sketch as the highlighted action on the right. It fits one row on a wide
  window and wraps whole groups at large interface sizes.
- One environment without workbenches or modes: tools are offered where they fit the selection
  and the edited sketch.
- A command palette, customisable keyboard shortcuts, and keyboard operation of everything,
  including views, highlighting in the viewport and typed points.
- Editing a feature (E) and finishing it, detaching a sketch or placing it on the selected plane
  or face, taking the selected axis or plane in a revolve or datum, deleting a parameter, going
  to the first failed feature (F8) and dismissing or hiding tips are commands in the palette.
- The shortcut editor finds commands by their keys as well as their names: typing `ctrl+z` or
  just `z` lists what those keys do.
- Shortcuts such as Ctrl+S work while typing in a field: the typed value is committed first, so
  it is what gets saved. Copy, paste, undo and the other editing keys stay with the field.
- Dark, light and high-contrast themes, interface sizes from 75% to 200%, and a choice of
  micrometres, millimetres, centimetres or metres.
- Preferences are grouped into General, Appearance, Navigation and Graphics tabs, switched with
  the mouse, the arrow keys on a tab, or Ctrl+Tab and Ctrl+Page Down from anywhere in the dialog;
  the last tab stays open, and Restore defaults resets only the tab in view.
- Graphics preferences, applied at once: vsync, a frame rate limit (30 to 144 frames a second or
  the display's rate) that saves power while the view moves, anti-aliasing off, 2×, 4× or 8× as
  the graphics card allows, an enhanced shading with sky and fill light, crisper highlights and a
  soft rim that makes curved faces easier to read, and coarse or smooth curves on bodies. Options
  the graphics card cannot do are greyed out with the reason, and the Graphics tab names the
  graphics card, backend and driver, with Copy details for bug reports. The choices are kept when
  the graphics driver resets.
- An orthographic view, switched on and off with O, from the View menu or in Preferences, which
  remember the choice. Zooming, orbiting, panning, fitting and picking work the same way in both
  views.
- Bodies, sketches and datum planes and axes can be hidden and shown again from the feature
  tree, with H for the selection and Alt+H for everything; a sketch is hidden once it is extruded
  or revolved, so it no longer covers the faces it made or takes their clicks.
- The principal planes, axes and origin can be hidden too, all at once or one by one, from the
  top of the feature tree, with H on them in the view or from the palette. The model remembers
  which are hidden, and hidden planes still appear while choosing where to start a sketch.
- A feature opened from the toolbar or the view scrolls its panel into view in the feature tree.
- caditor reopens its window at the size and maximised state it was left in (and, under X11, at
  the same place), with the side panel as wide and its sections open or closed as before.
- At large interface sizes and in narrow windows the menu bar, status bar and parameter table
  wrap or shrink instead of overlapping, long notices wrap instead of being cut off, and the
  navigation hint wraps clear of the axis triad.
- Edges, sketch curves, points and the grid keep their size at larger interface sizes and on
  high-resolution screens, and picking them is as forgiving as at 100%.
- A body or drawing too large for the graphics card's buffers is drawn in parts, or as much of
  it as fits, instead of leaving the whole window blank.
- When the graphics driver resets or the graphics card is lost, caditor opens it again and
  carries on drawing instead of freezing or closing, and no work is lost.
- caditor starts on older and OpenGL-only graphics cards, draws windows wider than 8192 pixels,
  and uses the power-saving graphics card of a laptop rather than waking the discrete one.
- Changed bodies are prepared for display in the background, so the window stays responsive
  while a large body is redrawn after an edit.
- Round bodies look round: cylinders, holes, revolved parts, fillets and other curved faces are
  drawn with far more facets (at most 6° apart, however small the hole) and their outlines follow
  the shading, instead of showing flat facets.
- A large drawing is added to its sketch in the background, so importing thousands of curves no
  longer holds up the window, and an edit made while it is read is kept alongside it.
- Sketches with thousands of curves no longer slow down every frame: how each sketch is shown
  and how far it reaches is worked out once per change instead of several times per frame.
- Large models and sketches stay smooth to orbit, pan and zoom: the view keeps what it has drawn
  and sends it to the graphics card again only when the model, the selection or what is under
  the pointer changes, instead of rebuilding everything every frame, and hovering over a sketch
  of thousands of curves only recolours it.
- Sketch circles, arcs and splines, and the shapes previewed while drawing or trimming, are drawn
  as finely as the zoom needs: small ones with a few segments, large ones and close-ups smooth
  however far you zoom in, and box selection follows the curves as they are drawn.
- Screen readers can read and operate the panels, menus and dialogs through AT-SPI.
- An arc's end snaps only to points on its circle and to where its circle crosses other curves,
  and a circle's rim only to points, so the arc stays where it was drawn and every "On …" label
  becomes a constraint.
- An arc over half a turn can be typed: a typed end goes the shorter way round, and X (Reverse
  the arc) sends the arc, typed or drawn, the other way.
- Screen readers name icon buttons by what they do instead of reading a symbol, read each
  property's caption with its field, and tell the shortcut editor's Add and Reset buttons apart
  by the command they change.
- A welcome dialog with three sample models and first-run tips.
- Help › About caditor shows the version; `caditor --version` prints it.
- Hovering a constraint in the feature tree highlights what it constrains, and clicking it edits
  the sketch with that constraint selected.
- A new or opened model starts with nothing selected and every body chosen for export, so
  commands never act on something picked in the previous model.
- Orbiting stops when looking straight down or up instead of turning the model upside down, and
  levels the horizon of a view that was turned to face a tilted sketch.
- Fit all frames the model rather than the origin planes, and fitting a small part or a small
  selection fills the view however small it is.
- A measure tool (I, the Measure button or View › Measure) reads the selection without changing
  the model: where a vertex or sketch point is, an edge's length, a circle's radius, diameter and
  centre, a face's area, and between two items their shortest distance with its X, Y and Z parts,
  the angle between lines and planes, the gap between parallel planes and the distance between
  axes, drawn in the view as a line labelled with its length. It also gives each body's volume,
  surface area and centroid. Values found numerically or from the display mesh are marked ≈ and
  say how close they are, and every value can be copied. Corners of bodies can now be picked.

### Installation

- Release archives for 64-bit Linux with an installer that adds the menu entry, icon and the
  `.caditor` file type.
- A new logo, an extruded C with the sketch it was drawn from, for the menu entry, `.caditor`
  files, the window icon, caditor's own title bar and the About dialog, installed as a scalable
  icon and at every common size.
- The installer works in folders whose names hold `&`, `%` or `|`, and an install that fails
  part way removes what it had copied.
- On Arch Linux, `makepkg -si` in `packaging/arch` builds and installs caditor from a source
  checkout as a pacman package.
- The installer refuses to run outside an unpacked release archive, saying how to get one,
  instead of reporting an install that copied nothing.
