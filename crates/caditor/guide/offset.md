# Offset

{command:sketch.offset} copies a chain of lines, arcs or circles at a distance to one side. Select
the chain first, or click a curve with the tool to take the chain it belongs to. Move the pointer
to the side you want: the preview follows the side and the distance.

Click to place it, or type the distance and press Enter. A negative distance goes to the other
side, so select, choose Offset, type the distance and press Enter without the mouse.

The copy stays at that distance as the original changes. Smooth joints stay smooth and corners
between lines stay sharp. A rounded corner that the distance shrinks to nothing, such as the inside
of a rounded rectangle offset by more than its corner radius, is left out and its neighbours meet
in a sharp corner.

A notch or tab narrower than twice the distance closes up, as when an outline is offset by a wall
thickness: its curves are left out and the curves either side meet where their copies cross, or
become one straight line when they lie along one line, as the two halves of an edge a notch
interrupts do. When the two halves lie at different heights, a short line square to them steps
from one copy to the other at the distance beyond the higher half's corner, and follows it. Only what the distance uses up is left out, so the walls of a notch with a narrow
floor meet in a point and a narrow slot inside a wider notch closes onto the wider notch's floor.
A closed notch stays closed if the original is later widened; offset again to open it.

## Both sides

Offset has three ways, chosen by running {command:sketch.offset} again while it is active, from
the corner menu of its button or from the Sketch menu's ways of drawing:
{command:sketch.offset.one_side}, {command:sketch.offset.both_round} and
{command:sketch.offset.both_flat}. The two ways to both sides put a copy at the distance on each
side, so the pointer or the typed value sets the distance on each side, not the width.

An open chain, such as a path a slot follows or a wall drawn by its middle, is closed into one
outline round it: round ends are half circles about the chain's end points, flat ends are lines
square to the chain through them. Both copies and the ends follow the chain as it changes, so the
slot or wall is ready to extrude in one step. The outline is ordinary geometry even when the chain
is construction, since it is drawn to be extruded. A closed chain gets one copy inside and one
outside.

Ends that would run into each other, or into the copies, are refused in words. The ends of an
ellipse's spline copies cannot be closed, so an elliptical arc is offset to one side at a time.

An ellipse or elliptical arc offsets on its own, as a fit-point spline that keeps within a
hair of the distance. It does not follow the ellipse afterwards, since no constraint can hold a
spline at a distance from an ellipse along its whole length; the words beside the pointer say so.
