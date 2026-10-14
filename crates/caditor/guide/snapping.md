# Snapping

While drawing, the pointer snaps to what is near it, and a label beside it names the snap. A
snapped point keeps the relation as a constraint, so a line ending on a circle stays on it.

## What it snaps to

- Points, the sketch origin and the middle of lines, arcs and elliptical arcs.
- The centre of a closed outline such as a rectangle, slot, triangle or regular polygon. The whole
  outline lights up, and on an outline of an even number of sides so do the two corners that will
  hold the point there. On an outline of an odd number of sides the centre is the middle of
  its sides' middles: for straight sides that is the middle of its corners, and an arc among them
  pulls it toward its bulge. The point stays there as the outline changes.
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

Dragging a circle's rim to a sketch point, such as a corner of a rectangle, snaps it there: the
circle passes through the point and stays on it when you let go.

## Directions and tracking

A line near horizontal or vertical, or near parallel or perpendicular to a nearby line, takes that
direction exactly. What you hover is remembered for a while: points, the middle of a line or arc,
where two curves cross, the centre of an outline, the right, top, left and bottom of a circle and
the ends of an ellipse's axes. Moving level with one, or straight above or below it, shows a dashed
guide and lines the new point up with it; from the middle of a line a guide also runs square to the
line, along its perpendicular bisector. Where two guides cross, the point lands on the crossing.

A lined-up point keeps that as a constraint, whatever it was lined up with: level with a point or a
line's middle, straight above a crossing, on the perpendicular bisector, along or square to a line
through the top of a circle. When the line, crossing, outline, circle or ellipse moves, the point
moves with it. Its mark beside the point is H or V for level or upright, a slanted stroke through a
ring for along or square to a line, and a triangle round a dot for the centre of an outline.

## Controlling it

- Hold Ctrl to place a point exactly under the pointer, with no snapping or guides.
- Hold Alt to step along the grid: the point lands on the nearest grid crossing unless something to
  snap to is nearer, a body's corner or edge included. Alt also reaches further for it than the
  pointer alone does.
- {command:view.toggle_snapping} turns snapping off for good; Alt still snaps, and the hint
  below the prompt says snapping is off.
- {command:view.toggle_grid_snapping} puts free points on the grid's crossings. It snaps to the
  grid you see at the current zoom, the spacing named at the bottom of the view such as "Grid 10 mm",
  and to nothing finer: the faint lines between are left out until you zoom in far enough to
  draw them in full. Alt steps along the same grid.

- Preferences › Sketching › Constraints drawing infers turns off single kinds while snapping stays
  on: horizontal, vertical, parallel, perpendicular, tangent, coincident with a point, on a curve,
  at a midpoint, at the centre of an outline, at an intersection, at a quadrant or axis end, and
  the tracks from points you passed over. A kind that is off is neither offered nor kept as a
  constraint while drawing, and the pointer snaps to whatever else is near. Dragging still snaps to
  everything.

Dragging geometry with Select snaps the same way, bodies included, with Alt too: a point dragged
onto a body's corner or edge projects it and stays on it, in the same undoable change. Dragging a
curve or a selection, whichever of its points comes nearest a sketch point, middle, centre,
quadrant or axis end lands there and is joined to it, not only the point nearest where you pressed.
