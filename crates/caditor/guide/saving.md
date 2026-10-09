# Saving and recovery

## Saving

{command:file.save} writes the model to its `.caditor` file; {command:file.save_as} writes it under
a new name. The title shows Unsaved, and the window's title a `*`, while there are changes not yet
saved. Closing, opening another model or quitting asks first: Save, Cancel, or close without saving.

A save never overwrites the file in place. caditor writes a new copy beside it, makes sure it is on
the disk and only then puts it in place, so a crash or a full disk during a save leaves the old file
whole. If another program changed the file since you opened it, the save asks whether to save a
copy or replace it; replacing keeps the other contents as a [version](version-history).

{command:file.revert} throws away the changes since the last save in one step; Undo brings them
back.

## Opening

{command:file.open} starts in the folder a model was last opened from. File › Open recent lists the
models opened or saved lately. One that cannot be found any more is shown greyed out with "(not
found)"; opening it anyway says why and takes it off the list. Hover an entry for its remove button
to take it off the list alone, or use {command:file.forget_recent_1} and its numbered neighbours;
the file itself is never touched.

## The recovery journal

From the first change, every change is also written to a hidden journal beside the file (for an
untitled model, in caditor's own folder), within seconds. If caditor, the computer or the power
fails, the next start offers to restore the unsaved work, undo history included;
{command:file.recover} offers it again later. Saving clears what the journal held.

When the journal cannot be written, for example on a full or read-only disk, the status bar shows
**Not protected** until it can be written again. Save soon, or Save as somewhere else.

## Damaged files

A damaged file opens with everything that can still be read, and a report says what was lost; one
bad feature never stops the rest from opening. Saving keeps a copy of the damaged original.

See also [version history](version-history), [exporting](exporting), [templates](templates) and,
on Windows, [Microsoft Defender](windows-defender).
