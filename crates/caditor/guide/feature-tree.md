# The feature tree

The Features section of the model panel lists the steps that build the model, computed from top to
bottom. Each feature uses only what is above it.

## Opening and changing a feature

Double-click a row, press Enter on it or use its pencil button to open it: a sketch for editing,
another feature with its settings under the row. Every change applies at once and can be undone;
the check mark, Enter or Escape closes it. Double-clicking a face in the view opens the feature that
made it.

- {command:model.rename_feature} renames the chosen row.
- Drag a row to move it, or use {command:model.move_feature_up} and {command:model.move_feature_down};
  a move that would put a feature above something it uses is refused, saying why.
- {command:model.suppress_feature} switches a feature off without deleting it. Suppressed rows are
  struck through.
- {command:model.delete_feature} deletes the chosen rows. When others depend on them, a dialog
  lists those and lets you delete them too or keep them.

Click a row to choose it; Ctrl-click and Shift-click choose several. The eye on a row hides or
shows what it made. The "⋯" button holds the same actions as the right-click menu.

## The rollback bar

The bar at the end of the tree marks where the model stops. Drag it up, or use
{command:model.rollback_up} and {command:model.roll_to_here}, to see the model as it was at that
point; new features are added where the bar stands. {command:model.roll_to_end} brings it back.

## When a feature fails

A failed feature is marked in its row with what went wrong and what to do; features that do not
depend on it keep working. Show where frames the place in the view, and
{command:model.first_failed} takes you to the first failure. A feature whose reference had to be
found again shows a warning with Update references.

The model recomputes in the background, so the window never waits. {command:model.recompute} runs
it again after a stop.

## Organising

- {command:model.group_features} puts the chosen rows in a folder; double-click it to rename.
- {command:model.filter_features} filters the rows by name or kind, such as "fillet".
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
