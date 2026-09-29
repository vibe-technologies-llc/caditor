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
- Angles at a corner of a chain of lines are measured inside the corner, tangents where a line
  meets an arc count fully towards a constrained sketch, and a constraint that would shrink a
  line to nothing is reported as a conflict.
- Conflicting constraints in large sketches are found several times faster and never hold up
  the recompute for long; the constraints named are always a smallest set that cannot hold
  together, and when a conflict cannot be pinned down the message names the geometry that
  failed to solve and offers to go to its newest constraint.
- Extrude and revolve sketch regions into new bodies or add to, remove from or intersect
  existing ones; fillet and chamfer edges; shell bodies with open faces; datum planes and axes.
- A fillet or chamfer whose edge was split by an earlier cut lists it as that edge in pieces
  instead of as an edge that is no longer there, and a shell lists a split opening the same way.
- Shells whose walls are thicker than a rounded edge or corner leave that rounding out of the
  cavity, corners where four or more faces meet become a short ridge inside it, and a thickness
  that would shrink a wall past nothing names the edge where it happens.
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

### Interface

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
- At large interface sizes and in narrow windows the menu bar, status bar and parameter table
  wrap or shrink instead of overlapping, long notices wrap instead of being cut off, and the
  navigation hint wraps clear of the axis triad.
- Edges, sketch curves, points and the grid keep their size at larger interface sizes and on
  high-resolution screens, and picking them is as forgiving as at 100%.
- A body or drawing too large for the graphics card's buffers is drawn in parts, or as much of
  it as fits, instead of leaving the whole window blank.
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

### Installation

- Release archives for 64-bit Linux with an installer that adds the menu entry, icon and the
  `.caditor` file type.
- The installer works in folders whose names hold `&`, `%` or `|`, and an install that fails
  part way removes what it had copied.
