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

A datum follows what it was made from. Its panel shows what it is defined by and its offsets and
angles; {command:model.datum_use_selected} defines it again from the selection. Double-click a
datum in the view to open it.

See also [coordinate systems](coordinate-systems).
