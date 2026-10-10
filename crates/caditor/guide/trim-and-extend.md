# Trim and extend

{command:sketch.trim} cuts away a piece of a line, circle, arc, ellipse or elliptical arc: click
the piece, and it is cut
back to the curves crossing it. Drag across several pieces to trim them all at once. The piece
under the pointer shows in red first.

{command:sketch.extend} lengthens a line, arc or elliptical arc: click near its end, and it grows
to the next curve in its way, an elliptical arc going on round its ellipse.

Every other curve cuts, construction lines and the sketch's axes included. Constraints that still
make sense are kept on the pieces, and the new ends are joined to what cut them, so the outline
stays closed.

A trimmed ellipse becomes an elliptical arc, and the two pieces of an elliptical arc trimmed in
the middle keep one ellipse. Splines cannot be trimmed or extended, whole ellipses and circles
cannot be extended, having no ends, and neither can projected geometry; the tool says why when
you aim at one.

From the keyboard, {command:view.highlight_next} steps through the pieces and
{command:view.activate_highlighted} trims the highlighted one.
