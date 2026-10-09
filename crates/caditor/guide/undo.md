# Undo and redo

Every change to the model can be undone: sketch edits, feature values, parameters, reordering,
hiding and restoring an older version alike.

- {command:edit.undo} takes back the last change and {command:edit.redo} brings it back.
- {command:edit.undo_history} lists the steps Undo and Redo hold. Hover one to see what it changed;
  click it to go back or forward to that point in one go.

Undo history survives a crash: the [recovery journal](saving) records undo and redo too, so a
restored session can still undo.

Things that are not part of the model are not undone: the camera, the selection, the side panels
and preferences. Restoring preferences to their defaults offers its own Undo button.
