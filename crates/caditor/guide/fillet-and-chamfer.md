# Fillet and chamfer

{command:model.fillet} rounds edges of a body and {command:model.chamfer} bevels them. Select one
or more edges of one body, then choose the tool. A selected face stands for every edge around it,
and a body chosen in the tree, with nothing selected in the view, for every edge of the body. The
face or body itself is kept, not the edges it had then: an edge a later change adds to that face
or body (a hole cut into the face, say) is rounded too, and the panel lists such an entry as
**All edges of** the face or body.

A fillet or chamfer can also take edges by a rule, kept as the model changes. **Add edges by rule**
in the panel offers **Every convex edge** (the outside corners of the body, including the rim of a
pocket), **Every concave edge** (inside corners, such as a pocket's floor and walls) and **Every
edge made by** one of the features before it, which takes the edges that feature made where it
cut or joined the body, such as a pocket's rim, floor and corners. Edges that continue smoothly
(already rounded, say), and the line where two holes touch, are left out of the convex and concave
rules, and a face or body taken whole leaves out that line too: the walls meet back to back there,
so no fillet or chamfer of any size fits, and choosing that edge alone is refused saying so. The panel lists the rule with
the number of edges it takes now, and an edge a later change adds or removes follows the rule.
{command:model.add_convex_edges} and {command:model.add_concave_edges} do the same from the
palette, and {command:model.add_edges_of_selected_feature} takes the edges made by the feature of
each selected face. A rule that matches no edge now is refused, saying so. Leaving out one edge of
a rule turns that entry into its edges as they are now, like a face or body taken whole.

While it is open, the edges of the body before it can be clicked in the view to add or leave them
out; the panel lists the chosen ones in words, each with a button to leave it out, and hovering
one lights it in the view. Leaving out one edge of a face or body taken whole turns that entry into
its edges as they are now, less the one left out. Edges that continue smoothly are taken together
as a chain.

An edge ending where earlier fillets meet, such as an upright edge of a box whose top edges are
already rounded, is rounded or bevelled up into them. A fillet of the same radius ends in the
round ball corner that filleting all the edges at once makes; any other size, or a chamfer, meets
the earlier fillets along the curve where they cross. An upright edge of a boss whose foot was
rounded earlier cannot be closed off there yet: the fillet fails, saying to move it above the one
that rounded the foot.

An edge chosen along an extruded or revolved side keeps all of itself when its sketch curve is
later cut in two by [Trim](trim-and-extend) or split: every piece along the old edge stays rounded,
and the panel lists the edge as split into pieces. An edge chosen after the cut stays on its own
piece. When a piece no longer lies along the old edge, the fillet stops and says the edge was
split, so you can choose the pieces again rather than lose one without notice.

- Fillet takes a **Radius**, Chamfer a **Distance**. An arrow at the middle of the first edge,
  standing on the rounded or cut face, drags it, on convex and concave edges alike; type a value
  with the pointer on the arrow to set it exactly. A chamfer of **Two** distances has an arrow on
  each face where its distance ends, and one of **Angle** an arrow for the distance on the face
  it is measured from and a turning arrow where the cut meets the other face, which drags the
  angle. On a curved face the arrow stands on the face itself and follows its curve as you drag.
- A chamfer's **Distances** row chooses **Equal** (one distance on both faces), **Two** (a second
  distance on the other face) or **Angle** (a distance and the angle from the first face).
  **Measure from the other face** swaps which face takes the first distance. The same choices are
  {command:model.chamfer_equal}, {command:model.chamfer_two_distances},
  {command:model.chamfer_distance_angle} and {command:model.flip_chamfer}.
- A new fillet or chamfer starts from the radius, distances and form last used on one, kept
  between sessions; a value naming a parameter the model lacks falls back to the usual 1 mm.
- Typing a value previews it before you press Enter; when it cannot be made, the reason shows under
  the field and the body before it stays shown.

A [pattern](patterns) or [mirror](mirror) of features repeats a fillet or chamfer chosen with the
features it blends, so every copy is rounded too.

{command:select.tangent_edges} adds the edges continuing the selected ones, and a
[selection set](selection) can hold edges you fillet often. [Delete face](delete-face) takes a
fillet or chamfer off again, an imported body's too. To round sketch corners instead, see
[Sketch fillet](sketch-fillet).
