# Fillet and chamfer

{command:model.fillet} rounds edges of a body and {command:model.chamfer} bevels them. Select one
or more edges of one body, then choose the tool.

While it is open, the edges of the body before it can be clicked in the view to add or leave them
out; the panel lists the chosen ones in words, each with a button to leave it out, and hovering
one lights it in the view. Edges that continue smoothly are taken together as a chain.

- Fillet takes a **Radius**, Chamfer a **Distance**.
- A chamfer's **Distances** row chooses **Equal** (one distance on both faces), **Two** (a second
  distance on the other face) or **Angle** (a distance and the angle from the first face).
  **Measure from the other face** swaps which face takes the first distance. The same choices are
  {command:model.chamfer_equal}, {command:model.chamfer_two_distances},
  {command:model.chamfer_distance_angle} and {command:model.flip_chamfer}.
- Typing a value previews it before you press Enter; when it cannot be made, the reason shows under
  the field and the body before it stays shown.

{command:select.tangent_edges} adds the edges continuing the selected ones, and a
[selection set](selection) can hold edges you fillet often. To round sketch corners instead, see
[Sketch fillet](sketch-fillet).
