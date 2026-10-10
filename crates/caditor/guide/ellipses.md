# Ellipse and elliptical arc

{command:sketch.ellipse} draws an ellipse: click its centre, then the end of its major axis, then
a point setting the minor radius.

{command:sketch.elliptical_arc} draws part of an ellipse: the centre, the end of the major axis
and a point setting the minor radius and where the arc starts, then its end. It runs the way the
pointer sweeps; {command:sketch.reverse_arc} sends it the other way.

Both share the Curve button with the spline. They have no key of their own; use the button, the
Sketch menu or {command:palette}.

A [Smart dimension](dimensions) on an ellipse adds its major and minor radii. Horizontal and
Vertical level its major axis. Equal gives ellipses the same radii, Midpoint puts a point at the
middle of an elliptical arc, and Tangent makes a line touch an ellipse, or a circle or arc touch it
where the two share a point. Drawing snaps to the far end of the major axis, the ends of the minor
axis and the middle of an elliptical arc.

[Trim](trim-and-extend) opens an ellipse into an elliptical arc and shortens or splits an
elliptical arc; [splitting and breaking](sketch-editing) work on elliptical arcs, and
[Mirror](sketch-mirror) and the [patterns](sketch-patterns) copy ellipses with their radii held.
Ellipses cannot yet be extended, offset or rounded by a sketch fillet.
