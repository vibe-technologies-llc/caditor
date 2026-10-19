# Spur gear

{command:sketch.gear} draws the outline of an involute spur gear, of a pair of meshing gears or of
a sprocket for roller chain, each as closed profiles ready to extrude. Find it in the corner menu
of the Polygon button, the Sketch menu or {command:palette}.

While it is active the Spur gear panel opens with a choice of what to draw: **Single gear**, **Gear
pair** or **Sprocket**. Below it sit that choice's values. Each takes units and expressions using
your parameters, such as `m` or `2 * m`.

## A single gear

The gear's values are:

- **Module**: the pitch diameter over the number of teeth. Gears that mesh share it.
- **Teeth**: a whole number of teeth.
- **Pressure angle**: the angle the teeth push along, usually 20°.
- **Profile shift**: how far the teeth move outward, in modules. A positive shift lets a gear with
  few teeth avoid undercut.
- **Root fillet**: the radius rounding the bottom of each gap; 0 leaves it sharp.
- **Bore**: the diameter of the hole in the middle; leave it empty for none.
- **Rotation**: how far the whole gear turns about its centre; at 0° a tooth points along the
  sketch's horizontal axis.

The panel shows the pitch, tip, root and base diameters these give, or says in words what is
wrong. A tooth count that would be undercut at the chosen profile shift is refused, naming the
fewest teeth and the least shift that would do; so are teeth that come to a point, a fillet too
large for the gap and a bore reaching the roots.

## A pair of gears

A gear pair draws the gear above and a second gear that meshes with it. It has the same values,
plus three for the second gear:

- **Meshing teeth**: the second gear's whole number of teeth.
- **Meshing bore**: the diameter of the second gear's hole; leave it empty for none.
- **Direction**: where the second gear's centre lies from the first's, as an angle from the
  sketch's horizontal axis; at 0° it is to the right.

The second gear shares the module, pressure angle and root fillet, and takes the opposite profile
shift, so the pair meshes at the standard centre distance: the module times the sum of the two
tooth counts over two. Its centre is that far from the first gear's along the direction, and it
is turned so a tooth of one lies in a gap of the other with no play between them. The panel shows
the centre distance and both gears' diameters, and says in words which gear cannot be cut when one
is undercut.

## A sprocket

A sprocket is cut for ISO 606 roller chain. Choose the **Chain** (the A and B series from 05B to
48B, each with its pitch), then the **Teeth** (9 to 150), an optional **Bore** and the
**Rotation** (at 0° the first gap lies along the sketch's horizontal axis). The pitch
circle passes through the roller centres and is the pitch over the sine of 180° over the number of
teeth across. Each gap is a seat arc that holds the roller, a flank arc running out of it and a
tip arc on the tip circle between gaps. The panel shows the pitch, tip, root and roller
diameters; a tooth count whose teeth would come to a point is refused in words.

## Placing it

- Click where the centre goes: on a point, the gear centres on it; anywhere else it gets a new
  point. The gear follows the pointer as a preview.
- The panel's button, or Enter in the view, draws it at the one selected point, else at the
  origin.
- From the keyboard, highlight a point with {command:view.highlight_next} and choose it with
  {command:view.activate_highlighted}.

Whatever is drawn is one change that {command:edit.undo} takes back; a pair or a sprocket is
placed by its first gear's or its own centre.

## Changing a gear

A gear stays a gear: it keeps the values you typed, with the parameters they use, and is drawn
again from them whenever they change. Change `teeth` under Parameters and the teeth are
added or taken away; the faces that were made from the other teeth keep their names, so later
features that use them carry on.

Select the gear by clicking its outline, then choose {command:sketch.gear}: the panel shows its
values, and its button updates the gear in one change, keeping its centre and every constraint on
it. **Draw a new gear instead** leaves the selected gear as it is and draws another.

The two gears of a pair are two gears, the second following the first. Change the first gear's
module, pressure angle, root fillet, profile shift, teeth or rotation, or a parameter any of
them uses, and both are drawn again: the second gear takes the same module, pressure angle and
root fillet, the opposite profile shift, a turn that keeps the teeth meshing and its centre at
the new centre distance. Select the second gear and choose {command:sketch.gear} to change what
is its own, its teeth, its bore and the direction of its centre; the panel says which values it
follows. A second gear that cannot be cut, such as one with too few teeth for the profile shift,
fails the sketch and names it.

The centre is an ordinary sketch point: constrain it, dimension its distance from the origin or
from other geometry, or fix it, and the gear moves with it; dragging the gear drags the centre.
Deleting the gear deletes its centre too, unless a curve also uses that point; deleting the centre
deletes the gear. The second gear's centre is held by the pair at the centre distance, so
dragging it drags the first gear with it, and a constraint that would move it elsewhere is
reported as a conflict. Deleting the first gear of a pair deletes the second as well; deleting the
second leaves the first. Copying a pair copies the first gear alone.

A sprocket is stored the same way: it keeps its chain, teeth, bore and rotation, with the
parameters they use, and is drawn again whenever they change. Select it and choose
{command:sketch.gear}: the panel shows the **Chain** and the values, and its button updates the
sprocket in one change, keeping its centre and constraints. The chain is picked from the list, not
typed, so a parameter cannot change it. A sprocket never meshes with a gear, and a tooth count
whose teeth would come to a point, or a bore reaching the roots, fails the sketch and names it.
Copying a sprocket copies its chain too.

## What it draws

Each gear tooth flank is a spline traced along the involute within a ten-thousandth of the module,
joined to arcs at the tip and root, the root fillets and, below the base circle, a short radial
line. A sprocket is made of arcs only. The pitch, base, root and tip circles of a gear (the pitch,
tip and root circles of a sprocket) are drawn as construction geometry around each centre.

A gear or sprocket is not made of sketch curves: its outline cannot be trimmed, dimensioned or
constrained piece by piece, only through its values and its centre. It still extrudes,
cuts and exports like any outline. When its values cannot make a gear, such as a tooth count with
a fraction or fewer than 5 teeth, the sketch fails and names the gear and what to change.
