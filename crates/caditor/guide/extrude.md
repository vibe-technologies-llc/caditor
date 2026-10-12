# Extrude

{command:model.extrude} sweeps the closed regions of a sketch straight out of its plane into a
body, or cuts them into one.

## Choosing what to extrude

- With the sketch being edited, or curves or regions of one sketch selected, it extrudes that
  sketch. Selected regions alone are swept; selecting curves that close up sweeps what they
  enclose.
- With flat faces of one body selected that lie in one plane, it extrudes them outward together,
  through a hidden sketch that follows their edges.
- With nothing selected, it takes the last sketch not yet swept. When features already sweep
  some of its regions, only the remaining ones are taken, so each part of a sketch can be
  extruded on its own with its own taper and distance.

In the open panel, click regions in the view to add or leave them out, and
{command:model.clear_chosen_regions} starts from none.

## Extent

- **One side** runs to an end; **Symmetric** runs the same distance both ways; **Two sides** gives
  each side its own end.
- Each end is a **Distance**, **Through all**, **Up to next** (the next face of the body it
  changes) or **Up to face** (a face or plane you select).
- A cut whose distance reaches more than ten times the size of the bodies it cuts offers **Cut
  through all instead**, which keeps cutting through however the bodies grow.
- **Past the face** moves an Up to next or Up to face end beyond the face it reaches, or short of
  it when negative.
- On one side and without an offset, Up to next follows a curved next face, or several faces at
  different heights, and Up to face takes a curved face.
- While it is open, drag the arrow at the end in the view to change its distance. A named
  distance keeps its name; one that follows other parameters does not drag (see
  [parameters](parameters)). An arrow at the edge of the end drags the **Taper**, one standing on
  an Up to face end drags how far it goes past the face, and a thin wall has an arrow for its
  **Thickness**.
- A dragged arrow stops on a corner, an edge's middle or a round edge's centre under the pointer,
  and pulls to a flat face it passes, the readout saying what it stopped at; hold Ctrl to drag
  freely. With the pointer on an arrow, type a value and press Enter to set that field exactly;
  it is previewed as you type, and Escape leaves it as it was. A value the field itself would
  refuse, such as zero, is refused there too, saying why.
- {command:model.reverse_direction} sends a one-sided extrusion the other way, like the panel's
  **Reverse direction**.
- Typing a **negative distance** on one side goes the other way. The distance becomes its positive
  value and the extrusion is reversed in the same change; on an **Add** it also switches to **Remove**
  and the other way round, so a sketch on a top face that grew up cuts down into the body, and a
  notice says what changed. A new body or an intersection only reverses. An expression that comes out
  negative, such as `-depth` or `width - 30`, keeps its parameters (`depth`, `-(width - 30)`). It is one
  undo, and dragging the arrow back through its start does the same. Symmetric and two-sided
  distances, and zero, are refused. The flip happens when the value is typed: if a parameter later
  makes the distance negative, the extrusion fails and says to enter a distance above zero, rather than
  flipping again.

**Start** begins at the sketch plane, an offset from it, or another face or plane, and the
distance arrows stand at the ends measured from there. **Taper** angles the sides inward or
outward.

**Direction** runs the extrusion square to the sketch, or **Along an edge or axis**: a straight
edge, sketch line or axis you select, measuring the distance along it.

## Result

- **New body**, **Add** to a body, **Remove from body** (a cut) or **Intersect with body**.
- A new extrusion adds to the body it reaches, overlapping or touching it, else makes a new body;
  reaching several, it adds to the one it overlaps most. With faces of a body selected along with
  the sketch, it adds to that body.
- A sketch on a face of a body cuts into that body by default.
- **Fill**: Solid fills the regions; **Thin wall** makes a wall of the sketch's curves with a
  thickness, inside, outside or centred. A sketch of open curves starts as a thin wall.

The **Sketch** row names the sketch swept, with a button beside it to edit that sketch; so does
{command:model.edit_sketch} on a face the extrusion made.

A new extrusion starts from the distance last set on one, and every other tool that starts from a
value (fillet, chamfer, shell, hole, pattern counts) likewise from the last one used, kept between
sessions; a value naming a parameter the model lacks falls back to the usual starting value.

All distances and angles take [expressions](expressions). See also [Revolve](revolve).
