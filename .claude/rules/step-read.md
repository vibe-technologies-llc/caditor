---
paths:
  - "crates/caditor-step/src/part21.rs"
  - "crates/caditor-step/src/read/**"
---

# STEP reader

## Part 21 parser (`part21.rs`)

- Reads the header, data sections and edition 3 `ANCHOR`, `REFERENCE` and `SIGNATURE` sections
  (skipped); nesting is limited by `MAX_NESTING`. The tree borrows from the text; instances are
  found by binary search.
- Damage is survived and reported as notes, never refused: a damaged header is skipped (never
  used), a missing `END-ISO-10303-21;` is accepted, an unreadable entry is skipped and counted, a
  repeated entity id keeps its first definition.

## `read_step`

- Returns every `MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS`, `FACETED_BREP` and closed
  `SHELL_BASED_SURFACE_MODEL` as named kernel solids plus notes, or a `ReadError` in words;
  surface bodies are left out with a note. A file with no solids is refused as
  `ReadError::NoSolids(Held)`, naming its schema and any open surface bodies, tessellated shapes
  or wireframes it holds. A body that fails does not stop the others; when none is left the first
  failure is `ReadError::NotRebuilt { name, entity, reason }`, else the first placement that left a
  body out is `ReadError::NotPlaced { name, misplacement }` (`Misplacement` also words the notes).
- Every solid goes through `SolidBuilder::build`: valid, or a sentence naming the entity. A solid
  whose faces cross (`Solid::find_crossing`) is refused naming the face entities; an inconclusive
  check imports it with a note that features built on it may fail.
- Hostile-input bounds: `MAX_SPLINE_DEGREE`, checked knot arithmetic before expansion, one
  `MAX_WORK` budget per file for everything built, `MAX_DEPTH` and `MAX_INSTANCES` for assemblies.
  Curves, surfaces, placements and solids are memoised per entity and units.

## Units and precision

- Units come from each representation's context (SI prefixes, conversion-based units); a note
  names every length unit converted, and a file naming none is read as millimetres, with a note.
- Declared precision is the coarsest length uncertainty of the context, clamped between
  `LINEAR_RESOLUTION` and `COARSEST_PRECISION` so a hostile file sets neither zero nor a huge
  tolerance.
- Solids are still validated at `LINEAR_RESOLUTION`, since the kernel cannot hold looser geometry:
  precision never loosens validity. It decides what the file means (composite segments joining, a
  polygon having a plane, a torus being a horn), which healing earns a note (farther than it from
  the faces; nearer heals silently as the file's own noise) and how a refusal reads (a file
  exported too coarsely for caditor).

## Assemblies

- Followed from each solid's representation to the roots through transformation relationships and
  `MAPPED_ITEM`s: one solid per placement. A product's only body takes the product's name; copies
  take their occurrences' names when each has a distinct one, else a number.
- The child representation comes from `NEXT_ASSEMBLY_USAGE_OCCURRENCE` and
  `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION`, else is guessed from which side is some assembly's
  child; `rep_1` is carried into `rep_2`, so the transform is inverted when the parent is listed
  first.
- A placement is a `Similarity`: a `CARTESIAN_TRANSFORMATION_OPERATOR_3D` may scale (a positive
  factor within the kernel's range) and mirror (a given second axis against the right-handed one),
  applied with `Solid::mapped` so the copy keeps its names and validates. A part, or copy, with an
  unreadable placement, a cycle, or too deep a nesting is left out with a note while the others
  import; a copy scaled past what the kernel models is left out with its own note
  (`Misplacement::CopyUnplaceable`).

## Geometry and healing

- Every kernel surface and curve, B-splines in all forms (unclamped ones clamped by knot
  insertion), trimmed and composite curves, `OFFSET_CURVE_3D` as dense polylines that healing
  rebuilds on the faces.
- `DEGENERATE_TOROIDAL_SURFACE`: a spindle is the revolution of the tube's rational arc on one side
  of the axis; a horn turns the whole tube circle about its touch point, so its two poles are one
  vertex. The inside of a horn is refused (no volume).
- `OFFSET_SURFACE` of a plane, cylinder, sphere, torus or cone is the exact surface of the same
  kind; one leaving no surface, or of a spline, extrusion or revolution, is refused in words.
- Vertices off their faces move onto all of them by damped least squares; edges farther than a
  quarter of the resolution from either face are rebuilt with `IntersectionCurve::through`.
- The outer loop is the `FACE_OUTER_BOUND`, else the one using a seam, else the largest by area;
  faces bounded only by `VERTEX_LOOP`s get a pole-to-pole seam. `POLY_LOOP` faces get line edges
  shared by corner position and a plane from the polygon when none is named.
