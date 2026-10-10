# Hole

{command:model.hole} drills holes at the free points and circles of a sketch.

- With one flat face selected, it puts a hole on the face, on solid material away from its edges,
  in a hidden sketch. While it is open, drag the square at its centre to move it on the face, or
  an arrow to move it along one direction; the arrow at the bottom of a blind hole drags its
  depth and the one at its rim its diameter (making the size Custom). The square stops on a round
  edge's centre or an edge's middle under the pointer, and on a corner; hold Ctrl to drag freely.
  With the pointer on a handle, type a value (or x, y for the square) and press Enter. Its panel's **Placed on** row moves it too: **Choose in the view** and click where the
  hole goes on any flat face, or type its **Position X** and **Position Y** on the face. **Edit
  the sketch** under **Sketch** opens the hidden sketch to dimension the point or add more holes.
- With one curved face selected (the round wall of a cylinder, a cone or a sphere), it drills
  square into the face and asks you to click where the hole goes. Drag the square or an arrow at
  its centre to slide it over the face, staying square to it, or use **Choose in the view** on
  **Placed on** to put it somewhere else.
- With a sketch selected or being edited, it drills at each of its loose points and circles.

A new hole starts from the diameter and blind depth last used on one, kept between sessions
(values naming a parameter the model lacks fall back to 6 mm and 10 mm).

The panel sets:

- **Size**: Custom, or a metric screw size with its **Fit** (close, normal, loose, or tapped with
  the thread named, its class, hand and length).
- **Style**: Plain, Counterbore, Countersink or Stepped, with the head sizes.
- **Diameter**, or **Sized by** the sketch's circles.
- **Shape**: Round, or Slot with a length and angle.
- **Depth**: Blind with a depth, Through all, Up to next (the next flat face along each hole's
  axis) or Up to face (a face or plane you select), the last two with an offset past the face; a
  blind round hole can have a 118° drill point.

Choosing a size fills in exact values; typing any size makes the hole Custom again, and the
drills are previewed as you type. {command:model.reverse_direction} drills the other way, like
**Reverse direction**. A tapped hole
shows its thread in the view like a [thread](thread).
