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

- **Distribution** says what the distance means: **Spacing**, how far apart neighbouring copies
  are, or **Extent**, how far from the original the last copy is, the copies spread evenly over
  it. The distance field is named after the choice.
- **Count** is how many along the row, the original included.
- **Direction** is the way the row runs. By default it is the **Angle** from the sketch's
  horizontal axis, such as 45 for a slanted row. Press **Choose a line in the view** and click a
  line or axis (or highlight one with the keys and press Space) to run the row along it instead:
  the copies keep following the line when it turns, so an edge of the shape itself works too,
  such as a side of a hexagon. **Use an angle instead** goes back to the angle; Escape stops
  choosing.
- **Sides**: **One side** runs the copies from the original one way, and **Flip the direction**
  sends them the other way; **Both sides** puts Count copies on each side of the original.
- **Rows** above 1 makes a grid, with the same choices for the rows: **Row spacing** (or **Row
  extent**), **Row direction**, **Row angle**, **Row sides** and **Flip the row direction**. Left
  without a row angle or line, the rows step square to the row, also square to a chosen line.

## Circular pattern

{command:sketch.circular_pattern} repeats about a point. Select that point with the geometry, or,
once the geometry is chosen (press Enter if you selected it in the tool), click the point or press
**Use the origin**; the panel's **Centre** names it. Then set:

- **Spread**: **Full** spaces the copies evenly over a whole turn; **Angle** runs them from the
  original to the last copy over the **Total angle**, a negative angle turning clockwise;
  **Symmetric** turns them both ways from the original, the **Angle each way** reaching the last
  copy on each side.
- **Count**, how many, the original included; on each side when the spread is symmetric.

For patterns of bodies and features, see [Patterns](patterns).

The copies are held by construction lines from the centre. A circular pattern of points is a bolt
circle: a [hole](hole) drills the original and every copy, and a point pattern places a copy at
each, while the centre point the lines start from is left alone.
