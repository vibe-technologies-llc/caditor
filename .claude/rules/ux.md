# User experience

UX is the top product priority: a feature that works but is confusing, modal or fragile is not
finished. Each rule below avoids a failure mode FreeCAD is known for.

## Model stability

- References to geometry survive upstream edits. Never identify a face, edge or entity by position
  or creation order: use stable IDs and, for generated topology, names derived from the feature
  that made it. Editing an early sketch never silently rewires later features.
- A failing feature fails alone, marked in the tree with a plain-language reason and a fix, while
  every feature not depending on it keeps its last good result. Recompute never leaves the model
  half-updated.

## One environment

- No workbenches or mode switches to reach a tool. The context (the selection, the edited sketch)
  filters what is offered.
- One navigation and selection model, done right, rather than several competing styles.

## Feedback

- Sketches always show how constrained they are: the remaining degrees of freedom as a count and
  by colour. Conflicting or redundant constraints are named with the entities involved.
- Errors say what happened, which feature or entity is involved and what to do next, never solver
  or kernel internals (exception text, raw indices).
- Every numeric field accepts units and expressions referring to named parameters, which are
  first-class, not a separate spreadsheet.

## Responsiveness

- The UI thread never blocks on recompute, meshing, file I/O or solving. Long work runs in the
  background, shows progress and can be cancelled.
- Undo and redo cover every document change, parameter edits and reordering included.

## Accessibility

- Everything works from the keyboard: every action is a palette command, and the viewport has
  keyboard views, camera moves, highlighting and typed coordinates. An action only a mouse can
  reach is not finished.
- Every panel text colour meets 4.5:1 against its background in both themes (7:1 for body text in
  high contrast), tested in `appearance.rs`. Colours come from `appearance::tokens` or the current
  visuals, never fixed values; text on the dark 3D view uses the tested colours and backdrop of
  `canvas.rs`.
- High contrast reaches the 3D view: its scene palette holds every line and point to 3:1 against
  the canvas and dimmed bodies, edges and highlighted faces to 3:1 on a body (`scene_palette.rs`),
  and no state is told by colour alone (`app-sketching.md`).
- The interface scales from 75% to 200%; panels and toolbars wrap rather than clip.

## Look

- Inter for text, Phosphor icons from `icons.rs` (never Unicode or emoji glyphs), theme tokens for
  colour. Panels and dialogs use the shared widgets in `widgets.rs` (property grids, cards,
  callouts, pills, tool buttons, the titled dialog with its primary action rightmost), not their
  own spacing and colours.
- Status reads the same everywhere: an icon and a tone (error, warning, success, info), problems
  as a callout saying what to do next.

## Units

- SI only: lengths in micrometres, millimetres, centimetres and metres, angles in degrees or
  radians. Imperial units are never offered in the UI, preferences or new features.
- Foreign files in imperial units are still read, converted to millimetres, and the import report
  says so.
