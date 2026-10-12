# Configurations

A configuration is a named set of values for one model, so the sizes of one part (a bracket for M4,
M6 and M8 screws) live in one file. {command:model.configurations} opens the table.

- Each row is a configuration; the one marked active is the one the model shows.
- **Configure a value** adds a column: a [parameter](parameters)'s expression, whether a feature is
  suppressed, or a body's colour.
- **Add configuration**, rename, duplicate and delete manage the rows. Deleting the active
  configuration switches to the one below it, or the one above when it was the last.
- A parameter's cell is refused when, with the other values of that configuration, the parameter
  would depend on itself, so every configuration can always be switched to. To swap which of two
  values follows the other, first type a plain value into the one that uses the other.
- A parameter named in a configuration, as a column or in a cell, counts as in use. Deleting it
  writes its value into those cells and removes its column.

Switching is one change, so Undo takes it back, and the model recomputes as after any edit. Typing
the name of a configuration in the command search offers to switch to it.

Editing a configured value while a configuration is active changes that configuration too.

## Configurations in a spreadsheet

**Export to CSV** in the table (or {command:file.export_configurations}) writes one row per
configuration to a file a spreadsheet opens: the first column is `Configuration`, and every
configured value has a column headed by its kind and its name in the model:

- `Parameter: width` holds an expression such as `40 mm`, which may use other parameters.
- `Suppressed: Fillet 2` holds `yes` or `no`.
- `Colour: Body 1` holds `#rrggbb`, or `default` for the body's own colour.

**Import from CSV** (or {command:file.import_configurations}) reads such a file back and shows what
would happen before anything changes: which columns are known, which become configured values and
which name something the model does not have (a parameter, feature or body that is not there, or a
heading that does not start with `Parameter:`, `Suppressed:` or `Colour:`) and are left out, and
for each row whether it is new, replaces a configuration of the same name, or is left out with the
reason (a cell that cannot be read, a name used twice, parameters that depend on each other in a
circle). A blank cell leaves that value as it is. Unticking **Replace configurations the model
already has** keeps the configurations the model has and adds only the new ones. The import is one
change, so Undo takes it back. When the model has no active configuration the new rows are added
without switching to them.

## Exporting every configuration

When a model has two or more configurations, [Export](exporting) offers **Every configuration**:
it writes one file per configuration beside the chosen one, with the configuration's name added to
the file name, without changing the open model. Characters a file name cannot hold become `-`, and
configurations whose names would still make the same file, such as "M4/20" and "M4-20" or "A" and
"a", each get their own, the later one numbered: "bracket M4-20 (2).stl". When some of those files
already exist, one prompt lists them before anything is written, and Cancel leaves them as they
are. Cancel during the export keeps the files already written.
