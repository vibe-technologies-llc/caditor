# How constrained a sketch is

Every sketch shows how much of it can still move. The pill beside its name in the sketch ribbon,
and in its row in the tree, reads one of:

- **N degrees of freedom left**: that many independent ways the geometry can still move or
  change size. Add [constraints](constraints) or [dimensions](dimensions) until none are left.
- **Fully constrained**: nothing can move.
- **Conflicting constraints**: some constraints cannot all hold. Click the pill to go to the
  problem in the sketch's card, then remove or change one of them.
- **The sketch has an error** when it cannot be solved for another reason; click the pill too.

A second pill counts redundant constraints, which repeat what others already say; they do no harm
but are best removed.
- **Not solved yet**, while the model is recomputing.

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

Drag a point to see how it moves. {command:sketch.select_free} selects everything still free.
Open ends of an outline that does not close are ringed in yellow and counted in the ribbon, so a
profile that will not make a region is seen while drawing.

[Constrain automatically](automatic-constraints) can add the missing constraints for you.
