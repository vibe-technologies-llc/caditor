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
- Extrude and revolve sketch regions into new bodies or add to, remove from or intersect
  existing ones; fillet and chamfer edges; shell bodies with open faces; datum planes and axes.
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
- Earlier versions thin out as they age: the ten newest are always kept, then one per hour for
  a day, one per day for a month, one per week for a year and one per month beyond, so a model
  file no longer grows with every save.
- Models larger than 256 MiB keep saving and keep their earlier versions and unsaved changes,
  instead of saving once and then failing.
- Models whose names are as long as the file system allows can be saved, and saving in a folder
  shared with another computer never removes that computer's save in progress.
- Opening, importing or saving over a device, pipe or file larger than 2 GiB is refused with
  the reason instead of hanging or closing caditor.
- Import of DXF drawings into sketches and of STEP models as bodies; export of bodies as STEP,
  STL and 3MF.
- Drawings whose blocks repeat heavy splines are refused with the reason before they exhaust
  memory, and a large array of a block that draws nothing, such as labels, no longer refuses the
  drawing.
- Importing a drawing with nothing caditor can draw, such as only text or hatches, says so and
  lists what was left out.
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

### Interface

- One environment without workbenches or modes: tools are offered where they fit the selection
  and the edited sketch.
- A command palette, customisable keyboard shortcuts, and keyboard operation of everything,
  including views, highlighting in the viewport and typed points.
- Dark, light and high-contrast themes, interface sizes from 75% to 200%, and a choice of
  micrometres, millimetres, centimetres or metres.
- Screen readers can read and operate the panels, menus and dialogs through AT-SPI.
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
