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

The panel shows the pitch, tip, root and base diameters these give, or says in words what is
wrong. A tooth count that would be undercut at the chosen profile shift is refused, naming the
fewest teeth and the least shift that would do; so are teeth that come to a point, a fillet too
large for the gap and a bore reaching the roots.

## A pair of gears

A gear pair draws the gear above and a second gear that meshes with it. It has the same values,
plus two for the second gear:

- **Meshing teeth**: the second gear's whole number of teeth.
- **Meshing bore**: the diameter of the second gear's hole; leave it empty for none.

The second gear shares the module, pressure angle and root fillet, and takes the opposite profile
shift, so the pair meshes at the standard centre distance: the module times the sum of the two
tooth counts over two. It is drawn that far to the right of the first gear, turned so a tooth of
one lies in a gap of the other with no play between them. The panel shows the centre distance and
both gears' diameters, and says in words which gear cannot be cut when one is undercut.

## A sprocket

A sprocket is cut for ISO 606 roller chain. Choose the **Chain** (the A and B series from 05B to
48B, each with its pitch), then the **Teeth** (9 to 150) and an optional **Bore**. The pitch
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

## What it draws

Each gear tooth flank is a spline traced along the involute within a ten-thousandth of the module,
joined to arcs at the tip and root, the root fillets and, below the base circle, a short radial
line. A sprocket is made of arcs only. The pitch, base, root and tip circles of a gear (the pitch,
tip and root circles of a sprocket) are drawn as construction geometry around each centre.

What is drawn is drawn once: its curves do not follow later changes to the values or the
parameters they used. To change it, delete it and draw it again.
