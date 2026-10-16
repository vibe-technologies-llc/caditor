# Helix

{command:model.helix} adds a helix: a curve winding around an axis at a steady pitch, for a
[Sweep](sweep) or [Pipe](pipe) to follow. Sweep a triangle of your own along it for a thread, a
rectangle for a flat-wound spring, or run a pipe along it for a coil with a section a [Coil](coil)
does not offer. It makes no body. It is in the Reference group of the ribbon, the Model menu and the
command palette.

The axis is a principal axis, a datum axis, a coordinate system's axis, a straight edge, the axis
of a round face (a cylinder or cone) or a line of a sketch. Select it, then choose Helix. With
nothing selected the helix winds around the Z axis and starts choosing its axis in the view: click
an axis, edge, round face or sketch line, or press Escape to keep the Z axis.

The helix starts at the axis's own start (the origin of a principal axis, the start of an edge or
sketch line, the base of a round face) on the side the X axis points to (the Y axis for a helix
around X), at its radius, and rises along the axis. Around the Z axis it starts on the X axis, so a
profile drawn on the XZ plane across the point where it starts is carried along it. A **Start
angle** turns where it starts round the axis: 90° around Z starts it on the Y axis, so a profile
drawn on the YZ plane is carried along it instead.

## Panel

- **Axis** names the axis wound around: choose another from the list, use the selected one, or
  choose in the view. **Reverse direction** makes the helix rise the other way along it.
- **Sized by** picks which two of turns, pitch and height you type, as for a coil; turns need not
  be whole.
- **Diameter** is the helix's diameter at its start.
- **Taper** narrows or widens it as it rises: a positive angle widens it, a negative one draws it
  in, within 80° either way.
- **Starts at** starts the helix at the angle of a point: select a corner of a body (the end of an
  edge is one), the centre of a round edge, a sketch point or a datum point, then use the
  selection or choose it in the view. The helix then begins on the half-plane through that point
  and its axis, and follows the point when it moves; the **x** beside the name goes back to the
  typed angle. A point on the axis has no angle round it, so the helix fails saying so until you
  pick another point or remove it.
- **Start angle** turns where the helix starts round its axis, counter-clockwise seen from the
  axis's end, whichever way the helix turns; 0° starts it where the axis's X direction points. It
  is hidden while a point is chosen, and used again when the point is removed.
- **Turning**: **Counter-clockwise** (right-handed) or **Clockwise** (left-handed), seen from its
  end looking back along the axis.

Every size takes units and expressions. The helix is drawn in the view like a datum and hides with
the datums.

While the helix is open, arrows in the view drag its sizes: one from the axis toward where the
helix starts for the diameter, one at the helix a turn up for the pitch (when it is sized by a
pitch) and one at its end for the height (when it is sized by a height); dragging either back
past where the helix starts reverses it. Each drag is one undoable
change and follows the same steps and snapping as other arrows; Ctrl drags freely, and typing a
number while holding or hovering an arrow sets the value as the panel field would.

## Sweeping along it

Choose the helix as the path of a sweep or pipe: select the profile's regions and click the helix,
then choose Sweep, or click it while the Path row is choosing. The profile winds around the axis as
it rises, keeping its place against the axis (a triangle pointing at the axis keeps pointing at
it), so a thread profile stays a thread profile all the way.

- The profile's turns must stay apart: a profile taller than the pitch runs into itself and the
  sweep fails saying so.
- A profile reaching across the axis cannot wind around it and fails.
- Keep the profile **Square to path**; kept parallel to its sketch it would slide along itself.

A helix a sweep follows can be deleted; the sweep then fails until the deletion is undone or a new
path is chosen.
