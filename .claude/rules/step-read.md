---
paths:
  - "crates/caditor-step/src/part21.rs"
  - "crates/caditor-step/src/read/**"
---

# STEP reader

## Part 21 parser (`part21.rs`)

- Reads the header, data sections and edition 3 `ANCHOR`, `REFERENCE` and `SIGNATURE` sections
  (skipped); nesting is limited by `MAX_NESTING`. Text past 4 GiB is `ReadError::TooLarge`, so every
  offset fits a `u32`.
- The tree is flat arenas in `Exchange`, never a box per record or list: a parameter value is a
  one-byte kind plus a `u64` payload (number bits, a reference, or a packed offset and length into
  the text, the values or the uppercased names); lists and records name their run of values by span.
  Text and names stay offsets into the file, and an unreadable entry truncates every arena back to
  where it began. `Parameter`, `List`, `Record` and `Instance` are copyable views decoding on
  access. Parsed size is about 1.2 times the file; instances are found by binary search.
- Damage is survived and reported as notes, never refused: a damaged header is skipped, a missing
  `END-ISO-10303-21;` accepted, an unreadable entry skipped and counted, a repeated entity id keeps
  its first definition. A file ending inside `DATA` (an interrupted download) keeps every entity read
  and is `cut_short`, noted as such; whether its solids are complete is left to `NoSolids` and
  `NotRebuilt`.

## `read_step`

- Returns every `MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS`, `FACETED_BREP` and closed
  `SHELL_BASED_SURFACE_MODEL` as named kernel solids plus notes, or a `ReadError` in words; surface
  bodies are left out with a note. No solids is `ReadError::NoSolids(Held)`, naming the schema and
  any open surface bodies, tessellated shapes or wireframes. A failing body does not stop the others;
  when none is left the first failure is `ReadError::NotRebuilt { name, entity, fault }`, else the
  first placement that left a body out is `ReadError::NotPlaced { name, misplacement }`
  (`Misplacement` also words the notes).
- Every solid goes through `SolidBuilder::build`: valid, or a sentence naming the entity. A reader
  failure is a `Problem` (entity id and `Fault`, `read/fault.rs`): `Fault` is an enum of one variant
  per failure, carrying typed data (a kernel error as its source, an offset's `OffsetRefusal`, the
  STEP kind's name) and worded by its `Display` to continue "its entity #N …"; no reader function
  builds a message string. `Fault::Build` carries the `BuildError` and the file's precision, and
  `describe_build` words every `BuildError` and `ValidationError` in an exhaustive `match` (no ids or
  topology terms; raw text goes to the debug log), so a new kernel variant fails to compile until
  worded; `no_note_about_a_body_that_cannot_be_rebuilt_shows_kernel_internals` pins it. A solid whose
  faces cross (`Solid::find_crossing`) is refused naming the face entities; an inconclusive check
  imports it with a note that features built on it may fail.
- `STEP_CORPUS=<dir>` (narrowed by `STEP_CORPUS_ONLY=<part of a name>`) makes
  `every_file_of_a_corpus_is_read_and_reported` read every file there through `read_step_copies`
  (as import does, so no part is lost to the separate-copies cap) and print copies, volumes, notes
  or refusal and time; run it in release against files whose volumes another reader gives.
- Cancellation: `read_step` and `read_step_copies` poll the kernel interrupt (`kernel.md`) before
  parsing and before each solid, answering `ReadError::Cancelled`; nothing installs one by default. A
  body's build also polls it before each looser healing and before trying without its unreadable
  faces (`healed`, `build_one`), since each retry rebuilds and may mesh the whole body again.
- Solids are built in parallel (`Builds::of`, `built_together`): each distinct solid (one per shells
  and units) is a job over scoped threads, each with its own geometry memo per units and the caller's
  interrupt (`current_interrupt`), against the file's one `MAX_WORK` budget (`Work`, atomic). Results
  are taken in file order, so names, notes and refusals match a sequential read. One solid builds on
  the calling thread. `read_step_copies_reporting` hands a `ReadProgress` to a callback the workers
  call.
- `read_own_step` reads text caditor's own writer just made from an already checked solid without
  `Solid::find_crossing` (`Crossings::Trusted`), which dominated reading back a 400-face lead screw;
  a model file's import text and all other text go through `read_step`.
- Hostile-input bounds: `MAX_SPLINE_DEGREE`, checked knot arithmetic before expansion, one
  `MAX_WORK` budget per file, `MAX_DEPTH` and `MAX_INSTANCES` (placements walked for one part), and
  `MAX_WALKED_PLACEMENTS` for placements kept across the file (`Structure::walk` memoises each
  representation's `Placements` behind an `Arc`): once spent each further walk keeps one placement
  per parent, so every part is still placed and memory stays bounded, with a note that only the first
  copies were imported. Representations a plain relationship makes identical join one class as the
  relationships are read (`Classes`, smaller into larger, never searched per lookup), and a face
  loop's fins are peeled from a deque, so a crafted file costs time linear in its size. Names are shared `Arc<str>`, a styled item's context list one `Arc<[u64]>`
  (`Instanced::by_context` holds the one id a style adds), repeated style and context references are
  read once (`once_each`). Copies are capped per file in solid order, with the same note:
  `read_step_copies` at `MAX_SHARED_COPIES` (its copies share one solid each), `read_step` at
  `MAX_SEPARATE_COPIES` (each copy is a solid of its own). A printer assembly (the VZ330, 3,894
  copies, its bed among the last parts) must not lose whole parts to the cap. Curves, surfaces,
  placements and solids are memoised per entity and units.

## Colours, opacity and layers (`presentation.rs`)

- Each solid gets a `colour`, `opacity` and `layer`, or none, and `FaceLook`s for its faces. Every
  styled item counts, whichever representation lists it: `STYLED_ITEM`, `OVER_RIDING_STYLED_ITEM`
  (wins over plain styles of the same item) and `CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM`, simple
  or complex instances. A style on an `ORIENTED_FACE` is its face's.
- Styles are read by structure, never searched, and every part counts, not the first found:
  `PRESENTATION_STYLE_ASSIGNMENT` (or `..._BY_CONTEXT`), `SURFACE_STYLE_USAGE` (`.POSITIVE.` and
  `.BOTH.` give the outside, used first; `.NEGATIVE.` only when nothing else is stated),
  `SURFACE_SIDE_STYLE`, `SURFACE_STYLE_FILL_AREA` with `FILL_AREA_STYLE_COLOUR`,
  `SURFACE_STYLE_RENDERING` and `..._WITH_PROPERTIES` (colour, `$` allowed, counts after a fill
  colour), and `SURFACE_STYLE_TRANSPARENT`. A `COLOUR_RGB` in 0-1 (0-255 when a channel passes 1) or
  one of the eight named `DRAUGHTING_PRE_DEFINED_COLOUR`s gives the colour; a transparency in 0-1
  gives the opacity as one minus it. `NULL_STYLE` states the item plain. Curve, point and text styles
  are skipped silently.
- Fusion writes no transparency but names each `COLOUR_RGB` after its appearance (`Opaque(r,g,b)`,
  `Acrylic (Clear)`, `Glass (Smoked)`): a name holding a word of `SEE_THROUGH_APPEARANCES` (25% or
  50%, the larger winning) and none of `OPAQUE_APPEARANCES` gives that opacity when the style states
  no `SURFACE_STYLE_TRANSPARENT`. The same name gives the colour its `Finish` (`finish.rs`,
  `appearance_finish`, whole words in any case, first match wins in the order the code lists them);
  `Opaque(r,g,b)` and other names give none. A finish travels with its colour: a level stating a
  colour states that colour's finish.
- What cannot be understood (unknown colour or style kind, colour name not among the eight,
  transparency outside 0-1, hatched or tiled fill, missing style, a style for one copy matching none)
  is named by entity in one note, at most `MOST_NAMED_PROBLEMS` listed, only when it styles something
  imported; what it styles keeps the look it has without it.
- Inheritance: a face takes the first stated value from itself, its shell, its solid, then the links
  placing this copy, innermost first. A level stating a colour states its opacity too (opaque without
  a transparency), so a coloured face on a see-through body is opaque; a level stating only a
  transparency leaves the colour to the levels above.
- Assembly instances: each placement carries its path (`Placements::paths`: its representations, the
  relationship or `MAPPED_ITEM` of every link and each `NEXT_ASSEMBLY_USAGE_OCCURRENCE`), so a plain
  style on a `MAPPED_ITEM` or relationship styles the copies it places, and a
  `CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM` (or `PRESENTATION_STYLE_BY_CONTEXT`) applies only to
  copies whose path holds every element of its context, the longest context winning. Looks are
  worked out per placement, shared between copies with the same styled path (`placement_key`).
- The body's look is the first stated by its solid, shells or the links above; when none states
  anything, the colour and opacity most faces share (ties to the first face's) and the finish of the
  first face of that colour, so a part whose faces are all styled (SolidWorks, NX) takes its main
  colour. Each face differing from the body in colour, opacity or finish is a `FaceLook` by index
  among the kernel faces (`Built::faces` maps them to file faces); a face with no colour under a
  coloured body takes the body's.
- A faceted body (`Healing::Faceted`) carries its faces' looks too: each flat face takes the file
  face covering most of its area among its triangles (`FacetedSolids::sources`); a face only of
  closing fans has none.
- A `PRESENTATION_LAYER_ASSIGNMENT` puts its items on its layer, a styled item counting as the item
  it styles; the first layer naming an item keeps it. A solid takes the layer on itself, else on one
  of its shells, else the one every face shares.

## Units and precision

- Units come from each representation's context (SI prefixes, conversion-based units); a note names
  every length unit converted, and a file naming none is read as millimetres, with a note.
- Declared precision is the coarsest length uncertainty of the context, clamped between
  `LINEAR_RESOLUTION` and `COARSEST_PRECISION` so a hostile file sets neither zero nor a huge
  tolerance.
- Solids are still validated at `LINEAR_RESOLUTION`, since the kernel cannot hold looser geometry:
  precision never loosens validity. It decides what the file means (composite segments joining, a
  polygon having a plane, a torus being a horn), which healing earns a note (farther than it from the
  faces; nearer heals silently) and how a refusal reads.

## Assemblies

- Followed from each solid's representation to the roots through transformation relationships and
  `MAPPED_ITEM`s: one solid per placement. A product's only body takes the product's name; copies
  take their occurrences' names when each is distinct, else a number.
- Each solid is built once and every placement handed to a placer (`read_placed`): `read_step` maps
  each copy into a solid of its own, `read_step_copies` returns `StepCopy`s sharing the built solid
  (`Arc`) with their `Similarity`, which import uses to store a part once.
- The child representation comes from `NEXT_ASSEMBLY_USAGE_OCCURRENCE` and
  `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION`, else is guessed from which side is some assembly's child;
  `rep_1` is carried into `rep_2`, so the transform is inverted when the parent is listed first.
- A placement is a `Similarity`: a `CARTESIAN_TRANSFORMATION_OPERATOR_3D` may scale (a positive
  factor within the kernel's range) and mirror, applied with `Solid::mapped` so the copy keeps its
  names and validates. A part or copy with an unreadable placement, a cycle or too deep a nesting is
  left out with a note while the others import; one scaled past what the kernel models gets its own
  note (`Misplacement::CopyUnplaceable`).

## Geometry and healing

- Every kernel surface and curve, B-splines in all forms (unclamped ones clamped by knot insertion),
  trimmed and composite curves, `HYPERBOLA` as rational quadratic arcs of at most
  `MAX_HYPERBOLA_SPAN` and `PARABOLA` as one quadratic Bezier, both reaching `MODEL_EXTENT` from
  their centre, `OFFSET_CURVE_3D` as dense polylines that healing rebuilds on the faces.
- `DEGENERATE_TOROIDAL_SURFACE`: a spindle is the revolution of the tube's rational arc on one side
  of the axis; a horn turns the whole tube circle about its touch point, so its two poles are one
  vertex. The inside of a horn is refused (no volume).
- `OFFSET_SURFACE` (`offset.rs`) of a plane, cylinder, sphere, torus or cone is the exact surface of
  the same kind; one leaving no surface is refused in words. Of an extrusion or revolution it is the
  same sweep of the offset profile (a line or circle exactly, any other fitted); of a spline surface,
  a fitted bicubic spline. A fit samples the exact offset at the basis's own parameters, each basis
  span (at most `MAX_SURFACE_SAMPLES` or `MAX_CURVE_SAMPLES` a direction) cut into pieces
  interpolated cubically and joined C0 at span ends, since an offset is only C1 at the basis's knots;
  pieces are halved where midpoints miss until within `TIGHT_FIT`. The fit is accepted within the
  file's precision and otherwise refused saying how close it came, as is an offset whose normal turns
  against the basis's (`Folds`). Fitting charges the work budget and polls the interrupt. A reversed
  basis (below) offsets the other way.
- `POINT_REPLICA`, `CURVE_REPLICA` and `SURFACE_REPLICA` are their parent mapped by their
  `CARTESIAN_TRANSFORMATION_OPERATOR_3D` (`structure::operator`); a mirrored surface replica whose
  kernel normal runs against STEP's (`similar`'s `Sense`) is recorded in `Geometry::reversed`, which
  offsets and trimmed surfaces inherit and `plan_face` folds into the face's sense.
- Vertices off their faces move onto all of them by damped Levenberg-Marquardt least squares; edges
  farther than a quarter of the resolution from either face are rebuilt with
  `IntersectionCurve::through`. The exact and bent builds follow a vertex however far its faces'
  common point lies (`settle_on` with an unbounded reach): a Fusion thread's run-out corner, where
  the flank grazes the bore, lies 0.035 mm from where it is written and the body is exact there.
- A body the exact build refuses is built again in `Healing::Extended` before any looser mode: each
  spline face's surface is continued past its domain by a tenth of its boundary span
  (`BSplineSurface::extended`), except across a seam, a pole or sides within the file's precision of
  each other, since exporters (Fusion 360 threads) trim a spline exactly where its neighbours meet it
  and their common corner lies a hair beyond. Only a retry, never the first build, as continuing a
  spline that winds over itself can mislead projection.
- A void that faces outward once its `ORIENTED_CLOSED_SHELL`'s orientation is applied (Autodesk
  writes the shell already facing its cavity and still marks it `.F.`) is turned round and the body
  built again (`Topology::outward_voids`, from `SolidBuilder::inward_shells`).
- A fin, an edge a loop runs out along and straight back (Autodesk exports leave them on cylinders),
  bounds nothing and is left out with its tip vertex, unless one end is a pole of the face, where
  such a fold is the seam reaching an apex.
- The outer loop is the `FACE_OUTER_BOUND`, else the one using a seam, else the largest by area;
  faces bounded only by `VERTEX_LOOP`s get a pole-to-pole seam. `POLY_LOOP` faces get line edges
  shared by corner position and a plane from the polygon when none is named.

## Snapped faces (`snap.rs`)

- A body the exact and extended builds refuse is built in `Healing::Snapped` when the file declares
  a precision, and refused at once in that mode when nothing snaps; `Healing::Bent` snaps first
  too, so a body needing both is bent around its snapped faces.
- Round faces move first (`round_faces`). Two cylinders, cones, spheres or tori meeting along an
  edge, with normals within `PARALLEL_ANGLE` at its middle sample, are a pair when the feet of the
  edge's samples on one lie farther than a quarter of the resolution off the other. A pair already
  meeting exactly holds both its faces; pairs are then taken in face order, a moved face counting
  as moved, and a face moved or moved onto is not moved again. Either face of a pair may move onto
  the other: the candidates are the face offset along its normal by the gap at the middle foot (a
  new radius), translated by that gap, turned and slid onto the other's axis or centre (a sphere's),
  and that alignment offset; a candidate counts when every foot lies on it within a quarter of the
  resolution, and the one the face's own edges as written lie closest to wins, then the smaller
  move. Of the two faces, the one whose winner leaves its written edges closer (by more than a
  quarter of the resolution) moves, else the one meeting fewer neighbours tangentially, then fewer
  neighbours, then the later entity: a fillet whose axis is off by a hair goes back onto the
  shaft's rather than the shaft onto it, keeping every edge as written.
- Planes move next, against the round faces as snapped. A plane meeting a cylinder, cone or torus
  along an edge, with normals within `PARALLEL_ANGLE`, becomes that face's tangent plane at the
  foot of the edge's middle sample, when every sample's foot on the round face lies on that plane
  within a quarter of the resolution and some lies farther off the plane as written (a plane
  meeting it across along a ruling is left alone). Planes meeting along an edge or at a corner,
  parallel within `PARALLEL_ANGLE` and apart there by more than a quarter of the resolution, join a
  group whose planes all become one: a plane already snapped to a round face, else the one the
  shared edges and corners lie closest to; each other keeps its normal's sense and its own x axis.
- A plane's move is the largest distance from the plane as written to the new one over the corners
  of the box its edge samples span in the plane (an overestimate for a turn, exact for a shift); a
  round face's is the largest distance from its feet of its edge samples (thinned evenly to
  `MOST_OUTLINE_SAMPLES`, so a pair costs a bounded amount) to the new surface. One past the
  precision or under a quarter of the resolution is not made. Faces at a corner or edge shared by
  more than `MOST_FACES_AT_A_CORNER` faces are not paired there, so a crafted file stays linear.
- An edge of one face only (a seam) left off its moved face is rebuilt as that face's parameter
  line through its vertices (`seam_through`: a line on a cylinder or cone, a meridian on a sphere,
  a tube or ring circle on a torus), accepted when both ends lie on it within the resolution.
- The note gives the faces moved (flat, curved, or just faces when both) and the largest move
  (`a_flat_face_just_off_the_cylinder_it_is_tangent_to_is_moved_onto_it_and_imports_exact`,
  `halves_of_a_split_face_apart_within_the_precision_are_moved_onto_one_plane_and_import_exact`,
  `a_cylinder_just_off_the_cylinder_it_is_tangent_to_is_moved_onto_it_and_imports_exact`,
  `a_fillet_whose_axis_is_a_hair_off_the_shaft_s_is_moved_onto_it_and_imports_exact`,
  `a_shaft_written_a_hair_off_its_fillet_is_moved_onto_it_and_its_seam_follows`).

## Bent faces (`conform.rs`)

- A body the snapped build refuses is built in `Healing::Bent` before `Healing::Faceted`
  when the file declares a precision. Each edge between two faces is studied from samples of its file
  curve: one whose faces' normals stay within `TANGENT_ANGLE` (measured off poles) and that runs
  along an iso-line of a spline face (within the precision; one at the domain bound is a side) is
  bent, with one face its master and a spline it runs along its slave. An edge meeting a rigid face
  across is bent too when it runs along a side of a spline, the rigid face its master: a side ending
  within the precision short of the plane it meets leaves nothing to trace
  (`a_spline_side_bowing_short_of_the_plane_it_meets_across_is_bent_onto_it`). Every other edge is
  traced after bending with `IntersectionCurve::through`.
- Rigid faces (every surface but a spline) are always masters. Between two splines the master is the
  one the edge is slanted on, else the one earlier in an order putting those first and smaller nets
  before larger; a ring of such constraints refuses the mode.
- The gap between tangent CATIA fillets is not noise: along their edges it is zero at one face's
  knots and bulges between them. So a slave side is fitted to the master itself (its line through a
  closest-point map, its surface, or the rigid face), the master's knot lines crossed by that map
  become knots of the slave, and spans are halved until within `BEND_TOLERANCE`
  (`BSplineSurface::bent_side`).
- A vertex settles onto its rigid faces and the splines it lies on no line of, which must then hold
  it within a quarter of the resolution; every other spline at it is pinned to it.
- Faces are bent in master order: a spline is restricted to its fitted lines, so each becomes a side,
  then each side is refitted (strict targets on the master along its slave edges, its own curve
  elsewhere with vertex moves blended in, pins at vertices). The edge is the slave's side curve
  between its vertices. A vertex or side moving farther than the precision refuses the mode; the note
  gives the faces bent and the largest bend.

## Unreadable faces

- A body every build refuses is tried once more without the faces whose surface cannot be read
  (`unreadable_faces`; edges and other entities still refuse the body). A void or extra lump holding
  one is left out and the rest built exact. Faces lost from the outer shell are left out of a
  `Healing::Faceted` build whose mesh is closed by `closed_over`: open loops after welding are fanned
  from their centres (a lost face with holes does not close and the original refusal stands).
  `Built::lost` words the note: which faces, why, and how the hole was closed or what was left out.

## Faceted fallback

- A body neither the exact, snapped nor bent build accepts is built in `Healing::Faceted` when the file
  declares a precision: intersection edges are traced only between elementary faces, and an edge
  whose ends miss its vertices keeps the file's curve with the miss blended in (`loose::met_at_ends`).
  A vertex settles no farther than its worst gap (nor the precision): where faces meet nearly
  parallel or tangent their common point lies far along them, and following it would leave every edge
  there missing the vertex, while the file's corner already lies within the precision of all faces.
  A vertex whose spline edges (those touching a face not of an elementary kind, which are never
  traced) end farther than the precision from it moves to the centre of the box of their ends when
  that brings every end within the precision: Autodesk merges two corners of a pinched corner patch
  0.017 mm apart into one vertex written at one of them. Traced edges are left out of it, since
  tracing rebuilds them from their faces whatever the file's curve ends at.
- If every vertex and edge then lies within `FACETED_REACH` (4) times the declared precision of its
  faces (the precision alone when the file declares the kernel's resolution, which claims exact
  geometry), the unvalidated body is meshed (`SolidBuilder::unvalidated_mesh`, `SMOOTH`, then coarser while `faceted_solids` finds too
  many faces) and imported as the one planar solid `faceted_solids` makes, with a note per body;
  otherwise the exact refusal stands. `Built::faceted` says which (`Faceting`): within the precision
  the note says the faces meet only within it; beyond it, the note gives how far apart they lie and
  that the shape may be off by as much, since a part left out loses more than one a few hundredths
  of a millimetre off. Like a mesh import it skips `find_crossing`, since
  `faceted_solids` already leaves out self-folding shells.
- Faceted bodies lose their curved faces (no fillets on them, no exact measures). Snapping and
  bending cover only part of such files yet; the rest is the STEP import entries in
  `docs/BUGS.md`.
