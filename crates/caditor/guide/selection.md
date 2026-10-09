# Selecting

Click a face, edge, vertex, sketch curve or datum to select it. Shift-click or Ctrl-click adds or
removes items. Click empty space to clear the selection. Hover shows what a click would take, with
its name.

## Boxes, lassos and painting

Drag across empty space to select with a box. Dragging left to right takes what lies wholly inside;
right to left also takes what the box touches. Only what you can see is taken, unless
{command:view.toggle_select_through} is on.

- {command:view.toggle_lasso} draws a freehand outline instead of a box.
- {command:view.toggle_paint_selection} selects faces by painting over them.

## Choosing what to pick

View › Selection filter limits picking to whole bodies, faces, edges, vertices or sketch geometry,
and {command:select.priority} steps through body, face and edge. The status bar names an active
filter; its button clears it. Tools still pick what they need.

When items lie behind each other, hold the mouse button still for half a second, or press
{command:view.list_under_pointer}, to list everything under the pointer and choose from it.

## Growing the selection

- {command:select.all} takes every face, edge or vertex of the shown bodies.
- {command:select.tangent_edges} and {command:select.tangent_faces} add what continues smoothly.
- {command:select.face_edges} replaces faces with the edges around them.
- {command:select.hole} takes every wall of a hole, and {command:select.body} the whole body.

## Selection sets

{command:select.save_set} keeps the selected faces, edges and bodies in the model under a name.
{command:select.sets} selects, replaces, renames or deletes them, and a set can feed any tool, such
as a [fillet](fillet-and-chamfer).

## From the keyboard

{command:view.highlight_next} and {command:view.highlight_previous} step a highlight through what is
in the view, and {command:view.activate_highlighted} selects it. See
[working from the keyboard](keyboard).
