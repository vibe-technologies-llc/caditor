# Undo and redo

Every change to the model can be undone: sketch edits, feature values, parameters, reordering,
hiding and restoring an older version alike.

- {command:edit.undo} takes back the last change and {command:edit.redo} brings it back. The Edit
  menu names the step each would take, such as Undo Extrude 1 distance.
- {command:edit.repeat} runs the last modelling or sketch command again, such as Fillet, a datum
  plane or a hole, on whatever is selected now. The Edit menu, the palette and the view's
  right-click menu name it, such as Repeat Fillet.
- {command:model.cancel_feature} undoes every step made since the open feature was opened, and
  closes it ([the feature tree](feature-tree)).
- {command:edit.undo_history} lists the steps Undo and Redo hold, with Now beside the change the
  model is at. Hover one to see what it changed; click it to go back or forward to that point in
  one go, or click Before these changes to go back to before every change listed.
- {command:file.revert} goes back to the model as it was last saved, in one step that Undo takes
  back.

Undo history survives a crash: the [recovery journal](saving) records undo and redo too, so a
restored session can still undo.

Things that are not part of the model are not undone: the camera, the selection, the side panels
and preferences. Restoring preferences to their defaults offers its own Undo button.
