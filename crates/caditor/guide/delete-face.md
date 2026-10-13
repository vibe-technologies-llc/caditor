# Delete face

{command:model.delete_face} removes faces of a body and closes the gap they leave by extending the
faces around them until they meet, so a fillet, a chamfer, a hole or a boss goes without remodelling
the body. It works on any body, an [imported](imported-bodies) one with no history included. Select
the faces, then choose Delete face. It has no ribbon button; use the Model menu or
{command:palette}.

- A fillet or chamfer goes when its faces are chosen: the faces on either side grow back to the sharp
  edge. Choose the whole chain of a rounded rim at once, corner patches included; one face of a
  chain alone cannot be closed. {command:select.feature_faces} picks every face a fillet made, and
  {command:select.similar} every round of the same radius, an imported body's too.
- A hole or pocket goes when every face inside it is chosen, the walls and the floor of a blind one:
  the face it was cut into closes over it. {command:select.hole} picks a whole hole.
- A boss goes when its side and top are chosen: the face it stood on closes under it.
- A chamfer or rounded rim of a hole goes alone, leaving the hole with a sharp rim.

While it is open, the body is shown as it will be and the deleted faces of the body before it are
drawn see-through in the selected colour; click a face to delete it, or a see-through one to keep
it. The panel lists the chosen faces in words, each with a button to keep it, and hovering one
lights it in the view. **Choose in the view** in the panel first adds the faces of the body
selected then.

The faces that stay keep their names, so later features still find them. When the faces around the
gap cannot close it, the feature fails alone and says which face to choose with it.
