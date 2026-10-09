# Constraints

Constraints hold geometry in relation to other geometry. Select the items, then choose a
constraint from the Constrain group of the sketch ribbon, Sketch › Constraints or its Shift key.
A disabled button says on hover what to select.

## The constraints

- **Coincident** joins two points, or puts points on a curve.
- **Midpoint** puts a point at the middle of a line or arc.
- **Concentric** gives circles, arcs and ellipses one centre.
- **Collinear** puts lines on one straight line.
- **Fix** locks points where they are.
- **Horizontal** and **Vertical** level lines, or line up two points.
- **Parallel** and **Perpendicular** relate lines; a line perpendicular to a circle runs through
  its centre.
- **Tangent** makes a line and a curve, or two curves, touch smoothly.
- **Curvature** makes a spline run on from the curve at its end with no kink in its bending.
- **Equal** gives lines the same length, or circles and arcs the same radius.
- **Symmetric** mirrors two points, lines, circles or arcs about a line or a point.

With more than two items, chaining constraints relate every item to the first one selected.

## When constraints disagree

A constraint already in the sketch is not added twice, and one that contradicts another is refused,
naming it. When a new constraint would make the sketch impossible, caditor checks it first and
refuses it, naming the constraints it conflicts with. The [sketch's status](sketch-status) shows
conflicting and redundant constraints.

Constraint marks sit beside the geometry; hover one to light what it holds, click it to select it
and Delete removes it. {command:sketch.toggle_constraint_active} switches the selected constraints
off without deleting them.

See also [dimensions](dimensions) and [constraining automatically](automatic-constraints).
