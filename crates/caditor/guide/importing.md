# Importing

{command:file.import} reads a drawing, a model or a mesh. You can also drop files onto the window.

## Drawings: DXF and SVG

A drawing becomes sketch geometry: in the sketch being edited, else in a new sketch named after the
file. Before it is added, a dialog asks the unit to read it in (as the file says, by default), a
scale, whether to centre it on the origin, the plane for a new sketch and, for a drawing of several
layers, which layers to take. It shows the size the drawing will have. Dashed lines come in as
construction geometry.

{command:sketch.toggle_first_dimension_scales} then sizes an outline traced from a picture with one
dimension; see [dimensions](dimensions).

## Models and meshes

STEP files (`.step`, `.stp`, also compressed) come in as one body per part, with their colours and
layers. STL, OBJ and 3MF meshes come in as bodies made of flat facets; STL and OBJ files carry no
unit, so they are read as millimetres. Each body is an [imported body](imported-bodies) in the
tree.

## The report

Anything worth knowing (the unit used, objects left out, curves fitted, edges repaired) is listed in
a report when the import finishes. A file in inches or feet is converted to millimetres, and the
report says so.

Reading runs in the background; the status bar shows it with Cancel, and
{command:file.cancel_import} stops it.
