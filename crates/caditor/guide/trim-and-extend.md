# Trim and extend

{command:sketch.trim} cuts away a piece of any curve: a line, circle, arc, ellipse, elliptical
arc, spline or conic. Click the piece, and it is cut back to the curves crossing it. Drag across several pieces to trim them all at once. The piece
under the pointer shows in red first.

{command:sketch.extend} lengthens a line, arc, elliptical arc or open spline: click near its
end, and it grows to the next curve in its way, an elliptical arc going on round its ellipse and a
spline carrying on along its own curve the way its end is heading.

Every other curve cuts, construction lines and the sketch's axes included. Constraints that still
make sense are kept on the pieces, and the new ends are joined to what cut them, so the outline
stays closed.

A trimmed ellipse becomes an elliptical arc, and the two pieces of an elliptical arc trimmed in
the middle keep one ellipse. A trimmed spline keeps exactly the shape it had: what is left is a
spline of its own with control points to drag, and a closed spline cut twice opens. A conic stays a
conic, with the rho of the piece that is left. A spline through fit points keeps the fit points on
the piece left and passes through the cut, so its shape near the cut can shift slightly. Extending
a spline only goes as far as it keeps heading the way its end points: an end that would curl back
before meeting anything is not extended. Whole ellipses, circles and closed splines cannot be
extended, having no ends, and neither can projected geometry; the tool says why when you aim at
one. To trim or extend a projected curve, first keep it as an ordinary curve
([Project](project-and-intersect)).

From the keyboard, {command:view.highlight_next} steps through the pieces and
{command:view.activate_highlighted} trims the highlighted one.
