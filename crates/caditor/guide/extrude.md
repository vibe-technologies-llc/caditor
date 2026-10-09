# Extrude

{command:model.extrude} sweeps the closed regions of a sketch straight out of its plane into a
body, or cuts them into one.

## Choosing what to extrude

- With the sketch being edited, or curves or regions of one sketch selected, it extrudes that
  sketch. Selected regions alone are swept; selecting curves that close up sweeps what they
  enclose.
- With one flat face of a body selected, it extrudes that face outward, through a hidden sketch
  that follows the face.
- With nothing selected, it takes the last sketch not yet swept.

In the open panel, click regions in the view to add or leave them out, and
{command:model.clear_chosen_regions} starts from none.

## Extent

- **One side** runs to an end; **Symmetric** runs the same distance both ways; **Two sides** gives
  each side its own end.
- Each end is a **Distance**, **Through all**, **Up to next** (the next face of the body it
  changes) or **Up to face** (a face or plane you select).
- **Past the face** moves an Up to next or Up to face end beyond the face it reaches, or short of
  it when negative.
- On one side and without an offset, Up to next follows a curved next face, or several faces at
  different heights, and Up to face takes a curved face.
- While it is open, drag the arrow at the end in the view to change its distance. A named
  distance keeps its name; one that follows other parameters does not drag (see
  [parameters](parameters)).
- {command:model.reverse_direction} sends a one-sided extrusion the other way, like the panel's
  **Reverse direction**.

**Start** begins at the sketch plane, an offset from it, or another face or plane. **Taper** angles
the sides inward or outward.

**Direction** runs the extrusion square to the sketch, or **Along an edge or axis**: a straight
edge, sketch line or axis you select, measuring the distance along it.

## Result

- **New body**, **Add** to a body, **Remove from body** (a cut) or **Intersect with body**.
- A sketch on a face of a body cuts into that body by default.
- **Fill**: Solid fills the regions; **Thin wall** makes a wall of the sketch's curves with a
  thickness, inside, outside or centred. A sketch of open curves starts as a thin wall.

All distances and angles take [expressions](expressions). See also [Revolve](revolve).
