# Constrain automatically

The Constrain automatically panel works out constraints for you on the edited sketch. It has three
tasks:

- {command:sketch.find_relations} finds the relations the drawing already shows: lines that look
  horizontal, ends that touch, equal lengths. Tick the kinds to look for and set how loose a match
  may be.
- {command:sketch.dimension_from_datum} adds the dimensions that leave the sketch fully
  constrained, measured from the selected point or the origin.
- {command:sketch.check} looks for flaws the eye misses: ends that nearly meet, curves of no
  length and curves lying on others, each with its fix where there is one.

The panel lists what it proposes and lights it in the view; hover a row to see that item alone. The
button at the bottom adds the whole proposal as one change, which Undo takes back. A flaw's own
button fixes just that one.

The panel works on the sketch as last solved and is redone after every change. It closes when you
finish the sketch.

See also [how constrained a sketch is](sketch-status).
