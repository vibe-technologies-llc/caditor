# Datum planes, axes and points

Datums are reference geometry: planes to sketch on, axes to revolve or pattern about, points to
measure from. They have no volume and are not exported.

- {command:model.plane} starts from the selected plane or flat face, offset from it or turned
  about a selected axis. Three points make a plane through them, two planes one midway, an axis and
  a point one through both, and a curved face with a point one touching the face.
- {command:model.axis} runs along a selected edge, round face or sketch line, where two planes
  meet, through two points, or square to a plane through a point.
- {command:model.point} sits at the selected point, corner, centre of a round edge or face, or
  where an axis meets a plane, with offsets.

With nothing selected, a plane starts on the XY plane and a point at the origin. A selection
holding nothing they can use, such as a sketch region, a constraint or a centre of mass, is
refused with the choices that would work.

A datum follows what it was made from. Its panel shows what it is defined by and its offsets and
angles, previewed in the view as you type them; an open offset plane also has an arrow to drag
its offset, and a turned plane an arrow along its turn that drags its angle, turning the plane onto a corner
or an edge's middle under the pointer. {command:model.datum_use_selected} defines it again from the selection. Double-click a
datum in the view to open it.

A point at the middle of an edge, or at a distance along one, and a plane square to an edge keep to
the whole edge when [Trim](trim-and-extend) or split later cuts its sketch curve in two: while the
pieces still run on end to end, the middle and the distance are taken along all of them. When a gap
opens between the pieces there is no one edge left to follow, so the datum stops and says so, and
you choose the piece it should follow, rather than it moving to one piece without notice.

See also [coordinate systems](coordinate-systems).
