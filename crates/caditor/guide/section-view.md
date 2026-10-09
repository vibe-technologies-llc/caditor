# Section view

{command:view.section} cuts every body at one or more planes so you can look inside the model. It
only changes what the view shows: nothing in the model is edited, and the planes are forgotten when
the model is closed.

- {command:view.section_add} adds a plane through the middle of the model; up to six cut at once.
- Each plane starts from the XY, XZ or YZ plane, or from a datum plane, a coordinate system's
  plane, a flat face or a sketch, taken with {command:view.section_use_selected}. It follows that
  geometry when the model changes.
- **Offset** moves the plane, **Tilt** and **Turn** turn it; each takes units and
  [expressions](expressions).
- {command:view.section_flip} keeps the other side, and {command:view.section_remove} drops the
  current plane.
- **Cut faces** are drawn hatched or filled in the body's colour.

Selecting, box selection and [Measure](measure) reach only what the section shows, and an exported
image keeps the cut.

## Slicing while sketching

{command:sketch.slice_bodies} cuts away the bodies on your side of the sketch plane while a sketch
is edited, so a sketch inside a body is drawn on its section rather than seen through its faces.
