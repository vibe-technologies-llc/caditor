# Measure

{command:view.measure} opens the Measure panel. Measuring never changes the model, unless you
make a reading a parameter or keep it, below.

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
view, so it can be measured to. Mass properties are worked out exactly from each body's faces the
first time something asks for them, in the background; until then a body's card says so, and on a
large assembly the total follows a few seconds later.

## Relative to

Once the model has a [coordinate system](coordinate-systems), **Relative to** reads positions and
directions along its axes instead of the world's.

Values marked ≈ are approximate, with a note saying why. **Copy all** puts every reading on the
clipboard as text.

Each reading has a menu, on its "⋯" button or a right-click: **Copy value** copies that reading as
shown, with its unit, and for a length, angle or area **New parameter from this value** adds a
[parameter](parameters) holding it in the model's unit, named after the reading (`distance1`),
and puts the cursor in its name to rename it. It is one change Undo takes back.

## Keeping a measurement

For a distance or an angle between two items, or the length, radius or area of one, the row's menu
also has **Keep this measurement**. It adds a Measurement to the feature tree that takes the
reading again every time the model is recomputed, and a parameter named after it (`distance1`)
that holds what it reads; the cursor goes to the parameter's name to rename it, say to
`clearance`. The reading is drawn in the view with its value (the closest points joined by a
line), so a clearance can be watched while upstream features change. Hide it from its row like a
datum.

Features below the measurement in the tree can use its name in their values: an extrusion of
`clearance - 1 mm` follows the gap it measures. A measurement is taken where it stands in the tree,
so a feature above it cannot use it, and neither can another parameter, which is worked out before
the model; either is refused saying why. Its parameter shows the reading and cannot be edited.

What it measures is followed like any reference: faces, edges and corners keep their names across
upstream edits. When one is gone the measurement fails with the reason, and features using its
value fail with it, pointing back to it; everything else carries on. Deleting the measurement
leaves its parameter, if something uses it, as an ordinary parameter holding the value read when
it was kept.

To find bodies that overlap, see [Check interference](interference).
