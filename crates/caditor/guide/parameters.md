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
- With six or more parameters a filter field heads the section; it keeps the rows whose name,
  expression or owner contains what is typed.
- {command:model.delete_unused_parameters}, also the button under the tables, deletes every
  parameter nothing refers to in one change.
- Deleting a parameter in use writes its expression into each place that used it, so nothing
  changes shape. Undo brings it back.
- A value that cannot be worked out shows an error mark; hover it for the reason.

## Naming a value where it is typed

In any value field, of a feature or a sketch dimension, type `name = expression`, such as
`depth = 12 mm`. That creates a parameter of that name holding the expression, and the field uses
it. These named values are listed under Model parameters, each with the feature or dimension it
belongs to; click that line to go to the feature or dimension. Typing a plain expression in the field again unnames it.

Dragging an arrow or ring in the view keeps a named value: the drag changes the parameter's
expression, so the name stays. A value that uses other parameters, such as `depth` or
`h = depth * 2`, does not drag; hovering its arrow says which parameters to change instead.

## Finding and sharing

{command:palette} finds parameters by name and takes you to the value. A value read in
[Measure](measure) becomes a parameter from its row's menu.
{command:file.export_parameters} writes them to a CSV file a spreadsheet opens;
{command:file.import_parameters} reads one back and shows what would be added, changed or left out
before anything is applied. The import is one change, so Undo takes it back. The file starts with a
`caditor-parameters` row naming its format; keep it when you edit the file in a spreadsheet. A file
without it, such as one an older version of caditor wrote, reads `10 mm^2` the way that version did,
as (10 mm)², so add the row only to a file written the current way.

To keep several sets of values in one model, see [Configurations](configurations).
