# Trim and extend

{command:sketch.trim} cuts away a piece of any curve: a line, circle, arc, ellipse, elliptical
arc, spline or conic. Click the piece, and it is cut back to the curves crossing it. Drag across several pieces to trim them all at once. The piece
under the pointer shows in red first.

{command:sketch.extend} lengthens a line, arc, elliptical arc or open spline: click near its
end, and it grows to the next curve in its way, an elliptical arc going on round its ellipse and a
spline carrying on along its own curve the way its end is heading.

Every other curve cuts, construction lines included. The sketch's reference axes only cut a curve when no other sketch curve crosses it, so an overhang trims back to its real crossing in one click. A curve lying along another, a line on a line, an arc on the same circle or a spline drawn along a line, an arc or another spline (two offsets of one curve by the same distance), cuts it once where the stretch they share begins and once where it ends; a closed curve lying all along another is not cut by it. Constraints that still
make sense are kept on the pieces, and the new ends are joined to what cut them, so the outline
stays closed. A point that was the midpoint of a line or arc stays on it, as a point on the curve,
when it still lies on what is left (a trimmed line's far piece takes it when it lies there), and is
let go when the cut took its part. Equal lengths with a line you shorten are let go, since the
line is no longer as long as the other.

A trimmed ellipse becomes an elliptical arc, and the two pieces of an elliptical arc trimmed in
the middle keep one ellipse. A trimmed spline keeps exactly the shape it had: what is left is a
spline of its own with control points to drag, and a closed spline cut twice opens. A conic stays a
conic, with the rho of the piece that is left. A spline through fit points is also kept exactly,
and what is left of it is a spline drawn from control points. A fit point on the piece left that
carries a constraint, a tangent handle or another curve stays where it was, held on the piece, so
its constraints and the curves ending on it still hold; a tangent handle becomes a tangent between
its line and the piece. Only a closed spline cut across the place where its loop starts and ends
is redrawn through its points, so its shape near the cut can shift slightly. A spline's own
tangent handles never cut it. Extending
a spline only goes as far as it keeps heading the way its end points: an end that would curl back
before meeting anything is not extended. Whole ellipses, circles and closed splines cannot be
extended, having no ends, and neither can projected geometry; the tool says why when you aim at
one. An end that is shared with another curve, fixed, or held by a dimension, a symmetry or another
constraint is not moved: the tool names the constraint, and deleting it lets the end extend. To trim or extend a projected curve, first keep it as an ordinary curve
([Project](project-and-intersect)).

From the keyboard, {command:view.highlight_next} steps through the pieces and
{command:view.activate_highlighted} trims the highlighted one.
