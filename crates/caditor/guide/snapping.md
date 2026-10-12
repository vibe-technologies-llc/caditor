# Snapping

While drawing, the pointer snaps to what is near it, and a label beside it names the snap. A
snapped point keeps the relation as a constraint, so a line ending on a circle stays on it.

## What it snaps to

- Points, the sketch origin and the middle of lines, arcs and elliptical arcs.
- The centre of a closed outline such as a rectangle, slot or regular polygon. The whole outline
  lights up, and on an outline of an even number of sides so do the two corners that will hold the
  point there.
- Where two curves cross, and the right, top, left and bottom of circles and arcs.
- The far end of an ellipse's major axis and the ends of its minor axis; the point stays at the
  end however the ellipse is turned.
- Any line, circle, arc, spline or axis, and the extension of a line past its ends.
- The point where a line from its start would touch a circle or arc.
- The corners of bodies made before the sketch, the middles of their straight and round edges, the
  centres of round edges and any point on an edge, as they fall on the sketch plane. The label
  names the body, such as "Corner of Base". The corner or edge is
  [projected](project-and-intersect) into the sketch with what you draw, and the point stays on
  it; one undo takes both away.

## Directions and tracking

A line near horizontal or vertical, or near parallel or perpendicular to a nearby line, takes that
direction exactly. What you hover is remembered for a while: points, the middle of a line or arc,
where two curves cross, the centre of an outline, the right, top, left and bottom of a circle and
the ends of an ellipse's axes. Moving level with one, or straight above or below it, shows a dashed
guide and lines the new point up with it; from the middle of a line a guide also runs square to the
line, along its perpendicular bisector. Where two guides cross, the point lands on the crossing.

A point lined up with a sketch point keeps that as a constraint, so it stays level with it. Lined
up with anything else, such as a line's middle, it lands exactly there but nothing holds it, and the
label ends "not kept there"; add a dimension or constraint if it should stay.

## Controlling it

- Hold Ctrl to place a point exactly under the pointer, with no snapping or guides.
- Hold Alt to step along the grid: the point lands on the nearest grid crossing unless something to
  snap to is nearer.
- {command:view.toggle_snapping} turns snapping off for good; Alt still snaps, and the hint
  below the prompt says snapping is off.
- {command:view.toggle_grid_snapping} puts free points on the grid's crossings.

Dragging geometry with Select snaps the same way, bodies included: a point dragged onto a body's
corner or edge projects it and stays on it, in the same undoable change.
