# Working from the keyboard

Everything in caditor works from the keyboard.

## Search commands

{command:palette} lists every command, including ones without a button, and once you type it also
finds features, parameters, saved views and selection sets by name. The line under the list says
what Enter will do, or why the highlighted command is not available now; pressing Enter on one
that is not available closes the palette and shows that reason as a notice.

- Commands are also found by other names they are known by: "zoom extents" finds
  {command:view.fit}, "ruler" finds {command:view.measure}, "home" the Isometric view. Features are
  found by their kind too, so "fillet" or "round" finds every fillet whatever it is called.
- Settings that are on or off, such as snapping or select through, show On or Off beside them.
  Turning one with no visible effect from the keyboard or the search says in the status bar what it
  is now.
- Recent lists the commands you last ran from the search, and caditor remembers them the next
  time it starts.

## Shortcuts

{command:file.shortcuts} lists every command with its keys. Search by the start of its words or
by keys, add or remove a binding, or reset one or all of them. A command whose keys you changed is
marked Changed, and Changed only lists just those. Taking keys another command uses, by adding
them or by resetting a command to keys someone else now has, asks first. Escape, Enter and Tab
cannot be bound. Shortcuts shown in this guide are your current ones.

## In the view

- The arrows orbit, Shift with the arrows pans, and Page Up and Page Down zoom; see
  [moving around the view](navigation). {command:view.previous} goes back to the view before.
- {command:view.highlight_next} and {command:view.highlight_previous} step through what is in the
  view; {command:view.activate_highlighted} selects it, as a click would, and Enter opens it as a
  double-click would.
- {command:view.context_menu} opens the view's right-click menu at the highlighted item, else at
  the selection; the arrows move through it and Enter runs an entry.
- In a sketch, the highlight also reaches constraints and dimensions, and Enter on a dimension
  edits its value. With {command:view.toggle_dimensions} on, it reaches the dimensions and values
  shown on the model the same way.
- With a drawing tool active, type a number, or `=` before a parameter's name, to place a point
  exactly, or use {command:sketch.type_value}; see [sketches](sketches).

## In panels and dialogs

Tab moves between fields and buttons, Enter confirms and Escape cancels. A value is committed when
you press Enter or leave the field; Escape puts back the old value. In dialogs, Enter runs the
highlighted main button.

Ctrl and Alt shortcuts work while a field has focus: the field is committed first, so
{command:file.save} saves a value you have just typed.
