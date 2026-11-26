# Rectangle

{command:sketch.rectangle} draws a rectangle with square corners. It has three ways of drawing:

- {command:sketch.rectangle.corners}: click two opposite corners. The sides stay horizontal and
  vertical, as in the next way.
- {command:sketch.rectangle.center}: click the centre, then a corner; it stays centred on that
  point.
- {command:sketch.rectangle.three_points}: click the two ends of one side, then a point setting
  the width. The rectangle can lie at any angle.

Pressing the tool's key again while it is active steps to the next way; the menu of the arrow beside the
ribbon button lists them all. The way last used is remembered while caditor runs.

## Its size

- While you drag, the sides move in round steps and settle on rounder sizes nearby, so 25 by 25 is
  reached without typing; see [snapping](snapping).
- With the first corner or the centre placed, type the size as `40 x 25`, width then height, and
  press Enter. One length alone, such as `25`, draws a square. The size is the whole rectangle's
  in both ways, the rectangle goes to the side of the first point the pointer is on (a negative
  value sends it to the other side), and both values are kept as dimensions. `x, y` with a comma
  still places the corner by its position.
- Otherwise add a dimension to two sides; see [dimensions](dimensions).
