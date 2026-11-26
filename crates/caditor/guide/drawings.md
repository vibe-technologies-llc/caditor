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
  and parameters like any value; the value is kept as a length. The circle can also be dragged on
  the sheet: drag inside it to move it, or its rim to make it larger or smaller.
- **Scale** is twice the scale of the sheet by default (the nearest ISO 5455 ratio at least that
  large, for example 2:1 on a sheet at 1:1), or any ISO 5455 ratio.

The circle is drawn on its view as a thin line with the letter beside it. The detail view stands
in the column right of the other views (until it is placed by hand), under the label **B (2:1)**: the letter and its scale. It
shows the lines of the view inside the circle, clipped exactly to it, and the circle round them;
the hatching of a section carries on inside it. A detail of a view that is not on the sheet, or
that could not be drawn, says so in the panel. Deleting a section view also deletes the details of
it.

The title block takes the title, part number, revision, author and organisation from
[model properties](templates) (the title falls back to the model's file name), and shows the scale,
sheet size, projection with its ISO symbol, the units (mm) and the drawing's name.

## Placing views by hand

Each view stands where the drawing lays it out until it is dragged on the sheet, in the panel's
preview or in the sheet view. Dropping a view places it there, and each drop can be undone.

- The top, right and left views stay lined up with the front view: the top view moves only up and
  down, the right and left views only across. Dragging the front view carries them with it.
- The isometric, section and detail views move freely, their labels and circles with them.
- A view placed by hand keeps its place when the scale changes, growing or shrinking about its
  middle, and when other views are added or removed. Its place is kept from the middle of the
  sheet, so a larger or smaller sheet keeps the views together; a view that would fall off a
  smaller sheet is held at its edge.
- **Layout** in the panel lists the views placed by hand, each with a button putting it back where
  the drawing lays it out. Right-clicking a view on the sheet offers the same as **Reset
  position**.

Without dragging, click a view to pick it, drawn with a solid outline: the arrow buttons above the
sheet move it 10 mm on the sheet, and with the sheet focused (Tab) the arrow keys move it 1 mm,
10 mm with Shift. Escape lets it go; with no view picked, the arrow keys move the sheet.

## Dimensions and notes

Dimensions are measured from the model, never typed: each one refers to edges of a view by the
names the model gives them, the way features do, so it follows every edit and shows the length
the model has now. They are drawn as ISO 129-1 asks: thin lines, open arrows, extension lines
starting a little off the part and running a little past the dimension line, and the value above
the line in the sheet's lettering, readable from the bottom or the right of the sheet. When a
length is too short for its arrows, they move outside its extension lines and point in, each with
a short stub of line beyond it; when it is too short for its value, the value moves outside too,
beside the arrow on the side where you placed the dimension, above the line drawn on under it. An
angle's arc too short does the same along the arc, and a circle too small for an arrow inside it
takes its arrows from outside.

Values keep clear of each other. Where a dimension's value would fall on a note, a callout or the
value of a dimension added before it, the value slides along its line, moves to the other end of a
short one, or a radius or diameter swings its leader round the circle (staying on an arc), until it
stands clear; the dimension line stays where you placed it. When nothing clears, the value stays
as placed, and a value you moved with **Value along** always stays where you put it.

{command:model.drawing_dimension} (or **Add a dimension** above the sheet) dimensions edges of a
front, top, right, left, section or detail view; the isometric view is not drawn true to size, so
nothing is measured in it. Click an edge, then a second one, then where the dimension line stands:

- The same straight edge twice measures its length; two parallel straight edges the distance
  between them; two other straight edges the angle between them, on the side you place it.
- The same circle twice measures its diameter, the same arc its radius. A circle seen at a slant,
  as an ellipse, is not measured; dimension it in a view that sees it round.
- Two other edges measure between their nearest ends, or the centres of circles and arcs:
  horizontally, vertically or straight between them, as where you place it suggests.

The value shows trailing zeros left off, in millimetres. A placed dimension stays picked: drag it
or use the arrow keys (Shift for larger steps) to move it, Delete to remove it, and Escape to let
it go. **Annotations** in the panel lists them, each with **Measures** (horizontal, vertical or
straight; radius or diameter), **Tolerance**, **Decimals** (the most it shows), **Value along**
(how far the value stands from the middle of its line, in sheet millimetres; at 0 the value keeps
clear of others by itself), **Re-attach** and
**Delete**. A dimension stands in its view's own model lengths, so it moves with its view.

**Tolerance** says how far the made size may stray from a length, radius or diameter, written as
ISO 129-1 does:

- **Plus or minus** a deviation: `20 ±0.1`.
- **Deviations**, an upper and a lower one written small beside the value, the upper above:
  `20` with `+0.02` over `−0.01`. A deviation of nothing is written `0`.
- **Limits**, the largest and smallest size, the largest above.
- **ISO 286 fit**, a letter and a grade such as `⌀10 H7`: H for a hole from its size up; f, g, h,
  js, k, n and p for shafts from running to press fits; grades IT5 to IT11. **Show the
  deviations** writes them from ISO 286's table after the fit, and the panel says what they are
  for the size measured. The table covers sizes up to 500 mm; past it the fit is written alone.

When an edge a dimension measures disappears after an edit, the dimension is kept and drawn
dashed in the error colour with a question mark for its value, and the panel says what it lost.
**Re-attach** picks its edges again, keeping where it stands and its decimals; or delete it.

{command:model.drawing_note} (or **Add a note** above the sheet) writes a note: click where it
stands, type it, click an edge if a leader should point at it, then **Place the note** (or Enter).
A note holds up to 200 characters over as many lines as you like: Shift+Enter starts a new line and
Enter places the note (or keeps the new text in the panel). Its lines stand one under the other,
centred on the note's place with the leader beside the first line, and are exported the same way
to PDF, SVG and DXF. In the panel its text can be changed, its leader removed and the note
deleted; a note whose edge is gone keeps its text and the panel says so.

## Centre marks and centrelines

A centre mark crosses a circle seen round with two thin long-dash-dot lines a little past its rim,
and a centreline runs along a cylinder seen from the side, a hole or a shaft, a little past each
end, as ISO 128 draws them. Each follows the circle or cylinder it marks, so it moves and grows
with the model.

{command:model.drawing_centre_marks} (or **Add centre marks to this view** above the sheet) marks
every circle seen round and every cylinder seen from the side in the view you picked on the sheet:
circles sharing a centre, such as both ends of a hole, take one mark, and a counterbore and its
bore one centreline. What already has a mark is left as it is, so the command can be run again
after the model gains holes. {command:model.drawing_centre_mark} (or **Add a centre mark or
centreline** above the sheet) marks one at a time: click a circle or the outline of a cylinder.

A centre mark is picked by a click and removed with Delete or in the panel; it does not move on
its own. One whose circle or cylinder is gone is no longer drawn, and the panel says so.

## Hole and thread callouts

{command:model.drawing_callout} (or **Add a callout to a hole** above the sheet) writes what a
hole or a thread is: click an edge of a hole, or of a face a thread was put on, then where the
text stands. A leader runs from the text to the edge, ending on a circle pointing at its centre.
The callout reads the feature each time it is drawn, so editing the hole changes it:

- A hole: its diameter `⌀6.6` and, for a blind hole, its depth `↧10`; a slot its width and
  overall length.
- A counterbore `⌴⌀11 ↧6.4`, each step of a stepped hole the same way, or a countersink
  `⌵⌀12 × 90°`, on a line of its own below.
- A tapped hole its thread above the drill, `M8-6H ↧12` then `⌀6.8 ↧15`; a thread its
  designation and length.

The symbols are drawn as lines, so they come out the same in every format. A callout is dragged
or moved like a note. One whose hole or edge is gone is drawn dashed in the error colour as `?`,
and the panel says what it lost.

## The sheet in the view

{command:view.drawing_sheet} (also **Show the sheet in the view** under the panel's preview, and
in the View menu) shows the open drawing's sheet across the whole view instead of the model, with
the Drawings panel beside it, so the views are larger to arrange and every setting stays at hand.
**Back to the 3D view**, the same command again or Escape (once no view is picked) shows the
model again. Editing a sketch shows the model too.

## Working out the views

Views are worked out in the background while the panel is open and again whenever the model
changes; the panel shows how many are done and the rest of caditor stays usable. A view that
cannot be drawn says why in the panel, and the other views are drawn as usual.

The sheet below the settings is a preview: drag the paper to move it, pinch or hold Ctrl and scroll
to zoom about the pointer, or use **Zoom in**, **Zoom out** and **Fit the sheet**; the percentage
beside them is how large the sheet is drawn. With the sheet focused (Tab) and no view picked, the
arrow keys move it. The sheet view works the same way.

## Exporting

{command:file.export_drawing_sheet} (or **Export…** in the panel) writes the sheet at its real
size in millimetres, with every view where the sheet shows it, as:

- **PDF**, to print at 100% or send.
- **SVG**, for documents and the web.
- **DXF**, for other CAD programs, with the outlines on layer `Visible`, hidden lines on a dashed
  layer `Hidden`, section hatching on `Hatching`, cutting lines on a dash-dot layer `Cutting`,
  centre marks and centrelines on a dash-dot layer `Centre lines`, dimensions (as DXF dimensions,
  their tolerance part of the text), notes and callouts on `Annotations` and the frame and title
  block on `Frame`.

The file dialog proposes the format the last sheet was written in, PDF at first; type another of
the three extensions to change it.

Lines, circles, arcs and ellipses stay exact; other curves are written as short straight pieces,
and the notice says how many. A dimension, centre mark or callout that lost what it refers to is
left out of the file, and a note whose edge is gone is written without its leader; the notice
says how many.
