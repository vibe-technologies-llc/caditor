---
paths:
  - "crates/caditor-file/src/import/**"
  - "crates/caditor-file/src/export/**"
---

# DXF and STEP import, export

## DXF import (`import/`)

- `parse_dxf` reads ASCII and binary DXF (group codes with typed values) into a `Drawing` of 2D
  `DrawingCurve`s in millimetres plus plain-language notes.
- Text is UTF-8 when valid, else in the `$DWGCODEPAGE` the header declares (found by scanning the
  header's tokens before decoding): Windows pages 874 and 1250 to 1258 and ISO 8859-1 are tables in
  `code_page.rs`; any other page, or none, reads as 1252; undefined bytes become U+FFFD; `\U+XXXX`
  escapes become their characters.
- Tokens are read lazily (text borrowed unless it must be decoded) and grouped into records one at
  a time. Only the header, layers, MLINESTYLEs, blocks and entities are kept; records are moved,
  never copied; extended data (codes 1000 to 1071) is dropped. At most `MAX_DRAWING_VALUES` values
  in all and in any one record, else `ImportError::TooManyValues`.
- `$INSUNITS`: every code up to the US survey units; an unknown code reads as millimetres with a
  note naming it; none reads as millimetres with a note, unless `$MEASUREMENT` is 0 (imperial),
  then inches, also with a note.
- Damage (`DamagedAt`, `Damaged`) ends reading where it is found: the records before the one it
  cuts short are interpreted as usual and a note says where, so one bad line costs the rest of the
  file, not all of it. The record it interrupts is dropped, since it may be missing fields. A file
  damaged before any section, or whose records before the damage draw nothing, is still refused
  with the damage as the error.
- Entities on off or frozen layers, and on the non-plotting `DEFPOINTS` layer (dimension
  definition points), are left out.
- Linetypes: an LTYPE with dash elements (code 73 above zero) is dashed; an entity's own linetype
  (code 6) wins, `BYLAYER` or none takes its layer's, and `BYBLOCK` takes the dashing of the INSERT
  that places it. A dashed curve (never a point) is listed in `Drawing::construction`, by index
  into `curves`, and `drawing_transaction` adds it as construction geometry, its end points
  ordinary; a note counts them.
- Blocks and INSERTs: base point, scale, rotation, column and row arrays, nested with cycle and
  depth limits; at most `MAX_EXPANDED_OBJECTS` objects and cells visited in all; block content on
  layer 0 takes the insert's layer.
  - Each block's content is decoded once and shared by its instances, with each entity's space,
    visibility and layer state and each INSERT's placement read then, not on every visit.
  - An array of a block that draws nothing is visited once, what it leaves out counted per cell.
  - At most `MAX_DRAWING_POINTS` points and knots in all, else `ImportError::TooDetailed`.
- Entities:
  - LINE, POINT, CIRCLE, ARC, ELLIPSE, LWPOLYLINE, POLYLINE (bulges become arcs, 3D polylines
    lines).
  - SPLINE: control points with knots up to degree 9 and a weight per control point or none, else
    its fit points. With an end tangent (codes 12 and 13) the fit points give the cubic AutoCAD
    draws (`FitPoints::cubic`): chord-length parameters as knots, the tangent scaled by the
    polyline's length as the end derivative, a zero second derivative at an end without one, one
    tridiagonal system. Otherwise fit points are rebuilt by `BSpline::through`.
  - HATCH boundaries (`hatch.rs`; the paths between code 91 and the pattern data, so seed points
    are never geometry): polyline paths with bulges, always closed; edge paths of lines, arcs,
    elliptic arcs (angles read as angles and turned into ellipse parameters, as ezdxf does) and
    splines; clockwise edges by their complementary angles. Text-box paths are skipped; the fill is
    not imported (a note counts hatches). A path whose source objects (code 330) are all drawn,
    shown entities of the same model space or block is left to them (no double outline for an
    associative hatch); one without sources is imported even where lines trace it.
  - SOLID and TRACE (corners 1, 2, 4, 3 in their object system) and 3DFACE (visible edges only)
    become their outlines (`outline.rs`).
  - MLINE (`mline.rs`): a line per element between the vertices' miter points (first element
    parameter, as ezdxf; dashes ignored), closed or with the square, round and inner-arc caps and
    joint miters its MLINESTYLE (from OBJECTS, by handle, else name) asks for; without a style only
    the element lines.
- Object coordinate systems follow the arbitrary axis algorithm.
- Everything becomes a 3D shape (point, line, parametric conic, NURBS or fit points), is
  transformed, then flattened onto XY: conics projecting to circles become circles and arcs
  (counter-clockwise); other conics and splines not already in the sketch's uniform form are fitted
  within a millionth of the drawing's size, sampled with knot spans found by binary search.
- Paper space and invisible entities are skipped silently; text, dimensions and other annotations
  are counted in a note. Curve cap: `MAX_DRAWING_CURVES`.
- `drawing_transaction` turns a drawing into one transaction on an existing or new sketch, dropping
  curves shorter than the joint tolerance and joining ends closer than a millionth of the drawing's
  size with `Coincident` constraints.

## STEP import (`import/model.rs`)

- `read_step_file` reads STEP through `caditor-step` and canonicalises each solid (written by
  caditor's own writer and read back, so what is stored is exactly what later loads), giving one
  `ImportedBody` per solid, or per lump (each canonicalised alone) when a multi-lump solid reads
  back as several, plus notes. Text that is not valid UTF-8 is read as Latin-1, with a note.
- `bodies_transaction` adds an `Import` feature per body under unique names. The model file stores
  an import as its source name and STEP text (`import` records); an unreadable one loads empty, with
  a report.

## Export (`export/`)

- The one place besides import that follows foreign formats. `export_bodies` writes each
  `ExportBody` (name and solid) as:
  - STEP through `caditor-step` (resolution ignored, no triangle count); or
  - a tessellation at a `MeshResolution` (coarse, standard or fine: a chord that is a fraction of
    the largest body's diagonal, and 20°, 10° or 5° between triangles), keeping only positions the
    triangles use and dropping collapsed triangles, written as binary STL (every body in one
    surface, facet normals from the winding, counted in one pass and written in a second) or 3MF (one
    named object per body, millimetres, coordinates to six decimals with trailing zeros dropped).
- Files are saved atomically like a model. Cancellation is checked between bodies and before
  writing; failures are sentences naming the body. A mesh export leaves out a body that cannot be
  meshed and returns its error in `Exported::left_out` (the app says so in a notice that outlasts
  edits); it fails only when no body could be meshed. A STEP export does the same through
  `write_step_keeping_what_can_be`, which rolls a body that cannot be written back out of the file
  (`Data::roll_back`) and names its index; `write_step` stays strict and returns that error.
- `export_png` (`export/image.rs`) writes 8-bit RGBA with straight alpha and an sRGB chunk
  through the pure-Rust `png` crate (fast compression), checking the pixel count against the size,
  cancellation before encoding and before writing, then saving atomically; errors are
  `ImageExportError` variants, a write failure in the words of `reason::writing`.
- The 3MF package uses a small ZIP writer (`zip.rs`): deflate through `miniz_oxide` unless storing
  is smaller, CRC32, no ZIP64.
