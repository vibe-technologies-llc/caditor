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

## Tangent handles

{command:sketch.add_tangent_handle} gives a fit point of an open spline drawn through fit points a
handle: a construction line from the point along the curve. The curve leaves the point in the
handle's direction, and the handle's length is how strongly it pulls: a longer handle carries the
direction further before the curve turns away. Select the fit point (with the spline too when the
point is shared by another curve) and run the command from the Modify menu, the right-click menu or
the palette. The handle starts along the curve as it was, so adding it changes the shape very
little.

Drag the handle's free end to turn and stretch it, or constrain it like any line: make it
**Horizontal** or **Vertical**, give it an **Angle** to another line or a length to fix the pull,
make it **Parallel** to an edge, all of which the curve then follows. The handle is shown with a
tangent mark; select that mark and use Disable to let the curve go back to passing the point
freely, or delete the handle line to remove it. A point takes one handle; the spline's two ends
take them too, which sets the direction the spline starts or finishes in. Undo takes a handle away.

Mirroring or patterning a spline together with its handles gives the copies handles of their own,
so they keep the same shape. Trimming keeps the handles on fit points still on the kept piece.
Closed splines, splines drawn by control points and conics take no handles.

## Adding control points

{command:sketch.insert_spline_point} gives a spline drawn from control points one more control
point without changing its shape. Select the spline, and a point if you want the new control point
near a place along it; with no point, it goes into the spline's longest stretch. Several splines
selected together each get one, and the command is also in the right-click menu on a spline. Add
points one at a time to refine a stretch as far as you like; Undo takes the point away.

To keep the curve exactly, the two control points beside the new one move a little along the
control polygon. If either has a constraint or another curve on it, the command stops and names it,
or, when you gave no place, picks a stretch that leaves them alone. Closed splines take points the
same way, all the way round. A spline of two control points is straight, and its new point lies on
that line where you asked; a spline of three becomes one of four on exactly the same curve, its
middle point moving towards one end and the new point appearing towards the other.

Splines through fit points and conics cannot take a point without changing shape, and the command
says which. Taking a control point away while keeping the shape is not possible exactly.

A spline drawn by control points shows its control polygon dashed while its sketch is edited;
{command:view.toggle_control_polygons} hides or shows it. A spline through fit points shows none,
since the fit points are its handles.

Splines cut other curves and can be trimmed, extended, split and broken themselves, keeping their
shape; see [Trim and extend](trim-and-extend) and
[splitting and breaking](sketch-editing). For a curve shaped by a single value, see
[Conic](conic).
