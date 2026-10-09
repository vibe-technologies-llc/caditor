# Changing sketch geometry

## Moving, turning and scaling

- {command:sketch.move} moves the selected geometry: type where to, or `@` and an offset.
- {command:sketch.rotate} turns it by an angle, counter-clockwise, about the one selected loose
  point, else about the middle of the selection.
- {command:sketch.scale} scales it by a factor the same way.

Constraints still hold, so geometry tied to the rest may not go all the way. With one dimension
selected, Move places its label instead. Projected geometry, the origin and the axes never move:
dragging them says so beside the pointer.

## Splitting and breaking

- {command:sketch.split_curve} cuts a line or arc at a selected point on it.
- {command:sketch.break_curves} breaks the selected lines and arcs at every crossing.
- [Trim and Extend](trim-and-extend) cut away or lengthen pieces with a click.

## Copy and paste

{command:sketch.copy}, {command:sketch.cut} and {command:sketch.paste} move geometry with its
constraints between sketches, models and even other caditor windows. Pasting puts the copy under
the pointer and selects it; dimensions keep parameters of the same name in the target model.

## The context menu

Right-click a curve or point for the constraints the selection can take, construction, splitting,
deleting, moving, cut, copy and paste, selecting and Smart dimension; right-click empty space to
paste or finish the sketch. While a shape is half drawn the menu offers
{command:sketch.take_back_point}, {command:sketch.finish_shape} for a chain of lines, tangent arcs
or a spline, {command:sketch.cancel_shape}, reversing an arc and typing an exact value. See
[selecting](selection).

## Selecting

{command:sketch.select_all} selects every curve and point of the sketch, and
{command:sketch.select_free} selects what is not yet held. A double-click on a curve selects the
chain of lines and arcs joined to it.

See also [offset](offset), [mirror](sketch-mirror) and [patterns](sketch-patterns).
