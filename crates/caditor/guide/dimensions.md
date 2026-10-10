# Dimensions

Dimensions fix sizes: lengths, distances, angles, radii and diameters. A new dimension starts at
the size drawn, and its value field opens at once; type a value or an [expression](expressions),
for example `width / 2` or `30 deg`.

## Smart dimension

{command:sketch.dimension} picks the right dimension from what you click:

- A line gives its length, a circle its diameter and an arc its radius.
- Two lines that are not parallel give the angle between them.
- An ellipse alone gives its major and minor radii; with a point or any other curve, the
  distance between them, measured where they come closest. An elliptical arc and a line sharing
  an end give the angle between them.
- Anything else gives the distance between the two items, the origin and axes included.

Clicking two points, or a lone line, waits for a third click that places the dimension: above or
below gives the horizontal distance, beside them the vertical one, elsewhere the aligned one. For
an arc, placing it inside gives its sweep and beyond it its length. Enter adds the dimension of a
single pick, and Escape lets go of the picks; changing to another tool lets go of them too.

The Dimension group of the ribbon also has each dimension on its own: Distance, Horizontal
distance, Vertical distance, Angle, Radius and Diameter.

## Changing a dimension

Double-click its label to edit the value; drag the label to move it. Typing `name = value` names
the value as a [parameter](parameters) that other values can use. The sketch's card in the
[feature tree](feature-tree) lists every dimension too.

Zoomed far out, a dimension of something only a few pixels across shrinks to a small dot on it;
hover the dot to read the dimension, click it to select it and show its label, or double-click it
to edit the value. A dimension that measures nothing (two points in one place), a selected or
highlighted one and one in conflict always keep their label.

## Reference dimensions

A dimension added to geometry that is already fully held only shows the measured size, in
parentheses; it is a reference. {command:sketch.toggle_constraint_active} makes it drive the
geometry instead, or turns a driving dimension into a reference.

{command:sketch.toggle_first_dimension_scales} makes the first dimension of an unconstrained
drawing scale the whole sketch, which sizes a traced or imported outline in one step.
