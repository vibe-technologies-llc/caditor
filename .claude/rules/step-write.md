---
paths:
  - "crates/caditor-step/src/write/**"
  - "crates/caditor-step/src/lib.rs"
  - "crates/caditor-step/src/fixtures.rs"
---

# STEP writer

- `caditor-step` speaks STEP (ISO 10303-21, AP214); it depends only on the expression, kernel and
  geometry crates, and only `caditor-file` uses it.
- `write_step` writes one body as one product named after it, and several as an assembly: a root
  product named after the model (its `SHAPE_REPRESENTATION` holds one identity placement per part)
  and one part product per body with its own `ADVANCED_BREP_SHAPE_REPRESENTATION`, placed by a
  `NEXT_ASSEMBLY_USAGE_OCCURRENCE` and a `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` with an identity
  `ITEM_DEFINED_TRANSFORMATION`, so other programs list the bodies as parts and the reader gives
  them back by name. Each lump is one `MANIFOLD_SOLID_BREP`, or a `BREP_WITH_VOIDS` whose voids
  are `ORIENTED_CLOSED_SHELL`s of inverted faces. One shared context; millimetres and radians,
  uncertainty `LINEAR_RESOLUTION`, the application named by name and version only.
- `write_step_detailed` takes `StepDetails` (empty in `write_step` and imports): title (root
  product name, over the body or model name), part number (its id), description (the product's and
  `FILE_DESCRIPTION`, else the model name), revision (`PRODUCT_DEFINITION_FORMATION` id), author
  and organisation (`FILE_NAME`, otherwise empty).
- `write_step_keeping_what_can_be` writes every body it can, rolls a failed one back and lists it
  in `left_out`; it errors only when no body is writable. `write_step` fails on the first error.

## Presentation

- A `StepBody` with a `colour`, an `opacity` below 100 or `faces` gets one `STYLED_ITEM` per solid,
  pointing at a `PRESENTATION_STYLE_ASSIGNMENT` written once per distinct colour and opacity, all
  in one `MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION`; a body with none writes no
  presentation. A colour is a surface fill of `COLOUR_RGB` (0-255 over 255, `.BOTH.` sides).
- An opacity below 100 (a percent) adds a `SURFACE_STYLE_RENDERING_WITH_PROPERTIES`
  (`.NORMAL_SHADING.`, the same colour or `$`) holding a `SURFACE_STYLE_TRANSPARENT` of one minus
  the opacity, the form AP214 and AP242 readers take for a see-through body. A body with face looks
  but neither colour nor opacity styles its solids with `NULL_STYLE`, so the reader keeps it plain
  instead of taking the look most faces share.
- `StepBody::faces` (`FaceLook`s by index among the solid's faces; `Shapes` records each written
  `ADVANCED_FACE`) styles each face differing from the body with its whole look (own colour else
  the body's, own opacity else the body's). The reader gives body and face looks back exactly.
  `lump_faces` lists each lump's faces in written order, so import can carry face looks onto lumps
  it stores apart.
- A `FaceLook`'s `finish` is not written: AP214's only place for one, the Phong reflectances of
  `SURFACE_STYLE_RENDERING_WITH_PROPERTIES`, is something neither caditor's reader nor the
  exporters it reads map back to a finish, so it would not round trip.
- A `StepBody` with a `layer` is listed in one `PRESENTATION_LAYER_ASSIGNMENT` per distinct layer
  name; a blank name writes none.
- A `StepBody` with `threads` (`StepThread`: designation, start, direction, length) gets per thread
  a `PROPERTY_DEFINITION('thread',<designation>,…)` on its part (the root product when it is the
  only body), represented by a `REPRESENTATION('thread', …)` of a
  `DESCRIPTIVE_REPRESENTATION_ITEM('designation',…)`, an `AXIS2_PLACEMENT_3D` at the start along the
  direction and a `MEASURE_REPRESENTATION_ITEM('length',…)`, joined by a
  `PROPERTY_DEFINITION_REPRESENTATION`; written after styles and layers, only for written bodies.
  The reader ignores them.

## Geometry

- Shells are told apart by the sign of their meshed volume (only bodies with several shells are
  meshed); unsortable shells are `WriteError::Shells`, a non-finite value or unsupported geometry
  `WriteError::Geometry`.
- Every kernel surface and curve has an exact STEP form. Non-obvious: cones with a negative half
  angle on a flipped axis; a spindle or horn torus as `DEGENERATE_TOROIDAL_SURFACE` selecting the
  outer part (`.T.`), since `TOROIDAL_SURFACE` must not cross itself; rational B-splines as the
  complex entity; intersection curves as the cubic B-spline of their Hermite segments over the
  edge, each segment halved (at most `MAX_WRITTEN_SPLITS` times, new nodes at `refined_point`)
  until the cubic stays within `WRITTEN_DEVIATION` (a quarter of the resolution) of the refined
  curve at its quarter points. Without this, a traced edge on a thread's flanks bows over a
  micrometre off its faces and reads back invalid, so import's round trip drops the part. A face's
  `same_sense` is the kernel face sense (inverted for void shells), since kernel normals are
  STEP's.

## Text

- The text is one buffer: header first, then each entity as `Data::add` takes it; a body that
  cannot be written truncates the buffer back to its `Checkpoint`.
- `Data` writes each point, direction and placement once, keyed by the exact bits of its
  coordinates (`-0.0` as `0.0`) in `BTreeMap`s, so output is deterministic.
- Reals print as the shortest round-tripping decimal with a point; text escapes quotes,
  backslashes and non-ASCII (`\X2\` in the BMP, `\X4\` beyond).
- Every face kind must stay valid, closed and of the same volume in an outside checker
  (`STEP_SAMPLES=<dir>` writes the fixtures for one).
