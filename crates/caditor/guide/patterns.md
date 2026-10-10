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

Select an axis, straight edge, round face or sketch line with it to set the direction or axis;
otherwise a principal axis is used.

## Along a curve or at points

- The curve is every curve of a sketch joined end to end into one chain: lines, arcs, splines or
  a closed loop such as a circle. Make other curves of the sketch construction geometry.
- Select a curve or point of the sketch to use it; otherwise the last sketch above the pattern
  that nothing else uses and that fits is taken.
- Along a curve the copies are carried from where the curve starts, as its start would move to
  reach each place, so the body need not lie on the curve. At points each copy is moved from the
  **Base point**, the origin unless you choose another.

## The panel

- **Direction**, an optional **Second direction**, or the **Axis**, each from a list or the
  selection.
- The count and, for a linear pattern, **Spacing** between copies or the **Total length**, each
  previewed as you type. {command:model.reverse_direction} reverses the first direction, the turn
  of a circular pattern, or the way along the curve.
- For a curve: **Curve**, the **Count**, **Spaced** **Evenly** over the whole curve (round a
  closed one) or **By distance** with its **Spacing**, and **Copies** **Kept as they are** or
  **Turned with the curve**.
- For points: **Points**, the sketch whose lone points take the copies, and the **Base point**.
- **Instances**: a grid with one box per copy. Uncheck a box, or click a copy in the view, to
  leave it out; a point pattern leaves out a copy you click and **Bring the copies back** returns
  them.

Counts and spacings take [expressions](expressions). See also
[patterns in a sketch](sketch-patterns).
