# Project and intersect

These tools bring model geometry into the edited sketch. What they add is drawn in purple, cannot
be dragged and follows its source whenever the model changes.

## Project

{command:sketch.project} copies an edge, a corner or every edge round a face of a body, or a curve
or point of another sketch, onto this sketch. Hover shows what a click would add. Projected curves
make regions and take constraints like drawn ones.

You rarely need it before drawing: a drawing tool that [snaps](snapping) to a body's corner or
edge projects it for you.

## Intersect

{command:sketch.intersect}, also in the small menu in the corner of the Project button, draws where
the model crosses the sketch plane. Click a face for the cut
through that face, or Shift-click it for the cut through the whole body
({command:sketch.intersect_body} from the keyboard). Click a datum or principal plane for the line
where it crosses the sketch, drawn as construction.

Only geometry made before the sketch in the [feature tree](feature-tree) can be used.
