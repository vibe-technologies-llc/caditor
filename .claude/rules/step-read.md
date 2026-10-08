---
paths:
  - "crates/caditor-step/src/part21.rs"
  - "crates/caditor-step/src/read/**"
---

# STEP reader

## Part 21 parser (`part21.rs`)

- Reads the header, data sections and edition 3 `ANCHOR`, `REFERENCE` and `SIGNATURE` sections
  (skipped); nesting is limited by `MAX_NESTING`. A text past 4 GiB is `ReadError::TooLarge`, so
  every offset fits a `u32`.
- The tree is flat arenas in `Exchange`, never a box per record or list: every parameter value is a
  one-byte kind plus a `u64` payload (number bits, a reference, or a packed offset and length into
  the text, the values or the uppercased names), and a list or record names its run of values by
  span. Items of a list land contiguously because a list's items wait on a pending stack until it
  closes. Text and names stay offsets into the file, a name with lowercase letters is uppercased once
  into a shared buffer, a simple instance holds its record inline and a complex one a span of the
  record arena; an unreadable entry truncates every arena back to where it began. `Parameter`,
  `List`, `Record` and `Instance` are copyable views decoding on access. An 89 MB file holds about
  1.2 times its size while parsed (the boxed tree held 3.3 times); instances are found by binary
  search.
- Damage is survived and reported as notes, never refused: a damaged header is skipped (never
  used), a missing `END-ISO-10303-21;` is accepted, an unreadable entry is skipped and counted, a
  repeated entity id keeps its first definition. A file ending inside `DATA` (no `ENDSEC;`, or cut
  mid-entity, as an interrupted download leaves it) keeps every entity read before the end and is
  `cut_short`, noted as such; whether its solids are complete is left to `NoSolids` and
  `NotRebuilt`.

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
- `STEP_CORPUS=<dir>` (narrowed by `STEP_CORPUS_ONLY=<part of a name>`) makes
  `every_file_of_a_corpus_is_read_and_reported` read every file there and print its solids,
  volumes, notes or refusal and time; run it in release against files whose volumes another
  reader gives.
- Cancellation: `read_step` and `read_step_copies` poll the kernel interrupt (`kernel.md`) before
  parsing and before each solid, answering `ReadError::Cancelled`; nothing installs an interrupt
  by default, so an uncancelled read never sees it.
- Hostile-input bounds: `MAX_SPLINE_DEGREE`, checked knot arithmetic before expansion, one
  `MAX_WORK` budget per file for everything built, `MAX_DEPTH` and `MAX_INSTANCES` for assemblies.
  Curves, surfaces, placements and solids are memoised per entity and units.

## Colours and layers (`presentation.rs`)

- Each solid gets a `colour` and a `layer`, or none. A `STYLED_ITEM` (an `OVER_RIDING_STYLED_ITEM`
  wins) gives its item the first surface colour found within `SEARCH_DEPTH` references of its
  styles: a `COLOUR_RGB` (channels clamped to 0–1, times 255) or a `DRAUGHTING_PRE_DEFINED_COLOUR`
  of the eight named ones; curve, point and text styles are skipped. A
  `PRESENTATION_LAYER_ASSIGNMENT` puts its items on its layer, a styled item counting as the item
  it styles; the first layer naming an item keeps it.
- A solid takes the value on itself, else on one of its shells, else the value every face shares;
  a face without one, or faces differing, give none, so a part of mixed face colours keeps the
  default look rather than one of them.

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
- Each solid is built once and every placement handed to a placer (`read_placed`): `read_step`
  maps each copy into a solid of its own, `read_step_copies` returns them as `StepCopy`s sharing
  the built solid (`Arc`) with their `Similarity`, which import uses to store a part once.
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
  insertion), trimmed and composite curves, `HYPERBOLA` as rational quadratic arcs of at most
  `MAX_HYPERBOLA_SPAN` in its parameter and `PARABOLA` as one quadratic Bézier (its parameter kept),
  both reaching `MODEL_EXTENT` from their centre (a hyperbola trimmed by a parameter value is
  trimmed at the point it names), `OFFSET_CURVE_3D` as dense polylines that healing
  rebuilds on the faces.
- `DEGENERATE_TOROIDAL_SURFACE`: a spindle is the revolution of the tube's rational arc on one side
  of the axis; a horn turns the whole tube circle about its touch point, so its two poles are one
  vertex. The inside of a horn is refused (no volume).
- `OFFSET_SURFACE` of a plane, cylinder, sphere, torus or cone is the exact surface of the same
  kind; one leaving no surface, or of a spline, extrusion or revolution, is refused in words.
- Vertices off their faces move onto all of them by Levenberg–Marquardt least squares (the damping
  grows until a step lowers the worst gap, so nearly tangent faces cannot make it overshoot); edges
  farther than a quarter of the resolution from either face are rebuilt with
  `IntersectionCurve::through`.
- A body the exact build refuses is built again in `Healing::Extended` before any looser mode: each
  spline face's surface is continued past its domain by a tenth of its boundary span
  (`BSplineSurface::extended`), except across a seam, a pole or sides within the file's precision
  of each other, since exporters (Fusion 360 threads) trim a spline exactly where its neighbours
  meet it and their common corner lies a hair beyond. It is a retry, never the first build, as
  continuing a spline that winds over itself can mislead projection.
- A fin, an edge a loop runs out along and straight back (Autodesk exports leave them on
  cylinders), bounds nothing and is left out with its tip vertex, unless one of its ends is a pole
  of the face, where such a fold is the seam reaching an apex.
- The outer loop is the `FACE_OUTER_BOUND`, else the one using a seam, else the largest by area;
  faces bounded only by `VERTEX_LOOP`s get a pole-to-pole seam. `POLY_LOOP` faces get line edges
  shared by corner position and a plane from the polygon when none is named.

## Bent faces (`conform.rs`)

- A body the exact and extended builds refuse is built in `Healing::Bent` before
  `Healing::Faceted` when the file declares a precision. Each edge between two faces is studied from samples of its file curve:
  one whose faces' normals stay within `TANGENT_ANGLE` and that runs along an iso-line of a spline
  face (within the precision; one at the domain bound is a side) is bent, with one face its master
  and a spline it runs along its slave; every other edge is traced after bending with
  `IntersectionCurve::through`.
- Rigid faces (every surface but a spline) are always masters. Between two splines the master is
  the one the edge is slanted on, else the one earlier in an order that puts those first and smaller
  nets before larger; a ring of such constraints refuses the mode.
- The gap between tangent CATIA fillets is not noise: measured along their edges it is zero at one
  face's knots and bulges between them. So a slave side is fitted to the master itself (its line
  through a closest-point map, its surface, or the rigid face), the master's knot lines crossed by
  that map become knots of the slave, and spans are halved until the side is within
  `BEND_TOLERANCE` of its targets (`BSplineSurface::bent_side`).
- A vertex settles onto its rigid faces and the splines it lies on no line of, which must then hold
  it within a quarter of the resolution; every other spline at it is pinned to it. A target's end
  takes up what that settling leaves.
- Faces are bent in master order: a spline is restricted to its fitted lines, so each becomes a
  side, then each side is refitted: strict targets on the master along its slave edges, its own
  curve elsewhere with the vertex moves blended in, pins at vertices, corners an earlier side set
  held. The edge is the slave's side curve between its vertices, exact on the slave and within the
  tolerance on the master. A vertex or side moving farther than the precision refuses the mode; the
  note gives the faces bent and the largest bend.

## Faceted fallback

- A body neither the exact nor the bent build accepts is built again in `Healing::Faceted` when the
  file declares a precision: intersection edges are traced only between elementary faces, and an
  edge whose ends
  miss its vertices keeps the file's curve with the miss blended in (`loose::met_at_ends`, a line
  rebuilt through its vertices). If every vertex and edge then lies within the declared precision of
  its faces, the unvalidated body is meshed (`SolidBuilder::unvalidated_mesh`, `SMOOTH`, then
  coarser while `faceted_solids` finds too many faces) and imported as the one planar solid
  `faceted_solids` makes, with a note per body; otherwise the exact refusal stands. Like a mesh
  import it skips `find_crossing`, since `faceted_solids` already leaves out self-folding shells.
- Faceted bodies lose their curved faces (no fillets on them, no exact measures). Bending covers
  only part of such files yet; the rest is the roadmap item in `docs/TODO.md`.
