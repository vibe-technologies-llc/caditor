# Parameters

Parameters are named values the whole model can use: a wall thickness, a hole spacing, a count.
Change one and every sketch and feature using it follows.

## The Parameters section

The Parameters section of the model panel lists them with a name, an
[expression](expressions) and the value it gives. {command:model.add_parameter} adds one.

- Type a name and an expression, such as `wall` and `2.5 mm`. An expression may use other
  parameters: `inner = outer - 2 * wall`.
- Hover a value to see what uses the parameter. A parameter nothing uses has a muted mark before
  its value.
- Right-click a row to move it up or down, add a note or delete it.
- Deleting a parameter in use writes its expression into each place that used it, so nothing
  changes shape. Undo brings it back.
- A value that cannot be worked out shows an error mark; hover it for the reason.

## Naming a value where it is typed

In any value field, of a feature or a sketch dimension, type `name = expression`, such as
`depth = 12 mm`. That creates a parameter of that name holding the expression, and the field uses
it. These named values are listed under Model parameters, each with the feature or dimension it
belongs to. Typing a plain expression in the field again unnames it.

## Finding and sharing

{command:palette} finds parameters by name and takes you to the value.
{command:file.export_parameters} writes them to a CSV file a spreadsheet opens;
{command:file.import_parameters} reads one back and shows what would be added, changed or left out
before anything is applied. The import is one change, so Undo takes it back.
