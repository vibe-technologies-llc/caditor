# Line

{command:sketch.line} draws connected lines, one click per corner. Each new line starts where the
last ended, joined to it.

- Press Escape, or click the last point again, to stop.
- Clicking the chain's first point closes the outline and stops.
- Backspace takes back the last segment while it is the newest change.
- Press and drag to draw a single line in one movement.

While drawing, the line snaps to horizontal, vertical, parallel and perpendicular directions and to
nearby geometry; a label names the snap and the length and angle show beside the pointer. Hold
Ctrl to place a point exactly where the pointer is.

Switch to {command:sketch.tangent_arc} mid-chain to carry on with an arc that leaves the last line
smoothly; switching back to Line carries on from the arc's end. See [arcs](arcs).

Type `40 < 30` for a line 40 long at 30 degrees, or `@40, 0` for one 40 to the right. See
[sketches](sketches) for typed points and [snapping](snapping).
