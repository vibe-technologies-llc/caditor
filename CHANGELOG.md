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
- References to faces and edges survive edits to earlier sketches and features. A feature that
  fails is marked in the tree with the reason and what to do, and everything that does not
  depend on it keeps working.
- A body whose first feature fails stays in view, tinted, in its last good shape, and edits that
  leave geometry unchanged or only rename something do not rebuild what follows them.
- Named parameters and unit-aware expressions in every field.

### Files

- The `.caditor` model format: compressed, checksummed, and partially readable when damaged.
- Every change can be undone, a recovery journal restores unsaved work after a crash, and each
  file keeps the versions its saves replaced so they can be restored later.
- Unsaved work is flushed to the recovery journal when caditor is stopped by logout or a signal,
  and a status bar pill says when it cannot be protected while caditor keeps retrying.
- Save As adds `.caditor` to names such as "Bracket v1.2", asks before replacing a file the
  dialog did not name, and refuses a model open in another window.
- Import of DXF drawings into sketches and of STEP models as bodies; export of bodies as STEP,
  STL and 3MF.

### Interface

- One environment without workbenches or modes: tools are offered where they fit the selection
  and the edited sketch.
- A command palette, customisable keyboard shortcuts, and keyboard operation of everything,
  including views, highlighting in the viewport and typed points.
- Dark, light and high-contrast themes, interface sizes from 75% to 200%, and a choice of
  millimetres, centimetres or metres.
- A welcome dialog with three sample models and first-run tips.
- Help › About caditor shows the version; `caditor --version` prints it.

### Installation

- Release archives for 64-bit Linux with an installer that adds the menu entry, icon and the
  `.caditor` file type.
