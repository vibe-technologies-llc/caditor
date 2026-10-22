# Move and copy bodies

{command:model.move} moves a body; {command:model.copy_body} places a moved copy and keeps the
original. Select a face, edge or vertex of the body, or choose it in the tree, then choose the
command. **Result** in the panel switches between the two: **Move body** moves the body itself,
**New body** places a moved copy as a body of its own and leaves the original where it is. A copy
that later features use cannot be turned back into a move; the panel says which features use it.

## The panel

- A distance along each axis and a turn about each.
- **Turn about**: the body's centre, the origin, or an axis you pick, with its angle.
- **Directions**: the world, or a [coordinate system](coordinate-systems), whose axes the moves
  follow.

## In the view

While a move is open, arrows and squares at the body's centre drag it along an axis or in a plane,
and rings turn it. Drags go in round steps, an arrow or square stopping on a corner, an edge's
middle or a round edge's centre of another body under the pointer (an arrow only near that
point's level, unless the pointer leaves the arrow's line to reach for it), and an arrow also on a
face its line meets. A ring dragged onto such a point turns the body until the spot you grabbed lines
up with it about the axis; hold Ctrl to drag freely. With
the pointer on a handle, type a value and press Enter to set its field. The values land in the
panel's fields, where they stay editable; a named value keeps its name, and one that follows other
[parameters](parameters) does not drag.

To put a face flush against another, see [Mate](mate).
