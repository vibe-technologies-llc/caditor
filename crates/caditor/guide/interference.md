# Check interference

{command:view.interference} opens a panel that looks for bodies overlapping or touching. It never
changes the model.

- With nothing chosen, it checks every pair of shown bodies.
- With one body chosen (a face, edge or vertex of it, or its row in the tree), it checks that body
  against every other.
- With several chosen, it checks them against each other.

Each finding is a card: an overlap gives the volume and size of the shared material, a touch where
the bodies meet. **Show where** frames it in the view, where overlaps are outlined and every finding
is marked. Findings appear as each pair is checked, and only pairs whose bodies changed are checked
again after an edit.
