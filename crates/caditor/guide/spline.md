# Spline

{command:sketch.spline} draws a smooth curve shaped by control points. Click each control point in
turn; the curve follows them without passing through the ones in the middle. Press Enter, or click
the last control point again, to finish.

The length and angle of the leg from the last control point show as you draw, and each control
point can be levelled with the one before, like a line.

## Shaping a spline

Drag its control points with Select. Constrain them like any points, and use **Tangent** or
**Curvature** to make a spline run on smoothly from the curve at its end; see
[constraints](constraints). A [Blend curve](blend-curve) joins two curves with a spline that does
this for you.

Splines cut other curves when trimming, but cannot themselves be trimmed or extended.
