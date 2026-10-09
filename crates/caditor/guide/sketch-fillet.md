# Sketch fillet and chamfer

{command:sketch.fillet} rounds the corner where two lines or arcs meet with an arc tangent to
both. Click near the corner, then move the pointer to set the radius and click, or type the radius
and press Enter.

{command:sketch.chamfer} cuts the corner with a straight line instead. Type:

- `5` for the same distance along both curves,
- `5, 3` for a distance along each, in order,
- `5 < 45` for a distance along the first curve and the angle of the cut.

The values are kept as typed, parameters included. These work on sketch corners; to round the edges
of a body use [Fillet and Chamfer](fillet-and-chamfer).
