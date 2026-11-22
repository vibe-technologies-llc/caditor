# Move and copy bodies

{command:model.move} moves one or several bodies; {command:model.copy_body} places moved copies and
keeps the originals. Select faces, edges or vertices of the bodies, or choose them in the tree, then
choose the command; with only one body shown, nothing needs selecting. **Result** in the panel
switches between the two: **Move body** moves the bodies themselves, **New body** places each moved
copy as a body of its own and leaves the originals where they are. A copy that later features use
cannot be turned back into a move; the panel says which features use it.

## The panel

- A distance along each axis and a turn about each.
- **Turn about**: the centre of the bodies' box, the origin, or an axis you pick, with its angle.
- **Directions**: the world, or a [coordinate system](coordinate-systems), whose axes the moves
  follow.
- **Bodies**: the bodies moved together, each with a remove button; **Use selected** adds the bodies
  selected now, and **Choose in the view** adds or leaves out each body you click until you press
  Escape.

## In the view

While a move is open, arrows and squares at the bodies' centre drag them along an axis or in a
plane, and rings turn them. Drags go in round steps, an arrow or square stopping on a corner, an
edge's middle or a round edge's centre of another body under the pointer (an arrow only near that
point's level, unless the pointer leaves the arrow's line to reach for it), and an arrow also on a
face its line meets. A ring dragged onto such a point turns the bodies until the spot you grabbed
lines up with it about the axis; hold Ctrl to drag freely. With the pointer on a handle, type a value
and press Enter to set its field. The values land in the panel's fields, where they stay editable; a
named value keeps its name, and one that follows other [parameters](parameters) does not drag.

To put a face flush against another, see [Mate](mate).
