# Trim and extend

{command:sketch.trim} cuts away a piece of a line, circle or arc: click the piece, and it is cut
back to the curves crossing it. Drag across several pieces to trim them all at once. The piece
under the pointer shows in red first.

{command:sketch.extend} lengthens a line or arc: click near its end, and it grows to the next curve
in its way.

Every other curve cuts, construction lines and the sketch's axes included. Constraints that still
make sense are kept on the pieces, and the new ends are joined to what cut them, so the outline
stays closed.

Splines and ellipses cannot be trimmed or extended, and neither can projected geometry; the tool
says why when you aim at one.

From the keyboard, {command:view.highlight_next} steps through the pieces and
{command:view.activate_highlighted} trims the highlighted one.
