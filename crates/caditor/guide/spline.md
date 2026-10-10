# Spline

{command:sketch.spline} draws a smooth curve. It has four ways of drawing; press the tool's key
again to go to the next, or choose one from the Curve button's menu:

- {command:sketch.spline.control}: the curve follows the points you click without passing through
  the ones in the middle.
- {command:sketch.spline.fit}: the curve passes through every point you click, travelling
  between them in step with how far apart they are, so points bunched close together and far
  apart do not make it loop or overshoot.
- {command:sketch.spline.closed_control} and {command:sketch.spline.closed_fit}: the same, closed
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

Fit-point splines from files saved before caditor spaced them by their points keep the shape
they were saved with, which can overshoot between unevenly spaced points. Select them and run
{command:sketch.respace_fit_splines} to space them by their points like a new one; the points stay
where they are and the curve between them changes. Undo puts the old shape back.

A spline drawn by control points shows its control polygon dashed while its sketch is edited;
{command:view.toggle_control_polygons} hides or shows it. A spline through fit points shows none,
since the fit points are its handles.

Splines cut other curves when trimming, but cannot themselves be trimmed or extended. For a curve
shaped by a single value, see [Conic](conic).
