# Patterns in a sketch

Two tools repeat the selected geometry, keeping the copies in step with the original. Neither has a
key or a button of its own: open the menu of the arrow beside the Mirror button, or use the
Sketch menu or {command:palette}.

Select the geometry before starting, or select it in the tool: click items to add or remove them,
or drag a box around them. While either tool is active its panel sits on the right: **Shape**
switches between the two, **Repeats** counts what is repeated, and the copies are previewed as
you type. **Make the copies** or Enter makes them. Every value takes units and
[expressions](expressions) using your parameters, and the copies keep following them.

## Rectangular pattern

{command:sketch.rectangular_pattern} repeats along one or two directions:

- **Count** is how many along the row, the original included, and **Spacing** how far apart.
- **Angle** turns the row from the sketch's horizontal axis, such as 45 for a slanted row.
- **Rows** above 1 makes a grid: **Row spacing** sets how far apart the rows are, and **Row
  angle** which way they step, square to the row when left empty.

## Circular pattern

{command:sketch.circular_pattern} repeats about a point. Select that point with the geometry, or,
once the geometry is chosen (press Enter if you selected it in the tool), click the point or press
**Use the origin**; the panel's **Centre** names it. Then set:

- **Count**, how many in all, the original included.
- **Total angle**: 360 degrees spaces them evenly over a full turn; a smaller angle, such as 90,
  runs from the original to the last copy.

For patterns of bodies and features, see [Patterns](patterns).

The copies are held by construction lines from the centre. A circular pattern of points is a bolt
circle: a [hole](hole) drills the original and every copy, and a point pattern places a copy at
each, while the centre point the lines start from is left alone.
