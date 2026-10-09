# Patterns

{command:model.linear_pattern} repeats a body along one or two directions;
{command:model.circular_pattern} repeats it about an axis.

## What is repeated

- Select a face, edge or vertex of the body, or choose it in the tree, to repeat the whole body.
- Choose extrusions, revolves, holes or primitives in the tree to repeat only those features on
  their body, such as a row of holes.

Select an axis, straight edge, round face or sketch line with it to set the direction or axis;
otherwise a principal axis is used.

## The panel

- **Direction**, an optional **Second direction**, or the **Axis**, each from a list or the
  selection.
- The count and, for a linear pattern, **Spacing** between copies or the **Total length**.
- **Instances**: a grid with one box per copy. Uncheck a box, or click a copy in the view, to
  leave it out.

Counts and spacings take [expressions](expressions). See also
[patterns in a sketch](sketch-patterns).
