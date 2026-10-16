# Spline

{command:sketch.spline} draws a smooth curve. It has four ways of drawing; press the tool's key
again to go to the next, or choose one from the Curve button's menu:

- {command:sketch.spline.fit}: the curve passes through every point you click, travelling
  between them in step with how far apart they are, so points bunched close together and far
  apart do not make it loop or overshoot. This is the way the tool starts in.
- {command:sketch.spline.control}: the curve follows the points you click without passing through
  the ones in the middle.
- {command:sketch.spline.closed_fit} and {command:sketch.spline.closed_control}: the same, closed
  smoothly back to the first point, so the spline bounds a region that extrudes.

Press Enter, or click the last point again, to finish; clicking the first point closes an open
spline. Enter with too few points placed says how many the spline still needs. The length and angle of the leg from the last point show as you draw, and each point can be
levelled with the one before, like a line.

## Shaping a spline

Drag its points with Select. Constrain and dimension them like any points, and use **Tangent** or
**Curvature** to make a spline run on smoothly from the curve at its end; see
[constraints](constraints). A [Blend curve](blend-curve) joins two curves with a spline that does
this for you.

A point held on a closed spline slides all the way round it, past where the spline started.

## Adding control points

{command:sketch.insert_spline_point} gives a spline drawn from control points one more control
point without changing its shape. Select the spline, and a point if you want the new control point
near a place along it; with no point, it goes into the spline's longest stretch. Several splines
selected together each get one, and the command is also in the right-click menu on a spline. Add
points one at a time to refine a stretch as far as you like; Undo takes the point away.

To keep the curve exactly, the two control points beside the new one move a little along the
control polygon. If either has a constraint or another curve on it, the command stops and names it,
or, when you gave no place, picks a stretch that leaves them alone.

Splines through fit points, conics, closed splines and splines of fewer than four points cannot
take a point without changing shape, and the command says which. Taking a control point away while
keeping the shape is not possible exactly.

A spline drawn by control points shows its control polygon dashed while its sketch is edited;
{command:view.toggle_control_polygons} hides or shows it. A spline through fit points shows none,
since the fit points are its handles.

Splines cut other curves and can be trimmed, extended, split and broken themselves, keeping their
shape; see [Trim and extend](trim-and-extend) and
[splitting and breaking](sketch-editing). For a curve shaped by a single value, see
[Conic](conic).
