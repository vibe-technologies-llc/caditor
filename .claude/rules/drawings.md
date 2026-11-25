---
paths:
  - "crates/caditor-document/src/drawings.rs"
  - "crates/caditor-document/src/drawings_tests.rs"
  - "crates/caditor-document/src/drawing_views.rs"
  - "crates/caditor-document/src/drawing_sections.rs"
  - "crates/caditor-document/src/drawing_details.rs"
  - "crates/caditor-document/src/drawing_sheet.rs"
  - "crates/caditor-file/src/drawings.rs"
  - "crates/caditor-file/src/tests/drawings.rs"
  - "crates/caditor-file/src/export/drawing_sheet.rs"
  - "crates/caditor-file/src/export/pdf.rs"
  - "crates/caditor/src/drawings_panel.rs"
  - "crates/caditor/src/sheet_views.rs"
  - "crates/caditor/src/sheet_canvas.rs"
  - "crates/caditor/src/sheet_export.rs"
  - "crates/caditor/src/ui_tests/drawings.rs"
  - "crates/caditor/guide/drawings.md"
---

# Technical drawings

A drawing lays out standard views, section views and detail views of the model's bodies on an ISO sheet with a
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
  shown body; at most `MAX_DRAWING_BODIES`), `hidden_lines`, `sections` (`SectionView`s, below) and `details` (`DetailView`s, below).
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
- A `SectionView` is a `letter` (one of `VIEW_LETTERS`, A to Z without I, O and Q as ISO 128
  advises, unique among the drawing's sections and details together, so at most
  `MAX_SECTION_VIEWS`; `Drawing::new_section` takes the first free one, `unused_letter`, and is the
  drawing's key for the view), its `parent` (`StandardView::SECTIONED`:
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
  side it looks toward. `DrawnView` (`Standard`, `Section(letter)` or `Detail(letter)`) keys every drawn view.
- A `DetailView` is a `letter` (the same `VIEW_LETTERS` pool, so a letter is a section or a detail,
  never both: `ViewLetterRepeated`; at most `MAX_DETAIL_VIEWS`), its `parent` (`DetailParent`: a
  `StandardView`, drawn or not, or a section's letter, which the drawing must have:
  `DetailParentMissing`), a `centre` and `radius` in the parent view's own model coordinates
  (millimetres in the view's frame, `HiddenLineView::frame`, never sheet lengths, so a change of
  sheet scale leaves the circle on the same part; `DetailOutOfRange` past `MAX_DETAIL_EXTENT`, not
  finite or under `MIN_DETAIL_RADIUS`) and a `scale` (`DetailScale::Enlarged`, the default, is
  `ScaleRatio::enlarging` of twice the sheet scale: the smallest ISO 5455 ratio at least that,
  50:1 at most; or a chosen `ScaleRatio`, checked as the drawing's is). The values are plain
  numbers, not expressions, so a detail names no parameter. `adding_detail_view` (returns its
  letter) builds the transaction; the app changes and deletes through `changing_drawing`, deleting
  a section's details with it since a detail of a missing section is refused. `Document::scaled`
  multiplies the circle by the factor and shifts its centre by the model shift along the parent's
  view axes (`Rescaler::view_shift`).
- `positions` maps a `DrawnView` to where the user placed it: the centre of the view's box on the
  sheet, in sheet millimetres from the middle of the sheet's view area, at most `MAX_VIEW_PLACE`
  either way (`ViewPlaceOutOfRange` past it or not finite). A view without one is laid out
  automatically. `normalized` drops positions of views the drawing no longer has (an unticked
  standard view, a deleted section or detail), so a reused letter never inherits one. Sheet
  positions are not model lengths: `Document::scaled` leaves them alone.

## Views (`drawing_views.rs`)

- `StandardView::look_and_up`: front looks along +Y with Z up (sheet x is model X), top along -Z
  with Y up, right along -X, left along +X, isometric along (-1, 1, -1) with Z up.
- `drawing_view(view, solids)` runs kernel `hidden_lines` (`kernel-hidden-lines.md`) at
  `MeshQuality::SMOOTH`'s tolerance for the bodies' extent, under `catch_unwind`, and keeps sharp
  edges and silhouettes only: tangent edges and seams are left out, as ISO 128 draws them. It is
  interruptible (`Cancelled`); no solid is `NoBodies`. `ViewLines::bounds` sizes the view on the
  sheet. `drawn_bodies` resolves which bodies a drawing draws in an `Evaluation` (with every body
  standing that is not hidden when none are chosen) and names the chosen ones missing.

## Details (`drawing_details.rs`)

- `detail_view(detail, parent)` takes the parent's finished `ViewLines` and keeps its pieces
  inside the `Disc` of the circle: a line by exact segment-circle clipping (`Disc::clip_segment`),
  any other curve by sampling at a thousandth of the radius and bisecting each crossing of the
  circle to full precision (`BISECTIONS`), so a kept piece's ends lie on the circle, and a periodic
  curve crossing its seam comes out as two pieces. Pieces whose box misses the circle are skipped
  first. The result is a `ViewLines` of view `Detail(letter)` with `bounds` the circle's box, the
  parent's `section` (so hatching and the no-hidden-lines rule carry over) and `clip` the disc,
  which `drawn_hatching` uses to clip a section's hatch lines on the sheet. It polls
  `interrupt::check` per piece and runs under `catch_unwind`. The parent being unavailable is
  `DrawingViewError::ParentFailed` or `ParentNotDrawn`.

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
  and detail views stand in a column right of the grid (`SideColumn`), as wide as the widest,
  sections first and then details in the drawing's order, each under its label ("A–A" or
  "B (2:1)", `SECTION_LETTER_HEIGHT`), the column centred down the view area. A detail is placed at
  its own scale (`DetailScale::ratio_at` the sheet's), so the column's size depends on the sheet
  scale; the label rows and gaps are fixed sheet lengths. `Fit` takes the largest preferred ratio
  at which the grid and the column fit (`Grid::fits`, tried from 50:1 down); a chosen ratio that does
  not fit still lays out, with `SheetPlan::fits` false for the app to say so.
- `lay_out_sheet(drawing, fields, extents, cuts)` takes each finished section's cutting plane by
  letter. Its `CuttingLine` on the parent view (ISO 128-44) runs across the parent's extent
  `CUTTING_OVERSHOOT` past it each way, as a chain line with `CUTTING_END` thick ends; open arrows
  (`SheetMark` lines) touch each end pointing the way the section looks and the letter stands
  beyond each tail. A section whose parent is not on the sheet (or not yet worked out) is in
  `SheetPlan::unmarked`.
- A detail's circle is a `SheetMark::Circle` on its parent's placed view (the circle scaled by the
  parent's scale) with the letter outside it up and to the right, and again round the detail view
  in the column; a detail whose parent is not placed has no circle on a parent.
- Placing by hand (`by_hand`) runs after the automatic layout, which it never changes: the grid,
  the column and the `Fit` scale come from the automatic layout alone, so a view without a position
  stands exactly where it did before positions existed. A placed view's box centre is put at its
  position (clamped into the view area). `PlacedView::extent` is the view's box on the sheet.
  `ViewMovement` says how a view may move: with the front drawn, the top view moves only up
  (`Up`, its x stays the front's) and the right and left views only across (`Across`, their y
  stays the front's); everything else, and every view when the front is not drawn, is `Free`.
  An aligned view without a position follows the front's shift whole, one with a position takes
  its own coordinate along its axis and the front's across it. `SheetPlan::moved` turns a drag
  into positions: it keeps only the allowed part of the motion, and moving the front also shifts
  the stored positions of its aligned views, so dragging the front carries every aligned view.
  Side-column labels and the circle round a detail view are drawn from the final placed box
  (`side_labels`), so they travel with a placed view; cutting lines and circles on a parent follow
  the parent where it stands.
- Positions are kept from the view area's middle rather than as offsets from the automatic place,
  because the automatic place moves whenever any view is added, removed, resized or rescaled,
  which would drag a hand-placed view with it; and rather than from the sheet's corner, because a
  change of sheet size or orientation then keeps the arrangement centred where the automatic
  layout also centres its views, the clamp holding any view that would fall off a smaller sheet at
  its edge. A scale change grows each placed view about its own centre.

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
- Each drawing's `details` is a field written only when it has any, each detail `Lenient`, so older
  drawings read with none and an older reader drops them (nothing computed depends on them). A
  detail is `letter`, `parent` (`standard` with a view key, or `section` with a letter), `centre`
  as two numbers, `radius` and `scale` (`enlarged` or `ratio`). Loading takes the letters the
  sections left (a repeated or unusable letter takes the next free one), leaves out with a report a
  detail that is unreadable, past `MAX_DETAIL_VIEWS`, whose parent is unknown or a section the
  drawing lacks, or whose circle cannot be used, and gives an unusable scale `Enlarged`. The
  corpus holds `model-1/detail-views.caditor` and `journal-1/detail-views.journal`.
- Each drawing's `positions` is a field written only when it has any: a list of `view` (`standard`
  with a view key, `section` or `detail` with a letter) and `at` (two numbers), each `Lenient`, so
  older drawings read with none and lay out automatically, and an older reader drops them (losing
  only where the views were placed). Loading leaves out, with one report per drawing, a position
  that is unreadable, names a view the drawing lacks, repeats a view or lies past
  `MAX_VIEW_PLACE`. The corpus holds `model-1/placed-views.caditor` and
  `journal-1/placed-views.journal`.
- Each drawing's `sections` is a field written only when it has any, each section `Lenient`, so
  drawings stored before section views read with none and an older reader drops them (losing only
  the section views; nothing computed depends on them). Loading repairs a section as
  `file-format.md` says (letter, parent, plane and place each fall back, with a report). The corpus
  holds `model-1/section-views.caditor` and `journal-1/section-views.journal`.

## Export (`export/drawing_sheet.rs`, `export/pdf.rs`)

- `export_sheet(path, plan, views, hidden_lines, format, cancel)` builds one `Figure` at the
  sheet's real size from the same `SheetPlan` the app shows, so views placed by hand are written
  where the sheet shows them: view pieces on `Layer::Visible` and `Layer::Hidden`, hatch lines
  (`drawn_hatching`, clipped to a detail's circle) on `Layer::Hatching`, cutting chain lines on `Layer::Cutting` with their thick ends on `Visible`,
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

## App (`drawings_panel.rs`, `sheet_canvas.rs`, `sheet_views.rs`, `sheet_export.rs`)

- Commands: Drawings (toggles the right-hand `DrawingsTool` panel), New drawing (of the bodies
  selected in the view or chosen in the tree, else every shown body; opens it), Export drawing
  sheet… (opens the panel and exports once the views are ready) and Drawing sheet in the view
  (`Command::SheetInView`, a View toggle, in the View menu and the palette; refused with no
  drawing). Every setting in the panel is one
  undoable `changing_drawing`; Delete is undoable.
- `DrawingViews` works out each wanted view (standard, section and detail) on its own `drawing views`
  thread, keyed by the view (a section by its whole `SectionView` and its resolved place, so a
  parameter edit redoes it and a failing offset is its outcome) and the drawn bodies' result
  `Arc`s, so a view is redone only when a body it draws or its section settings change; a newer
  basis bumps a shared ticket that interrupts the view in flight. A panic becomes `Panicked`. A
  detail is keyed by its `DetailView` and its parent's outcome `Arc` (by pointer), so it is redone
  when either changes, and is `Working` until the parent is done; a parent that failed or is not
  among the drawing's views gives `ParentFailed` or `ParentNotDrawn`. A
  failing view is a callout naming it and why; the others draw as usual. Closing the panel or a
  new session forgets everything.
- The panel's Section views rows set each section's parent view (combo), plane (the two the
  parent sees as lines), Through Centre or Offset (switching takes the plane's current place) with
  a Distance value field (units and parameters, the value kept), Look the other way and Delete;
  Add a section view adds one. A section whose cutting line is not drawn (`unmarked`) is a
  warning callout.
- The panel's Detail views rows set each detail's parent (the drawing's views and its sections,
  combo), centre across and up and radius (value fields: units and parameters, evaluated to a
  length when committed), and scale (twice the sheet's, or an ISO 5455 ratio), with Delete; Add a
  detail view puts a circle of a quarter of the parent's smaller side on the lower left
  corner of its box (where lines meet), on the first drawn view. Deleting a section deletes the details enlarging it in the same
  transaction.
- `sheet_canvas.rs` paints the plan in theme tokens (paper `raised`, lines `text`, hidden lines
  and hatching `text_muted`, hidden dashed, cutting lines chained with thick ends), fitted to the
  space it is given whole: `zoom_delta` (pinch, Ctrl and scroll) zooms about the pointer
  (`SheetCamera::zoom_about`), the Zoom in, Zoom out and Fit buttons zoom about the middle, each
  zoom button disabled at its bound (`LEAST_ZOOM`, `MOST_ZOOM`) and followed by a muted percentage.
  The panel preview and the sheet view use the same `SheetCanvas` (camera, picked view, drag).
- A drag starting on a grip (`grip_at`, within `GRIP_REACH` points) moves it, anywhere else pans:
  a detail circle's rim (resizes, the radius growing by how much farther from the centre the
  pointer is than where it was pressed), the inside of a detail circle (moves its centre, in the
  parent's model lengths) or a view's box (the smallest one under the pointer). While it is held,
  the canvas lays out a copy of the drawing with the drag applied, so views, labels, cutting lines
  and circles move live; the drop is one `changing_drawing` (Move drawing view, Move detail
  circle, Resize detail circle). Hovered grips are outlined dashed (a circle drawn heavier) in
  `accent_text` with a grab or resize cursor; the picked view has a solid, heavier outline, so
  picked and hovered never differ by colour alone.
- Keyboard: a click picks the view under it and focuses the sheet. With the sheet focused and a
  view picked, the arrow keys move that view `NUDGE` (1 mm on the sheet; `FAR_NUDGE`, 10 mm, with
  Shift), each press one undoable change through `SheetPlan::moved` (so a top or side view keeps
  its alignment), and Escape lets the view go; with none picked the arrow keys pan. The sheet holds
  arrows (and Escape while a view is picked) through a focus lock filter. The toolbar's Move the
  picked view left, right, up and down buttons move it `FAR_NUDGE` per click, the single-pointer
  way to move a view without dragging (WCAG 2.5.7); they are disabled with the reason when no view
  is picked or the picked one cannot move that way.
- Reset position: the sheet's context menu on a view (disabled with a reason when it is not placed)
  and the panel's Layout rows (one per placed view, "Reset the position of …") remove its position
  in one change; the Layout section otherwise says every view is laid out automatically.
- The sheet in the view (`DrawingsTool::sheet_in_view`, `show_in_view`) is an opaque `egui::Area`
  over the viewport's rect, like Home: Back to the 3D view, the drawing's name and the zoom
  buttons above the canvas, which takes the rest. While it is shown the start card stays away
  (`ViewportState::cover`), the 3D view takes no keys, the panel replaces its preview with a note
  and Show the sheet in this panel, and the canvas is focused once. Escape (no dialog, no text
  field) first lets a picked view go, then shows the model again; so do the command, both buttons,
  closing the panel and editing a sketch.
- Rename opens a name field focused with the name selected (once the click's release has passed,
  since egui drops focus then); a refused name is said in a notice and stays in the field to be
  corrected. New drawing is disabled at `MAX_DRAWINGS` with the refusal on hover, and the empty
  Section views and Detail views lines are muted.
- Export goes through `FileCommand::ExportSheet` with a `SheetJob` (plan plus each view's
  outcome), `Purpose::Sheet`, the drawing folder and the ordinary output confirmation, written on
  the file worker. The picker proposes the drawing's name in the format the last sheet was written
  in (`app-files.md`, Export memory), and the format written is the picked name's extension.
