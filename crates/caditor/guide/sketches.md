# Sketches

A sketch is a flat drawing on a plane: lines, arcs, circles, splines and points, held in shape by
[constraints](constraints) and [dimensions](dimensions). Features such as [Extrude](extrude) and
[Revolve](revolve) turn its closed regions into bodies.

## Starting a sketch

Select a principal plane, a datum plane or a flat face, then choose {command:model.new_sketch}. With
nothing selected, it asks you to click one in the view; Escape stops choosing. A sketch on a face
moves with that face when the model changes. Its row in the tree says what it lies on and offers
Detach and {command:model.place_sketch}.

## Editing a sketch

The view turns to face the sketch, other features dim and the sketch ribbon appears. Double-click a
sketch in the tree, or one of its curves, to edit it again later. {command:sketch.finish} or
Escape ends editing.

- Pick a tool from the sketch ribbon, by its key, or from {command:palette}. Escape steps back one
  stage at a time and then returns to Select; holding it down takes only one step.
- While a shape is half drawn, Backspace or Delete takes back the last point.
- Changing how a shape is drawn halfway keeps its first point when it means the same in the new
  way, such as a rectangle's first corner or a circle's or polygon's centre; otherwise finish or
  cancel the shape first.
- With Select, drag a point or curve to move it; drag across empty space to box-select.
  Double-click a curve to select the chain it belongs to.
- Delete removes the selection, curves and constraints alike.

## Placing points exactly

With a drawing tool active, start typing a number to open the point field:

- `20, 10` places a point at x 20 and y 10 from the origin.
- `@15, 0` places it 15 to the right of the last point.
- `40 < 30` is 40 long at 30 degrees from the sketch's x axis.
- A lone length, such as `25`, goes that far from the last point toward the pointer.
- An angle alone, such as `< 30`, locks the direction from the last point: the pointer then sets
  only the length, and a click places the point with the angle kept. Escape lets go of the
  direction.

Values are [expressions](expressions), so parameters and units work. Letters choose tools, so to
start with a parameter's name type `=` first, as in `=width, 20`, or put the name in brackets, as
in `(width), 20`. The same works in the value field of Offset, Sketch fillet, the patterns and
Tangent circle. {command:sketch.type_value} opens the field without typing anything first.

While the text reads as a point, the shape being drawn follows it and shows its size, but nothing
is placed until you press Enter. Clicking elsewhere keeps the text, shown dimmed: click the field
or type again to carry on, or press Escape to clear it.

What you type is kept as dimensions, unless {command:sketch.toggle_typed_dimensions} is turned off.

## Drawing tools

- Shapes: [Point](point), [Line](line), [Rectangle](rectangle), [Circle](circle), [Arcs](arcs),
  [Slot](slot), [Polygon](polygon), [Spline](spline), [Ellipse](ellipses) and
  [Spur gear](gear).
- Changing curves: [Trim and extend](trim-and-extend), [Offset](offset),
  [Mirror](sketch-mirror), [Patterns](sketch-patterns), [Sketch fillet](sketch-fillet) and
  [Blend curve](blend-curve).
- Using the model: [Project and intersect](project-and-intersect), and
  [Tangent circle](tangent-circle) for circles touching other curves.

## Construction geometry

{command:sketch.construction} makes the selected curves construction geometry, drawn dashed. It
guides the drawing and takes constraints but never makes regions. With nothing selected it switches
whether new curves are drawn as construction.

See also [snapping](snapping), [changing sketch geometry](sketch-editing) and
[the sketch's status](sketch-status).
