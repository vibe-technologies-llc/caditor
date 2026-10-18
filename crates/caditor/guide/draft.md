# Draft

{command:model.draft} tilts flat faces of a body so a mould can release it: each chosen face turns
about the line where it meets the **neutral plane** until it makes the **draft angle** with the
**pull direction**, and the faces around it extend or trim to follow. It works on any body, an
[imported](imported-bodies) one with no history included. Select the faces, such as the walls of a
block or a pocket, then choose Draft. It has no ribbon button; use the Model menu or
{command:palette}.

- **Draft angle** is how far each face leans from the pull direction, narrowing the body toward it;
  a negative angle widens it instead. 3 deg to start with.
- **Own angle** lists the faces drafted at an angle of their own instead of the draft angle. **Choose
  faces** starts choosing in the view: each click on a flat face of the body gives it its own angle
  (the draft angle to start with) or the draft angle again, until Escape. Each listed face has its
  own angle field; its remove button gives it the draft angle again.
- **Neutral plane** is where the faces keep their place: a principal plane, a datum plane or a flat
  face of a body, from the list or with **Choose in the view**. To start with it is the flat face the
  first chosen face stands on, such as the floor of a block or a pocket.
- **Pull direction** is the way the mould comes off, square to the neutral plane. The panel says
  which way it points; **Flip the pull direction** turns it round. Choosing a neutral plane points it
  at the faces again.
- **Draft the faces tangent to these too** takes every face joining a chosen one smoothly, so a chain
  of faces drafts as one. Only flat faces can be drafted: a round face in the chain fails the draft
  saying which one.
- **Split the faces at the neutral plane** is a parting line: a face crossing the neutral plane is
  split there, the part on the pull side drafted by the draft angle (or its own angle) and the part
  beyond by **Angle beyond**, narrowing the body the other way, as for a part moulded in two halves.

While it is open, click faces in the view to draft them or leave them out; hovering a face listed in
the panel lights it in the view. **Choose in the view** in the panel first adds the faces of the body
selected then.

The drafted faces keep their names, so later features still find them. A face along the neutral
plane, a curved face, an angle of 89 deg or more, or a draft that would make a face vanish or faces
cross fails the feature alone, saying which face or edge and what to do. Draft before rounding the
edges beside the faces, since a fillet along a drafted face's edge cannot follow it. Every angle
accepts units, expressions and parameters.
