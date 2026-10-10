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
  by default, so an uncancelled read never sees it. A body's build polls it before each looser
  healing and before trying without its unreadable faces (`healed`, `build_one`), since a
  cancelled build fails like any other and each retry rebuilds and may mesh the whole body again.
- Solids are built in parallel (`Builds::of`, `built_together`): each distinct solid (one per
  shells and units) is a job, shared out over up to the available parallelism of scoped threads
  that each keep their own geometry memo per units and install the caller's interrupt
  (`current_interrupt`), polling it before each job, against the file's one `MAX_WORK` budget
  (`Work`, atomic). Results are taken in file order, so names, notes and refusals are those of a
  sequential read; a solid with the same shells and units as an earlier one takes its build. A
  file of one solid builds on the calling thread. `read_step_copies_reporting` hands a
  `ReadProgress` (solids built of solids to build) to a callback the workers call.
- `read_own_step` reads text caditor's own writer has just made from a solid that was already
  checked (import's canonicalising) without `Solid::find_crossing` (`Crossings::Trusted`), which
  was 92% of reading back a 400-face lead screw (87 s); a model file's import text and every other
  text go through `read_step`.
- Hostile-input bounds: `MAX_SPLINE_DEGREE`, checked knot arithmetic before expansion, one
  `MAX_WORK` budget per file for everything built, `MAX_DEPTH` and `MAX_INSTANCES` for assemblies.
  Curves, surfaces, placements and solids are memoised per entity and units.

## Colours, opacity and layers (`presentation.rs`)

- Each solid gets a `colour`, an `opacity` and a `layer`, or none, and `FaceLook`s for its faces.
  Every styled item counts, whichever representation lists it (a
  `MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION` or none): `STYLED_ITEM`,
  `OVER_RIDING_STYLED_ITEM` (its values win over plain styles of the same item) and
  `CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM`, simple or complex instances. A style on an
  `ORIENTED_FACE` is its face's.
- Styles are read by their structure, never searched: `PRESENTATION_STYLE_ASSIGNMENT` (or
  `PRESENTATION_STYLE_BY_CONTEXT`), `SURFACE_STYLE_USAGE` (`.POSITIVE.` and `.BOTH.` give the
  outside, used first; `.NEGATIVE.` only when nothing else is stated), `SURFACE_SIDE_STYLE`,
  `SURFACE_STYLE_FILL_AREA` with `FILL_AREA_STYLE_COLOUR`, `SURFACE_STYLE_RENDERING` and
  `..._WITH_PROPERTIES` (their colour, `$` allowed, counts after a fill colour; their
  `SURFACE_STYLE_TRANSPARENT` and reflectances), and `SURFACE_STYLE_TRANSPARENT` loose in a side
  style, so a style's every part counts, not the first found. A `COLOUR_RGB` in 0–1 (or 0–255 when
  a channel passes 1) or a `DRAUGHTING_PRE_DEFINED_COLOUR` of the eight named ones gives the colour;
  a transparency in 0–1 gives the opacity, a percent of one minus it. `NULL_STYLE` states the item
  plain. Fusion's exporter names each `COLOUR_RGB` after its appearance (`Opaque(r,g,b)`,
  `Acrylic (Clear)`, `Glass (Smoked)`) and writes no transparency, so a colour whose name holds one
  of `SEE_THROUGH_APPEARANCES` as a word (clear, transparent, glass: 25%; translucent, smoked,
  frosted, tinted: 50%, the larger winning) and none of `OPAQUE_APPEARANCES` (opaque, coat) gives
  its level that opacity when the style states no `SURFACE_STYLE_TRANSPARENT`. Curve, point, text
  and the surface's curve styles are skipped silently.
- What cannot be understood (an unknown colour or style kind, a colour name not among the eight, a
  transparency outside 0–1, a hatched or tiled fill, a style missing from the file, a style for
  one copy matching none) is named by entity in one note, at most `MOST_NAMED_PROBLEMS` listed,
  only when it styles something imported (a solid, shell, face or a link placing one); what it
  styles keeps the look it has without it.
- Inheritance: a face takes the first stated value from itself, its shell, its solid, then the
  links that place this copy, innermost first. A level stating a colour states its opacity too
  (opaque without a transparency), so a coloured face on a see-through body is opaque; a level
  stating only a transparency leaves the colour to the levels above.
- Assembly instances: each placement carries its path (`Placements::paths`: its representations,
  the relationship or `MAPPED_ITEM` of every link and the `NEXT_ASSEMBLY_USAGE_OCCURRENCE` of each),
  so a plain style on a `MAPPED_ITEM` or relationship styles the copies it places, and a
  `CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM` (or a `PRESENTATION_STYLE_BY_CONTEXT`) applies only
  to copies whose path holds every element of its context, the longest context winning. Looks are
  worked out per placement, shared between copies whose styled path is the same
  (`placement_key`).
- The body's look is the first stated by its solid, its shells or the links above; when none
  states anything, the colour and the opacity most of its faces share (ties to the first face's),
  so a part whose faces are all styled (SolidWorks, NX) takes its main colour. Each face whose
  colour or opacity differs from the body's is a `FaceLook` by its index among the kernel faces
  (`Built::faces` maps them to file faces): its colour when it differs (a face with none under a
  coloured body cannot say so and takes the body's), its opacity (100 is explicitly solid) when it
  differs.
- A faceted body (`Healing::Faceted`) carries its faces' looks too: each flat face takes the file
  face covering most of its area among the triangles it was made of (`FacetedSolids::sources`,
  `SolidBuilder::unvalidated_mesh`'s face per triangle); a face only of closing fans has none.
- A `PRESENTATION_LAYER_ASSIGNMENT` puts its items on its layer, a styled item counting as the
  item it styles; the first layer naming an item keeps it. A solid takes the layer on itself, else
  on one of its shells, else the one every face shares.

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
- `OFFSET_SURFACE` (`offset.rs`) of a plane, cylinder, sphere, torus or cone is the exact surface
  of the same kind; one leaving no surface is refused in words. Of an extrusion or revolution it is
  the same sweep of the offset profile: a line or a circle whose offset stays concentric exactly,
  any other profile fitted; of a spline surface, a fitted bicubic spline. A fit samples the exact
  offset at the basis's own parameters, each basis span (at most `MAX_SURFACE_SAMPLES` or
  `MAX_CURVE_SAMPLES` a direction, neighbouring spans grouped past that) cut into pieces
  interpolated cubically and joined C0 at the span ends, since an offset is only C1 at the basis's
  knots; a point at a pole takes a nudged normal and a collapsed row or column of the basis stays
  one point. Pieces are halved in the direction whose midpoints miss until every midpoint is within
  `TIGHT_FIT`; the fit is accepted within the file's precision (`LINEAR_RESOLUTION` when none is
  declared) and otherwise refused saying how close it came, as is an offset whose normal turns
  against the basis's (`Folds`) anywhere in the basis's domain. Fitting charges the work budget
  and polls the interrupt each round. A reversed basis (below) offsets the other way.
- `POINT_REPLICA`, `CURVE_REPLICA` and `SURFACE_REPLICA` are their parent mapped by their
  `CARTESIAN_TRANSFORMATION_OPERATOR_3D` (read as for assemblies, `structure::operator`, its origin a
  plain point); a mirrored surface replica whose kernel normal runs against STEP's (`similar`'s
  `Sense`) is recorded in `Geometry::reversed`, which offsets and trimmed surfaces inherit and
  `plan_face` folds into the face's sense. A line replica's trims by value scale with it.
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

## Unreadable faces

- A body every build refuses is tried once more without the faces whose surface cannot be read
  (`unreadable_faces`; edges and other entities still refuse the body). A void or extra lump
  holding one is left out and the rest built as usual, exact. Faces lost from the outer shell are
  left out of a `Healing::Faceted` build whose mesh is closed by `closed_over`: the open loops
  after welding are fanned from their centres (a lost face with holes does not close and the
  original refusal stands). `Built::lost` words the note: which faces were lost, why, and that the
  hole was closed with flat facets or which hollows or pieces were left out.

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
