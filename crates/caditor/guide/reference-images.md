# Reference images

{command:model.insert_reference_image} places a picture, such as a photo of a part or a scan of a
drawing, in the model so you can trace over it. It is in the Model menu under Datums, in the menu
of the ribbon's Reference group and in {command:palette}. It asks for the picture's file, then
puts the picture on the plane or flat face you selected, or on the plane of the sketch you are
editing, or on the XY plane when nothing is selected. The picture is kept inside the model file,
so the model opens with it on any computer.

caditor reads PNG, JPEG, WebP (lossy and lossless), JPEG XL, GIF (its first frame), BMP, TIFF (its
first page, 16-bit too), QOI and TGA pictures, telling them apart by what the file holds rather
than by its name. AVIF and HEIC pictures cannot be read yet; save them as PNG or JPEG first. A
picture may be up to 64 MB, 16,384 pixels on a side and 40 million pixels in all.

## Placing and sizing it

The picture shows as a row in the feature tree. Open it to change:

- **Placed on**: the plane or flat face it lies on, chosen from the list, the selection or the
  view.
- **Position X** and **Position Y**: where its middle lies on the plane.
- **Angle**: how far it is turned on the plane.
- **Flip left to right**: shows it mirrored, for a photo taken from the other side.
- **Width**: how wide it is in the model; its height follows the picture's shape.

To size it from something in the picture whose length you know, click **Calibrate by two points**,
click the two ends of that length on the picture, then type the real distance into **Distance
between the points**. The picture is scaled so the two points lie that far apart. Like every
value, the distance accepts units and expressions, so a [parameter](parameters) can drive it.
**Calibrate again** picks two new points, and **Size by width** goes back to a width.

## How it shows

- **Opacity** lets the sketch and the bodies show through the picture.
- **Shows**: **Behind bodies** hides the picture where a body stands in front of it; **In front of
  bodies** shows it over everything.
- **Locked** makes the picture impossible to click, so clicks pass through to the sketch and to
  what lies behind it. An unlocked picture can be clicked to select it, and double-clicked to open
  it.

Sketch lines always draw over the picture, and drawing tools snap to sketch geometry as usual, so
you can trace its outlines with lines, arcs and splines. Hide the picture like a datum, from its
row's eye or {command:view.toggle_datums}.

Changing the picture, its place and how it shows are ordinary changes you can undo, and they are
kept in the recovery copy of the model like every other change.
