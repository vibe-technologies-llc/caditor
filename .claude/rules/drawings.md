---
paths:
  - "crates/caditor-document/src/drawings.rs"
  - "crates/caditor-document/src/drawings_tests.rs"
  - "crates/caditor-document/src/drawing_views.rs"
  - "crates/caditor-document/src/drawing_sheet.rs"
  - "crates/caditor-file/src/drawings.rs"
  - "crates/caditor-file/src/tests/drawings.rs"
  - "crates/caditor-file/src/export/drawing_sheet.rs"
  - "crates/caditor-file/src/export/pdf.rs"
---

# Technical drawings

A drawing lays out standard views of the model's bodies on an ISO sheet with a frame and a title
block, and exports it as PDF, SVG or DXF. It is model content, worked out from the kernel's hidden
lines, never a picture of the 3D view.

## In the document (`drawings.rs`)

- `Drawings` holds the model's `Drawing`s in the user's order and `next_id`, the table's own
  counter: a `DrawingId` is never reused, `same_content` ignores the counter and a set given to
  `Edit::SetDrawings` never lowers it (`first_free_id` of the given set or the current one,
  whichever is larger). It is content in the `same_content`/`transaction_to`/`heap_size` family
  (`document.md`), ignored by recompute, with one whole-set edit whose inverse holds the previous
  set; `Touched::drawings` names it in the undo history.
- A `Drawing` is a `name` (one trimmed line via `drawing_name`, unique ignoring case, at most
  `MAX_DRAWING_NAME_CHARS`), `size` (`SheetSize`, ISO 216 A4 to A0, A3 by default), `orientation`,
  `projection` (`Projection::FirstAngle` by default, or `ThirdAngle`), `scale` (`DrawingScale::Fit`
  or a `ScaleRatio` of whole terms up to `MAX_SCALE_TERM`; `ScaleRatio::PREFERRED` are ISO 5455's
  ratios from 50:1 to 1:1000), `views` (a set of `StandardView`s), `bodies` (empty draws every
  shown body; at most `MAX_DRAWING_BODIES`) and `hidden_lines`. At most `MAX_DRAWINGS` per model.
  `checked` refuses the rest in words (`EditError::Drawing*`); a body id is stored like any (its
  maker storable) and raises the feature counter like a selection set's.
- Builders, each one checked transaction: `adding_drawing(bodies)` ("Drawing N", returns its id),
  `changing_drawing(id, label, change)`, `deleting_drawing(id)`.

## Views (`drawing_views.rs`)

- `StandardView::look_and_up`: front looks along +Y with Z up (sheet x is model X), top along -Z
  with Y up, right along -X, left along +X, isometric along (-1, 1, -1) with Z up.
- `drawing_view(view, solids)` runs kernel `hidden_lines` (`kernel-hidden-lines.md`) at
  `MeshQuality::SMOOTH`'s tolerance for the bodies' extent, under `catch_unwind`, and keeps sharp
  edges and silhouettes only: tangent edges and seams are left out, as ISO 128 draws them. It is
  interruptible (`Cancelled`); no solid is `NoBodies`. `ViewLines::bounds` sizes the view on the
  sheet. `drawn_bodies` resolves which bodies a drawing draws in an `Evaluation` (with every body
  standing that is not hidden when none are chosen) and names the chosen ones missing.

## Sheet (`drawing_sheet.rs`)

- `lay_out_sheet(drawing, fields, extents)` is pure layout in millimetres, origin bottom left. ISO
  5457 frame: 20 mm filing margin on the left, 10 mm elsewhere (20 mm on A0 and A1), centring marks
  at each side's middle. Title block `TITLE_BLOCK_WIDTH` by `TITLE_BLOCK_HEIGHT` at the frame's
  bottom right: Title (falling back to the model's name), Part number, Revision, Organisation,
  Drawn by (author), Drawing (its name), Scale, Size, Projection with the ISO 128 symbol drawn as
  marks, Units (mm) and Sheet 1 / 1 (`TitleFields::of` the model properties). Text heights are cap
  heights; widths come from `caditor-lettering`'s Inter (`text_width`), so an overlong value is
  shrunk to 2 mm then cut with an ellipsis, never run past its cell.
- Views sit in a 3 by 3 grid of the view area: the front in the middle, the others lined up with it
  by projection (first angle: top below, right view on the left; third angle the opposite), the
  isometric top right; each row and column as wide as its largest view, `VIEW_GAP` between. `Fit`
  takes the largest preferred ratio at which the grid fits; a chosen ratio that does not fit still
  lays out, with `SheetPlan::fits` false for the app to say so.

## File (`caditor-file/src/drawings.rs`)

- The optional document-level record `drawings` (`file-format.md`), written only when there is one,
  keys as text (`SheetSize::key` and the like) so an unknown value is a reported fallback, each
  drawing `Lenient`. Journal: snapshot field `drawings`, edit `set_drawings`. Loading repairs and
  reports: an unreadable drawing, one past `MAX_DRAWINGS` or with a repeated id is left out, names
  are repaired like saved views', unknown size, orientation or projection take the default, an
  unusable scale fits, unknown views and unstorable bodies are dropped; `next_id` becomes at least
  `first_free_id`. Models from before drawings read with none (the corpus holds
  `model-1/drawings.caditor` and `journal-1/drawings.journal`).

## Export (`export/drawing_sheet.rs`, `export/pdf.rs`)

- `export_sheet(path, plan, views, hidden_lines, format, cancel)` builds one `Figure` at the
  sheet's real size: view pieces on `Layer::Visible` and `Layer::Hidden`, frame, marks and title
  block on `Layer::Frame`. Lines, circles, arcs and ellipses stay exact; other curves become
  polylines within the sheet chord and are counted in `SheetExported::approximated`. Written
  atomically. `SheetFormat::of` picks by extension (PDF by default).
- SVG is the page itself (`width`/`height` in mm, viewBox the sheet), strokes per layer (0.5 mm
  visible, 0.25 mm hidden dashed 3/1.5, 0.35 mm frame). DXF names its layers with lineweights
  and a `DASHED` linetype on `Hidden`, so `parse_dxf` reads hidden lines as construction.
- PDF is caditor's own writer, no dependency: PDF 1.4, one page sized in points, millimetre
  coordinates through a `cm` transform, arcs and ellipses as Béziers, text as filled Inter outlines
  from `caditor-lettering` (no embedded font), the producer `APPLICATION` and nothing identifying
  the user.
