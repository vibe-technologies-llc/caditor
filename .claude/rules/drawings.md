---
paths:
  - "crates/caditor-document/src/drawings.rs"
  - "crates/caditor-document/src/drawings_tests.rs"
  - "crates/caditor-document/src/drawing_views.rs"
  - "crates/caditor-document/src/drawing_sections.rs"
  - "crates/caditor-document/src/drawing_details.rs"
  - "crates/caditor-document/src/drawing_dimensions.rs"
  - "crates/caditor-document/src/drawing_dimensions_tests.rs"
  - "crates/caditor-document/src/drawing_centres.rs"
  - "crates/caditor-document/src/drawing_centres_tests.rs"
  - "crates/caditor-document/src/drawing_callouts.rs"
  - "crates/caditor-document/src/drawing_callouts_tests.rs"
  - "crates/caditor-document/src/drawing_fits.rs"
  - "crates/caditor-document/src/drawing_fits_tests.rs"
  - "crates/caditor-document/src/drawing_sheet.rs"
  - "crates/caditor-file/src/drawings.rs"
  - "crates/caditor-file/src/tests/drawings.rs"
  - "crates/caditor-file/src/export/drawing_sheet.rs"
  - "crates/caditor-file/src/export/pdf.rs"
  - "crates/caditor/src/drawings_panel.rs"
  - "crates/caditor/src/sheet_views.rs"
  - "crates/caditor/src/sheet_canvas.rs"
  - "crates/caditor/src/sheet_export.rs"
  - "crates/caditor/src/sheet_tools.rs"
  - "crates/caditor/src/ui_tests/drawings.rs"
  - "crates/caditor/src/ui_tests/drawing_marks.rs"
  - "crates/caditor/guide/drawings.md"
---

# Technical drawings

A drawing lays out standard views, section views and detail views of the model's bodies on an ISO sheet with a
frame and a title block, with dimensions measured from the model (toleranced if wanted), notes,
centre marks and centrelines, and hole and thread callouts, and exports it as PDF, SVG or DXF. It is model content, worked out from the kernel's hidden
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
- `dimensions` (`DrawingDimension`) and `notes` (`DrawingNote`) are the drawing's annotations, each
  with an `AnnotationId` from the drawing's own counter `next_annotation` (`new_annotation_id`,
  `first_free_annotation`), shared by both kinds, never reused: `set_drawings` keeps each drawing's
  counter at least the one it had, and `Drawing::same_content` (so `Drawings::same_content`)
  ignores it. At most `MAX_ANNOTATIONS` together; an id repeated, a dimension's `place` past
  `MAX_DETAIL_EXTENT` or not finite, its `text_offset` past `MAX_TEXT_OFFSET`, `decimals` past
  `MAX_DIMENSION_DECIMALS`, a note's text empty or past `MAX_NOTE_CHARS` (`note_text` makes it one
  trimmed line in `normalized`) or its `at` past `MAX_VIEW_PLACE` are refused in words.
- A dimension is a `view` (`DrawnView`), a `measure` (`DimensionMeasure`: `Linear` between two
  `EdgePoint`s, an edge's `EdgeSpot` `Start`, `End` or `Centre`, `Horizontal`, `Vertical` or
  `Aligned` (`DimensionDirection`); `Parallel` between two straight edges; `Radius` or `Diameter`
  of a round edge; `Angle` between two straight edges), a `place` in the view's own model
  coordinates (like a detail's circle, so it stays by the part when the view moves, and
  `Document::scaled` moves it with the part by `Rescaler::view_shift` of its view or a detail's
  parent), `decimals` (the most shown, trailing zeros dropped) and `text_offset` (sheet millimetres
  along its line). Edges are referred to by `EdgeName` alone, never by position, so a dimension
  follows edits as features do. A dimension whose view is gone is kept, not dropped: it shows as
  lost. A note is `text`, `at` (sheet millimetres from the view area's middle, like a position) and
  an optional `NoteLeader` (a view and an `EdgeName`).
- A dimension's optional `tolerance` (`DimensionTolerance`): `Symmetric` (±, above zero),
  `Deviations` or `Limits` (an upper and a lower deviation, upper above lower), or `Fit` (an
  `IsoFit` and whether its deviations are written). Deviations are plain millimetres (degrees on an
  angle) of at most `MAX_DEVIATION` either way, not expressions and not scaled by
  `Document::scaled`, since a tolerance is a manufacturing limit, not model geometry. A fit on an
  angle is refused (`FitOnAngle`); `is_usable_on` is the same check for the file reader.
- `centres` (`CentreMark`) and `callouts` (`DrawingCallout`) share the annotation counter and
  `MAX_ANNOTATIONS` (`annotation_count`). A centre mark is a view and a `CentreTarget`: `Circle`
  (an `EdgeName` seen round: a centre mark) or `Cylinder` (a `FaceName` whose silhouettes are two
  parallel lines: a centreline). It has no place of its own; it stands where its target is. A
  callout is a view, the `FeatureId` of the hole or thread it reads (stored and counted like a
  drawing's bodies in `set_drawings`), the `EdgeName` its leader points at and `at` (sheet
  millimetres like a note, `CalloutPlaceOutOfRange` past `MAX_VIEW_PLACE`).

## ISO 286 fits (`drawing_fits.rs`)

- `IsoFit` is a `FitLetter` (hole H; shafts f, g, h, js, k, n, p) and a grade in `FIT_GRADES`
  (IT5 to IT11), written and read as its code ("H7", "js6"). `deviations(size)` gives the upper
  and lower deviation in millimetres from ISO 286-1's tables, kept small and exact: the standard
  tolerance grades and the fundamental deviations of those letters over the thirteen main size
  ranges up to `MAX_FIT_SIZE` (500 mm), a size on a step belonging to the range it ends (rounded to
  a millionth first, so measuring noise keeps it there). js is ±IT/2 with odd values of IT7 to IT11
  rounded down to even, as ISO 286-2 tabulates; k is raised only for IT4 to IT7. Sizes past the
  table, or not above zero, have none. The tests hold it to published ISO 286-2 values.

## Views (`drawing_views.rs`)

- `StandardView::look_and_up`: front looks along +Y with Z up (sheet x is model X), top along -Z
  with Y up, right along -X, left along +X, isometric along (-1, 1, -1) with Z up.
- `drawing_view(view, solids)` runs kernel `hidden_lines` (`kernel-hidden-lines.md`) at
  `MeshQuality::SMOOTH`'s tolerance for the bodies' extent, under `catch_unwind`, and keeps sharp
  edges and silhouettes only: tangent edges and seams are left out, as ISO 128 draws them. It is
  interruptible (`Cancelled`); no solid is `NoBodies`. `ViewLines::bounds` sizes the view on the
  sheet. `drawn_bodies` resolves which bodies a drawing draws in an `Evaluation` (with every body
  standing that is not hidden when none are chosen) and names the chosen ones missing.

## Dimensions and notes (`drawing_dimensions.rs`)

- Measuring works on the view's `ViewLines`, exact projections in orthographic views: `seen_edge`
  gathers the pieces whose `ViewSource::Edge` carries the name (visible and hidden alike) into one
  `SeenEdge`, the union of their ranges on the shared flat curve; pieces of that name from two
  different edges are `AnnotationProblem::EdgeAmbiguous` (never a guess), none is `EdgeGone`. A
  detail view measures on its parent's unclipped lines (`measured_lines_of`), so its ends are the
  edge's, not the circle's. The isometric view (and a detail of it) measures nothing
  (`is_measured_view`, `Isometric`), since it is not drawn true to size.
- `dimension_geometry` turns a measure into a `DimensionGeometry` in view coordinates: a spot is
  an end of a straight or open edge (a whole circle has none: `NoEnd`) or the centre of a round
  one (`Curve2::Circle`, or an ellipse whose radii agree to a millionth; else `NotRound`); a
  `Parallel` pair must agree in direction to a sine of 1e-6 (`NotParallel`), its witness points
  the `place` clamped onto each edge; an `Angle` meets at the lines' crossing (`NoAngle` when
  parallel) and takes, of the four angles, the one the `place` lies in, under π.
- `pick_edge` finds the nearest edge piece within reach of a sheet point across the placed views;
  `dimension_from_picks` makes what two picks mean: one straight edge twice is its length, one
  round edge twice its diameter (whole) or radius (arc), two straight edges `Parallel` or `Angle`,
  anything else `Linear` between each pick's nearest end or centre, its direction from where the
  place lies (`DimensionDirection::from_place`: within the points' span across is horizontal,
  within it up is vertical, else aligned). Picks in two views, in the isometric view or on a
  spline are refused in words (`DimensionRefusal`).
- `SheetPlan::annotate(drawing, lines)` lays every annotation out on the sheet into
  `SheetPlan::annotations` (`SheetAnnotation`: marks, one `SheetText` with its angle, the
  `SheetMeasure` export writes as a DXF dimension, and the `AnnotationProblem` if any), so the
  canvas and export draw the same thing; `plan_of` in the app calls it. Text is
  `DIMENSION_TEXT_HEIGHT` (3.5 mm) cap height. Arrowheads, extension lines with their gap and
  overshoot, the lift of the value above its line and `readable_angle` (readable from the bottom or
  the right) come from `caditor_sketch::annotation`, shared with sketch export. A linear
  dimension's line passes through the place, with extension lines only where a point is off it; a
  radial one leads from the centre (radius) or across (diameter) out past the arc toward the place,
  the value beyond; an angle is an arc through the place, with extension lines along each edge
  where the arc runs past it. A lost dimension is laid out at its place as a short line and its
  value as `?` (`R?`, `⌀?`, `?°`), with the problem; one whose view lines are still being worked
  out is left off until they are. A note stands at its place; a leader runs from a shoulder beside
  the text to the middle of its edge with an arrowhead, or is left out with the problem when the
  edge is gone.
- A tolerance is laid out with the value (`value_lines`, `ValueLines`): plus or minus and a fit
  without deviations on the value's line (`20 ±0.1`, `⌀10 H7`); deviations stacked beside it at
  `DEVIATION_TEXT_HEIGHT` (2.5 mm), the lower one's foot on the value's, the upper above, the group
  centred where the value alone would stand; limits as two full lines, the lower where the value
  would be. `deviation_text` writes a deviation signed (`+0.015`, `−0.01` with a real minus) to four
  decimals at most, and nothing as `0`. A fit whose size is past the table is written alone.
- `SheetAnnotation` holds several `texts` (`text()` is the first, `content()` all joined): bounds,
  `distance_to` and `text_corners` cover every one, at either anchor; an annotation with no marks
  and no text has no bounds and is never picked.
- `moved_annotation` moves a dimension's place by a sheet vector over its view's scale and a note
  or callout by the vector, clamped into the view area like positions; a centre mark does not move.
  `annotation_at` finds the nearest within reach (text box or any mark).

## Centre marks (`drawing_centres.rs`)

- `centre_geometry` reads a target on the view's measured lines (a detail's parent's, like a
  dimension): a circle is `seen_edge` round (`NotRound` otherwise); a cylinder gathers the
  silhouette pieces of its `FaceName` (`OutlineGone` with none, `EdgeAmbiguous` from two faces),
  which must be straight, parallel to a sine of 1e-6 and lie on exactly two lines (`NotCylinder`
  otherwise), the axis running midway over their joint extent. A centre mark is two lines through
  the centre along the sheet's axes reaching `CENTRE_OVERSHOOT` (2 mm) past the rim, a centreline
  the axis reaching that past each end, both drawn as long-dash-dot chain lines. A lost one is
  kept, laid out with no marks and its problem, so it is not drawn and the panel names it.
- `centre_targets` finds every whole circle seen round (one per centre, the largest, so a hole's
  two ends or a counterbore share one mark) and every cylinder seen from the side (one per axis,
  the longest); `new_centres` leaves out what the view already marks. `pick_centre` picks the
  nearest edge or silhouette (`nearest_piece`, which `pick_edge` also uses) and makes it a target
  with `centre_target_at`.

## Callouts (`drawing_callouts.rs`)

- `callout_feature` finds what an edge belongs to in the drawn bodies: a Thread whose face
  resolves to one of the edge's faces first, else the hole whose feature made one of them (the
  original of a patterned copy); anything else is `CalloutRefusal::NotAHole`.
- `callout_lines` reads the feature now, with the evaluation's parameter values, into
  `CalloutLine`s of text and `CalloutSymbol`s (⌀, ↧ depth, ⌴ counterbore, ⌵ countersink): a tapped
  hole's thread designation and depth first (its own depth, else the hole's), then the bore
  (`⌀d ↧depth`, the depth only when blind; a slot as width × overall length), then a line per
  counterbore or step (`⌴⌀d ↧depth`) or the countersink (`⌵⌀d × angle°`); a thread's designation
  and depth. Holes sized by circles take their circle's diameter (and scale the heads like the
  feature); when the circles differ the hole's point is found from the edge's face origin
  (`Hole::point_of`). A feature gone is `FeatureGone`, an expression that does not evaluate
  `CalloutValue`. `callout_texts` reads every callout of a drawing; the app passes them to
  `SheetPlan::annotate`, so the plan follows hole edits each time it is laid out.
- A callout is laid out as lines of `CALLOUT_TEXT_HEIGHT` text centred on `at`, symbols drawn as
  marks in the text's place (Inter has none of these glyphs, and marks come out the same in every
  format), and a leader from a shoulder beside the first line to its edge: on a circle seen round
  it ends on the rim pointing at the centre, else at the edge's nearest point. A lost feature or
  edge makes it `?` with the problem and no leader; a view still being worked out leaves only the
  leader off.

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
- Each drawing's `dimensions` and `notes` are fields written only when it has any, each `Lenient`,
  with `next_annotation` written when not zero, so older drawings read with none and an older
  reader drops them (losing only the annotations; nothing computed depends on them). A dimension
  is `id`, `view` (as positions write it), `measure` (`linear` with `from` and `to`, each `edge` as
  hex and `spot` key, and `direction` key; `parallel` or `angle` with `first` and `second`;
  `radius` or `diameter` with the edge), `place` (two numbers), `decimals` and `text_offset` (when
  not zero); a note `id`, `text`, `at` and `leader` (when any: `view` and `edge`). Loading leaves
  out, with one report per drawing and kind, a record unreadable, with a repeated id, an unknown
  view or measure, an unreadable edge or a place out of range, an empty note or one out of range,
  and those past `MAX_ANNOTATIONS`; more decimals than the most are cut to it, an unknown direction
  is aligned, an overlong note is cut, an unreadable leader dropped (each reported); a dimension in
  a view the drawing lacks is kept, shown lost. `next_annotation` becomes at least the first free
  id. The corpus holds `model-1/dimensions-and-notes.caditor` and
  `journal-1/dimensions-and-notes.journal`.
- A dimension's `tolerance` is a field written only when it has one (`symmetric` with a number,
  `deviations` or `limits` with `upper` and `lower`, or `fit` with its code and `deviations`), and
  each drawing's `centres` (`id`, `view`, `target` as `circle` or `cylinder` with a hex name) and
  `callouts` (`id`, `view`, `feature`, `edge` as hex, `at`) are fields written only when it has
  any, each `Lenient`, so drawings from before them read without them and an older reader drops
  them (losing only these annotations). Loading drops a tolerance that does not read or would be
  refused, with a report, keeping its dimension; and leaves out, with one report per drawing and
  kind, a centre mark or callout unreadable, with a repeated id (shared with dimensions and notes),
  an unknown view or target, an unreadable name, a feature id past the storable ones or a place
  out of range. The corpus holds `model-1/centre-marks-callouts-and-tolerances.caditor` and
  `journal-1/centre-marks-callouts-and-tolerances.journal`.
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
  frame, marks (arrows included) and title block on `Layer::Frame`, and the plan's annotations on
  `Layer::Annotation` ("Annotations", 0.25 mm): a dimension as one `Shape::Dimension` (a DXF
  `DIMENSION` with its own block; marks and value in SVG and PDF, a tolerance's further texts in
  `Dimension::more`, drawn in the block, and the DXF text override all of them joined), a note or
  callout as its lines and texts, and centre marks and centrelines on `Layer::Centre` ("Centre
  lines", 0.25 mm long-dash-dot like cutting lines, `DASHDOT` in DXF). A lost dimension, centre
  mark or callout is left out and a lost leader not drawn, each counted in
  `SheetExported::left_out` beside `dimensions`, `notes`, `centres` and `callouts`, which the export
  notice reports. Lines, circles, arcs and ellipses stay exact; other curves become
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
- Annotations (`sheet_tools.rs`): Add a dimension and Add a note (icon buttons beside the move
  buttons) and the commands Dimension the drawing and Add a note to the drawing
  (`DrawingsTool::start_tool`, refused with no drawing; in the Model menu and the palette) start a
  `SheetTool` on the shared canvas; a row under the toolbar says the next step and holds Stop
  adding. The dimension tool takes two `pick_edge` clicks (the picks and the edge under the pointer
  drawn heavier in `accent_text`), checked with `dimension_from_picks` as they come (a refusal is a
  notice, the step kept), then previews the dimension at the pointer and places it on the next
  click or drop: one Add dimension change, the new one picked and the tool kept for the next.
  Re-attach (the panel) runs it with `replacing`, keeping the dimension's id, decimals and offset,
  and ends after one. The note tool takes a click for its place, focuses the Note text field under
  the toolbar, takes a click on an edge as its leader (elsewhere moves the note) and is placed by
  Place the note or Enter as one Add note change. Escape steps the tool back, then ends it
  (`SheetCanvas::let_go`, also from the sheet view's Escape).
- A click without a tool picks the annotation under it before a view (`annotation_at`), shown with
  a solid outline round its bounds; a drag on one moves it (Move dimension, Move note); with it
  picked and the sheet focused the arrow keys and move buttons move it, and Delete or Backspace
  deletes it (Delete dimension, Delete note). Plain Delete is otherwise Delete selection's, so the
  app lets the focused canvas claim it before dispatching commands (`DrawingsTool::claim_delete`).
  Lost annotations are drawn dashed in `error`, the rest in `text`; values turn with their line.
- The panel's Annotations rows (after Layout) name each dimension ("Dimension 1: 40
  horizontal distance in the front view", numbered by id plus one, so names never repeat) with
  Re-attach and Delete, an error callout saying what it lost, and Measures (direction, or radius
  and diameter), Tolerance (a combo of No tolerance, Plus or minus, Deviations, Limits and, but on
  an angle, ISO 286 fit; then Deviation, Upper and Lower deviation value fields, or Fit and Grade
  combos and Show the deviations), Decimals and Value along (`text_offset`), with a muted line
  giving a fit's deviations at the size measured or a warning past the table; each note with
  Change the text (a field like Rename), Remove the leader and Delete, and a warning when its
  leader lost its edge; each centre mark or centreline ("Centre mark 3 of a circle in the top
  view") and callout ("Callout 4 of Hole 1 in the top view", its words on hover) with Delete and
  an error callout when lost. Angles take no tolerance in the panel, since its fields are lengths.
- Centre marks and callouts (`sheet_tools.rs`): Add a centre mark or centreline and Add a callout
  to a hole (toolbar buttons and the commands of those names) start `SheetTool::Centre` (one click
  on a circle or a cylinder's outline adds one, refused in words on anything else or on what is
  already marked, the tool kept) and `SheetTool::Callout` (a click on an edge, checked with
  `callout_feature`, then a click where the text stands, previewed at the pointer). Add centre
  marks to this view (toolbar button and command `DrawingCentreMarks`) adds `new_centres` of the
  picked view in one Add centre marks change; it is refused without a picked view
  (`SheetRefusal::NoViewPicked`) and says so when nothing is left to mark. A picked centre mark is
  deleted like any annotation but never dragged or nudged (its move buttons say why).
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
