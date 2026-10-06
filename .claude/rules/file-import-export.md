---
paths:
  - "crates/caditor-file/src/import/**"
  - "crates/caditor-file/src/export/**"
---

# DXF and STEP import, export

## DXF import (`import/`)

- `parse_dxf` reads ASCII and binary DXF into a `Drawing` of 2D `DrawingCurve`s in millimetres plus
  plain-language notes; only header, layers, MLINESTYLEs, blocks and entities are kept.
- Text is UTF-8 when valid, else in the header's `$DWGCODEPAGE` (`code_page.rs`; unknown reads as
  1252).
- Hostile-input limits, each an `ImportError` naming it: `MAX_DRAWING_VALUES`,
  `MAX_EXPANDED_OBJECTS` (block expansion), `MAX_DRAWING_POINTS`. Curves past `MAX_DRAWING_CURVES`
  are left out with a note and the first ones import.
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
  bounding box (arcs by their sweep) on the origin. It adds a note per change it makes and leaves
  the original untouched.
- Dashed linetypes make a curve construction geometry (`Drawing::construction`, added by
  `drawing_transaction` with ordinary end points), with a note.
- Blocks and INSERTs nest with cycle and depth limits; a block's content is decoded once and shared
  by its instances.
- SPLINE uses control points up to `MAX_SPLINE_DEGREE`, else fit points: with an end tangent the
  cubic AutoCAD draws (`FitPoints::cubic`), otherwise `BSpline::through`.
- HATCH imports boundary paths only, never the fill (a note counts hatches); a path whose source
  objects are all drawn entities of the same space is left to them, so an associative hatch does
  not double its outline.
- Everything becomes a 3D shape in its object coordinate system (arbitrary axis algorithm), is
  transformed, then flattened onto XY: conics projecting to circles become circles and arcs; other
  conics and non-uniform splines are fitted within a millionth of the drawing's size.
- `drawing_transaction` makes one transaction on an existing or new sketch, dropping curves shorter
  than the joint tolerance and joining ends within it with `Coincident` constraints.

## STEP import (`import/model.rs`)

- `read_step_file` unpacks gzip first (`.stpz`, or the gzip magic), within `MAX_FILE_SIZE` and with
  the trailer's CRC and size checked, else `DamagedArchive` or `UnpacksTooLarge`. Non-UTF-8 text
  is read as Latin-1 with a note.
- Each solid is canonicalised: written by caditor's own writer and read back, so what is stored is
  exactly what later loads. One `ImportedBody` per solid, or per lump (each canonicalised alone)
  when a multi-lump solid reads back as several; a body that cannot be stored is left out with a
  note.
- `bodies_transaction` adds an `Import` feature per body under unique names; the model file stores
  it as an `import` record (`file-format.md`).

## Export (`export/`)

- `export_bodies` writes STEP, or tessellates at a `MeshResolution` (a chord fraction of the
  largest body's diagonal plus an angle between triangles) into binary STL (all bodies in one
  surface), 3MF (one named object per body, millimetres), OBJ (one named object per body, global
  1-based indices, millimetres, Z up, no normals) or binary glTF (`.glb`: one node and mesh per
  body, f32 positions in metres with Y up, `x, z, -y` of the model's, so winding is kept, and the
  position bounds glTF requires; no normals, which the format defines as flat).
- Saved atomically like a model. Cancellation is checked between bodies and before writing;
  failures are sentences naming the body. A body that cannot be meshed or written (a panic
  included) is left out and returned in `Exported::left_out` (the app says so in a notice that
  outlasts edits); the export fails only when no body could be written. STEP does this through
  `write_step_keeping_what_can_be`.
- `export_sketch` (`export/dxf.rs`) writes the solved curves of one sketch as an ASCII DXF of version
  AC1015: millimetres in the sketch's own 2D coordinates on layer 0, `LINE`, `CIRCLE`, `ARC` (a
  full sweep is a circle), clamped `SPLINE` (planar, with its knots and control points) and `POINT`
  for a point no curve uses. Construction curves are counted and left out; a sketch with nothing
  else is `ExportError::NoCurves`. The result reads back through `parse_dxf` as the same curves.
  `SketchFormat::of` picks DXF or SVG from the path; `export/svg.rs` writes SVG in millimetres with
  y flipped (`-y`), a 1 mm margin in the viewBox, a 0.1 mm black hairline, `line`, `circle`,
  elliptical-arc `path` (sweep flag 0, since the flip keeps the drawn direction), splines as
  `polyline`s and points as small filled circles.
- `export_png` writes 8-bit RGBA, straight alpha, sRGB chunk, through the pure-Rust `png` crate,
  atomically, checking the pixel count and cancellation; errors are `ImageExportError` variants.
- 3MF is a ZIP from a small writer (`zip.rs`; deflate through `miniz_oxide` unless storing is
  smaller, CRC32, no ZIP64): deflate and CRC32 live only here, for the foreign format.

## Failures

- Reading and writing a file fail as `ReadFailure` and `WriteFailure` (`reason.rs`), one variant per
  cause an `io::Error` can name, written as a clause the UI completes ("… because it no longer
  exists"); `LoadError::Unreadable`, `ImportError::Reading` and `ExportError::Writing` carry them.
  A job that panicked is `LoadError::Crashed` or `ImportError::Crashed`, not a message. A save fails
  as a `SaveError` of its own variants (`Writing(WriteFailure)`, `ModelTooLarge`, `ChangedOnDisk`,
  …) and the recovery journal as a `JournalFailure`; `ImageExportError::Writing` carries a
  `WriteFailure` too.
