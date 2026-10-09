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
- While it is open, drag the arrow at the end in the view to change its distance.

**Start** begins at the sketch plane, an offset from it, or another face or plane. **Taper** angles
the sides inward or outward.

## Result

- **New body**, **Add** to a body, **Remove from body** (a cut) or **Intersect with body**.
- A sketch on a face of a body cuts into that body by default.
- **Fill**: Solid fills the regions; **Thin wall** makes a wall of the sketch's curves with a
  thickness, inside, outside or centred. A sketch of open curves starts as a thin wall.

All distances and angles take [expressions](expressions). See also [Revolve](revolve).
