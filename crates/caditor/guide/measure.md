# Measure

{command:view.measure} opens the Measure panel. Measuring never changes the model, unless you
make a reading a parameter or keep it, below.

## One or two items

Select a vertex, edge, face, sketch point or curve, datum or axis to read it: a point's position, an
edge's length, a face's area and perimeter (around its outer edge), a circle's radius and centre,
an arc's sweep, a plane's normal. Select two items for a
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
shown, with its unit, and for a length, angle, area, volume or mass **New parameter from this
value** adds a [parameter](parameters) holding it in the model's unit (a mass as a plain number of
grams), named after the reading (`distance1`), and puts the cursor in its name to rename it. It is
one change Undo takes back.

## Keeping a measurement

For a distance, an angle or an offset along X, Y or Z between two items, or the length, radius,
sweep, area or perimeter of one, the row's menu also has **Keep this measurement**. A point's
position offers **Keep this measurement along X**, **Y** and **Z**, keeping one coordinate. A
body's mass card offers it for the volume, the surface area, the mass (once the body has a
density) and, along X, Y or Z, the centre of mass; so does the position of a centre of mass marked
in the view. It adds a
Measurement to the feature tree that takes the reading again every time the model is recomputed,
and a parameter named after it (`distance1`) that holds what it reads; the cursor goes to the
parameter's name to rename it, say to `clearance`. The reading is drawn in the view with its value
(the closest points joined by a line), so a clearance can be watched while upstream features
change. Hide it from its row like a datum. An offset or a position kept while **Relative to**
names a coordinate system is measured along that system's axis, a position from its origin.

A kept volume, mass or centre of mass is worked out exactly from the body's faces as the model is
recomputed, only while a measurement asks for it and only again when the body changes, so
changing the density alone updates the mass at once. A kept mass is a plain number of grams, and
fails, saying so, while its body has no density.

Features below the measurement in the tree can use its name in their values: an extrusion of
`clearance - 1 mm` follows the gap it measures. Other parameters can use it too (`gap = clearance
/ 2`), and features below the measurement can use those. A measurement is taken where it stands in
the tree, so a feature above it cannot use it, directly or through another parameter; that is
refused naming the chain. Its parameter shows the reading and cannot be edited.

Open a measurement from its row to change it. **Reads** switches what it reads among what its items
allow: Distance, Angle or Offset along an axis for two items (a point has no angle), Length,
Radius, Sweep, Area, Perimeter, Volume, Mass or Position along an axis for one (a round edge has a
radius and a sweep, a face an area and a perimeter, a body a volume, an area, a mass and the
position of its centre of mass, a point a position). **From**, **To** or **Of** name the items;
**Use selected** takes the one selected item instead, or **Choose in the view** waits for a click;
a measurement of a body takes the body of the face, edge or corner chosen. An offset's or a
position's **Along** row picks the X, Y or Z axis, or takes any axis, straight edge, round face or
sketch line the same way; a position along such an axis is measured from the point the axis
passes through. Each change is one change Undo takes back.

What it measures is followed like any reference: faces, edges and corners keep their names across
upstream edits. When one is gone the measurement fails with the reason, and features using its
value fail with it, pointing back to it; everything else carries on. The parameter's stored value
follows each reading, so a model opened in an older caditor holds the latest reading, and
deleting the measurement leaves its parameter, if something uses it, as an ordinary parameter
holding the last reading, so the features using it keep their shape.

To find bodies that overlap, see [Check interference](interference).
