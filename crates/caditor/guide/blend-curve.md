# Blend curve

{command:sketch.blend_curve} joins the ends of two lines, arcs or splines with a smooth spline, and
keeps the joint smooth as they change. Besides its key, it is in the small menu in the corner of the
Offset button. It has two ways:

- {command:sketch.blend_curve.tangent}: the spline leaves each end along its curve's direction.
- {command:sketch.blend_curve.curvature}: it also bends as tightly as each curve does there, so
  neither joint shows a kink in its curvature.

Click near the end of the first curve, then near the end of the second; the spline is previewed
before the second click. Clicking a chosen end again lets it go, and Escape lets go of it.

See [Spline](spline).
