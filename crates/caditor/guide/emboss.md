# Emboss

{command:model.emboss} raises the closed outlines of a sketch from a face of a body, or sinks them
into it, by a depth. Draw the outlines or write [text](text) on the face (or on a plane parallel to
it), then select the face and the sketch and choose Emboss. A sketch drawn on a face needs nothing
else selected: Emboss takes that face. It has no ribbon button; use the Model menu, the right-click
menu of faces or {command:palette}.

- **Faces to emboss** lists the faces the outlines stand on. Click faces in the view to add them or
  leave them out; **Choose in the view** first adds the faces of the body selected then. Several
  faces must lie in one plane, or on one cylinder or cone.
- **Outlines** is the sketch, chosen from the earlier ones or by clicking one of its curves or
  letters. Every closed shape is used and the holes inside shapes stay open, so the middle of an
  `o` or a `B` is left as it is. Curves that enclose nothing are ignored.
- **Depth** is how far the outlines stand out or sink in, as a length or an expression using your
  [parameters](parameters). A new emboss starts from the depth last set on one, kept between
  sessions.
- **Raise** adds the outlines to the face; **Recess** cuts them into it.

A flat face takes the outlines straight, as an extrusion from the face would: the sketch must lie
parallel to it. On a face of a cylinder or cone the outlines are wrapped round it the way
[Split face](split-face) wraps them: the sketch lies on a plane along the axis, such as one touching
it, its distances along the axis stay as they are and those across it become lengths round the
surface, so text drawn 30 mm long wraps into text 30 mm round. The depth is then measured square to
the surface, out of it for Raise and into it for Recess, and a bore takes the outlines the same way.
A sphere or a torus cannot be unrolled flat, so those faces are refused.

While it is open the body is shown with the emboss, and each face listed in the panel lights up in
the view when hovered.

The raised tops and recessed floors are faces of the feature, named from it, so a later
[fillet](fillet-and-chamfer) can round the letters or a colour can be given to them, and they keep
their names when the text or the depth changes. The outlines must lie wholly inside the chosen
faces: one reaching the edge of a face, or running off it, fails the emboss alone, saying which
outline and face. A depth that reaches the axis of a cylinder, or a recess that leaves nothing of
the body, fails it too.
