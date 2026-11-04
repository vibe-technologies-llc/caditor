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
  progressive spring's; **Table** follows a table of pitches by turn (see below). The turns, pitch
  and height above then size the helix by its mean pitch (by its table, for a table): typing a
  height and turns, the start pitch is solved for, and typing a height and the pitch, the turns.
  A flat spiral takes the same three, sized by its outer diameter in place of the height.
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
whole), or, with a changing pitch (below), by the pitch that changes: an outer diameter and turns
solve the start pitch, an outer diameter and the pitch the turns. An outer diameter no larger than
the inner one fails saying so. A flat spiral has no taper and no closed ends;
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

A flat spiral takes a changing pitch the same way, for a spiral spring whose turns open up or close
together as they wind outward: the radial growth each turn follows the pitch at that turn. A pipe
along it must keep its turns apart at the narrowest pitch the spiral reaches (the start pitch, the
final pitch or a row's), and fails before it is built, naming that pitch and the least it needs,
if not.

## A table of pitches

For a pitch that no even change describes (a spring closing up in steps, a coil that opens and
tightens again), choose **Table** under **Pitch**. The pitch above is the pitch at the start, and
each row says the turn the helix has reached its pitch at: **Row 1 at turn** 2 with **Row 1 pitch**
3 mm means the pitch changes evenly from the start pitch to 3 mm over the first two turns. Between
rows the pitch changes evenly; after the last row it stays at the last row's pitch. Two rows at
first, the second at the helix's last turn. **Add a row** under the rows adds one halfway between
the last row and the helix's end, at the last row's pitch; **Remove the last row** removes it (one row stays). Turns
must grow from row to row, the first after the start and the last within the turns the helix
makes, and a helix takes at most 24 rows; a row out of place fails the helix saying which.

A table replaces a progressive pitch's final pitch, and choosing **Progressive** clears it. It
works with closed ends: the end turns wind at the end pitch and the table applies between them,
its turns counted from the start of the body between the ends (after the first end and its change
of pitch). The pitch at the start is then the pitch where the first end's change leaves off, a
row's turn is counted along the body, the last row must lie within the turns between the ends, and
the pitch stays at the last row's until the far end's change begins. A turn that is not whole puts
a row inside a turn. A flat spiral takes a table too, counting its turns from where it starts. As
with a progressive pitch, a sweep along a helix of changing pitch keeps the section's lean to the
axis it starts with.

## Sweeping along it

Choose the helix as the path of a sweep or pipe: select the profile's regions and click the helix,
then choose Sweep, or click it while the Path row is choosing. The profile winds around the axis as
it rises, keeping its place against the axis (a triangle pointing at the axis keeps pointing at
it), so a thread profile stays a thread profile all the way.

- The profile's turns must stay apart: a profile taller than the pitch (wider than it, along a
  flat spiral) runs into itself and the sweep fails saying so.
- A pipe along a flat spiral must leave a gap between its turns: one as thick as the pitch fails
  before it is built, naming the least pitch its section needs (a little more than its size, more
  where the inner turns are tight). With a changing pitch it is the narrowest pitch that counts.
- A profile drawn on the XZ plane across where a flat spiral round Z starts is carried round it,
  as along a helix.
- A profile reaching across the axis cannot wind around it and fails.
- Keep the profile **Square to path**; kept parallel to its sketch it would slide along itself.

A helix a sweep follows can be deleted; the sweep then fails until the deletion is undone or a new
path is chosen.
