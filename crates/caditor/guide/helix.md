# Helix

{command:model.helix} adds a helix: a curve winding around an axis at a steady pitch (or with the
closed ends of a compression spring, or a pitch that changes along it), for a
[Sweep](sweep) or [Pipe](pipe) to follow. Sweep a triangle of your own along it for a thread, a
rectangle for a flat-wound spring, or run a pipe along it for a coil with a section a [Coil](coil)
does not offer. It makes no body. It is in the Reference group of the ribbon, the Model menu and the
command palette.

{command:model.flat_spiral} adds the same feature as a **flat spiral**: a curve winding outward in
the plane square to the axis, its turns a steady pitch apart, like a clock spring or a watch's
hairspring. Pipe a round or square wire along it, or sweep a strip, for a spiral spring. The
**Shape** row of the panel turns a helix into a flat spiral and back.

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

- **Shape**: **Helix** rises along the axis; **Flat spiral** winds outward in the plane square to
  it. Changing it keeps the turns and the pitch (sizing by them if a height or outer diameter was
  typed).
- **Axis** names the axis wound around: choose another from the list, use the selected one, or
  choose in the view. **Reverse direction** makes the helix rise the other way along it.
- **Sized by** picks which two of turns, pitch and height you type, as for a coil; turns need not
  be whole.
- **Diameter** is the helix's diameter at its start.
- **Taper** narrows or widens it as it rises: a positive angle widens it, a negative one draws it
  in, within 80° either way.
- **Pitch**: **Even** winds at one pitch from end to end; **Progressive** changes the pitch
  evenly from the pitch above, at the start, to the **Final pitch** at the far end, as a
  progressive spring's. The turns, pitch and height above then size the helix by its mean pitch:
  typing a height and turns, the start pitch is solved for, and typing a height and the pitch, the
  turns. Not offered for a flat spiral.
- **Ends**: **Open** winds as the pitch above says from end to end; **Closed** closes the ends as
  a compression spring's are. **End turns** at each end wind at the **End pitch**, and the pitch
  then changes evenly to the helix's own over the next **End change** turns (half a turn at
  first, at least 0.05). The turns, pitch and height above then size the whole spring with its
  ends: the pitch is the one between the ends (the start pitch, if the pitch is progressive), and
  a height includes them. Each end with its change of pitch takes its end turns and its change, so
  the helix must make at least that many turns at both ends together; a progressive pitch needs
  turns left between the ends to change over. Not offered for a flat spiral.
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

## Flat spirals

A flat spiral lies in the plane through the axis's start square to the axis. It starts at its
**Inner diameter**, on the side the axis's X direction points to (or at the start angle or point),
and grows outward by its **Pitch** each turn, an Archimedean spiral. **Sized by** picks two of
turns, pitch and **Outer diameter**: an outer diameter and turns spread the turns evenly between
the two diameters, an outer diameter and a pitch make as many turns as fit (they need not be
whole). An outer diameter no larger than the inner one fails saying so. A flat spiral has no taper;
**Reverse direction** turns its plane over, so it winds the other way round from the same start, as
**Turning** does. Its arrows drag the inner diameter, the pitch (outward from where it starts) and
the outer diameter (at its end).

While the helix is open, arrows in the view drag its sizes: one from the axis toward where the
helix starts for the diameter, one at the helix a turn up for the pitch (when it is sized by a
pitch) and one at its end for the height (when it is sized by a height); dragging either back
past where the helix starts reverses it. Each drag is one undoable
change and follows the same steps and snapping as other arrows; Ctrl drags freely, and typing a
number while holding or hovering an arrow sets the value as the panel field would.

## Springs with closed ends

For a compression spring with closed and ground ends, choose **Closed** under **Ends**, set the end
turns (one or so) and an end pitch a hair more than the wire's size, and pipe the wire along it.
The turns of the closed ends cannot quite touch: an end pitch equal to the wire's size runs the
wire into itself and the pipe fails saying so. Grind the ends flat afterwards by cutting the
spring with an extrusion or a split at each end.

The pitch arrow then stands past the closed end, where the helix winds at its own pitch, and the
height arrow measures the whole spring.

## Progressive springs

For a progressive spring, whose coils close up as it is compressed, choose **Progressive** under
**Pitch** and set the **Final pitch**: the pitch changes evenly along the helix from the pitch at
its start to the final one. With closed ends it changes between the two closed ends' changes, over
the turns between them. A pipe or sweep along it keeps the section's lean to the axis that it
starts with, so a round wire is a hair out of round where the pitch differs from the start's.

## Sweeping along it

Choose the helix as the path of a sweep or pipe: select the profile's regions and click the helix,
then choose Sweep, or click it while the Path row is choosing. The profile winds around the axis as
it rises, keeping its place against the axis (a triangle pointing at the axis keeps pointing at
it), so a thread profile stays a thread profile all the way.

- The profile's turns must stay apart: a profile taller than the pitch (wider than it, along a
  flat spiral) runs into itself and the sweep fails saying so.
- A pipe along a flat spiral must leave a gap between its turns: one as thick as the pitch fails
  before it is built, naming the least pitch its section needs (a little more than its size, more
  where the inner turns are tight).
- A profile drawn on the XZ plane across where a flat spiral round Z starts is carried round it,
  as along a helix.
- A profile reaching across the axis cannot wind around it and fails.
- Keep the profile **Square to path**; kept parallel to its sketch it would slide along itself.

A helix a sweep follows can be deleted; the sweep then fails until the deletion is undone or a new
path is chosen.
