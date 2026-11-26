# Changing sketch geometry

## Moving, turning and scaling

- {command:sketch.move} moves the selected geometry: drag the X or Y arrows or the XY plane square
  in the view, or type where to, or `@` and an offset.
- {command:sketch.rotate} turns it by an angle, counter-clockwise, about the one selected loose
  point, else about the middle of the selection — drag the rotation ring or type an angle.
- {command:sketch.scale} scales it by a factor the same way — drag the scale handle or type a
  factor.

Sketch text and gears in the selection turn and scale with it: a text's angle turns and its height
scales, a gear's turn follows and its module, root fillet and bore scale, so a quarter turn leaves
the text reading along the turned geometry. A value set by a parameter stays as the parameter
says, as does a sprocket's size (its chain sets it) and a meshing gear's size (the gear it meshes
with sets it; turned together, the pair turns as one), and a note says what was kept.

Handles step in neat increments as you drag; hold Ctrl to move freely, click any handle to type an
exact value for that axis, angle or scale, and press Escape to cancel a drag or close the handles.
While typing a move, rotation or scale, the view marks the anchor or pivot and previews live what
will move, texts and gears included, as they will end up. Double-click sketch text or a gear with Select to edit it in its panel, or right-click
it and choose **Edit text** or **Edit gear**.

Constraints still hold, so geometry tied to the rest may not go all the way. With one dimension
selected, Move places its label instead. Projected geometry, the origin and the axes never move:
dragging them says so beside the pointer, and Move, Rotate and Scale leave them out of a selection
or, when nothing else is selected, say so. To move projected geometry, choose
{command:sketch.unlink_projection} first. A value entered while the sketch is still being
recalculated waits, shown in the status bar, and moves the geometry once the sketch is solved;
Escape or the status bar's Cancel drops it.

## Splitting and breaking

- {command:sketch.split_curve} cuts a line, arc, elliptical arc or spline at a selected point on
  it. A spline splits into two splines of exactly the same curve; a closed one is refused with the
  reason (trim a closed spline open first). A spline drawn through fit points splits into two
  splines drawn from control points, since its pieces cannot pass through the same points; a fit
  point inside a piece that carries a constraint, a tangent handle or another curve stays, held on
  that piece, and its handle becomes a tangent there, so nothing you held is lost.
- {command:sketch.break_curves} breaks the selected lines, arcs, elliptical arcs and splines at
  every crossing.
- [Trim and Extend](trim-and-extend) cut away or lengthen pieces with a click.

## Copy and paste

{command:sketch.copy}, {command:sketch.cut} and {command:sketch.paste} move geometry with its
constraints between sketches, models and even other caditor windows. Pasting puts the copy under
the pointer and selects it; dimensions keep parameters of the same name in the target model.

## The context menu

Right-click a curve or point for the constraints the selection can take, construction, splitting,
adding a [spline control point](spline), deleting, the tools that work on the selection under
**Modify** (offset, mirror, the patterns, sketch fillet and chamfer, tangent circle), moving, cut,
copy and paste, selecting and Smart dimension; right-click empty space to paste or finish the
sketch. While a shape is half drawn the
menu offers {command:sketch.take_back_point}, {command:sketch.finish_shape} for a chain of lines, tangent arcs
or a spline, {command:sketch.cancel_shape}, {command:sketch.chain_arc} in a line chain, reversing
an arc and typing an exact value. See
[selecting](selection).

## Selecting

{command:sketch.select_all} selects every curve and point of the sketch, and
{command:sketch.select_free} selects what is not yet held. A double-click on a curve selects the
chain of lines and arcs joined to it.

See also [offset](offset), [mirror](sketch-mirror) and [patterns](sketch-patterns).
