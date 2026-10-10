# How constrained a sketch is

Every sketch shows how much of it can still move. The pill under its name in the sketch ribbon,
and in its row in the tree, reads one of:

- **N degrees of freedom left**: that many independent ways the geometry can still move or
  change size. Add [constraints](constraints) or [dimensions](dimensions) until none are left.
  Click the pill to select what is still free.
- **Fully constrained**: nothing can move.
- **Conflicting constraints**: some constraints cannot all hold. Click the pill to go to the
  problem in the sketch's card, then remove or change one of them.
- **The sketch has an error** when it cannot be solved for another reason; click the pill too.
- **Not solved yet**, while the model is recomputing.
- **Suppressed**: the sketch is switched off in the feature tree and is not solved.
- **Below the rollback bar**: the sketch comes after the rollback bar in the tree, so it is not
  solved until the bar moves past it.

Beside it, counts say what else needs a look. In the sketch ribbon they are small icons with a
number beside the sketch's name, so the ribbon keeps its height; hover one for its words. In the
tree they are written out:

- **Redundant constraints** repeat what others already say. They do no harm but are best removed:
  click the count to select them, then delete them.
- **Open ends** are curve ends joined to nothing, ringed in yellow in the view. Where many lie
  close together, zoomed out, they share one double ring; point at it to see how many ends it
  stands for, and zoom in to tell them apart. An outline with open ends makes no region to
  extrude. Click the count to select them and bring them into view, then join them.
- **Points beyond their curves** are points held on a line or arc that lie past its drawn ends:
  a point on a line is held to the whole line. Extend the curve, or move the point, if that is not
  meant.

Pills and counts that do something when clicked are outlined like buttons; the others only read.

## Colours

- White curves and points can still move.
- Green ones are fully constrained.
- Red marks conflicting constraints and what they hold, orange redundant ones, which repeat what
  other constraints already say.
- Purple geometry is projected from the model and follows it.

With High contrast on, the form tells the same: geometry that can still move is drawn thin with
hollow points, held geometry heavy with solid points, and the marks of conflicting and redundant
constraints are framed.

## Finding what is free

Drag a point to see how it moves. {command:sketch.select_free} selects everything still free,
{command:sketch.select_open_ends} the open ends and {command:sketch.select_redundant} the
redundant constraints.

[Constrain automatically](automatic-constraints) can add the missing constraints for you.
