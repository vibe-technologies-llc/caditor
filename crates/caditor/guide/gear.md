# Spur gear

{command:sketch.gear} draws the outline of an involute spur gear as one closed profile, ready to
extrude. Find it in the corner menu of the Polygon button, the Sketch menu or {command:palette}.

While it is active the Spur gear panel holds the gear's values. Each takes units and expressions
using your parameters, such as `m` or `2 * m`:

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

## Placing it

- Click where the centre goes: on a point, the gear centres on it; anywhere else it gets a new
  point. The gear follows the pointer as a preview.
- The panel's button, or Enter in the view, draws it at the one selected point, else at the
  origin.
- From the keyboard, highlight a point with {command:view.highlight_next} and choose it with
  {command:view.activate_highlighted}.

Each gear is one change that {command:edit.undo} takes back.

## What it draws

Each tooth flank is a spline traced along the involute within a ten-thousandth of the module,
joined to arcs at the tip and root, the root fillets and, below the base circle, a short radial
line. The pitch, base, root and tip circles are drawn as construction geometry around the centre.
The gear is drawn once: its curves do not follow later changes to the values or the parameters
they used. To change it, delete it and draw it again.
