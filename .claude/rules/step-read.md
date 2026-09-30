---
paths:
  - "crates/caditor-step/src/part21.rs"
  - "crates/caditor-step/src/read/**"
---

# STEP reader

## Part 21 parser

- Files: `part21.rs` (parser) and `read/` (`read_step`).
- Reads the header, named and repeated data sections, and edition 3 `ANCHOR`, `REFERENCE` and
  `SIGNATURE` sections (skipped byte by byte past strings and comments); sorts complex instances by
  name; reads typed values and comments; decodes `\X\`, `\X2\`, `\X4\` and `\S\` (`\P` code pages
  skipped); limits nesting.
- The tree borrows from the text: names, enumerations and text are kept as written (uppercased
  copies only for lowercase names; text unquoted and decoded when read), lists and parameters are
  boxed slices, instances a vector sorted by id and found by binary search.
- An unreadable data entry (a stray byte, nesting too deep) is skipped to its semicolon and counted;
  a repeated entity id keeps its first definition; an integer beyond i64 is a real.

- `read_step` (its optional records, `Entity::find`, build no error message when absent) notes
  skipped and repeated entries and returns every `MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS`,
  `FACETED_BREP` and `SHELL_BASED_SURFACE_MODEL` with closed shells (each a lump) as named kernel
  solids plus notes, or a `ReadError` in words.
- Every solid goes through `SolidBuilder::build`: valid, or a sentence naming the entity. Faces
  meeting only farther apart than `LINEAR_RESOLUTION` are refused in those words (with the file's
  precision when that allowed the gap); a solid whose faces cross (`Solid::find_crossing`) is
  refused naming the two face entities, or the one face whose edges cross.

## Units and precision

- Units come from each representation's context: SI prefixes and conversion-based units such as
  inches and degrees (factor a simple or complex `MEASURE_WITH_UNIT`); a note names every length
  unit other than millimetres that was converted.
- Declared precision is in the units: the length `UNCERTAINTY_MEASURE_WITH_UNIT`s of the
  `GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT`, by their own unit or, for a bare `LENGTH_MEASURE`, the
  context's; coarsest wins. Clamped between `LINEAR_RESOLUTION` and `COARSEST_PRECISION` (0.01 mm),
  so a hostile file sets neither zero nor a huge tolerance; `LINEAR_RESOLUTION` when none.
- Solids are still validated at `LINEAR_RESOLUTION`, since the kernel cannot hold looser geometry,
  so precision never loosens validity. It decides what the file means (composite curve segments
  meeting within it join; a polygon needs a corner farther than it from its first to have a plane; a
  torus is a horn when the poles of its spindle lie within it of the point where the tube touches
  the axis), which healing earns a note (a vertex or edge farther than it from its faces; nearer
  ones heal silently as the file's own noise) and how a refusal reads (faces meeting only within it:
  a file exported too coarsely for caditor).

## Assemblies

- Followed from each solid's representation to the roots via
  `REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION`, untransformed relationships and `MAPPED_ITEM`s:
  one solid per placement. A product's only body takes the product's name and several bodies keep
  their own; placements of one solid take their occurrences' names when each has a distinct one,
  else a number. The child is the occurrence's child definition's representation (via
  `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` and `NEXT_ASSEMBLY_USAGE_OCCURRENCE`), else guessed from
  which side is some assembly's child; `rep_1` is carried into `rep_2`, so the transform is inverted
  when the parent is listed first.
- A placement is an `ITEM_DEFINED_TRANSFORMATION` between two `AXIS2_PLACEMENT_3D`s or a
  `CARTESIAN_TRANSFORMATION_OPERATOR_3D` (simple or complex), refused unless of unit scale and
  right-handed. A part with an unreadable placement is left out with a note, as are its copies
  placed that way, while the others are imported.
- Placements are memoised per representation (layered assemblies cost one visit per part).
  Assemblies deeper than `MAX_DEPTH`, or placing a part only inside itself, leave that solid out
  with a note; a file yields at most `MAX_INSTANCES` solids.

## Limits

- Spline degrees above `MAX_SPLINE_DEGREE` are refused as read; knot multiplicities must sum to
  points plus degree plus one (checked arithmetic) before any knot is expanded.
- Curves and surfaces are memoised by entity per units context (failures only when met at the top,
  since deeper ones depend on the nesting limit); a solid is built once per set of shells and units
  however many breps name them.
- Everything built (curves, surfaces, composite pieces, faces, solids) is charged to one `MAX_WORK`
  budget per file; past it the rest is refused as too intricate.

## Geometry

- Every kernel surface and curve, B-splines in all forms (Bézier ones with the standard piecewise
  knots, degree-fold at every joint; uniform and other unclamped ones clamped by knot insertion);
  trimmed and surface curves by basis; polylines; `COMPOSITE_CURVE`s (each segment trimmed by point
  or parameter, followed in its sense); `OFFSET_CURVE_3D`s as dense polylines edge healing rebuilds
  on the faces.
- Spindle `DEGENERATE_TOROIDAL_SURFACE`: the revolution of its tube's rational arc on one side of
  the axis (the apple or, mirrored, the lemon), so its normal stays the torus's. Horn: the whole
  tube circle turned about the point where it touches the axis, a revolution whose two poles are one
  vertex (its seam, when the face has only a `VERTEX_LOOP`, a closed edge on it); its inside is
  refused (no volume).
- `OFFSET_SURFACE` of a plane, cylinder, sphere, torus or cone is the exact surface of the same kind
  along the basis's normal, which it keeps (a plane moved, radii grown, a cone's reference circle
  moved along its axis, or to its apex when the offset radius there would be negative). An offset
  leaving no surface (inwards by a cylinder's, sphere's or torus tube's radius or more) or making a
  torus's tube reach its axis is refused in words, as is an offset of a spline, extrusion or
  revolution.

## Topology and healing

- `POLY_LOOP` faces get line edges shared by corner position, and a plane from the polygon when a
  plain `FACE` names none.
- Topology is surveyed first (which faces use each edge and vertex); vertices off their faces move
  onto all of them by damped least squares; edges not within a quarter of the resolution of both
  faces are rebuilt with `IntersectionCurve::through` (each sample is projected from the previous
  one's foot, and afresh only where that foot is not within the quarter resolution).
- Loops take their orientation from bounds, oriented edges and `same_sense` (voids from
  `ORIENTED_CLOSED_SHELL`). The outer loop is the `FACE_OUTER_BOUND`, else the one using a seam,
  else the largest by area. Faces bounded only by `VERTEX_LOOP`s get a pole-to-pole seam (spheres,
  closed spline surfaces and revolutions with two poles).
