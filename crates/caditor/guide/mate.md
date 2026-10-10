# Mate

{command:model.mate} moves a body by its geometry and keeps it there when that geometry changes.
It has no button or key; use the Model menu or {command:palette}. Select the body's part first,
then what it goes onto:

- a flat face of the body, then another body's face or a plane: the faces lie **flush**, or at a
  distance;
- a straight edge or round face of the body, then another axis: the axes line up
  (**concentric**);
- a flat face and an axis of the body, then a face and an axis to mate them onto (four picks):
  **flush and concentric** in one mate, as a bolt sits in a hole;
- a cylindrical or spherical face of the body, then a face or plane: the round face **rests on**
  the plane, a cylinder laid down along it;
- a flat face of the body, then a cylindrical or spherical face of another body: the flat face
  **rests on** the round one, turned to lie along a cylinder, touching it from outside (or, for a
  bore, from inside);
- a corner, round edge or sphere of the body, then a point or corner, or a face or plane: the
  **point** lands on the other point, or on the plane.

{command:model.mate_angle} takes a flat face or axis of the body and then a face, plane or axis,
and turns the body until the two stand at an **Angle**, 90 deg to begin with. At 0 deg two faces
face each other, as when flush; the body turns about the line where their planes meet, so a hinged
part keeps its hinge.

The panel shows each choice with Use selected and Choose in the view, a **Distance** to leave
between faces, the **Angle** of an angle mate (both previewed as you type), and, where the mate
has a side, whether the face points the same way, the axis the other way or the face rests on the
other side, which {command:model.reverse_direction} also flips. An angle or point mate has
no side to flip.

A mate keeps its kind: to change from faces to axes, make another mate. When what it uses is
gone or no longer fits, the mate is marked in the tree and says what to choose instead. See also
[Move and copy](move-and-copy).
