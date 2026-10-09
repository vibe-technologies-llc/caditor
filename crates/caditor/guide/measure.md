# Measure

{command:view.measure} opens the Measure panel. Measuring never changes the model.

## One or two items

Select a vertex, edge, face, sketch point or curve, datum or axis to read it: a point's position, an
edge's length, a face's area, a circle's radius and centre, a plane's normal. Select two items for a
**Between them** card with the distance, the offsets along each axis and the angle where it applies;
the closest points are joined by a line in the view.

Selected regions of a sketch read their area, perimeter, centroid and second moments of area, the
numbers a beam calculation needs.

## Mass properties

Under the readings, each body's volume, area, centre of mass and, once it has a density, its mass
and moments of inertia. They are for the bodies of the selection, else those chosen in the tree,
else every shown body, with a total for several. Set a density with
[Body colour and material](bodies). {command:view.toggle_centres_of_mass} marks each centre in the
view, so it can be measured to.

## Relative to

Once the model has a [coordinate system](coordinate-systems), **Relative to** reads positions and
directions along its axes instead of the world's.

Values marked ≈ are approximate, with a note saying why. **Copy all** puts every reading on the
clipboard as text.

To find bodies that overlap, see [Check interference](interference).
