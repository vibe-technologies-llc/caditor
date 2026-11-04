# Replace face

{command:model.replace_face} puts faces of a body onto another plane or round face: each chosen face
turns and moves until it lies on the **target**, and the faces around it extend or trim to meet it
there. It works on any body, an [imported](imported-bodies) one with no history included, so a wall of the
wrong height or angle on a STEP part is corrected without remodelling it. Select the faces, and
optionally the plane or face to replace them with, then choose Replace face. It has no ribbon
button; use the Model menu, the faces' right-click menu or {command:palette}.

- **Faces to replace** are faces of one body. Several faces can go onto one target at once, such as
  the tops of two bosses brought to one height.
- **Replace with** is the target: a principal plane, a datum plane, a plane of a coordinate system
  or a face of any body made before, this one included, from the list or with **Choose in the
  view**. A face of another body selected with the faces becomes the target when the feature is
  made, and so does a selected plane; with none selected the feature waits for one.

Each face keeps the side it faced: a target facing the other way does not turn the face over, so
replacing a top with the plane of another body's top raises or lowers it to that height. A target
leaning from the face tilts it, its neighbours trimmed or extended along the new slope. The target
must lean less than 89 deg from each face.

Faces that would lie on the target beside one another become one face. So the lower top of a step
replaced with the plane of the higher top levels the step, its riser going; a floor replaced with
the top of the body fills its pocket or blind hole; and two slopes of a roof replaced with one plane
become one flat top. The merged face takes the name of a replaced face, so later features holding
that face still find it, and those holding the other face are told to choose it again.

A round face follows a round target about its own axis, its neighbours extending or trimming to
meet it: a boss or bore replaced with a cylinder on the same axis takes that cylinder's radius, and
a cone, sphere or torus likewise. A cylinder also follows one of its own radius and direction
elsewhere, shifting across to it. Only flat faces go onto a plane and only round faces onto a round
target.

While it is open, click faces in the view to replace them or leave them out; hovering a face listed
in the panel lights it in the view. **Choose in the view** in the panel first adds the faces of the
body selected then.

The replaced faces keep their names, so later features still find them. A target square to a face,
a target on or past the far side of the body (which would turn it inside out), a round target no
motion can bring the face to, or a target that would make a face vanish, other than one squeezed
flat between two faces lying on the target, or faces cross fails the feature alone, saying which
face or edge and what to do. Replace before rounding the edges beside the faces, since a fillet
along a replaced face's edge cannot follow it.
