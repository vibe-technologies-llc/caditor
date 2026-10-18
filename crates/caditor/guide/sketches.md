# Sketches

A sketch is a flat drawing on a plane: lines, arcs, circles, splines and points, held in shape by
[constraints](constraints) and [dimensions](dimensions). Features such as [Extrude](extrude) and
[Revolve](revolve) turn its closed regions into bodies.

## Starting a sketch

Select a principal plane, a datum plane or a flat face, then choose {command:model.new_sketch}. With
nothing selected, it asks you to click one in the view, or to pick a principal plane, datum plane
or coordinate system plane from the **Plane** list of the **New sketch** card above the tree, which
Tab reaches (the same list picks planes in the panels of other tools); Escape or **Cancel** stops choosing. A sketch on a face
moves with that face when the model changes. Its row in the tree says what it lies on and offers
Detach and {command:model.place_sketch}.

## Editing a sketch

The view turns to face the sketch, other features dim and the sketch ribbon appears. Double-click a
sketch in the tree, or one of its curves, to edit it again later. {command:sketch.finish} or
Escape ends editing, keeping everything; a new sketch left with nothing in it is removed then, and
Redo or Undo brings it back. {command:sketch.cancel} (the cross beside Finish sketch, above Look at sketch) ends editing
and takes back every change made since the sketch was entered, as one step Undo brings back.

Opening a feature, entering another sketch or starting a new one finishes the sketch first. Unless
the feature is built on the sketch, as an extrusion of it is, the status bar says so, such as
"Finished Sketch 2 to edit Fillet 1", with an Undo button when that also created something, which
takes it back and returns to the sketch.

- Pick a tool from the sketch ribbon, by its key, or from {command:palette}. Escape steps back one
  stage at a time and then returns to Select; holding it down takes only one step.
- Click a group's name on the sketch ribbon (Draw, Modify, Constrain and the others, with the small
  arrow beside them) for a menu of everything in that group, the tools and commands without a
  button included: every way to draw each shape, Rotate and Scale, Copy and Paste, Curvature,
  Add the relations the drawing shows, Dimension fully from a datum point and more. The tool in
  use and each shape's current way are marked.
- A button with a small triangle in its corner holds more tools, such as Tangent circle under
  Offset: click the triangle for the menu, or hover it to see what is inside. Undo and Redo sit
  at the start of the sketch ribbon too.
- While a shape is half drawn, Backspace or Delete takes back the last point.
- Changing how a shape is drawn halfway keeps its first point when it means the same in the new
  way, such as a rectangle's first corner or a circle's or polygon's centre; otherwise finish or
  cancel the shape first.
- With Select, drag a point or curve to move it; drag across empty space to box-select.
  Double-click a curve to select the chain it belongs to. A dragged point snaps as a drawn one
  does, and the geometry it would land on is highlighted; a dragged curve or selection snaps
  whichever of its points comes nearest something to land on.
- The prompt at the top of the view says what to click next, with the three or four keys that
  matter most; **More keys** beside them (a button Tab reaches) shows the rest, and **Fewer keys**
  folds them again.
- A click that cannot be done leaves a short note beside the pointer as well as in the status bar.
- Delete removes the selection, curves and constraints alike.

## Placing points exactly

With a drawing tool active, start typing a number to open the point field:

- `20, 10` places a point at x 20 and y 10 from the origin.
- `@15, 0` places it 15 to the right of the last point.
- `40 < 30` is 40 long at 30 degrees from the sketch's x axis.
- A lone length, such as `25`, goes that far from the last point toward the pointer.
- An angle alone, such as `< 30`, locks the direction from the last point: the pointer then sets
  only the length, and a click places the point with the angle kept. Backspace or Escape lets go of
  the direction.
- A length followed by `<`, such as `25 <`, locks the length instead: the pointer then sets only
  the direction, and a click places the point that far from the last one. The prompt and the
  readout say the length (the radius for a circle or arc) is locked. Backspace or Escape lets go
  of it.

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
  [Slot](slot), [Polygon](polygon), [Spline](spline), [Ellipse](ellipses),
  [Spur gear](gear) and [Text](text).
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
