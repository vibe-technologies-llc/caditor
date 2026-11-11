# Drawings

A drawing lays out views of the model's bodies on an ISO sheet with a frame and a title block,
ready to print or send. Drawings are part of the model: they are saved with it, kept in its
version history, recovered after a crash, and every change to one can be undone.

{command:model.drawings} opens the Drawings panel; {command:model.new_drawing} makes a drawing of
the bodies selected in the view or the feature tree, or of every shown body when none is selected,
and opens it. A model keeps up to 32 drawings, each with a name of its own.

## Sheet and views

- **Sheet** is A4 to A0 (ISO 216), **Orientation** landscape or portrait.
- **Views** are front (looking along +Y), top, right, left and isometric. The front, top, right
  and left views line up with each other as **Projection** says: in first angle, used in most of
  Europe, the top view stands below the front view and the right view to its left; in third angle,
  used in North America, the other way round. The isometric view stands top right.
- **Scale** fits the sheet by default: the largest ISO 5455 scale (2:1, 1:2, 1:5, 1:10 and so on)
  at which the views fit. A chosen scale that is too large for the sheet says so; choose a smaller
  scale or a larger sheet.
- **Hidden lines** draws the edges behind the bodies dashed. Edges between faces that meet
  smoothly are left out.
- **Bodies**: every shown body, or those chosen with **Use the selected bodies**. A chosen body that
  is deleted later is named in the panel; the rest are still drawn.

## Section views

A section view shows the bodies cut along a plane, as if the part in front of the plane were
taken away. **Add a section view** in the panel adds one, lettered A, B, C and so on (I, O and Q
are left out, as ISO 128 advises); a drawing holds up to 23.

- **Cut on** is the front, top, right or left view the cutting plane is seen on as a line.
- **Plane** is the principal plane the cut runs along: the two that the chosen view sees as a
  line are offered (on the front view, YZ across X and XY across Z).
- **Through** cuts through the middle of the drawn bodies, or at an **Offset**: the plane's
  **Distance** from the origin along its axis, which takes units and parameters like any value.
  The expression is kept, so the plane follows a parameter edited later; a distance that cannot
  be worked out (a parameter that fails, a division by zero) is named in the panel and the section
  is left undrawn until it can.
- **Look the other way** keeps the bodies on the other side of the plane and looks at them from
  this side.

The cutting plane is drawn on its view as a thin chain line with thick ends, with arrows pointing
the way the section is seen and its letter at each arrow (ISO 128-44). The section view stands
right of the other views, labelled **A–A**; the faces the plane cuts are hatched with thin lines
at 45°, 2.5 mm apart on the sheet whatever the scale, and each cut body takes the other direction
from the one before it, so neighbouring parts read apart (ISO 128-50). The bodies behind the plane
are drawn as in the other views, but without hidden lines, which sections leave out. A plane that
crosses none of the bodies says so in the panel. When the view a section is cut on is not on the
sheet, the panel says its cutting line is not drawn.

## Detail views

A detail view enlarges a part of another view. **Add a detail view** in the panel adds one, with a
circle on the lower left corner of the first view; it takes the next letter, shared with the
section views (A, B, C and so on without I, O and Q), so a drawing holds up to 23 section and
detail views together.

- **Enlarges** is the view the circle is on: one of the views the drawing shows, or a section view.
- **Centre across**, **Centre up** and **Radius** place the circle in the view's own model
  lengths, so it stays on the same part of the model when the sheet scale changes. They take units
  and parameters like any value; the value is kept as a length.
- **Scale** is twice the scale of the sheet by default (the nearest ISO 5455 ratio at least that
  large, for example 2:1 on a sheet at 1:1), or any ISO 5455 ratio.

The circle is drawn on its view as a thin line with the letter beside it. The detail view stands
in the column right of the other views, under the label **B (2:1)**: the letter and its scale. It
shows the lines of the view inside the circle, clipped exactly to it, and the circle round them;
the hatching of a section carries on inside it. A detail of a view that is not on the sheet, or
that could not be drawn, says so in the panel. Deleting a section view also deletes the details of
it.

The title block takes the title, part number, revision, author and organisation from
[model properties](templates) (the title falls back to the model's file name), and shows the scale,
sheet size, projection with its ISO symbol, the units (mm) and the drawing's name.

## Working out the views

Views are worked out in the background while the panel is open and again whenever the model
changes; the panel shows how many are done and the rest of caditor stays usable. A view that
cannot be drawn says why in the panel, and the other views are drawn as usual.

The sheet below the settings is a preview: drag it to move it, pinch or hold Ctrl and scroll to
zoom, or use **Zoom in**, **Zoom out** and **Fit the sheet**. With the sheet focused (Tab), the
arrow keys move it.

## Exporting

{command:file.export_drawing_sheet} (or **Export…** in the panel) writes the sheet at its real
size in millimetres as:

- **PDF**, to print at 100% or send.
- **SVG**, for documents and the web.
- **DXF**, for other CAD programs, with the outlines on layer `Visible`, hidden lines on a dashed
  layer `Hidden`, section hatching on `Hatching`, cutting lines on a dash-dot layer `Cutting` and
  the frame and title block on `Frame`.

Lines, circles, arcs and ellipses stay exact; other curves are written as short straight pieces,
and the notice says how many.
