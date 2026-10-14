# Coil

{command:model.coil} winds a round or square section around an axis: a compression spring, a
coil of wire, a helical groove round a shaft or a conical spring. It needs no sketch, only an axis
and a few sizes. It is in the Solid group's menu of the ribbon, the Model menu and the command
palette.

The axis is a principal axis, a datum axis, a coordinate system's axis, a straight edge, the axis
of a round face (a cylinder or cone) or a line of a sketch. Select it, then choose Coil. With
nothing selected the coil winds around the Z axis and starts choosing its axis in the view: click
an axis, edge, round face or sketch line, or press Escape to keep the Z axis.

The coil starts at the axis's own start (the origin of a principal axis, the start of an edge or
sketch line, the base of a round face) and rises along it. Its section sits on the plane square to
the axis there, so a spring stands on that plane rather than reaching below it.

## Panel

- **Axis** names the axis wound around: choose another from the list of principal, datum and
  coordinate system axes, use the selected one, or choose in the view. **Reverse direction** makes
  the coil rise the other way along it.
- **Sized by** picks which two of the three lengths you type: **Turns and pitch**, **Height and
  turns** or **Height and pitch**. The third follows from them: the height is the pitch times the
  turns, measured from the start of the section's path to its end, so the wire adds its own size
  on top. Switching keeps the values you typed and works out the new one from what the coil is
  now. Turns need not be whole: 5.5 ends the coil half way round.
- **Diameter** is the coil's diameter at its start.
- **Section**: **Round** (a wire of the **Section diameter**) or **Square** (a section of the
  **Section width**, its sides along and across the axis). The section lies in a plane through
  the axis.
- **Position** places the section against the diameter: **Inside** keeps it within the diameter
  (its outside on it, as for a spring sized by its outer diameter), **Centred** runs its middle on
  the diameter, **Outside** keeps it beyond (as for a spring wound over a shaft of that
  diameter).
- **Taper** narrows or widens the coil as it rises, as a conical spring: a positive angle widens
  it, a negative one draws it in. It must stay within 80° either way.
- **Turning**: **Counter-clockwise** (a right-handed coil) or **Clockwise** (left-handed), seen from
  its end looking back along the axis.
- **Result**: **New body**, **Add to body**, **Remove from body** or **Intersect with body**, and
  the **Body** it changes. Removing a coil wound around a shaft's face cuts a helical groove into
  it.

Every size takes units and expressions, so a spring can follow parameters such as `wire_d` and
`coils`.

While the coil is open, arrows in the view drag its sizes: one along the base for the diameter, one
on top of the wire half way round the first turn for the section, one above the start of the wire
for the pitch (when the coil is sized by a pitch) and one at the end of the wire for the height
(when it is sized by a height). Each drag is one undoable change and follows the same steps and
snapping as other arrows; Ctrl drags freely, and typing a number while holding or hovering an arrow
sets the value as the panel field would, parameters included. Reverse direction is also the palette's Reverse the direction on the open coil.

## Limits

- The pitch must be larger than the section, so the turns stay apart: a coil whose turns would
  touch fails, saying how far apart they must be. A closed-end spring, whose turns touch, is
  modelled with a pitch a hair larger than the wire.
- The section may not reach the axis, at the start or (with a taper drawing it in) at the end.
- A coil makes from 0.05 to 500 turns.

The coil's faces are named after it and the turn they belong to ("Coil 1 surface", for a square
section "Coil 1 inside", "outside", "top" and "underside", and its "start" and "end"), so a later
feature keeps hold of them when the turns, pitch or diameter change.
