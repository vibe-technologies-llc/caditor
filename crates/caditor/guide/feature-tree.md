# The feature tree

The Features section of the model panel lists the steps that build the model, computed from top to
bottom. Each feature uses only what is above it.

## Opening and changing a feature

Double-click a row, press Enter on it or use its pencil button to open it: a sketch for editing,
another feature with its settings under the row. Every change applies at once and can be undone;
the check mark, Enter or Escape closes it, keeping the changes. The cross beside the check mark,
{command:model.cancel_feature}, closes it and takes back every change made since it was opened,
or the whole feature when the tool that made it opened it, as ordinary undo steps that Redo brings
back. When something else in the model changed meanwhile, or Undo already went back past the
opening, it says so and changes nothing. Double-clicking a face in the view opens the feature that
made it, and {command:model.edit_feature} with no row chosen opens the feature of the one face,
edge, datum or sketch curve selected.

The settings of a suppressed feature, or one below the rollback bar, are shown but cannot be
changed; the card says why and offers Unsuppress or Roll forward to here.

- {command:model.rename_feature} renames the chosen row.
- Drag a row to move it, or use {command:model.move_feature_up} and {command:model.move_feature_down};
  a move that would put a feature above something it uses is refused, saying why.
- {command:model.suppress_feature} switches a feature off without deleting it. Suppressed rows are
  struck through.
- {command:model.delete_feature} deletes the chosen rows. When others depend on them, a dialog
  lists those and lets you delete them too or keep them.

Click a row to choose it; Ctrl-click and Shift-click choose several. While the tree is filtered,
Shift-click takes only the rows shown. Escape with nothing selected in the view, or a click on
empty space in the view, lets go of the chosen rows. Hovering a row lights what it made in the
view.

The eye on a row hides or shows what it made. The "⋯" button holds the same actions as the
right-click menu; Hide or Show there, like Suppress, acts on every chosen row. Uses and Used by
list the features it builds on and those built on it; choose one to go to its row.

## The rollback bar

The bar at the end of the tree marks where the model stops. Drag it up, or use
{command:model.rollback_up} and {command:model.roll_to_here}, to see the model as it was at that
point; new features are added where the bar stands. {command:model.roll_to_end} brings it back.

## When a feature fails

A failed feature is marked in its row with what went wrong and what to do; features that do not
depend on it keep working. Show where frames the place in the view, and when the fix lies in the
feature's own settings its Edit button opens it. {command:model.first_failed} goes to the next
failure after the chosen row and {command:model.previous_failed} to the one before, both wrapping
around. A feature whose reference had to be found again shows a warning with Update references.

The model recomputes in the background, so the window never waits. {command:model.recompute} runs
it again after a stop.

## Organising

- {command:model.group_features} puts the chosen rows in a folder; double-click it to rename.
- {command:model.filter_features} filters the rows by name, by kind, such as "fillet", or by
  state: "failed", "outdated", "suppressed" or "hidden".
- {command:model.copy_features} and {command:model.paste_features} copy features through the
  clipboard, within a model or into another.

## Kinds of features

- From sketches: [Extrude](extrude), [Revolve](revolve) and [Hole](hole).
- Ready-made shapes: [Primitives](primitives).
- Changing a body: [Fillet and chamfer](fillet-and-chamfer), [Shell](shell),
  [Offset face](offset-face) and [Thread](thread).
- Several bodies: [Combine](combine), [Split](split), [Move and copy](move-and-copy),
  [Mate](mate), [Mirror](mirror), [Scale](scale) and [Patterns](patterns).
- Reference geometry: [Datums](datums) and [Coordinate systems](coordinate-systems).

See also [bodies](bodies), [imported bodies](imported-bodies) and [parameters](parameters).
