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

{command:sketch.rectangular_pattern} repeats along one direction or several, each a group of its
own in the panel:

- **Distribution** says what the distance means: **Spacing**, how far apart neighbouring copies
  are, or **Extent**, how far from the original the last copy is, the copies spread evenly over
  it. The distance field is named after the choice.
- **Count** is how many along the direction, the original included.
- **Runs along** is the way the direction runs. By default it is the **Angle** from the sketch's
  horizontal axis, such as 45 for a slanted row. Press **Choose a line for direction 1** and click
  a line or axis (or highlight one with the keys and press Space) to run it along that line
  instead: the copies keep following the line when it turns, so an edge of the shape itself works
  too, such as a side of a hexagon. **Use an angle for direction 1** goes back to the angle;
  Escape stops choosing.
- **Sides**: **One side** runs the copies from the original one way, and **Flip direction 1**
  sends them the other way; **Both sides** puts Count copies on each side of the original.
- **Add a direction** adds another group with the same choices, and every combination of steps
  becomes a copy: two directions make a grid, three a grid repeated along a third way, and so on.
  Direction 2 starts without an angle and runs square to direction 1, also square to a chosen
  line; a third and later ones start at 45°, and any left without an angle or line runs square to
  direction 1 too. **Remove direction 2** takes a direction away, and those after it move up a
  number, keeping their lines. Directions running along one line, or reaching one place by two
  routes (such as three sides of a hexagon at one spacing), are refused, since copies would land
  on each other.

## Circular pattern

{command:sketch.circular_pattern} repeats about a point. Select that point with the geometry, or,
once the geometry is chosen (press Enter if you selected it in the tool), click the point or press
**Use the origin**; the panel's **Centre** names it. Then set:

- **Spread**: **Full** spaces the copies evenly over a whole turn; **Angle** runs them from the
  original to the last copy over the **Total angle**, a negative angle turning clockwise;
  **Symmetric** turns them both ways from the original, the **Angle each way** reaching the last
  copy on each side.
- **Count**, how many, the original included; on each side when the spread is symmetric.

## Leaving out copies

Either pattern can skip single copies while it is set up. Point at a previewed copy: it is drawn
thicker, and the words beside the pointer say which copy a click leaves out. Click it and it is
drawn dashed and hollow instead of being made; click it again to make it. The panel's **Copies**
row has a box for every copy, laid out as the copies are (the original's box cannot be cleared),
so you can also leave copies out with Tab and Space; it counts how many are left out, and
**Bring the copies back** makes them all again. Changing a count keeps the copies you left out.

The copies still follow the original and every value: each is tied to the nearest copy made along
its direction by that many steps. A copy with every copy between it and the original left out
along both ways is tied across both steps at once. When such a copy runs along a chosen line,
that cannot be held and the panel says so: keep one of the copies in its row, or give the
directions angles.

For patterns of bodies and features, see [Patterns](patterns).

The copies are held by construction lines from the centre. A circular pattern of points is a bolt
circle: a [hole](hole) drills the original and every copy, and a point pattern places a copy at
each, while the centre point the lines start from is left alone.
