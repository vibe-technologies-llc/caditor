# Split face

{command:model.split_face} divides faces of a body into pieces without changing its shape, so a
part line, a stripe of another colour or a piece to draft or round can be had. Select the faces,
and with them a plane or the curves of a sketch to split along; with nothing else, they are split
along the YZ plane. It has no ribbon button; use the Model menu or {command:palette}.

- **Split along** holds the principal and datum planes, the other bodies made before it and the
  earlier sketches. A plane divides the faces where it crosses them. A sketch of one open chain of
  curves is carried straight on past its ends and swept through the body; a sketch of closed
  outlines is swept through the body the same way, so a circle marks a round patch on the faces it
  crosses. A sketch holding both, one open chain and closed outlines, divides the faces along all
  of them; the chain may start at, end at or pass through a corner of an outline, and is then
  carried straight on past that corner across the outline as past any other end. Another body divides the faces where its surface crosses them, and a
  curved face of another body where its whole surface, carried on past its edges, crosses them. **Use selected**
  and **Choose in the view** take a plane, a face, a sketch curve or another body, as
  {command:model.split_along_selected} does.
- **Carried**, shown for a sketch, says which way its curves are swept: **Square to the sketch**,
  or **Along an edge or axis**, which takes the selected straight edge, axis, round face or sketch
  line (or lets you click one in the view) so the curves land slanted on the faces, as a drawing
  projected at an angle would. A direction lying in the sketch's plane fails the split, saying so.
  **Wrapped round the faces** (also {command:model.wrap_split_curves}) lays closed outlines onto
  faces of one cylinder the way a label is wrapped round a bottle: the sketch lies on a plane
  along the cylinder's axis, such as one touching it, its distances along the axis stay as they
  are and those across it become lengths round the cylinder, so a rectangle drawn 30 mm long
  across the axis wraps into a patch 30 mm round and a slanted edge becomes a helix. Only closed
  outlines can be wrapped, and they must fit within once round the cylinder; a plane across the
  axis, a face that is not cylindrical, faces of several cylinders, open curves or outlines longer
  than the way round fail the split, saying so.
- Only the chosen faces are divided: a plane crossing the whole body leaves the other faces whole.

The pieces are named from the split and the side they lie on (in front of the plane or behind it,
inside the outline or outside it, inside the other body or outside it), so a colour, fillet or
draft given to one piece stays on it when the model changes above the split. A face colour given
to a face before the split was added above it carries to every piece of that face.

While it is open, the body is shown as it was before the split with the chosen faces marked and
the new edges drawn over them; click faces to split them or leave them out, and hovering a face
listed in the panel lights it in the view. A tool that crosses none of the chosen faces fails the
split alone, saying so.
