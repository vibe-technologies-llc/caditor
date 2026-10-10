# Selecting

Click a face, edge, vertex, sketch curve or datum to select it. Shift-click or Ctrl-click adds or
removes items. Click empty space to clear the selection; with nothing selected, a click on empty
space or Escape also lets go of the rows chosen in the [feature tree](feature-tree). Hover shows
what a click would take, with its name.

## The context menu

Right-click in the view for what you can do with what lies under the pointer. A right-click on
something not selected selects it alone first; on something selected, or on empty space, the
selection stays as it is. Over the model it offers editing the feature, then the tools that take
what is selected: New sketch on a flat face or plane, Fillet and Chamfer on edges, Extrude, Hole,
Offset face, Shell, Split face and Thread on faces, and Move, Copy, Mirror, Pattern, Split and
Scale for the body, under **Body** unless the whole body is selected. Then come Suppress, Rename
and Delete for the feature that made the selection, hiding, looking at a face, fitting,
measuring, growing the selection and listing everything under the pointer. Over empty space it
offers the views, Show everything, pasting features and the selection filter; while a feature is
open, reversing its direction, cancelling its changes or finishing it. A right-drag still turns
the view and never opens the menu. {command:view.context_menu} opens it from the keyboard at the
highlighted item or the selection, and each entry shows its keys. Tools that do not fit the
selection are left out; other entries that cannot run now are dimmed and say why when you point
at them.

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
- {command:select.feature_faces} adds every face made by the feature that made the selected faces,
  such as a boss as a whole.
- {command:select.loop} replaces an edge and a face it bounds with the edges of that face's loop
  holding the edge.
- {command:select.similar} adds every face or edge of the shown bodies of the same kind and size as
  the selected ones: the walls of holes of one radius, round faces of one radius, flat faces of one
  area, edges of one length, and says what it matched.
- {command:select.inverse} selects the faces, edges or vertices of the kind selected, or of the
  selection filter, that are not selected now.

## Selection sets

{command:select.save_set} keeps the selected faces, edges and bodies in the model under a name.
{command:select.sets} selects, replaces, renames or deletes them, and a set can feed any tool, such
as a [fillet](fillet-and-chamfer).

## From the keyboard

{command:view.highlight_next} and {command:view.highlight_previous} step a highlight through what is
in the view, and {command:view.activate_highlighted} selects it. See
[working from the keyboard](keyboard).
