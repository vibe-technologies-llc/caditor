---
paths:
  - "crates/caditor-document/src/drawings.rs"
  - "crates/caditor-document/src/drawings_tests.rs"
  - "crates/caditor-document/src/drawing_views.rs"
  - "crates/caditor-document/src/drawing_sections.rs"
  - "crates/caditor-document/src/drawing_sheet.rs"
  - "crates/caditor-file/src/drawings.rs"
  - "crates/caditor-file/src/tests/drawings.rs"
  - "crates/caditor-file/src/export/drawing_sheet.rs"
  - "crates/caditor-file/src/export/pdf.rs"
  - "crates/caditor/src/drawings_panel.rs"
  - "crates/caditor/src/sheet_views.rs"
  - "crates/caditor/src/sheet_export.rs"
  - "crates/caditor/src/ui_tests/drawings.rs"
  - "crates/caditor/guide/drawings.md"
---

# Technical drawings

A drawing lays out standard views and section views of the model's bodies on an ISO sheet with a
frame and a title block, and exports it as PDF, SVG or DXF. It is model content, worked out from the kernel's hidden
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
  shown body; at most `MAX_DRAWING_BODIES`), `hidden_lines` and `sections` (`SectionView`s, below).
  At most `MAX_DRAWINGS` per model. `checked` (given the document) refuses the rest in words
  (`EditError::Drawing*`), and an offset naming a parameter the model lacks (`MissingParameter`); a
  body id is stored like any (its maker storable) and raises the feature counter like a selection
  set's.
- A parameter a section's offset names is in use like one a feature names: `used_parameters`,
  `parameter_user_ids` (`ParameterUser::Drawing`) and `parameter_users` count it, so
  `RemoveParameter` refuses it, `inline_parameter` substitutes its expression into the offset in
  one `SetDrawings`, and `Document::scaled` rewrites the offset as a placed length (scaled about
  the centre, shifted along the plane's axis).
- Builders, each one checked transaction: `adding_drawing(bodies)` ("Drawing N", returns its id),
  `changing_drawing(id, label, change)`, `deleting_drawing(id)`, `adding_section_view(id)` (returns
  its letter); the app changes and deletes a section through `changing_drawing`.
- A `SectionView` is a `letter` (one of `SECTION_LETTERS`, A to Z without I, O and Q as ISO 128
  advises, unique in its drawing, so at most `MAX_SECTION_VIEWS`; `Drawing::new_section` takes the
  first free one and is the drawing's key for the view), its `parent` (`StandardView::SECTIONED`:
  front, top, right or left), a `plane` (`SectionPlane`: YZ, XZ or XY through a point on its
  axis), a `place` (`SectionPlace::Centre` of the drawn bodies' box, or `Offset`, the plane's
  distance from the origin along its axis, an `Expression` of length kept as typed so it follows
  the parameters it names) and `reversed`. `SectionPlace::resolve` evaluates it against
  `ParameterValues` when the views are worked out, to a `ResolvedPlace` (`Centre` or `Offset(mm)`)
  or a `DrawingViewError::Offset` (the evaluation error) or `OffsetOutOfRange` (not finite or past
  `MAX_SECTION_OFFSET`), so a value only a parameter can change is never refused when stored.
  The plane must be seen as a line in the parent (`SectionPlane::is_edge_on_in`), else
  `SectionPlaneFacesParent`. It looks like the standard view along the plane's axis (YZ as the right
  view, XZ as the front, XY as the top), the other way when `reversed`; the kept bodies are on the
  side it looks toward. `DrawnView` (`Standard` or `Section(letter)`) keys every drawn view.

## Views (`drawing_views.rs`)

- `StandardView::look_and_up`: front looks along +Y with Z up (sheet x is model X), top along -Z
  with Y up, right along -X, left along +X, isometric along (-1, 1, -1) with Z up.
- `drawing_view(view, solids)` runs kernel `hidden_lines` (`kernel-hidden-lines.md`) at
  `MeshQuality::SMOOTH`'s tolerance for the bodies' extent, under `catch_unwind`, and keeps sharp
  edges and silhouettes only: tangent edges and seams are left out, as ISO 128 draws them. It is
  interruptible (`Cancelled`); no solid is `NoBodies`. `ViewLines::bounds` sizes the view on the
  sheet. `drawn_bodies` resolves which bodies a drawing draws in an `Evaluation` (with every body
  standing that is not hidden when none are chosen) and names the chosen ones missing.

## Sections (`drawing_sections.rs`)

- `section_view(section, place, solids)` places the cutting plane (`cutting_plane` from the
  `ResolvedPlace`, normal along the look),
  sorts each body by its box's heights over it (`split::heights`): wholly in front is left out,
  wholly behind kept whole, across it cut by `split::half_space_solid` under the reserved id
  `SECTION_CUT` and kernel `boolean` intersection (empty left out). No body cut is `PlaneMisses`;
  a failing half space or boolean is `CuttingSpace` or `Cut`, a cancel `Cancelled`, a panic
  `Panicked`; it polls `interrupt::check` between bodies. The kept bodies go through the same
  `lines_seen` as a standard view; `ViewLines::section` holds the `cutting` plane and a
  `HatchRegion` per cut body: the boundary of its cut faces (`section::is_cut_face` of
  `SECTION_CUT`) sampled at the view's tolerance into segments in view coordinates, at
  `HATCH_ANGLES` 45° and 135° alternately per cut body (ISO 128-50 neighbours told apart).
  `drawn_pieces` of a section leaves hidden pieces out whatever the drawing says.
- `hatch_lines(region, placed)` works on the sheet: lines at the region's angle `HATCH_SPACING`
  (2.5 mm) apart on the sheet whatever the scale (fewer, wider apart, past `MOST_HATCH_LINES`),
  each scan line cut even-odd against the boundary segments (half-open crossings, so a vertex
  counts once). Export and the preview both call it.

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
  isometric top right; each row and column as wide as its largest view, `VIEW_GAP` between. Section
  views stand in a column right of the grid (`SectionColumn`), as wide as the widest, stacked in the
  drawing's order under their "A–A" label (`SECTION_LETTER_HEIGHT`), the column centred down the
  view area; the label rows and gaps are fixed sheet lengths the scale fit takes off first. `Fit`
  takes the largest preferred ratio at which the grid and the column fit; a chosen ratio that does
  not fit still lays out, with `SheetPlan::fits` false for the app to say so.
- `lay_out_sheet(drawing, fields, extents, cuts)` takes each finished section's cutting plane by
  letter. Its `CuttingLine` on the parent view (ISO 128-44) runs across the parent's extent
  `CUTTING_OVERSHOOT` past it each way, as a chain line with `CUTTING_END` thick ends; open arrows
  (`SheetMark` lines) touch each end pointing the way the section looks and the letter stands
  beyond each tail. A section whose parent is not on the sheet (or not yet worked out) is in
  `SheetPlan::unmarked`.

## File (`caditor-file/src/drawings.rs`)

- The optional document-level record `drawings` (`file-format.md`), written only when there is one,
  keys as text (`SheetSize::key` and the like) so an unknown value is a reported fallback, each
  drawing `Lenient`. Journal: snapshot field `drawings`, edit `set_drawings`. Loading repairs and
  reports: an unreadable drawing, one past `MAX_DRAWINGS` or with a repeated id is left out, names
  are repaired like saved views', unknown size, orientation or projection take the default, an
  unusable scale fits, unknown views and unstorable bodies are dropped; `next_id` becomes at least
  `first_free_id`. Models from before drawings read with none (the corpus holds
  `model-1/drawings.caditor` and `journal-1/drawings.journal`).
- A section's place is written `offset` (a number) when the expression is a plain number, which
  older readers still read, else `expression` in stored text; loading repairs an expression that
  does not read or names a missing parameter (`restore_drawings` is given what exists) to the
  centre, with a report. The corpus holds `model-1/section-parameter-offsets.caditor`.
- Each drawing's `sections` is a field written only when it has any, each section `Lenient`, so
  drawings stored before section views read with none and an older reader drops them (losing only
  the section views; nothing computed depends on them). Loading repairs a section as
  `file-format.md` says (letter, parent, plane and place each fall back, with a report). The corpus
  holds `model-1/section-views.caditor` and `journal-1/section-views.journal`.

## Export (`export/drawing_sheet.rs`, `export/pdf.rs`)

- `export_sheet(path, plan, views, hidden_lines, format, cancel)` builds one `Figure` at the
  sheet's real size: view pieces on `Layer::Visible` and `Layer::Hidden`, hatch lines on
  `Layer::Hatching`, cutting chain lines on `Layer::Cutting` with their thick ends on `Visible`,
  frame, marks (arrows included) and title block on `Layer::Frame`. Lines, circles, arcs and ellipses stay exact; other curves become
  polylines within the sheet chord and are counted in `SheetExported::approximated`. Written
  atomically. `SheetFormat::of` picks by extension (PDF by default).
- SVG is the page itself (`width`/`height` in mm, viewBox the sheet), strokes per layer (0.5 mm
  visible, 0.25 mm hidden dashed 3/1.5, 0.25 mm hatching, 0.25 mm cutting chain 12/3/0.5/3,
  0.35 mm frame); PDF the same widths and dash patterns. DXF names its layers with lineweights,
  a `DASHED` linetype on `Hidden` and `DASHDOT` on `Cutting`, so `parse_dxf` reads hidden lines
  as construction.
- PDF is caditor's own writer, no dependency: PDF 1.4, one page sized in points, millimetre
  coordinates through a `cm` transform, arcs and ellipses as Béziers, text as filled Inter outlines
  from `caditor-lettering` (no embedded font), the producer `APPLICATION` and nothing identifying
  the user.

## App (`drawings_panel.rs`, `sheet_views.rs`, `sheet_export.rs`)

- Commands: Drawings (toggles the right-hand `DrawingsTool` panel), New drawing (of the bodies
  selected in the view or chosen in the tree, else every shown body; opens it) and Export drawing
  sheet… (opens the panel and exports once the views are ready). Every setting in the panel is one
  undoable `changing_drawing`; Delete is undoable.
- `DrawingViews` works out each wanted view (standard and section) on its own `drawing views`
  thread, keyed by the view (a section by its whole `SectionView` and its resolved place, so a
  parameter edit redoes it and a failing offset is its outcome) and the drawn bodies' result
  `Arc`s, so a view is redone only when a body it draws or its section settings change; a newer
  basis bumps a shared ticket that interrupts the view in flight. A panic becomes `Panicked`. A
  failing view is a callout naming it and why; the others draw as usual. Closing the panel or a
  new session forgets everything.
- The panel's Section views rows set each section's parent view (combo), plane (the two the
  parent sees as lines), Through Centre or Offset (switching takes the plane's current place) with
  a Distance value field (units and parameters, the value kept), Look the other way and Delete;
  Add a section view adds one. A section whose cutting line is not drawn (`unmarked`) is a
  warning callout.
- The sheet preview paints the plan in theme tokens (paper `raised`, lines `text`, hidden lines
  and hatching `text_muted`, hidden dashed, cutting lines chained with thick ends): drag pans, `zoom_delta` (pinch, Ctrl and scroll) and the Zoom in, Zoom out
  and Fit buttons zoom, arrow keys pan the focused sheet.
- Export goes through `FileCommand::ExportSheet` with a `SheetJob` (plan plus each view's
  outcome), `Purpose::Sheet`, the drawing folder and the ordinary output confirmation, written on
  the file worker.
