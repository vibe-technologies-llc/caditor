# Split face

{command:model.split_face} divides faces of a body into pieces without changing its shape, so a
part line, a stripe of another colour or a piece to draft or round can be had. Select the faces,
and with them a plane or the curves of a sketch to split along; with nothing else, they are split
along the YZ plane. It has no ribbon button; use the Model menu or {command:palette}.

- **Split along** holds the principal and datum planes, the other bodies made before it and the
  earlier sketches. A plane divides the faces where it crosses them. A sketch of one open chain of
  curves is carried straight on past its ends and swept through the body; a sketch of closed
  outlines is swept through the body the same way, so a circle marks a round patch on the faces it
  crosses. A sketch holding both, one open chain and closed outlines apart from it, divides the
  faces along all of them. Another body divides the faces where its surface crosses them. **Use
  selected** and **Choose in the view** take a plane, a flat face, a sketch curve or another body,
  as {command:model.split_along_selected} does.
- **Carried**, shown for a sketch, says which way its curves are swept: **Square to the sketch**,
  or **Along an edge or axis**, which takes the selected straight edge, axis, round face or sketch
  line (or lets you click one in the view) so the curves land slanted on the faces, as a drawing
  projected at an angle would. A direction lying in the sketch's plane fails the split, saying so.
- Only the chosen faces are divided: a plane crossing the whole body leaves the other faces whole.

The pieces are named from the split and the side they lie on (in front of the plane or behind it,
inside the outline or outside it, inside the other body or outside it), so a colour, fillet or
draft given to one piece stays on it when the model changes above the split. A face colour given
to a face before the split was added above it carries to every piece of that face.

While it is open, the body is shown as it was before the split with the chosen faces marked and
the new edges drawn over them; click faces to split them or leave them out, and hovering a face
listed in the panel lights it in the view. A tool that crosses none of the chosen faces fails the
split alone, saying so.
