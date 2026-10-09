# Configurations

A configuration is a named set of values for one model, so the sizes of one part (a bracket for M4,
M6 and M8 screws) live in one file. {command:model.configurations} opens the table.

- Each row is a configuration; the one marked active is the one the model shows.
- **Configure a value** adds a column: a [parameter](parameters)'s expression, whether a feature is
  suppressed, or a body's colour.
- **Add configuration**, rename, duplicate and delete manage the rows.

Switching is one change, so Undo takes it back, and the model recomputes as after any edit. Typing
the name of a configuration in the command search offers to switch to it.

Editing a configured value while a configuration is active changes that configuration too.

## Exporting every configuration

When a model has two or more configurations, [Export](exporting) offers **Every configuration**:
it writes one file per configuration beside the chosen one, with the configuration's name added to
the file name, without changing the open model. Cancel keeps the files already written.
