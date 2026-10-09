# Ellipse and elliptical arc

{command:sketch.ellipse} draws an ellipse: click its centre, then the end of its major axis, then
a point setting the minor radius.

{command:sketch.elliptical_arc} draws part of an ellipse: the centre, the end of the major axis
and a point setting the minor radius and where the arc starts, then its end. It runs the way the
pointer sweeps; {command:sketch.reverse_arc} sends it the other way.

Both share the Curve button with the spline. They have no key of their own; use the button, the
Sketch menu or {command:palette}.

A [Smart dimension](dimensions) on an ellipse adds its major and minor radii. Horizontal and
Vertical level its major axis. Ellipses cut other curves when trimming, but are not trimmed
themselves.
