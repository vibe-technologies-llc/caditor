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
  layer `Hidden` and the frame and title block on `Frame`.

Lines, circles, arcs and ellipses stay exact; other curves are written as short straight pieces,
and the notice says how many.
