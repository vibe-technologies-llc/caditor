# Rib

{command:model.rib} stiffens a body with a thin wall drawn as a single line: a gusset in the corner
of a bracket, a web across the inside of a box, a grid of walls under a lid. You draw only where
the rib runs; it is thickened and carried on until it meets the body, so it never floats in the
air or pokes out past a wall. It is in the Solid group's menu of the ribbon, the Model menu, the
command palette and the right-click menu of selected sketch curves.

Draw the rib's line in a sketch, as lines and arcs joined end to end. A sketch may hold several
such chains, crossing or apart, and each becomes a rib of its own; chains that branch, or curves
that close into an outline, are refused. Select a curve of the sketch, or the sketch in the tree,
then choose Rib. With nothing selected it takes the newest sketch nothing else uses yet, and joins
the body the sketch was drawn on (else the newest body).

## Two ways to run

- **Parallel to sketch** makes a rib: the material lies in the sketch's plane, on one side of the
  curves, and fills the space between them and the body; the thickness is measured across the
  sketch. Draw it on a plane through the part, as the sloping edge of a gusset.
- **Square to sketch** makes a web: the curves are thickened in the sketch's plane and the wall
  runs square to the sketch until it meets the body. Draw it on a plane over the part or on the
  rim of a box, as the top edge of the wall.

Either way the free ends of the curves run on straight along their last direction until they meet
a face of the body, so a line drawn short of the walls still reaches them. A line ending on an arc
continues from the arc's end, along its tangent.

A new rib starts on the side, or in the direction, where the body is. A rib drawn on a face of the
body starts square to its sketch.

## Panel

- **Sketch** is the sketch the rib follows; the list offers the sketches above it, those whose
  curves do not form open chains greyed out with the reason.
- **Body** is the body the rib joins.
- **Direction**: **Parallel to sketch** (a rib) or **Square to sketch** (a web).
- **Thickness** takes units and expressions, so a rib can follow a parameter such as `rib_t`.
- **Side**: **One side**, **Other side** or **Symmetric**. A rib parallel to its sketch is thick
  in front of the sketch (the way its normal points), behind it, or half each way. A web is thick
  to the left of its curves walked from their first end, to the right, or half each way.
- **Fill the other side of the curves** (a rib) or **Run the other way from the sketch** (a web)
  flips which way it goes; Reverse direction in the Model menu and palette does the same.

The rib is shown as it will be while the panel is open, and every change is one undoable step.

## When it fails

The rib fails alone, with the body before it kept, and says why:

- it meets no face of the body, or the body lies wholly on the other side of the sketch: reverse
  it, or move its curves over the body;
- an end runs on past the body without meeting a face (the curve named), or the rib is not closed
  off by the body on the side it fills: draw the curve so that, carried on, it runs into a wall,
  or reverse it;
- its curves lie inside the body's material, so it adds nothing;
- the curves of the sketch close on themselves or branch, fixed in the sketch;
- a web thicker than the bend of an arc allows, or one running into itself.

The rib's faces are named after it and the curves that made it ("Rib 1 side from Line 2", "Rib 1
top face", "Rib 1 side carried on from Arc 3"), so later fillets and references keep hold of them
when its thickness changes. Where a face of the rib lies flush with a face of the body (a web
level with the rim of a box), the two become one face of the body.
