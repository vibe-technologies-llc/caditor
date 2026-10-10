# Fillet and chamfer

{command:model.fillet} rounds edges of a body and {command:model.chamfer} bevels them. Select one
or more edges of one body, then choose the tool. A selected face stands for every edge around it,
and a body chosen in the tree, with nothing selected in the view, for every edge of the body. The
face or body itself is kept, not the edges it had then: an edge a later change adds to that face
or body (a hole cut into the face, say) is rounded too, and the panel lists such an entry as
**All edges of** the face or body.

While it is open, the edges of the body before it can be clicked in the view to add or leave them
out; the panel lists the chosen ones in words, each with a button to leave it out, and hovering
one lights it in the view. Leaving out one edge of a face or body taken whole turns that entry into
its edges as they are now, less the one left out. Edges that continue smoothly are taken together
as a chain.

- Fillet takes a **Radius**, Chamfer a **Distance**.
- A chamfer's **Distances** row chooses **Equal** (one distance on both faces), **Two** (a second
  distance on the other face) or **Angle** (a distance and the angle from the first face).
  **Measure from the other face** swaps which face takes the first distance. The same choices are
  {command:model.chamfer_equal}, {command:model.chamfer_two_distances},
  {command:model.chamfer_distance_angle} and {command:model.flip_chamfer}.
- A new fillet or chamfer starts from the radius, distances and form last used on one, kept
  between sessions; a value naming a parameter the model lacks falls back to the usual 1 mm.
- Typing a value previews it before you press Enter; when it cannot be made, the reason shows under
  the field and the body before it stays shown.

{command:select.tangent_edges} adds the edges continuing the selected ones, and a
[selection set](selection) can hold edges you fillet often. To round sketch corners instead, see
[Sketch fillet](sketch-fillet).
