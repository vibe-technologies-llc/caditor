---
paths:
  - "crates/caditor-step/src/write/**"
  - "crates/caditor-step/src/lib.rs"
---

# STEP writer

- `caditor-step` speaks STEP (ISO 10303-21, AP214, `SCHEMA`); it depends only on the kernel and
  geometry crates, and only `caditor-file` uses it.
- `write_step` writes one body as one product named after it, and several as an assembly: a root
  product named after the model whose `SHAPE_REPRESENTATION` holds one identity placement per
  part, and one part product per body (named after it, with its own
  `ADVANCED_BREP_SHAPE_REPRESENTATION`) placed by a `NEXT_ASSEMBLY_USAGE_OCCURRENCE` and a
  `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` with an identity `ITEM_DEFINED_TRANSFORMATION`, so other
  programs list the bodies as parts and the reader gives them back by name. Each lump is one
  `MANIFOLD_SOLID_BREP`, or a `BREP_WITH_VOIDS` whose voids are `ORIENTED_CLOSED_SHELL`s of
  inverted faces; every representation shares one context. Millimetres and radians, uncertainty
  `LINEAR_RESOLUTION`, the application named by name and version only.
- `write_step_detailed` takes `StepDetails` (empty in `write_step` and imports): the title names
  the root product (over the body or model name), the part number is its id, the description
  the product's and the header's `FILE_DESCRIPTION` (else the model name), the revision the
  `PRODUCT_DEFINITION_FORMATION` id, and the author and organisation fill `FILE_NAME`, which are
  otherwise empty.
- A `StepBody` with a `colour` gets one `STYLED_ITEM` per solid, each pointing at a
  `PRESENTATION_STYLE_ASSIGNMENT` (surface fill of `COLOUR_RGB`, the 0–255 channels over 255,
  `.BOTH.` sides) written once per distinct colour and opacity, all gathered in one
  `MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION` on the shape's context; uncoloured
  bodies write no presentation at all. The reader ignores it, since its items are styled items.
- A `StepBody` with an `opacity` below 100 (a percent, only written beside a colour) adds to its
  side style a `SURFACE_STYLE_RENDERING_WITH_PROPERTIES` (`.NORMAL_SHADING.`, the same colour)
  holding a `SURFACE_STYLE_TRANSPARENT` of one minus the opacity, the form AP214 and AP242
  readers take for a see-through body; the reader reads it back.
- A `StepBody` with a `layer` (the body's folder) is listed in one `PRESENTATION_LAYER_ASSIGNMENT`
  per distinct layer name, holding its solids; a blank name writes none.
- `write_step_keeping_what_can_be` writes every body it can, rolls a failed one back out and lists
  it in `left_out`; `write_step` fails on the first such error. Only when no body is writable is
  it an error.
- Shells are told apart by the sign of their meshed volume (only bodies with several shells are
  meshed); unsortable shells are `WriteError::Shells`, a non-finite value or unsupported geometry
  `WriteError::Geometry`.
- Every kernel surface and curve has an exact STEP form; the non-obvious ones: cones with a
  negative half angle on a flipped axis, rational B-splines as the complex entity, intersection
  curves as the cubic B-spline of their Hermite segments over the edge. A face's `same_sense` is
  the kernel face sense (inverted for void shells), since kernel normals are STEP's.
- The text is built in one buffer: the header first, then each entity as `Data::add` takes it; a
  body that cannot be written truncates the buffer back to its `Checkpoint`, so the output is
  never held twice.
- `Data` writes each point, direction and placement once, keyed by the exact bits of its
  coordinates (`-0.0` as `0.0`) in `BTreeMap`s, since the coordinates come from imported files and
  the output must be deterministic.
- Reals print as the shortest round-tripping decimal with a point; text escapes quotes,
  backslashes and non-ASCII (`\X2\` in the BMP, `\X4\` beyond).
- Every face kind must stay valid, closed and of the same volume in an outside checker
  (`STEP_SAMPLES=<dir>` writes the fixtures for one).
