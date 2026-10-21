# User experience

UX is the top product priority: a feature that works but is confusing, modal or fragile is not
finished. Each rule avoids a failure mode FreeCAD is known for.

## Model stability

- Never identify a face, edge or entity by position or creation order: use stable IDs and, for
  generated topology, names derived from the feature that made it. Editing an early sketch never
  silently rewires later features.
- A failing feature fails alone, marked in the tree with a plain-language reason and a fix; features
  not depending on it keep their last good result. Recompute never leaves the model half-updated.

## One environment

- No workbenches or mode switches. The context (selection, edited sketch) filters what is offered.
- One navigation and selection model.

## Feedback

- Sketches always show the remaining degrees of freedom as a count and by colour; conflicting or
  redundant constraints are named with the entities involved.
- Errors say what happened, which feature or entity and what to do next, never solver or kernel
  internals (exception text, raw indices).
- Every numeric field accepts units and expressions referring to named parameters.

## Responsiveness

- The UI thread never blocks on recompute, meshing, file I/O or solving: long work runs in the
  background, shows progress and can be cancelled.
- Undo and redo cover every document change, parameter edits and reordering included.

## Accessibility

- Everything works from the keyboard: every action is a palette command, and the viewport has
  keyboard views, camera moves, highlighting and typed coordinates.
- Every panel text colour meets 4.5:1 against its background in every theme (7:1 for body text in
  high contrast), checked by `appearance::check`, which refuses a loaded theme in words naming the
  failing pair. Colours come from `appearance::tokens` or the current visuals, never fixed values;
  text on the dark 3D view uses the tested colours and backdrop of `canvas.rs`.
- The 3D view takes one of six canvases, dark through grey to white, with the theme or on its own,
  each with its own chrome and scene palette under the same checks. A user-picked grid colour is
  held to the grid's target on the canvas in use (1.5:1, 3:1 in high contrast), and a body colour
  as drawn to 1.5:1: refused in words naming the colour, canvas and ratio, never drawn where the
  canvas would hide it.
- The dimmed body drawn while a sketch is edited holds `DIMMED_BODY_SEEN` (1.5:1) against the canvas
  on every canvas, its edges the same against it and the canvas; in high contrast the edges hold
  3:1 against both and the fill 1.5:1 on the dark canvases only, since lines drawn over it must hold
  3:1 and no body on the light canvases can differ 1.5:1 from them and stay that far from the
  hovered line colours.
- High contrast reaches the 3D view (`scene_palette.rs`): lines and points hold 3:1 against the
  canvas and dimmed bodies; bodies, edges and highlighted faces hold 3:1 on a body of any colour (a
  body too dark is drawn lightened, `ScenePalette::body`).
- No state is told by colour alone (`app-sketching.md`): selected geometry widens twice as much as
  hovered (`selection_widening`), a failed or outdated body's edges are dashed
  (`troubled_edges_dashed`), canvas labels sit on an opaque backdrop holding every label colour to
  7:1 (`canvas::backdrop`), and the move arrows take the palette's axis colours.
- The interface scales from 75% to 200%; panels and toolbars wrap rather than clip.

## Look

- Inter for text, Phosphor icons from `icons.rs` (never Unicode or emoji glyphs), theme tokens for
  colour. Panels and dialogs use the shared widgets in `widgets.rs` (property grids, cards,
  callouts, pills, tool buttons, the titled dialog with its primary action rightmost), not their
  own spacing and colours.
- Status reads the same everywhere: an icon and a tone (error, warning, success, info), problems as
  a callout saying what to do next.

## Units

- SI only: lengths in micrometres, millimetres, centimetres and metres, angles in degrees or
  radians. Imperial units are never offered in the UI, preferences or new features.
- Foreign imperial files are still read, converted to millimetres, and the import report says so.
