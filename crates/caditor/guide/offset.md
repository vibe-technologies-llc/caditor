# Offset

{command:sketch.offset} copies a chain of lines, arcs or circles at a distance to one side. Select
the chain first, or click a curve with the tool to take the chain it belongs to. Move the pointer
to the side you want: the preview follows the side and the distance.

Click to place it, or type the distance and press Enter. A negative distance goes to the other
side, so select, choose Offset, type the distance and press Enter without the mouse.

The copy stays at that distance as the original changes. Smooth joints stay smooth and corners
between lines stay sharp.

An ellipse or elliptical arc offsets on its own, as a fit-point spline that keeps within a
hair of the distance. It does not follow the ellipse afterwards, since no constraint can hold a
spline at a distance from an ellipse along its whole length; the words beside the pointer say so.
