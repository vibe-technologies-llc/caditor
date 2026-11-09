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

{command:sketch.project_outline} brings in the outer outline of a body as seen square to the sketch:
the highlighted body with the Project tool, or the one body chosen in the
[feature tree](feature-tree). It is what you would trace round the body's shadow on the sketch,
so a cylinder lying along the plane gives its two side lines and two ends, a sphere one circle and
a torus seen along its axis its rim, while a hole seen right through stays out. Outlines on curved
surfaces that have no simple shape come in as splines. Each curve follows the body as it changes. A large body's outline is worked out in the
background: the status bar shows it and **Cancel** (or Esc, or {command:view.cancel_outline}) stops it,
adding nothing. If the model changes meanwhile, nothing is added and the message asks you to project
it again.
If the outline comes to have another number of curves from the same edge or face, or a curve
changes kind, the sketch stops and says so: delete those curves and project the outline again.

A projected edge follows its body when an earlier sketch is changed. If [Trim](trim-and-extend) or
split cuts the edge's curve in two and the parts still run on end to end, the projection becomes the
whole of them, with its points and the [constraints](constraints) and
[dimensions](dimensions) on them kept. If a gap opens between the parts, the sketch stops and
says the edge was cut into parts: delete the projection and project the parts you want, rather
than one of them being taken without notice.

## Keeping a projection as ordinary curves

Select projected or intersected curves and choose {command:sketch.unlink_projection}, from the
Sketch menu, the right-click menu or the palette. They stay where they are, with their points,
constraints and dimensions, but no longer follow their source: they can be dragged, trimmed and
dimensioned like curves you drew, and a later change to the body or sketch they came from leaves
them alone. Undo links them again.

You rarely need it before drawing: a drawing tool that [snaps](snapping) to a body's corner or
edge projects it for you.

## Intersect

{command:sketch.intersect}, also in the menu of the arrow beside the Project button, draws where
the model crosses the sketch plane. Click a face for the cut
through that face, or Shift-click it for the cut through the whole body
({command:sketch.intersect_body} from the keyboard). Click a datum or principal plane for the line
where it crosses the sketch, drawn as construction.

Only geometry made before the sketch in the [feature tree](feature-tree) can be used.
