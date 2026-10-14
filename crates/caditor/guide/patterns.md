# Patterns

{command:model.linear_pattern} repeats a body along one or two directions;
{command:model.circular_pattern} repeats it about an axis. {command:model.curve_pattern} repeats
it along a curve of a sketch, and {command:model.point_pattern} places a copy at each lone point of
a sketch.

A new linear or circular pattern starts from the count last used on one, kept between sessions.

## What is repeated

- Select a face, edge or vertex of the body, or choose it in the tree, to repeat the whole body.
- Choose extrusions, revolves, holes or primitives in the tree to repeat only those features on
  their body, such as a row of holes.
- Choose fillets and chamfers with them to round or bevel each copy the same way: a filleted boss
  repeated with its fillet gives every copy its fillet, a raised count too. The blend follows the
  edges of the features repeated with it (edges of the body around them, such as where a boss
  stands on a plate, included); a fillet or chamfer chosen without the feature whose edges it
  blends fails and says so, and so does a copy whose edges meet the body differently from the
  original's. A fillet or chamfer on an edge of a corner an earlier repeated fillet or chamfer
  rounded or bevelled follows it onto every copy too.
- Choose a shell with them to hollow each copy as the original is: a boss opened at its top and
  repeated with its shell is opened and hollowed on every copy, the walls running into one
  cavity with the rest of the body. The pattern hollows the body again from where it stood before
  the shell, so it must come right after the shell: a feature changing the body in between makes
  the pattern fail saying to move it right below the shell.

Select an axis, straight edge, round face or sketch line with it to set the direction or axis. A
linear pattern otherwise runs along the X axis. A circular pattern never picks an axis for you: with
none selected it opens waiting for you to click the axis, straight edge or round face to turn about,
and makes no copies until you do. Until then its row in the tree shows a pointing hand and "Waiting
for an axis" rather than a failure, with Choose in the view to pick it later; the Axis reads None
chosen.

## Along a curve or at points

- The curve is every curve of a sketch joined end to end into one chain: lines, arcs, splines or
  a closed loop such as a circle. Make other curves of the sketch construction geometry.
- Select a curve or point of the sketch to use it; otherwise the last sketch above the pattern
  that nothing else uses and that fits is taken.
- Along a curve the copies are carried from where the curve starts, as its start would move to
  reach each place, so the body need not lie on the curve. At points each copy is moved from the
  **Base point**, the origin unless you choose another.

## The panel

- **Direction**, an optional **Second direction**, or the **Axis**, each from a list of the
  principal axes, datum axes and coordinate system axes above the pattern, the selection or Choose
  in the view.
- The count and, for a linear pattern, **Spacing** between copies or the **Total length**, each
  previewed as you type. {command:model.reverse_direction} reverses the first direction, the turn
  of a circular pattern, or the way along the curve.
- For a curve: **Curve**, the **Count**, **Spaced** **Evenly** over the whole curve (round a
  closed one) or **By distance** with its **Spacing**, and **Copies** **Kept as they are** or
  **Turned with the curve**.
- For points: **Points**, the sketch whose lone points take the copies, and the **Base point**.
- In the view, an arrow at the last copy of each direction drags the spacing; the count is typed
  in the panel. A circular pattern has two arrows along its turn, for its total angle and its
  count; the angle's arrow dragged onto a corner or an edge's middle turns until it reaches it. A curve pattern spaced by distance has an arrow at its last copy for the spacing, and one
  spread evenly an arrow at its second copy for the count; both slide along the curve as you
  drag. Type a value with the pointer on an arrow to set it.
- **Instances**: a grid with one box per copy, laid out like the copies seen from the top or
  front: the original at the bottom left, the first direction running to the right and the second
  upwards (reversing a direction flips it). Uncheck a box, or click a copy in the view, to leave it
  out; a point pattern leaves out a copy you click and **Bring the copies back** returns them.

Counts and spacings take [expressions](expressions). See also
[patterns in a sketch](sketch-patterns).
