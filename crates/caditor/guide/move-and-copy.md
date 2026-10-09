# Move and copy bodies

{command:model.move} moves a body; {command:model.copy_body} places a moved copy and keeps the
original. Select a face, edge or vertex of the body, or choose it in the tree, then choose the
command. **Make a copy** in the panel switches between the two.

## The panel

- A distance along each axis and a turn about each.
- **Turn about**: the body's centre, the origin, or an axis you pick, with its angle.
- **Directions**: the world, or a [coordinate system](coordinate-systems), whose axes the moves
  follow.

## In the view

While a move is open, arrows and squares at the body's centre drag it along an axis or in a plane,
and rings turn it. Drags go in round steps; hold Ctrl to drag freely. The values land in the
panel's fields, where they stay editable.

To put a face flush against another, see [Mate](mate).
