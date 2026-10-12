---
paths:
  - "crates/caditor-file/src/import/**"
  - "crates/caditor-file/src/export/**"
---

# DXF, SVG and STEP import, export

## DXF import (`import/`)

- `parse_dxf` reads ASCII and binary DXF into a `Drawing` of 2D `DrawingCurve`s in millimetres plus
  plain-language notes; only header, layers, MLINESTYLEs, blocks and entities are kept. Text is
  UTF-8 when valid, else `$DWGCODEPAGE` (`dxf/code_page.rs`; unknown reads as 1252).
- Hostile-input limits, each an `ImportError` naming it: `MAX_DRAWING_VALUES` (`DxfFile::read`
  charges every kept record and every layer, linetype, block and MLINESTYLE entry `RECORD_VALUES`
  before keeping it, so bare `POINT` records end there too), `MAX_EXPANDED_OBJECTS` (block
  expansion, per object and per decoded shape, both added to the `tally` the refusal tests read),
  `MAX_DRAWING_POINTS`. Curves past `MAX_READ_CURVES` are left out with a note, the rest of an item
  past it counted in one step. The sketch's
  `MAX_DRAWING_CURVES` applies in `Drawing::arranged` to the chosen layers' curves in drawing order
  with a note (so leaving layers out brings later curves in); `drawing_transaction` never adds more.
- Units: `$INSUNITS` converts to millimetres with a note; unknown or missing reads as millimetres
  with a note, except missing with imperial `$MEASUREMENT`, which reads as inches.
- Damage ends reading where found: earlier records are interpreted, a note says where, the
  interrupted record is dropped. Damage before any section or before anything was drawn is refused
  with the damage as the error.
- Left out silently: paper space, invisible entities, off, frozen and `DEFPOINTS` layers.
  Annotations (text, dimensions) are counted in a note. HATCH imports boundary paths only, never
  the fill (counted), and a path whose source objects are all drawn entities of the same space is
  left to them so an associative hatch does not double its outline.
- `Drawing::arranged` applies `DrawingOptions` without touching the original, a note per change: a
  replacement unit (`Drawing::unit_scale` is what the header applied, so the choice converts from
  the file's own numbers), a scale within `MIN_SCALE..=MAX_SCALE`, centring the outline's bounding
  box (arcs by sweep) on the origin, layers left out. Each curve keeps its layer index
  (`Drawing::curve_layers` into `Drawing::layers`, first-use order, names compared without case;
  layer 0 inside a block takes its INSERT's layer); only layers that kept a curve are listed.
- Dashed linetypes make a curve construction geometry (`Drawing::construction`), with a note.
  Blocks and INSERTs nest with cycle and depth limits; a block's content is decoded once and shared.
- SPLINE uses control points up to `MAX_SPLINE_DEGREE`, else fit points: with an end tangent the
  cubic AutoCAD draws (`FitPoints::cubic`), otherwise `DrawingCurve::FitSpline`, which
  `drawing_transaction` makes a centripetal fit-point spline (closed when flag 70 bit 1 is set and
  three points remain after dropping a repeated first point); past `MAX_FIT_POINTS`, or where no
  such spline can be drawn, the points are fitted with `BSpline::through`. Hatch boundary fit
  splines read the same way, always open.
- Everything becomes a 3D shape in its object coordinate system (arbitrary axis algorithm), is
  transformed, then flattened onto XY: conics projecting to circles become circles and arcs; other
  conics exact `DrawingCurve::Ellipse`s (`flatten::planar_ellipse`: principal axes of the projected
  conjugate diameters, a partial one by its end points counter-clockwise); non-uniform splines are
  fitted within a millionth of the drawing's size.
- `drawing_transaction` makes one transaction on an existing or new sketch: drops curves shorter
  than the joint tolerance, joins ends within it with `Coincident`, makes sketch ellipses and
  elliptical arcs, adds construction curves with ordinary end points.

## SVG import (`import/svg/`)

- `read_drawing` takes SVG for a `.svg`/`.svgz` file, or a file not named `.dxf` whose content
  starts with `<` (after a BOM and spaces), gzip or a UTF-16 BOM; anything else goes to `parse_dxf`.
  `parse_svg` unpacks gzip like `.stpz` (`model::unpacked`), decodes UTF-8 or UTF-16 by BOM, else
  Latin-1 with a note, and returns the same `Drawing` as DXF, flattened by DXF's `flatten`, so the
  import dialog, arrangement and `drawing_transaction` are shared.
- `import/xml.rs` is caditor's own XML reader, shared with 3MF, since roxmltree's tokenizer recurses
  per nesting level and overflows the stack on hostile depth: iterative, namespaces resolved,
  attribute values decoded (named entities, character references and the DOCTYPE's own text
  entities, which Illustrator uses for `xmlns`; an entity holding markup, nested past
  `MAX_ENTITY_DEPTH` or expanding past `MAX_ENTITY_WORK` is damage). Text is kept only inside
  `style`, `text`, `tspan`, `textPath` and `a`, as pieces between the children (`Node::content`)
  since layout needs their order. Past `MAX_DRAWING_ELEMENTS` the file is
  `ImportError::TooManyElements`, nesting past `MAX_DEPTH` is `ImportError::NestedTooDeep` and a
  tag with more than `MAX_ATTRIBUTES` attributes is `ImportError::TooManyAttributes` (3MF refuses
  the same way). The reader polls the kernel interrupt every `ELEMENTS_BETWEEN_INTERRUPT_POLLS`
  elements, so Cancel stops a crafted file's parse as `ImportError::Cancelled`. Namespaces are interned through a map and a prefix resolves through a stack per
  prefix, so a hostile file costs time linear in its size; each element records the last element
  of its subtree (`Node::encloses`), which is how a `use` tests for a loop. Damage keeps the
  elements before it with their open ancestors and a note names the line; damage before the root is
  `DamagedAt`.
- The root `svg` sizes the drawing (`sizing.rs`): with a viewBox and a width or height, a user unit
  is that length in millimetres over the viewBox's side (`preserveAspectRatio` meet takes the
  smaller, slice the larger, none each); mm, cm, in, pt, pc, px convert and every unit but
  millimetres gets a note; a bare number, and every user unit without viewBox or size, is a CSS
  pixel (96 to the inch, noted as 0.2646 mm). The viewBox origin is not subtracted, so caditor's own
  SVG export reads back where drawn; y is flipped to point up. `Drawing::unit_scale` is millimetres
  per user unit, so the dialog's unit choice reads user units as that unit. Inner lengths use
  96 px to the inch and percentages of the nearest viewport (radius: the normalised diagonal); a
  nested `svg` places its viewBox with x, y, width, height and `preserveAspectRatio`.
- `path` reads every command (implicit repeats, reflected S/T controls); damaged data is read up to
  the damage and counted. Béziers become exact sketch splines (as `BSpline::clamped`), one whose
  controls lie on its chord a line; elliptical arcs follow the SVG centre conversion (radii scaled
  up if too small, zero radius a line) and become a conic that `flatten` makes an arc when circular
  after the transforms, else an exact ellipse or elliptical arc. `rect` (rounded, auto `rx`/`ry`
  clamped to half the sides), `circle`, `ellipse`, `line`, `polyline`, `polygon` are read;
  zero-sized shapes draw nothing.
- `transform` lists compose down the tree; an unreadable one is ignored and counted. `use` places
  its target (`href` or `xlink:href`, by id) at x and y; a `symbol` is fitted like a nested `svg`
  into the use's width and height (else its own, else 100%), drawing nothing when either is zero.
  A target that is the use's own ancestor or already being placed, uses nested past `MAX_USE_DEPTH`
  and elements nested past `MAX_NESTING` are left out with a note. Inside a use every element and
  shape is charged against `MAX_EXPANDED_OBJECTS` (`TooManyCopies`); an element's properties and
  local shapes are decoded once and shared by its instances.
- Layers: below the root (or a single group wrapping everything, repeatedly) each group or nested
  `svg` is a layer named by `inkscape:label`, else `id`, else `Group <n>`; shapes outside them are
  on `Ungrouped`. Interned like DXF's.
- Style (`style.rs`, `css.rs`) cascades as SVG does from the `style` attribute, `<style>` sheets and
  the presentation attribute: highest `!important`, then inline over sheet, then specificity, then
  the later rule; a presentation attribute only when no declaration names the property. Sheets read
  simple selectors only (element name or `*`, classes, one id, comma groups); selectors with a
  combinator, pseudo-class or attribute test are ignored and counted, at-rules skipped. Rules are
  indexed by id, first class or element; past `MAX_STYLE_RULES` they are ignored with a note, and
  past `MAX_MATCHING_WORK` selector tests the remaining elements keep their own attributes only, so
  a hostile sheet cannot make matching quadratic.
- `display: none` and `visibility: hidden` leave elements out with a note; an element whose stroke
  and fill are both explicitly `none` in its cascade (not by defaults: a bare `line` is wanted) is
  left out and counted; inherited `stroke-dasharray` makes construction geometry; `clip-path` and
  `mask` are ignored and counted.
- Markers (`marker-start`/`mid`/`end`, `marker`, inherited) are drawn on `path`, `line`, `polyline`
  and `polygon` at every vertex the outline records (`path::Vertex`, directions in and out, arcs by
  tangents): viewBox fitted into `markerWidth` by `markerHeight` (default 3), `refX`/`refY` on the
  vertex, scaled by stroke width unless `markerUnits` is `userSpaceOnUse`, turned by `orient`.
  Content inherits from the marker's own ancestors, is charged against `MAX_EXPANDED_OBJECTS`, a
  marker drawn inside itself is left out; drawn even on an element with neither stroke nor fill;
  overflow clipping not applied.
- A `switch` draws its first child. `image` and `foreignObject` are counted as left out, other
  unknown SVG elements named in a note, definitions, styles and metadata skipped silently, other
  namespaces (Inkscape, Sodipodi) ignored. Shapes whose numbers overflow are left out and counted;
  the curve, point and empty limits are DXF's.

### SVG text

- Text becomes the outlines of its letters. `parse_svg` and `read_drawing` take `TextOutlines`:
  `InFont { upright, italic }` or `LeftOut`. Fonts are handed in, not bundled by `caditor-file`: the
  app passes Inter's upright (`fonts::INTER`, `InterVariable.ttf`) and italic (`fonts::INTER_ITALIC`,
  `InterVariable-Italic.ttf`, same Inter 4.1 release and licence, used by import only) faces, and
  headless importers must pass the same bytes to get the same letters. Without upright data, or
  bytes `ttf-parser` cannot read, each `text` is counted as left out; a `None` or unreadable italic
  draws italic text upright.
- Layout (`lettering.rs`): `text`, `tspan` and `a` content gathers into letters with inherited
  style; whitespace collapses as CSS unless `xml:space="preserve"`; a `tspan` with `display: none`
  adds no letters, one hidden or unpainted keeps its room but draws nothing. `x`, `y`, `dx`, `dy`
  lists and `rotate` (last angle repeating) apply per letter from each element's first letter,
  descendants overriding ancestors (SVG 2). A letter with absolute `x` or `y` starts a chunk,
  shifted by its first letter's `text-anchor`. The pen moves by advance plus `letter-spacing` and
  `word-spacing`, with GPOS `kern` pair kerning between two letters of the same weight and face
  unless the second is absolutely placed.
- `textLength` (innermost first): `lengthAdjust` `spacing` (default) spreads the difference over the
  gaps, `spacingAndGlyphs` stretches places and glyphs (`Placed::stretch`). `baseline-shift` moves
  a span's letters without moving the pen and adds up through nested spans (`TextStyle::rise`).
- `textPath` (`text_path.rs`) lays letters along the referenced shape (`href`/`xlink:href`, through
  its own `transform`) or its SVG 2 `path` data, sampled into a `Track` (at most
  `MAX_TRACK_POINTS`); `side="right"` walks it backwards, `startOffset` is where the run starts. The
  run is laid out straight first, then each letter's middle is put on the track and turned to the
  tangent; absolute `x`/`y` are ignored inside. A letter whose middle falls off either end is left
  out and counted. Text after the `textPath` carries on straight from the last on-path letter. A
  reference not naming a drawn shape is counted as missing and its text left out.
- Text style (`text_style.rs`) cascades `font-size` (16 px unset), `font-family`, `font-weight`,
  `font-style`, `text-anchor`, `letter-spacing`, `word-spacing` and the `font` shorthand (system
  font names ignored). Every family is drawn in Inter; a first family other than Inter or a generic
  sans-serif is named in one note (at most `MAX_NAMED_ELEMENTS`). Weight sets the `wght` axis
  (100 to 900); italic and oblique use the italic face, upright and noted only when none was handed
  in. A character without a glyph takes the missing glyph's advance, draws nothing and is counted.
- Glyphs (`font.rs`) are read per weight of each face (`Cut` per face, `Instance` of weight and
  italic, one `ttf_parser::Face` per weight, outlines cached per weight and glyph); quadratic and
  cubic segments become the same exact splines and lines `path` makes. A variable font keeps
  overlapping contours (Inter's `e`, `t`, `f`, `k`), which the sketch's even-depth regions would read
  as notches, so `overlap.rs` merges each glyph first: its segments build a kernel `Profile`, faces
  whose anchor has non-zero winding over the original contours (Béziers sampled 32 steps) are
  selected as lumps, and each loop's pieces become lines and Béziers (de Casteljau) meeting exactly
  at junctions. A glyph the profile cannot build keeps its contours as drawn; different glyphs that
  overlap are not merged. `read_drawing` installs its `CancelToken` as the kernel interrupt around
  `parse_svg`. Letters are charged like shapes; past `MAX_READ_CURVES` a glyph is counted without
  being merged.

## STEP import (`import/model.rs`)

- `read_step_file` unpacks gzip first (`.stpz`, or the gzip magic) within `MAX_FILE_SIZE`, trailer
  CRC and size checked, else `DamagedArchive` or `UnpacksTooLarge`. Non-UTF-8 text reads as Latin-1
  with a note.
- `read_step_file`, `read_mesh_file`, `read_dxf` and `read_drawing` take a `CancelToken`, checked
  between stages and installed as the kernel interrupt around parsing and building (STEP stops
  between solids with `ReadError::Cancelled`, meshing inside a shell). A cancelled read is
  `ImportError::Cancelled`, never a partial import.
- `parse_step` reads each part's copies (`read_step_copies`, `step-read.md`) and canonicalises each
  distinct part once, unplaced, by writing with caditor's own writer and reading back
  (`read_own_step`, which skips the crossing check the first read made), so what is stored is
  exactly what later loads. Parts are canonicalised in parallel, then copies placed in parallel
  (`in_parallel`: scoped threads with the caller's interrupt, results in input order, so bodies and
  notes match a sequential pass). `read_step_file_reporting` fills a `ReadingProgress` the app
  polls (`ReadingStage`). One `ImportedBody` per copy, or per lump of a copy (each canonicalised
  alone) when a multi-lump solid reads back as several; a part that cannot be stored is left out
  with a note per copy.
- A rigidly placed copy becomes an `Import` sharing the part's solid and STEP text (`Import::shared`)
  with a `BodyPlacement` (`body_placement`: turns about X, Y then Z, rounded with the shift to a
  billionth of a degree or millimetre, kept only when every corner of the part's box lands within a
  tenth of `LINEAR_RESOLUTION` of where the file puts it), so the user can move it like any placed
  import. A scaled or mirrored copy, or one the turns cannot reproduce, is mapped and canonicalised
  alone at the origin; if mapping fails it is left out with `Misplacement::CopyUnplaceable`'s note.
- Each `ImportedBody` carries the copy's STEP `colour`, `opacity`, `finish` (`caditor_step::Finish`
  turned into the document's) and `layer` as `group`, and its faces' looks as `ImportedFace`s (index
  in the stored solid, colour, opacity, finish; kept only when one differs from the body's), carried
  onto each lump through `caditor_step::lump_faces` for every lump whose face count matches. STEP
  opacity (percent) is snapped on reading to the nearest body opacity step, solid included
  (`nearest_opacity_step`, `document.md`), so a nearly solid body imports solid.
- `bodies_transaction` adds an `Import` feature per body under unique names (stored as `import` or
  `placed_import`, `file-format.md`). A coloured or see-through body gets one `SetBodyAppearance`;
  each `ImportedFace` a `FaceColour` naming it (`FaceName::imported`, origin `Imported`), coloured
  with its own colour, else the body's, else `DEFAULT_BODY_COLOUR`, so a face see-through but
  coloured by neither stays see-through, with its finish. A layered body goes in the folder named
  by its layer (one line, cut at `MAX_GROUP_NAME_CHARS`), a layer's bodies moved together at its
  first body; mesh imports have neither. The app refuses, before applying, an import whose STEP text
  plus the existing imports' (`FeatureKind::stored_text_len`, a text shared by copies counted once)
  would exceed `MAX_MODEL_RECORDS`, since that model could be neither saved nor journaled.

## Mesh import (`import/mesh.rs`)

- `read_mesh_file` picks the format by extension (`MeshFormat`): STL (binary when its size matches
  the triangle count, else text), OBJ (`v` and `f`, polygons fanned, `/` parts and negative
  indices) and 3MF (`zip_read.rs`: stored and deflated entries, no ZIP64, `Archive::read` telling a
  missing entry from a damaged or oversized one, `EntryError`; model part from `_rels/.rels`, build
  items and nested components placed by their transforms). Every model part and `_rels/.rels` is
  read with `import/xml.rs`, never roxmltree.
- 3MF model parts (`Package`): a build item or component with the Production extension's `path`
  (namespace `http://schemas.microsoft.com/3dmanufacturing/production/2015/06`, matched by
  namespace not prefix) takes its object from that part, as Bambu Studio and OrcaSlicer keep meshes
  in `3D/Objects/*.model`; otherwise from its own part. Object ids are scoped per part
  (`UntrustedMap`). `part_path` normalises a path (relative to the referring part's folder unless
  it starts with `/`; `..` escaping is `ModelPartOutsidePackage`), found without regard to case;
  each part is read at most once and only when something placed refers to it, so an absent one is
  `MissingModelPart` only then. Each part's `unit` converts its vertices and transform translations
  to millimetres. A part is parsed once into owned objects (`Shape`, `Reference`), failures kept per
  object and component and raised only when placed.
- 3MF budgets belong to the package: inflated bytes of every part (and `_rels/.rels`) come out of
  one `MAX_FILE_SIZE` (`UnpacksTooLarge`), every part's elements out of one `MAX_MODEL_ELEMENTS`
  (`ModelTooManyElements`). Component expansion is charged across parts: every placed object against
  `MAX_EXPANDED_OBJECTS`, placed vertices and triangles each against `MAX_MODEL_ELEMENTS`, counted
  before any vertex is pushed (`add_mesh`); past any the file is `ModelPlacesTooMuch`, and each
  placed object polls the kernel interrupt. An object reached again while its own components are
  being placed is `ComponentCycle` naming object and part; nesting past `MAX_COMPONENT_DEPTH` is
  damage. STL and OBJ have no units and read as millimetres with a note.
- Triangles become solids through the kernel's `faceted_solids` (`kernel.md`), each canonicalised
  like a STEP body and imported as an `Import` named after the file; what the kernel repaired or
  left out becomes import-report notes. Failures are their own `ImportError` variants
  (`DamagedMesh`, `ModelTooManyElements`, `ModelPlacesTooMuch`, `MissingModelPart`,
  `ModelPartOutsidePackage`, `ComponentCycle`, `MeshNotClosed`, `MeshTooDetailed`, `MeshNotSolid`).

## Export (`export/`)

- `export_bodies` writes STEP (`step-write.md`: a body with a look is styled with colour and, when
  see-through (`Look::opacity`), transparency, in the app's default colour for opacity alone;
  per-face looks from `ExportBody::faces`, `ExportFace` by index among the solid's faces, filled
  from the appearance's `face_colours` and `face_opacities`; a body in a folder
  (`ExportBody::group`) goes on a layer of the folder's name), or tessellates at a `MeshResolution`
  (chord fraction of the largest body's diagonal plus an angle between triangles) into:
  - STL (`MeshOptions::stl`, below).
  - 3MF: one named object per body, millimetres, a thumbnail when given; a body with a colour or
    material points into one `basematerials` group, its `base` named after the material else the
    body, coloured with the body's colour or the app's default (`ExportBody::look`).
  - OBJ: one named object per body, global 1-based indices, millimetres, Z up, no normals; bodies
    with a look name a material `<index>_<material or body>` (whitespace as `_`) from a `.mtl` of
    the same stem written first, `Kd` the colour; an existing `.mtl` not starting with caditor's
    header is never replaced and the OBJ goes without colours.
  - Binary glTF (`.glb`): one node and mesh per body, f32 positions in metres with Y up (`x, z, -y`
    of the model's, keeping winding) with the bounds glTF requires, no normals (flat); a body with a
    look gets a material named like its 3MF base, colour as linear `baseColorFactor`.
- STL has two `StlEncoding`s, streamed into the temporary through a buffer (`write_streamed`).
  Binary puts every body in one surface of f32 millimetres, the form every slicer reads; since f32
  rounds by over a micrometre past `stl::PRECISE_REACH` (32,768 mm), a model reaching beyond it is
  moved by its bounding-box centre rounded to whole millimetres, named in the 80-byte header
  (`moved by x y z`) and returned as `Exported::moved` for the app and CLI to say how to move it
  back. Text writes one `solid` per body, named with anything but printable ASCII as `_` (readers
  sniffing for binary bytes would misread it), six-decimal coordinates, never moved.
- `export_bodies` takes `ModelProperties`; notes are never exported, the rest go where the format
  has room (`EXPORTED_PROPERTIES`): STEP through `write_step_detailed`, 3MF as `Title`, `Designer`
  (author) and `Description` metadata plus the part number as each build item's `partnumber` when
  there is one body, glTF as `asset.extras` (`title`, `partNumber`, `revision`, `author`,
  `organisation`, `description`), OBJ as `# <Label>: <value>` lines after its header. STL has none.
- `ExportBody::threads` (`ExportThread`: designation, side, pitch, start, direction, length; filled
  by the app from `placed_threads`) carries cosmetic threads: STEP as a thread property of the
  body's part (`step-write.md`), glTF as the node's `extras.threads`, OBJ as a
  `# Thread: <designation>` line after the object's `o` line, 3MF as below. STL carries none.
- 3MF threads: one `metadatagroup` first in the body's `object` (before `mesh`), holding for thread
  n (from 1) six `metadata` entries with `preserve="1"` named `caditor:thread.<n>.<field>`:
  `designation`, `side` (`internal`/`external`), `pitch` and `length` (mm), `start` and `direction`
  (three space-separated numbers in the object's own coordinates, which are the model's; direction
  runs along the axis from the start). The `caditor` prefix is declared on `model` as
  `xmlns:caditor="urn:caditor:3mf"` only when some body has a thread, and is not in
  `requiredextensions`, so unaware consumers ignore it. The URI is a URN because the project
  publishes no site or identity to base an `http` one on; it is `CADITOR_NAMESPACE` and never
  changes once files carry it: new fields are new names, a changed meaning a new name.
- Exports save atomically like a model. Cancellation is checked between bodies and before writing;
  failures are sentences naming the body. A body that cannot be meshed or written (a panic
  included) is left out and returned in `Exported::left_out` (the app says so in a notice that
  outlasts edits); the export fails only when no body could be written. STEP does this through
  `write_step_keeping_what_can_be`.
- `export_png` writes 8-bit RGBA, straight alpha, sRGB chunk, through the pure-Rust `png` crate,
  atomically, streaming whatever `PixelRows` yields (bands of whole rows) into the encoder so the
  image is never held whole; it checks the pixel count and cancellation between bands. Errors are
  `PngExportError`: the source's own as `Pixels`, else an `ImageExportError`.
- 3MF is a ZIP from a small writer (`zip.rs`, `ZipWriter`) streamed into the temporary like STL:
  content types, relationships and thumbnail first, whole (deflated through `miniz_oxide` unless
  storing is smaller), then the model written straight into `miniz_oxide`'s streaming compressor
  with its CRC32 alongside and sizes patched into the local header once known, so the XML is never
  held. ZIP64 only where needed: the model reserves a ZIP64 field in its local header when
  `model_size_bound` allows over 4 GiB, and the central directory and end records switch to ZIP64
  for any size, offset or count past 32 or 16 bits. Deflate and CRC32 live only here, for the
  foreign format. The thumbnail is `MeshOptions::thumbnail` (RGBA) encoded to
  `Metadata/thumbnail.png` in memory (`image::encode_png`) with a `png` content type and an OPC
  thumbnail relationship; one that cannot be encoded is left out with a log line.

### Drawings

- `export_drawing` turns each source (`NamedSketch` or `NamedFace`, both kinds in one drawing,
  sketches first) into a `Figure` of 2D `Shape`s each on a `Layer` (`export/figure.rs`), arranges
  them by a `DrawingSheet` (`export/sheet.rs`) and writes with `export/dxf.rs` or `export/svg.rs`;
  `SketchFormat::of` picks DXF or SVG from the path. It returns `DrawingExported` (a
  `SketchExported`, a `FaceExported`, plus the parts wider than the sheet) and fails with `NoCurves`
  only when no sketch draws anything and there is no face. `export_sketches`/`export_faces` are
  its one-kind forms, `export_sketch`/`export_face` the one-source, default-sheet ones. Written
  atomically, cancellation checked between sources, while nesting and before writing.
- A sketch writes its solved curves in its own 2D coordinates on layer 0, and a `POINT` for a point
  no curve uses; an ellipse or elliptical arc is an `ELLIPSE` (`figure::drawn_ellipse`, longer
  radius as major axis since DXF wants a ratio at most 1). Construction curves are counted and left
  out (`Construction::LeftOut`), or with `Construction::OnLayer` written on layer `Construction`
  dashed and counted in `SketchExported::construction`. The DXF reads back through `parse_dxf` as
  the same curves, construction ones as construction.
- A face (`export/outline.rs`) writes a flat face's outer loop on `Outline` and inner loops on
  `Holes`, seen from outside the body: the face's plane with its origin where the model origin
  projects onto it, up along Z for a face more upright than 45 degrees and along Y otherwise, so a
  top face keeps the model's X and Y and a bottom one is mirrored. Lines, circles and arcs stay
  exact, an ellipse becomes an elliptical arc, a spline edge the exact piece of its curve
  (`BSpline::restricted`, weights kept), anything else (intersection curves) is sampled within
  1 micrometre or a millionth of the body's diagonal and written as one cubic `SPLINE`
  interpolating the samples (`outline::fitted`, kept only when its midpoints between samples stay
  within that chord of the curve), else a polyline; either is counted in
  `FaceExported::approximated`. A curved face is `ExportError::FaceNotFlat`; outlining runs under
  `catch_unwind`.
- `SheetLayout::SideBySide` (default) leaves the first source in its own frame and shifts each next
  right of what is placed by a gap of a tenth of the largest (at least 10 mm), bottoms level, so
  one source is written exactly where it is. `SheetLayout::Nested(Nesting)` (`export/nest.rs`)
  packs true shapes bottom-left from the origin, largest first: each source with its label band is
  traced into polylines (`Shape::traced`) and rasterised conservatively (every cell a curve touches
  plus the even-odd inside, so a ring's hollow stays free) into row runs on a grid of square cells,
  grown by the spacing in cells, and goes to the lowest, then leftmost, spot where its runs miss
  every placed run within the sheet width; with `Nesting::turns` it also tries quarter turns and
  the 15 degree step boxing it smallest (`Motion::turn`). A source wider than the sheet every way
  goes alone above the others and is counted in `too_wide`. `Nesting::new` refuses a non-positive
  width or negative spacing. Deterministic; a label is placed below its source's turned bounds.
- `Annotations::Included` adds, for a sketch, a dimension per dimensional constraint
  (`export/annotation.rs`, measured from the solved geometry, active or not; one touching a
  left-out construction curve is skipped) on layer `Dimensions`: distances as a dimension line
  parallel to the gap with extension lines, horizontal and vertical ones along X or Y, radius and
  diameter as a leader through the centre, an ellipse's radii as an `R` leader along that axis,
  angles as an arc at the vertex, arc sweep and length as an arc beyond the arc; millimetres to
  three decimals, degrees to two. Placement shares the canvas's `caditor_sketch::annotation`
  (`sketch.md`) in drawing units scaled by the text height: overlapping distances on one side take
  lanes (`annotation::lanes`, `LANE_SPACING` apart), and each label is kept clear of earlier ones in
  constraint order through `Obstacles` of `LABEL_CELL` cells (footprints grown by
  `LABEL_CLEARANCE`), trying `NEARBY_LANES` lanes outward by sliding along the dimension line
  (`SLIDES`) and turning a circle's or arc's leader (`LEADER_TURN`), then lanes further out up to
  `Style::lanes_to_clear`. No dimension is ever left out: thinning is a screen aid of the canvas.
  Every source gets its name as a label below it on `Labels` (nesting reserves the band). Text is
  `sheet::text_height` high: a fiftieth of the largest source, at least 2.5 mm.
- DXF is ASCII AC1015, millimetres (`$INSUNITS` 4), header and entities (layers named by the
  entities), plus, only when the Construction layer is used, a TABLES section with `CONTINUOUS` and
  `DASHED` linetypes and that layer (grey, dashed), each construction entity also naming `DASHED`
  since `parse_dxf` takes a dashed linetype for construction. Entities: `LINE`, `CIRCLE`, `ARC` (a
  full sweep is a circle), `ELLIPSE`, clamped planar `SPLINE` (weights when rational, flag 12),
  open `LWPOLYLINE`, `POINT`, `TEXT`. A dimension is a `DIMENSION` entity with subclass markers
  (rotated linear, 3-point angular, radial or diametric, style `STANDARD`, text as override) whose
  lines, arcs and text are an anonymous `*D<n>` block in BLOCKS, so readers that redraw dimensions
  and readers that show the block agree; `parse_dxf`, expanding blocks only through `INSERT`, reads
  the file back as the same curves with dimensions and labels counted in notes. Text escapes `%` as
  `%%%`, the diameter sign as `%%c`, the degree sign as `%%d`, other non-ASCII as `\U+XXXX`,
  control characters as spaces.
- SVG is in millimetres with y flipped (`-y`), a 1 mm margin in the viewBox, a 0.1 mm black
  hairline, `line`, `circle`, elliptical-arc `path`s (sweep flag 0 since the flip keeps the drawn
  direction), splines and polylines as `polyline`s, points as small filled circles, text as
  sans-serif `text` (XML-escaped, rotated like the DXF's). Shapes off layer 0 are grouped in a `g`
  whose `id` is the layer name; the Construction group is grey and dashed.

## Parameter files (`parameters.rs`)

- Parameters export as CSV (RFC 4180, `PARAMETERS_EXTENSION`), CRLF line ends: a version row
  `caditor-parameter-file,1` (a spreadsheet keeps it as an ordinary two-cell row), a header row
  `name,expression,value,note`, then one row per parameter in list order. The expression is written
  with parameter names (`Expression::to_text`) and carries its own units; `value` is the evaluated
  value for people (`error` when it does not evaluate) and ignored on reading. Cells holding a
  delimiter, quote, line break, surrounding spaces or a leading `=`, `+`, `-` or `@` are quoted so
  a spreadsheet never reads one as a formula. Written with `write_atomically`.
- `parse_parameters` finds `name` and `expression` columns by header in any order and case, an
  optional `note` (or `description`) column, ignoring others; the delimiter is the one of comma,
  semicolon or tab the header row uses most. Expressions are typed text. A leading version row
  (marker `caditor-parameter-file`, case-blind, then the number) is optional, so a file made by hand
  reads; a number above 1 is `NewerVersion`, a missing or non-numeric one `UnreadableVersion`. A
  first row starting with `caditor-parameters`, the marker of development builds before version 1,
  is refused whole as `FromEarlierBuild`. A change to the typed grammar raises the version. A
  byte-order mark, multi-line quoted cells and blank rows are accepted. A file over
  `MAX_PARAMETERS_FILE`, not UTF-8, without both columns, with an unclosed quote (named by line) or
  over `MAX_PARAMETER_ROWS` rows is refused whole; anything wrong in a row is left to
  `plan_parameter_import` (`document.md`).

## Failures

- Reading and writing fail as `ReadFailure` and `WriteFailure` (`reason.rs`), one variant per cause
  an `io::Error` can name, written as a clause the UI completes ("... because it no longer
  exists"); `LoadError::Unreadable`, `ImportError::Reading`, `ExportError::Writing` and
  `ImageExportError::Writing` carry them. A job that panicked is `LoadError::Crashed` or
  `ImportError::Crashed`, not a message. A save fails as a `SaveError` of its own variants
  (`Writing(WriteFailure)`, `ModelTooLarge`, `ChangedOnDisk`, ...) and the recovery journal as a
  `JournalFailure`.
