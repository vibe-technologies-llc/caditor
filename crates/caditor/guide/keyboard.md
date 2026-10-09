# Working from the keyboard

Everything in caditor works from the keyboard.

## Search commands

{command:palette} lists every command, including ones without a button, and once you type it also
finds features, parameters, saved views and selection sets by name. The line under the list says
what Enter will do, or why the highlighted command is not available now.

## Shortcuts

{command:file.shortcuts} lists every command with its keys. Search by name or by keys, add or
remove a binding, or reset one or all of them. Escape, Enter and Tab cannot be bound. Shortcuts
shown in this guide are your current ones.

## In the view

- The arrows orbit, Shift with the arrows pans, and Page Up and Page Down zoom; see
  [moving around the view](navigation).
- {command:view.highlight_next} and {command:view.highlight_previous} step through what is in the
  view; {command:view.activate_highlighted} selects it, as a click would, and Enter opens it as a
  double-click would.
- In a sketch, the highlight also reaches constraints and dimensions, and Enter on a dimension
  edits its value.
- With a drawing tool active, type a number to place a point exactly; see
  [sketches](sketches).

## In panels and dialogs

Tab moves between fields and buttons, Enter confirms and Escape cancels. A value is committed when
you press Enter or leave the field; Escape puts back the old value. In dialogs, Enter runs the
highlighted main button.

Ctrl and Alt shortcuts work while a field has focus: the field is committed first, so
{command:file.save} saves a value you have just typed.
