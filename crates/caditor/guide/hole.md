# Hole

{command:model.hole} drills holes at the free points and circles of a sketch.

- With one flat face selected, it puts a hole on the face, on solid material away from its edges,
  in a hidden sketch; open the sketch to move or dimension the point.
- With a sketch selected or being edited, it drills at each of its loose points and circles.

The panel sets:

- **Size**: Custom, or a metric screw size with its **Fit** (close, normal, loose, or tapped with
  the thread named, its class, hand and length).
- **Style**: Plain, Counterbore, Countersink or Stepped, with the head sizes.
- **Diameter**, or **Sized by** the sketch's circles.
- **Shape**: Round, or Slot with a length and angle.
- **Depth**: Blind with a depth, Through all, Up to next (the next flat face along each hole's
  axis) or Up to face (a face or plane you select), the last two with an offset past the face; a
  blind round hole can have a 118° drill point.

Choosing a size fills in exact values; typing any size makes the hole Custom again. A tapped hole
shows its thread in the view like a [thread](thread).
