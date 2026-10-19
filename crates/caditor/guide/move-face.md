# Move face

{command:model.move_face} shifts faces of a body along a direction and tilts them about an axis, the
faces around them extending or trimming to follow, so a wall at the wrong angle, a hole in the wrong
place or a boss on the wrong spot is put right without remodelling the body. It works on any body,
an [imported](imported-bodies) one with no history included. Select the faces, then choose Move face.
It has no ribbon button; use the Model menu or {command:palette}.

- **Distance** shifts the faces; a negative distance shifts them the other way. Typing previews it,
  and the arrow on the first chosen face, along the direction, drags it.
- **Direction** is what they shift along: square to the first flat face chosen (out of the body
  for a positive distance), or an axis, a straight edge, a round face's axis or a sketch line, from
  the list or with **Choose in the view**.
- **Angle** tilts the faces about **Tilt about**, an axis or straight edge; tilt a wall about the edge
  it stands on to lean it. A negative angle tilts the other way. The tilt comes first, then the shift. Once an axis is chosen
  an arrow on the edge of the first chosen face turns the faces about it when dragged.

To move a hole, choose its wall (and the floor of a blind one) and a direction across the face it is
cut into, such as the X or Y axis; {command:select.hole} picks a whole hole. To move a boss, choose
its side and top. A fillet or chamfer along a moved face's edge has to move with it, or the move fails
saying which edge cannot follow.

While it is open, click faces in the view to move them or leave them out; hovering a face listed in
the panel lights it in the view. **Choose in the view** in the panel first adds the faces of the body
selected then.

The moved faces keep their names, so later features still find them. A move that would make a face
vanish, faces cross or a neighbour unable to follow fails the feature alone, saying which face or
edge and what to do. Distance and Angle accept units, expressions and parameters. The arrows
snap like other arrows and Ctrl drags freely; typing a number while holding or hovering one sets
the value as the panel field would, and dragging is one undoable change.
