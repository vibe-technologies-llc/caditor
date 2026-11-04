# Sketch fillet and chamfer

{command:sketch.fillet} rounds the corner where two lines, arcs, elliptical arcs or splines meet
with an arc tangent to both. Click near the corner, then move the pointer to set the radius and
click, or type the radius and press Enter.

A spline is cut where the arc touches it, keeping exactly the shape it had, as
[Trim](trim-and-extend) does; what is left is a spline drawn from control points. The rounding is
tangent to the spline, and the sharp corner point is kept only when something else holds it (a
dimension, say), and then lies on the other curve alone, since no constraint holds a point on a
spline's continuation. A radius larger than the spline's own bend allows is refused. Splines are
rounded at the corners where their ends meet, or by clicking two curves, as below.

Two lines, arcs, circles, elliptical arcs or splines need not meet first. Click one, then the other: the
corner is where the two cross, or would meet if extended, nearest the clicks, and each is trimmed
or extended to it, keeping the part you clicked; an arc or elliptical arc stopping short is carried
on round its circle or ellipse, and a spline stopping short on along its own curve (the last piece of
it run on, as [Extend](trim-and-extend) does, so it stops where that curve turns away). A circle becomes the arc on the side you clicked, up to the next
place the other curve crosses it, so a lever's outline (two boss circles joined by two lines) takes
one fillet where each line meets a boss, the second fillet on a boss carrying its arc on round to
the other line. Selecting the two curves before
starting the tool does the same, keeping the longer part of a line or arc and the side of a circle
that leaves the blunter corner. Escape lets go of the first curve. An end that would move but is
joined to other geometry or fixed is refused, as is a pair that never meets, a closed spline, and a
whole circle against a spline (make the circle an arc first).

To round several corners at once, select them first: the points at the corners, or the curves
meeting there (select a whole outline to round all its corners). Starting the tool takes every
corner in the selection, says how many it takes and what it left out and why, and one radius rounds
them all in one step that one undo takes back. Clicking more corners before setting the radius adds
them.

A fillet or chamfer shortens the two curves, and constraints on them follow as a trim's do
([Trim and extend](trim-and-extend)): a point that was the midpoint of a shortened line stays on it,
as a point on the line, while it lies on what is left. Two lines meeting at the corner that were
equal in length stay equal when the cut shortens both alike, as a fillet always does and a chamfer
by one distance does; equal lengths with any other line are let go.

The tool remembers the last radius while caditor runs: it shows in the field, and with a corner
chosen Enter rounds it with that radius again, so a row of corners takes a click and Enter each.

{command:sketch.chamfer} cuts the corner with a straight line instead. Its button sits beside
Sketch fillet's in the Modify group of the sketch ribbon. Type:

- `5` for the same distance along both curves,
- `5, 3` for a distance along each, in order,
- `5 < 45` for a distance along the first curve and the angle of the cut, measured from the
  first curve's tangent where the cut meets it, an arc's, an elliptical arc's or a spline's
  included.

The values are kept as typed, parameters included. These work on sketch corners; to round the edges
of a body use [Fillet and Chamfer](fillet-and-chamfer).
