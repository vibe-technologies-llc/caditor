---
paths:
  - "crates/caditor-file/src/import/**"
  - "crates/caditor-file/src/export/**"
---

# DXF, SVG and STEP import, export

## DXF import (`import/`)

- `parse_dxf` reads ASCII and binary DXF into a `Drawing` of 2D `DrawingCurve`s in millimetres plus
  plain-language notes; only header, layers, MLINESTYLEs, blocks and entities are kept.
- Text is UTF-8 when valid, else in the header's `$DWGCODEPAGE` (`code_page.rs`; unknown reads as
  1252).
- Hostile-input limits, each an `ImportError` naming it: `MAX_DRAWING_VALUES`,
  `MAX_EXPANDED_OBJECTS` (block expansion, charged per object and per decoded shape, so a long
  polyline repeated in a huge array ends there), `MAX_DRAWING_POINTS`. Curves past
  `MAX_READ_CURVES` are left out with a note and the first ones read; the rest of an item past the
  limit is counted in one step, never walked shape by shape. The sketch's own limit,
  `MAX_DRAWING_CURVES`, applies in `Drawing::arranged` to the curves of the chosen layers, in
  drawing order with a note, so leaving layers out brings later curves in;
  `drawing_transaction` never adds more than it either.
- Units: `$INSUNITS` converts to millimetres with a note; an unknown or missing unit reads as
  millimetres with a note, except missing with imperial `$MEASUREMENT`, which reads as inches.
- Damage ends reading where it is found: earlier records are interpreted and a note says where, so
  one bad line costs the rest of the file, not all of it; the interrupted record is dropped, since
  it may miss fields. A file damaged before any section, or drawing nothing before the damage, is
  refused with the damage as the error.
- Left out silently: paper space, invisible entities, off, frozen and `DEFPOINTS` layers.
  Annotations (text, dimensions) are counted in a note.
- `Drawing::arranged` applies `DrawingOptions` to a parsed drawing: a unit that replaces the one
  the file names (`Drawing::unit_scale` is what the header applied, so the choice converts from
  the file's own numbers), a scale within `MIN_SCALE..=MAX_SCALE`, and centring the outline's
  bounding box (arcs by their sweep) on the origin, and layers left out. Each curve keeps the index
  of its layer (`Drawing::curve_layers` into `Drawing::layers`, in order of first use, names
  compared without case; an entity on layer 0 inside a block takes the layer of its INSERT); only
  layers that kept a curve are listed. It adds a note per change it makes and leaves the original
  untouched.
- Dashed linetypes make a curve construction geometry (`Drawing::construction`, added by
  `drawing_transaction` with ordinary end points), with a note.
- Blocks and INSERTs nest with cycle and depth limits; a block's content is decoded once and shared
  by its instances.
- SPLINE uses control points up to `MAX_SPLINE_DEGREE`, else fit points: with an end tangent the
  cubic AutoCAD draws (`FitPoints::cubic`), otherwise a `DrawingCurve::FitSpline`, which
  `drawing_transaction` makes a centripetal fit-point spline through the same points, closed when
  the SPLINE's closed flag (70, bit 1) is set and three points remain once a repeated first point
  is dropped (a closed one has no ends to join); a note counts them. Past `MAX_FIT_POINTS`, or
  where no such spline can be drawn, the points are fitted with `BSpline::through` as before.
  Hatch boundary splines given by fit points are read the same way, always open.
- HATCH imports boundary paths only, never the fill (a note counts hatches); a path whose source
  objects are all drawn entities of the same space is left to them, so an associative hatch does
  not double its outline.
- Everything becomes a 3D shape in its object coordinate system (arbitrary axis algorithm), is
  transformed, then flattened onto XY: conics projecting to circles become circles and arcs; other
  conics become exact `DrawingCurve::Ellipse`s (`flatten::planar_ellipse`: the principal axes of
  the projected conjugate diameters, the longer as the major axis, a partial one by its end points
  ordered counter-clockwise), which `drawing_transaction` makes sketch ellipses and elliptical
  arcs; non-uniform splines are fitted within a millionth of the drawing's size.
- `drawing_transaction` makes one transaction on an existing or new sketch, dropping curves shorter
  than the joint tolerance and joining ends within it with `Coincident` constraints.

## SVG import (`import/svg/`)

- `read_drawing` reads SVG for a `.svg` or `.svgz` file, or a file not named `.dxf` whose content
  starts with `<` (after a BOM and spaces), gzip or a UTF-16 BOM; anything else goes to
  `parse_dxf`. `parse_svg` unpacks gzip like `.stpz` (`model::unpacked`), decodes UTF-8 or
  UTF-16 by BOM, else Latin-1 with a note, and returns the same `Drawing` as DXF, flattened by the
  DXF reader's `flatten`, so the import dialog, arrangement and `drawing_transaction` are shared.
- `xml.rs` is caditor's own small XML reader, since roxmltree's tokenizer recurses per nesting
  level and overflows the stack on hostile depth: iterative, namespaces resolved, attribute values
  decoded (the five named entities, character references and the DOCTYPE's own text entities,
  which Illustrator uses for `xmlns`; an entity holding markup, nested past `MAX_ENTITY_DEPTH` or
  expanding past `MAX_ENTITY_WORK` is damage), text content kept only inside `style` elements
  (text decoded like attributes, CDATA as is, joined) and inside `text`, `tspan`, `textPath` and
  `a` as pieces placed between the children (`Node::content`), since text layout needs their
  order. Past
  `MAX_DRAWING_ELEMENTS` elements the file is `ImportError::TooManyElements`. Damage ends reading
  where it is found: the elements before it are kept with their open ancestors and a note names
  the line; damage before the root element is `DamagedAt`.
- The root `svg` sizes the drawing (`sizing.rs`): with a viewBox and a width or height, a user unit
  is that length in millimetres over the viewBox's side (`preserveAspectRatio` meet takes the
  smaller of the two, slice the larger, none each); mm, cm, in, pt, pc and px are converted and
  every unit but millimetres gets a note; a bare number is a CSS pixel, as is every user unit
  without a viewBox or size, 96 to the inch, with a note saying a pixel is 0.2646 mm. The
  viewBox's origin is not subtracted, so user coordinates keep their place and caditor's own SVG
  export reads back where it was drawn; y is flipped to point up. `Drawing::unit_scale` is the
  millimetres per user unit, so the dialog's unit choice reads user units as that unit.
- Lengths inside take units at 96 px to the inch and percentages of the nearest viewport (x of its
  width, y of its height, a radius of the normalised diagonal); a nested `svg` places its viewBox
  with x, y, width, height and `preserveAspectRatio` alignment.
- `path` reads every command, absolute and relative, with implicit repeats and the reflected
  controls of S and T; damaged data is read up to the damage and counted. Quadratic and cubic
  Béziers become exact sketch splines (three or four control points, clamped with no inner knots,
  as `BSpline::clamped` makes them) and a Bézier whose controls lie on its chord a line; elliptical
  arcs follow the SVG centre conversion (radii too small scaled up, a zero radius a line) and
  become a conic, which `flatten` makes an arc when circular after the transforms and otherwise an
  exact ellipse or elliptical arc. `rect` (rounded with
  auto `rx`/`ry` clamped to half the sides), `circle`, `ellipse`, `line`, `polyline` and `polygon`
  are read; zero-sized shapes draw nothing.
- `transform` lists (`matrix`, `translate`, `scale`, `rotate` about a point, `skewX`, `skewY`)
  compose down the tree; one that cannot be read is ignored and counted. `use` places its target
  (`href` or `xlink:href`, by id) at x and y; a `symbol` is fitted like a nested `svg`, its viewBox
  and `preserveAspectRatio` (default `xMidYMid meet`) into the use's width and height (else its
  own, else 100%), and draws nothing when either is zero; a target that is the use's own ancestor or already being placed, uses nested past
  `MAX_USE_DEPTH` and elements nested past `MAX_NESTING` are left out with a note. Inside a use
  every element and shape is charged against `MAX_EXPANDED_OBJECTS` (`TooManyCopies`), and each
  element's properties and local shapes are decoded once and shared by its instances.
- Layers: below the root (or below a single group that wraps everything, repeatedly) each group or
  nested `svg` is a layer named by its `inkscape:label`, else its `id`, else `Group <n>`; shapes
  outside them are on `Ungrouped`. Layers are interned like DXF's, names compared without case.
- Style (`style.rs`, `css.rs`): each property is cascaded as SVG does, from the `style` attribute,
  the document's `<style>` sheets (CSS, or no `type`) and the presentation attribute: the highest
  `!important`, then inline over sheet, then specificity (ids, classes, element), then the later
  rule wins, and a presentation attribute only when no declaration names the property. Sheets
  read simple selectors only (an element name or `*`, classes, one id, grouped with commas);
  a selector with a combinator, pseudo-class or attribute test is ignored and counted in a note,
  at-rules (`@media` included) are skipped, comments and `<!--` `-->` dropped. Rules are indexed by
  id, first class or element; past `MAX_STYLE_RULES` they are ignored with a note, and past
  `MAX_MATCHING_WORK` selector tests the rest of the elements keep their own attributes only, with
  a note, so a hostile sheet cannot make matching quadratic.
- `display: none` and `visibility: hidden` leave elements out with a note; an element whose stroke
  and fill are both set to `none` somewhere in its cascade (not by the defaults, since a bare
  `line` with no stroke is still wanted) is left out and counted; `stroke-dasharray` (inherited)
  makes curves construction geometry, as a dashed DXF linetype does; a `clip-path` or `mask` is
  ignored and counted, the shape imported whole.
- Markers (`marker-start`, `marker-mid`, `marker-end` and the `marker` shorthand, inherited) are
  drawn on `path`, `line`, `polyline` and `polygon` at every vertex the outline records
  (`path::Vertex`: each move, segment end and close, with the directions in and out, arcs by their
  tangents): the marker's viewBox fitted into `markerWidth` by `markerHeight` (default 3), `refX`
  and `refY` put on the vertex, scaled by the stroke width unless `markerUnits` is
  `userSpaceOnUse`, and turned by `orient` (an angle in deg, rad, grad or turn, `auto` along the
  bisector of the directions, `auto-start-reverse`). Marker content inherits from the marker's own
  ancestors, never the path, is charged against `MAX_EXPANDED_OBJECTS` like a use, and a marker
  drawn inside itself is left out as reusing itself. Markers are drawn even on an element left
  out for having neither stroke nor fill, as browsers draw them; overflow clipping is not
  applied. A `switch` draws its
  first child without conditions. `image` and `foreignObject` are counted as left out,
  other unknown SVG elements named in a note, definitions, styles and metadata skipped silently,
  and elements of other namespaces (Inkscape's, Sodipodi's) ignored. Shapes whose numbers overflow are left out and counted; the curve, point and empty limits
  are DXF's.
- Text becomes the outlines of its letters, given a font: `parse_svg` and `read_drawing` take
  `TextOutlines`, `InFont(bytes)` or `LeftOut`. The font is handed in rather than bundled by
  `caditor-file`, so Inter, which the app already embeds for its interface (`caditor`'s
  `fonts::INTER`, the variable `InterVariable.ttf`), is carried once; the app passes it for every
  drawing import, and anything importing headless must pass the same bytes to get the same
  letters. Without font data, or with bytes `ttf-parser` cannot read, each `text` is counted as
  left out as before.
- Layout (`lettering.rs`): the `text` element's content and its `tspan` and `a` children are
  gathered into letters, each with its inherited style; whitespace is collapsed as CSS's normal
  white space does (newlines and tabs are spaces, runs of spaces one, leading and trailing ones
  dropped) unless `xml:space="preserve"`; a `tspan` with `display: none` adds no letters, one
  hidden or unpainted keeps its room but draws nothing. `x`, `y`, `dx`, `dy` lists (lengths, units
  and percentages of the viewport) and `rotate` (its last angle repeating to the element's end)
  apply per letter from each element's first letter, a descendant overriding its ancestors, as
  SVG 2 resolves them. A letter with an absolute `x` or `y` starts a chunk, and each chunk is
  shifted by `text-anchor` of its first letter over the extent of its advances. The pen moves by
  each glyph's advance plus `letter-spacing` (and `word-spacing` after a space; lengths or em),
  with the font's pair kerning (the GPOS `kern` lookups of the Latin, else default, script, pair
  adjustments of both formats, at the default instance) between two letters of the same weight
  unless the second is placed absolutely.
- Text style (`text_style.rs`) cascades like the other properties: `font-size` (lengths, em, ex,
  rem, percentages of the parent's, the keywords, `larger` and `smaller`; 16 px when unset),
  `font-family`, `font-weight` (keywords, `bolder` and `lighter` as CSS steps them, 1 to 1000),
  `font-style`, `text-anchor`, `letter-spacing`, `word-spacing` and the `font` shorthand (style and
  weight words, then a size with an optional line height, then the families; a system font name
  is ignored). Every family is drawn in Inter: a first family other than Inter or a generic
  sans-serif is named in one note listing them (at most `MAX_NAMED_ELEMENTS`). The weight sets the
  font's `wght` axis, clamped to its range (100 to 900 for Inter); italic and oblique text is
  drawn upright and counted in a note, since only the upright face is carried. A character the
  font has no glyph for takes the advance of its missing glyph, draws nothing and is counted;
  `textPath` is left out and counted.
- Glyphs (`font.rs`) are read in font units at each weight (one `ttf_parser::Face` per weight,
  outlines cached per weight and glyph), scaled by size over units per em with y flipped, turned
  by the letter's rotation, placed at its position and taken through the element's transforms;
  their quadratic and cubic segments become the same exact Bézier splines and lines `path` makes.
  A variable font keeps overlapping contours (Inter's `e` is one contour whose bar crosses its
  bowl; `t`, `f` and `k` overlap pieces), which the sketch's even-depth regions would read as
  notches, so `overlap.rs` merges each glyph first: its segments build a kernel `Profile`, the
  faces whose anchor has a non-zero winding over the original contours (sampled 32 steps per
  Bézier) are selected as lumps, and each loop's pieces become lines and Bézier pieces (de
  Casteljau over the piece's range) meeting exactly at the loop's junctions. A glyph the profile
  cannot build keeps its contours as drawn. Letters of different glyphs that overlap one another
  are not merged. `read_drawing` installs its `CancelToken` as the kernel interrupt around
  `parse_svg`, so merging stops when the import is cancelled. Letters are charged like shapes, and
  past `MAX_READ_CURVES` a glyph is counted without being merged.

## STEP import (`import/model.rs`)

- `read_step_file` unpacks gzip first (`.stpz`, or the gzip magic), within `MAX_FILE_SIZE` and with
  the trailer's CRC and size checked, else `DamagedArchive` or `UnpacksTooLarge`. Non-UTF-8 text
  is read as Latin-1 with a note.
- `read_step_file`, `read_mesh_file`, `read_dxf` and `read_drawing` take a `CancelToken`: it is checked between
  the file's stages and installed as the kernel interrupt around parsing and building, so the STEP
  reader stops between solids (`ReadError::Cancelled`) and meshing stops inside a shell. A
  cancelled read is `ImportError::Cancelled`, never a partial import.
- `parse_step` reads the copies of each part (`read_step_copies`, `step-read.md`) and canonicalises
  each part once, unplaced: written by caditor's own writer and read back (`read_own_step`, which
  skips the crossing check the first read already made), so what is stored is exactly what later
  loads. The distinct parts are canonicalised in parallel, then the copies placed in parallel
  (`in_parallel`: scoped threads with the caller's interrupt, results in input order), so the bodies
  and notes are those of a sequential pass. `read_step_file_reporting` fills a `ReadingProgress`
  the app polls (`ReadingStage`: reading, solids built of solids, parts stored of parts). One `ImportedBody` per copy, or per copy of each lump (each
  canonicalised alone) when a multi-lump solid reads back as several; a part that cannot be stored
  is left out with a note per copy.
- A copy placed rigidly becomes an `Import` sharing the part's solid and STEP text (`Arc`s, through
  `Import::shared`) with its placement as a `BodyPlacement` (`body_placement`: turns about X, Y
  then Z taken from the rotation, rounded with the shift to a billionth of a degree or millimetre,
  and kept only when it lands every corner of the part's box within a tenth of
  `LINEAR_RESOLUTION` of where the file puts it), so the user can move it afterwards like any
  placed import. A copy scaled or mirrored, or one the turns cannot reproduce, is mapped and
  canonicalised on its own at the origin as before; one whose mapping fails is left out with
  `Misplacement::CopyUnplaceable`'s note.
- Each `ImportedBody` carries the copy's STEP `colour`, its `opacity` and its `layer` as `group`,
  and its faces' own looks as `ImportedFace`s (index in the stored solid, colour, opacity snapped
  like the body's with solid as 100, kept only when it differs from the body's). Face indices are
  carried onto each stored lump through `caditor_step::lump_faces` (the faces of each lump in the
  order written, which canonicalising keeps), for every lump whose face count matches.
  `bodies_transaction` makes each a `FaceColour` naming the imported face
  (`FaceName::imported`, origin `Imported`), coloured with its own colour, else the body's, else
  `DEFAULT_BODY_COLOUR`, so a face see-through but coloured by neither stays see-through.
  The STEP opacity (a percent) is snapped when the copy is read to the nearest of the body's
  opacity steps, solid included (`nearest_opacity_step`, `document.md`), so a nearly solid body
  imports solid. `bodies_transaction` sets a coloured or see-through body's appearance colour and
  opacity (one `SetBodyAppearance`) and puts a layered body in the
  folder of its layer's name (one line, cut at `MAX_GROUP_NAME_CHARS`), moving the bodies of a
  layer together at its first body so each layer is one folder; mesh imports have neither.
- `bodies_transaction` adds an `Import` feature per body under unique names; the model file stores
  it as an `import` or `placed_import` record (`file-format.md`). The app refuses, before applying
  it, an import whose STEP text with the model's existing imports (`FeatureKind::stored_text_len`,
  a text shared by copies counted once) would exceed `MAX_MODEL_RECORDS`, since that model could
  be neither saved nor journaled.

## Mesh import (`import/mesh.rs`)

- `read_mesh_file` picks the format by extension (`MeshFormat`): STL (binary when its size matches
  the triangle count, else text), OBJ (`v` and `f`, polygons fanned, `/` parts and negative indices
  read) and 3MF (`zip_read.rs`, a small reader of stored and deflated entries, no ZIP64; the model
  part from `_rels/.rels`, its `unit` converted to millimetres, build items and nested components
  placed by their transforms). STL and OBJ have no units, so they read as millimetres with a note.
- The triangles become solids through the kernel's `faceted_solids` (`kernel.md`), each
  canonicalised like a STEP body and imported as an `Import` named after the file; what the
  kernel repaired or left out becomes notes in the import report. Its failures are
  `ImportError` variants of their own (`DamagedMesh`, `MeshNotClosed`, `MeshTooDetailed`,
  `MeshNotSolid`).

## Export (`export/`)

- `export_bodies` writes STEP (a body with a look styled with its colour and, when see-through
  (`Look::opacity`), its transparency, `step-write.md`; a body with an opacity but no colour or
  material still has a look, in the app's default colour; its faces' looks (`ExportBody::faces`,
  `ExportFace` by index among the solid's faces, which the app fills from the appearance's
  `face_colours` and `face_opacities`) styled per face; a body in a folder
  (`ExportBody::group`, the making feature's group) on a layer of the folder's name), or tessellates at a `MeshResolution` (a chord fraction of the
  largest body's diagonal plus an angle between triangles) into STL (`MeshOptions::stl`, see
  below), 3MF (one named object per body, millimetres, a thumbnail when given; a body with a
  colour or material points
  into one `basematerials` group, its `base` named after the material or else the body and coloured
  with the body's colour or the app's default, via `ExportBody::look`), OBJ (one named object per body, global
  1-based indices, millimetres, Z up, no normals; bodies with a look name a material, `<index>_<material
  or body>` with whitespace as `_`, from a `.mtl` of the same stem written first beside it, whose
  `Kd` is the colour; an existing `.mtl` that does not start with caditor's header is never
  replaced, and the OBJ then goes without colours) or binary glTF (`.glb`: one node and mesh per
  body, f32 positions in metres with Y up, `x, z, -y` of the model's, so winding is kept, and the
  position bounds glTF requires; no normals, which the format defines as flat; a body with a look
  gets a material named like its 3MF base, its colour as a linear `baseColorFactor`).
- STL comes in two `StlEncoding`s, streamed into the temporary through a buffer
  (`write_streamed`) rather than built in memory. Binary puts every body in one surface of f32
  millimetres, the only form every slicer reads; since f32 rounds by more than a micrometre past
  `stl::PRECISE_REACH` (32,768 mm), a model reaching beyond it is moved by its bounding-box centre
  rounded to whole millimetres, named in the 80-byte header (`moved by x y z`) and returned as
  `Exported::moved` for the app and the command line to say how to move it back. Text writes one
  `solid` per body, named after it with anything but printable ASCII as `_` (readers sniffing for
  binary bytes would otherwise misread it), and coordinates to six decimals like 3MF's, never
  moved.
- `export_bodies` takes the model's `ModelProperties`; the notes are never exported, the rest go
  where the format has room (`EXPORTED_PROPERTIES`): STEP through `write_step_detailed`, 3MF as
  `Title`, `Designer` (author) and `Description` metadata plus the part number as each build item's
  `partnumber` when there is one body, glTF as `asset.extras` (`title`, `partNumber`, `revision`,
  `author`, `organisation`, `description`), OBJ as `# <Label>: <value>` lines after its header.
  STL has nowhere to put them.
- `ExportBody::threads` (`ExportThread`: designation, side, pitch, start, direction, length; the
  app fills it from `placed_threads` for the body) carry cosmetic threads where the format has
  room: STEP as a thread property of the body's part (`step-write.md`), glTF as the node's
  `extras.threads` (the designations), OBJ as a `# Thread: <designation>` line after the object's
  `o` line, 3MF as object metadata (below). STL carries none.
- 3MF threads are one `metadatagroup` first in the body's `object` (before its `mesh`), holding for
  thread n (from 1) six `metadata` entries with `preserve="1"`, named
  `caditor:thread.<n>.<field>`: `designation` (the notation text), `side` (`internal` or
  `external`), `pitch` and `length` (millimetres), `start` and `direction` (three space-separated
  numbers in the object's own coordinates, which are the model's, the vertices being written
  untransformed; the direction runs along the thread's axis from its start). Values use the
  coordinate format of the mesh. The `caditor` prefix is declared on the `model` element as
  `xmlns:caditor="urn:caditor:3mf"` only when some body has a thread, and is not listed in
  `requiredextensions`, so consumers that do not know it ignore the entries; a model without
  threads is written exactly as before. The namespace URI is a URN of our own because the project
  publishes no site or maintainer identity to base an `http` one on. It is `CADITOR_NAMESPACE` and
  never changes once files carry it, whatever the entries grow into: new fields are new names in
  it, a changed meaning a new name, never a new URI.
- Saved atomically like a model. Cancellation is checked between bodies and before writing;
  failures are sentences naming the body. A body that cannot be meshed or written (a panic
  included) is left out and returned in `Exported::left_out` (the app says so in a notice that
  outlasts edits); the export fails only when no body could be written. STEP does this through
  `write_step_keeping_what_can_be`.
- Drawings: `export_drawing` turns each source (a `NamedSketch` or a `NamedFace`, both kinds in
  one drawing, sketches first) into a `Figure` of 2D `Shape`s, each on a `Layer`
  (`export/figure.rs`), arranges them by a `DrawingSheet` (`export/sheet.rs`) and writes it with
  `export/dxf.rs` or `export/svg.rs`; `SketchFormat::of` picks DXF or SVG from the path. It
  returns `DrawingExported` (a `SketchExported` and a `FaceExported` plus the parts wider than the
  sheet) and fails with `NoCurves` only when no sketch draws anything and there is no face.
  `export_sketches` and `export_faces` are its one-kind forms, `export_sketch` and `export_face`
  the one-source, default-sheet ones. Written atomically, cancellation checked between sources,
  while nesting and before writing.
- A sketch writes its solved curves in its own 2D coordinates on layer 0, and a `POINT` for a point
  no curve uses; an ellipse or elliptical arc is an `ELLIPSE` (`figure::drawn_ellipse`, the longer
  radius as its major axis since DXF wants a ratio at most 1). Construction curves (`DrawingSheet::construction`) are counted and left out
  (`Construction::LeftOut`), or with `Construction::OnLayer` written on layer `Construction` in a
  dashed linetype and counted in `SketchExported::construction`; sketches with nothing to write
  are `ExportError::NoCurves`. The DXF reads back through `parse_dxf` as the same curves, the
  construction ones as construction.
- A face (`export/outline.rs`) writes a flat face's outer loop on layer `Outline` and its
  inner loops on `Holes`, as seen from outside the body: the face's plane with its origin where the
  model origin projects onto it, up along Z for a face more upright than 45° and along Y otherwise,
  so a top face keeps the model's X and Y and a bottom one is mirrored. Lines, circles and arcs stay
  exact, an ellipse becomes an elliptical arc (major axis the longer one, parameters negated when
  its frame faces away), a spline edge is the exact piece of its curve (`BSpline::restricted`,
  rational weights kept), and anything else (intersection curves) is sampled within 1 µm or a
  millionth of the body's diagonal and written as one cubic `SPLINE` interpolating the samples
  (`outline::fitted`, kept only when its middle between every two samples lies within that chord of
  the curve's), else as a polyline of them; either is counted in `FaceExported::approximated`. A
  curved face is `ExportError::FaceNotFlat`; outlining runs under `catch_unwind`.
- `SheetLayout::SideBySide` (the default) leaves the first source in its own frame and shifts each
  next one to the right of what is placed by a gap of a tenth of the largest (at least 10 mm),
  bottoms level, so one source is written exactly where it is. `SheetLayout::Nested(Nesting)`
  (`export/nest.rs`) packs true shapes bottom-left from the origin, largest first: each source,
  with its label band, is traced into polylines (`Shape::traced`, circles and text boxes closed)
  and rasterised on a grid of square cells into row runs, conservatively (every cell a curve
  touches, plus the inside by the even-odd rule, so a ring's hollow and an L's crook stay free),
  and its runs grown by the spacing in cells (square growth, so the gap between geometry is at
  least the spacing). The cell is the smaller of the sheet width and the parts' size (largest side,
  or the root of their summed box areas) over 400, but no finer than a thousandth of the largest
  part or a 64th of the spacing. Each goes to the lowest, then leftmost, cell where its grown runs
  miss every placed run within the sheet width, rows without a gap as wide as the part's widest
  run skipped at once; with `Nesting::turns` it also tries quarter turns, and the 15° step that
  boxes it smallest with its quarter turns, keeping the lowest spot (`Motion::turn`, exact for
  quarter turns). A source wider than the sheet every way goes alone above the others, narrowest
  way round, and is counted in `too_wide`. `Nesting::new` refuses a width that is not positive or
  a negative spacing. It is deterministic; a label is placed below its source's turned bounds.
- `Annotations::Included` adds, for a sketch, a dimension per dimensional constraint
  (`export/annotation.rs`, measured from the solved geometry, active or not; one touching a
  left-out construction curve is skipped) on layer `Dimensions`: distances as a dimension line
  parallel to the measured gap, offset away from the sketch's middle with extension lines (two
  lines are measured from the end of one farther along the other, so the dimension sits outside),
  horizontal and vertical distances along X or Y, radius and diameter as a leader through the
  centre (`R`, `⌀`), an ellipse's major and minor radii as an `R` leader from its centre along
  that axis, angles as an arc at the lines' vertex, arc sweep and length as an arc beyond
  the arc (`°`, `⌒`); values in millimetres to three decimals and degrees to two. Placement does
  not use the canvas's lanes or obstacle avoidance (`caditor`'s `annotation_layout.rs`, which this
  crate cannot reach), so crowded labels may overlap. Every source
  gets its name as a label below it on layer `Labels` (nesting reserves the band). Text is
  `sheet::text_height` high: a fiftieth of the largest source, at least 2.5 mm.
- DXF is ASCII, version AC1015, millimetres (`$INSUNITS` 4), header and entities (layers are
  named by the entities), plus, only when the Construction layer is used, a TABLES section of the
  `CONTINUOUS` and `DASHED` linetypes and that layer (grey, dashed), each construction entity also
  naming `DASHED`, since `parse_dxf` takes a dashed linetype for construction: `LINE`, `CIRCLE`, `ARC` (a full sweep is a circle),
  `ELLIPSE` (a full one from 0 to 2π), clamped planar `SPLINE` with knots, weights when rational
  (flag 12) and control points, open `LWPOLYLINE`, `POINT` and `TEXT` (labels left-aligned on the
  baseline, dimension text centred, turned to read from below or the right). A dimension is a
  `DIMENSION` entity with subclass markers (rotated linear, 3-point angular, radial or diametric,
  style `STANDARD`, its text as the override and placed by the user flag) whose drawn lines, arcs
  and text are an anonymous `*D<n>` block in a BLOCKS section, so readers that redraw dimensions
  and readers that show the block agree, and `parse_dxf`, which expands blocks only through
  `INSERT`, reads the file back as the same curves with the dimensions and labels counted in
  notes. Text escapes `%` as `%%%`, `⌀` as `%%c`, `°` as `%%d`, other non-ASCII as `\U+XXXX`
  and control characters as spaces.
- SVG is in millimetres with y flipped (`-y`), a 1 mm margin in the viewBox, a 0.1 mm black
  hairline, `line`, `circle`, elliptical-arc `path`s (sweep flag 0, since the flip keeps the drawn
  direction; a rotation for an ellipse, a full one in two halves), splines and polylines as
  `polyline`s, points as small filled circles and text as sans-serif `text` (XML-escaped, rotated
  like the DXF's). Shapes off layer 0 are grouped in a `g` whose `id` is the layer's name; the
  Construction group is grey and dashed.
- `export_png` writes 8-bit RGBA, straight alpha, sRGB chunk, through the pure-Rust `png` crate,
  atomically, streaming whatever `PixelRows` yields (bands of whole rows) into the encoder, so
  the image is never held whole; it checks the pixel count and cancellation between bands. Its
  errors are `PngExportError`: the source's own error as `Pixels`, else an `ImageExportError`.
- 3MF is a ZIP from a small writer (`zip.rs`, `ZipWriter`) streamed into the temporary like STL:
  the content types, the package relationships and the thumbnail go first, whole (deflated through
  `miniz_oxide` unless storing is smaller), then the model, written straight into `miniz_oxide`'s
  streaming compressor with its CRC32 alongside and its sizes patched into the local header once
  known, so the XML is never held. ZIP64 is used only where needed: the model reserves a ZIP64
  field in its local header when `model_size_bound` (from the counts of vertices and triangles,
  the longest coordinate and the escaped names) allows more than 4 GiB, and the central directory
  and end records switch to ZIP64 fields and records for any size, offset or count past their
  32 or 16 bits. Deflate and CRC32 live only here, for the foreign format.
- A 3MF thumbnail is `MeshOptions::thumbnail` (RGBA) encoded to `Metadata/thumbnail.png` in memory
  (`image::encode_png`), with a `png` content type and a package relationship of the OPC thumbnail
  type; one that cannot be encoded is left out with a log line, never failing the export.

## Parameter files (`parameters.rs`)

- Parameters export as CSV (RFC 4180, `PARAMETERS_EXTENSION`): a header row
  `name,expression,value,note`, then one row per parameter in list order, CRLF line ends. The
  expression is written with parameter names (`Expression::to_text`) and carries its own units;
  `value` is the evaluated value for people reading the file (`error` when it does not evaluate)
  and is ignored on reading. Cells holding a delimiter, quote, line break, surrounding spaces or a
  leading `=`, `+`, `-` or `@` are quoted, so a spreadsheet never reads one as a formula. The file
  is written with `write_atomically`.
- Reading (`parse_parameters`) finds the `name` and `expression` columns by header, in any order
  and case, an optional `note` (or `description`) column, and ignores the others; the delimiter is
  the one of comma, semicolon or tab the header uses most, so spreadsheets saving with semicolons
  read too. A byte-order mark, quoted cells over several lines and blank rows are accepted. A file
  over `MAX_PARAMETERS_FILE`, not UTF-8, without both columns, with an unclosed quote (named by
  line) or with more than `MAX_PARAMETER_ROWS` rows is refused as a whole; anything wrong in a
  row is left to `plan_parameter_import` (`document.md`).

## Failures

- Reading and writing a file fail as `ReadFailure` and `WriteFailure` (`reason.rs`), one variant per
  cause an `io::Error` can name, written as a clause the UI completes ("… because it no longer
  exists"); `LoadError::Unreadable`, `ImportError::Reading` and `ExportError::Writing` carry them.
  A job that panicked is `LoadError::Crashed` or `ImportError::Crashed`, not a message. A save fails
  as a `SaveError` of its own variants (`Writing(WriteFailure)`, `ModelTooLarge`, `ChangedOnDisk`,
  …) and the recovery journal as a `JournalFailure`; `ImageExportError::Writing` carries a
  `WriteFailure` too.
