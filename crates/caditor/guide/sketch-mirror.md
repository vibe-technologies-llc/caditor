# Mirror in a sketch

{command:sketch.mirror} copies the selected geometry mirrored about a line or an axis. With the
geometry already selected, click the line or axis to mirror about.

With nothing selected, the tool selects first: click items to add or remove them, or drag a box
around them, then press Enter and click the line or axis. Escape steps back to choosing what to
mirror, then lets go of it.

The copy stays mirrored as the original changes, held by Symmetric constraints; a mirrored circle
or ellipse also keeps its size by Equal. A mirrored half of
a symmetric part needs only half the dimensions.

## Drawing symmetrically

To draw both halves of a symmetric outline at once, select the one line or axis to draw about and
turn on {command:sketch.draw_symmetrically} (Sketch menu or the palette). While it is on, every
shape the drawing tools finish also gets its mirror image about that line, held by Symmetric
constraints as Mirror holds it, in the same change, so one undo takes both away. The preview shows
the image as you draw, and the prompt names the line drawn about.

Points drawn on the line are shared by the two halves, and a shape drawn from a point already
mirrored joins that point's image, so a half outline drawn from the line and back closes into one
outline. Run the command again to turn it off; it also turns off when the line is deleted or the
sketch is finished.

To mirror a whole body or features of the model, use [Mirror body](mirror).
