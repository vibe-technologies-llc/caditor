# Replace face

{command:model.replace_face} puts flat faces of a body onto another plane: each chosen face turns and
moves until it lies on the **target**, and the faces around it extend or trim to meet it there. It
works on any body, an [imported](imported-bodies) one with no history included, so a wall of the
wrong height or angle on a STEP part is corrected without remodelling it. Select the faces, and
optionally the plane or face to replace them with, then choose Replace face. It has no ribbon
button; use the Model menu, the faces' right-click menu or {command:palette}.

- **Faces to replace** are flat faces of one body. Several faces can go onto one target at once,
  such as the tops of two bosses brought to one height.
- **Replace with** is the target: a principal plane, a datum plane, a plane of a coordinate system
  or a flat face of any body made before, this one included, from the list or with **Choose in the
  view**. A face of another body selected with the faces becomes the target when the feature is
  made, and so does a selected plane; with none selected the feature waits for one.

Each face keeps the side it faced: a target facing the other way does not turn the face over, so
replacing a top with the plane of another body's top raises or lowers it to that height. A target
leaning from the face tilts it, its neighbours trimmed or extended along the new slope. The target
must lean less than 89 deg from each face.

While it is open, click faces in the view to replace them or leave them out; hovering a face listed
in the panel lights it in the view. **Choose in the view** in the panel first adds the faces of the
body selected then.

The replaced faces keep their names, so later features still find them. A curved face, a target
square to a face, a target on or past the far side of the body (which would turn it inside out), two
faces meeting along an edge that would both lie on the target, or a target that would make a face
vanish or faces cross fails the feature alone, saying which face or edge and what to do. Replace
before rounding the edges beside the faces, since a fillet along a replaced face's edge cannot
follow it.
