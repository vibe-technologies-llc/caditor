# Constraints

Constraints hold geometry in relation to other geometry. Select the items, then choose a
constraint from the Constrain group of the sketch ribbon, Sketch › Constraints or its Shift key.
A disabled button says on hover what to select.

## The constraints

- **Coincident** joins two points, or puts points on a curve.
- **Midpoint** puts a point at the middle of a line, arc or elliptical arc.
- **Concentric** gives circles, arcs and ellipses one centre.
- **Collinear** puts lines on one straight line.
- **Fix** locks points where they are.
- **Horizontal** and **Vertical** level lines, or line up two points. Lined up while drawing, a
  point can also be held level with or straight above the middle of a line or arc, where two
  curves cross, the centre of an outline, the top, bottom or sides of a circle, or an end of an
  ellipse's axis ([snapping](snapping)).
- **Parallel through** and **Perpendicular through** hold a point on the line through such a
  place running along another line or square to it; the perpendicular bisector of a line is one.
  **Centre** holds a point at the middle of an outline's corners, or of its sides' middles when
  an arc is among them. These come from drawing with
  the guides, not from a button.
- **Parallel** and **Perpendicular** relate lines; a line perpendicular to a circle runs through
  its centre.
- **Tangent** makes a line and a curve, or two curves, touch smoothly; a circle, arc, spline or
  another ellipse touches an ellipse where the two share a point, or anywhere along both when they
  share none.
- **Curvature** makes a spline run on from the curve at its end with no kink in its bending.
- **Equal** gives lines the same length, circles and arcs the same radius, or ellipses the same
  radii.
- **Symmetric** mirrors two points, lines, circles or arcs about a line or a point.

With more than two items, chaining constraints relate every item to the first one selected.

A corner, the centre of a round edge or an edge of a body made before the sketch can be selected
with them, where it falls on the sketch plane: clicking it with Select picks it, Shift adds it,
and from the keyboard {command:view.highlight_next} reaches it and
{command:view.activate_highlighted} adds it.
The constraint [projects](project-and-intersect) it into the sketch in the same undoable change.

## When constraints disagree

A constraint already in the sketch is not added twice, and one that contradicts another is refused,
naming it. When a new constraint would make the sketch impossible, caditor checks it first and
refuses it, naming the constraints it conflicts with. The [sketch's status](sketch-status) shows
conflicting and redundant constraints.

Constraint marks sit beside the geometry; hover one to light what it holds, click it to select it
and Delete removes it. Where the view is crowded some marks are left out, as are the marks of a
line only a few pixels long, until you zoom in or select the line. {command:sketch.toggle_constraint_active} switches the selected constraints
off without deleting them.

See also [dimensions](dimensions) and [constraining automatically](automatic-constraints).
