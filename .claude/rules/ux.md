# User experience

UX is the top product priority. A feature that works but is confusing, modal or fragile is not
finished. The list below names the failure modes that FreeCAD is known for; each one is a hard
requirement to avoid.

## Model stability

- References to geometry must survive upstream edits. Never identify a face, edge or entity by
  its position or creation order; use stable IDs and, for generated topology, names derived from
  the feature that created it. Editing an early sketch must not silently rewire later features.
- A failing feature fails alone. It is marked in the tree with a plain-language reason and a way
  to fix it, while every feature that does not depend on it keeps its last good result.
  Recompute never leaves the model in a half-updated state.

## One environment

- No workbenches and no mode switching to reach a tool. Every tool is available where it makes
  sense, and the context (the current selection, or the sketch being edited) filters what is
  offered.
- Keep one consistent navigation and selection model. Do not ship several competing navigation
  styles as a stand-in for getting the default right.

## Feedback

- Sketches always show how constrained they are, both as a count of remaining degrees of freedom
  and by colouring what is fully constrained and what is not. Conflicting or redundant constraints
  are named along with the entities involved.
- Error messages say what happened, which feature or entity is involved and what to do next.
  They never expose solver or kernel internals such as OCC exception text or raw indices.
- Every numeric field accepts units and expressions that refer to named parameters. Parameters
  are first-class and do not live in a separate spreadsheet.

## Responsiveness

- The UI thread never blocks on recompute, meshing, file I/O or solving. Long work runs in the
  background, shows progress and can be cancelled.
- Undo and redo cover every change to the document, including parameter edits and feature
  reordering, with no operation outside the history.
