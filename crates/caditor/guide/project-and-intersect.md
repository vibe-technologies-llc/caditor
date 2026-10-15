# Project and intersect

These tools bring model geometry into the edited sketch. What they add is drawn in purple, cannot
be dragged and follows its source whenever the model changes.

## Project

{command:sketch.project} copies an edge, a corner or every edge round a face of a body, a curve
or point of another sketch, a datum axis or point, the X, Y or Z axis or the origin onto this
sketch. While it is active the axes, the origin and the datum axes and points made before the
sketch are shown to click. An axis comes in as a line reaching across the bodies, or as a point
where it runs straight through the sketch. Hover shows what a click would add. Projected curves
make regions and take constraints like drawn ones.

Shift-click a curve or point of another sketch to project the whole of that sketch in one step:
every curve, construction ones included, and every point no curve uses, with one Undo for all of
them. {command:sketch.project_sketch} does the same from the keyboard on the highlighted curve, or
on the one other sketch chosen in the [feature tree](feature-tree). Anything already projected is
left out, and the message says how much.

A projected edge follows its body when an earlier sketch is changed. If [Trim](trim-and-extend) or
split cuts the edge's curve in two and the parts still run on end to end, the projection becomes the
whole of them, with its points and the [constraints](constraints) and
[dimensions](dimensions) on them kept. If a gap opens between the parts, the sketch stops and
says the edge was cut into parts: delete the projection and project the parts you want, rather
than one of them being taken without notice.

You rarely need it before drawing: a drawing tool that [snaps](snapping) to a body's corner or
edge projects it for you.

## Intersect

{command:sketch.intersect}, also in the small menu in the corner of the Project button, draws where
the model crosses the sketch plane. Click a face for the cut
through that face, or Shift-click it for the cut through the whole body
({command:sketch.intersect_body} from the keyboard). Click a datum or principal plane for the line
where it crosses the sketch, drawn as construction.

Only geometry made before the sketch in the [feature tree](feature-tree) can be used.
