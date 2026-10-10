# Ellipse and elliptical arc

{command:sketch.ellipse} draws an ellipse: click its centre, then the end of its major axis, then
a point setting the minor radius.

{command:sketch.elliptical_arc} draws part of an ellipse: the centre, the end of the major axis
and a point setting the minor radius and where the arc starts, then its end. It runs the way the
pointer sweeps; {command:sketch.reverse_arc} sends it the other way.

Both share the Curve button with the spline. They have no key of their own; use the button, the
Sketch menu or {command:palette}.

A [Smart dimension](dimensions) on an ellipse adds its major and minor radii, and with a point,
line, circle or arc the distance between them. Horizontal and Vertical level its major axis.
Equal gives ellipses the same radii, Midpoint puts a point at the middle of an elliptical arc,
and Tangent makes a line, circle or arc touch an ellipse, at the point they share or anywhere
along it. Drawing snaps to the far end of the major axis, the ends of the minor axis, which a
point keeps however the ellipse turns, and the middle of an elliptical arc.

[Trim and extend](trim-and-extend) open an ellipse into an elliptical arc, shorten or split an
elliptical arc and carry its ends round the ellipse to the next curve;
[splitting and breaking](sketch-editing) work on elliptical arcs, a
[sketch fillet or chamfer](sketch-fillet) rounds or cuts a corner where an elliptical arc meets
a line, arc or another elliptical arc, and [Mirror](sketch-mirror) and the
[patterns](sketch-patterns) copy ellipses with their radii held.
[Projecting](project-and-intersect) an ellipse onto a sketch facing the same way, or the opposite
way, brings it in as an ellipse; seen at an angle it comes in as a spline.

Ellipses cannot be offset: the curve at one distance from an ellipse is no ellipse, so nothing
would keep the copy at that distance. A tangent or distance between an ellipse and a spline or
another ellipse is not offered.
