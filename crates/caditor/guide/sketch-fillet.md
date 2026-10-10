# Sketch fillet and chamfer

{command:sketch.fillet} rounds the corner where two lines, arcs or elliptical arcs meet with an
arc tangent to both. Click near the corner, then move the pointer to set the radius and click, or
type the radius and press Enter.

To round several corners at once, select them first: the points at the corners, or the curves
meeting there (select a whole outline to round all its corners). Starting the tool takes every
corner in the selection, says how many it takes and what it left out and why, and one radius rounds
them all in one step that one undo takes back. Clicking more corners before setting the radius adds
them.

The tool remembers the last radius while caditor runs: it shows in the field, and with a corner
chosen Enter rounds it with that radius again, so a row of corners takes a click and Enter each.

{command:sketch.chamfer} cuts the corner with a straight line instead. It is also in the small menu
in the corner of the Sketch fillet button. Type:

- `5` for the same distance along both curves,
- `5, 3` for a distance along each, in order,
- `5 < 45` for a distance along the first curve and the angle of the cut, measured from the
  first curve's tangent where the cut meets it, an arc's or an elliptical arc's included.

The values are kept as typed, parameters included. These work on sketch corners; to round the edges
of a body use [Fillet and Chamfer](fillet-and-chamfer).
