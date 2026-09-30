---
paths:
  - "crates/caditor-kernel/src/naming/**"
  - "crates/caditor-kernel/src/build/**"
  - "crates/caditor-kernel/src/boolean/**"
  - "crates/caditor-kernel/src/blend/**"
  - "crates/caditor-kernel/src/shell/**"
  - "crates/caditor-document/**"
  - "crates/caditor-step/src/read/**"
---

# Naming and references

## Names (`naming/`)

- `FaceName`, `EdgeName` and `VertexName` are 128-bit FNV-1a digests over a canonical little-endian
  encoding with a tag byte per constructor.
- Stored in files: the encoding and the pinned digests in `naming/tests.rs` never change. Later
  generators are new constructors with new tags.
- Face constructors:
  - `side(feature, PieceId)`
  - `start_cap` and `end_cap(feature, RegionKey)`
  - `blend(feature, edge)`, `corner(feature, vertex)`, `shell(feature, original)`
  - `imported(feature, index)`
- Edge constructors:
  - `between` (unordered face pair)
  - `seam(face)` for the profile seam of a full revolution
  - when several edges share a name, `between_at(left, right, from, to)` with the vertex names (sets
    of faces around each end) and the faces oriented by the edge (in a fixed order when both ends
    have the same name, as on a closed edge, so reversing keeps the name)
  - as a last resort `occurrence`, ordered by position (midpoints on a grid of a hundred
    resolutions, so rounding noise cannot swap them)
- Vertices are named `VertexName::of_faces` by the set of faces around them; `vertex_names(solid)`
  gives every vertex's name (`EdgeNaming` and the document's `NameIndex` use it). Faces keep their
  names through parameter changes, so a vertex does too.
- `FaceOrigin` says in words what a face came from: side of an entity, start or end cap (with the
  raw feature and entity ids), `Fillet`, `Chamfer`, `Shell`, `Imported`.
- `Solid::imported(feature)` names an imported solid: `FaceName::imported(feature, index)` by the
  face's position in the solid (its order in the stored STEP text, which never changes),
  `FaceOrigin::Imported`, and edges `between` their faces, disambiguated like sweeps.
- Boolean fragments of a split face keep its name and origin; pieces keep their edge's name and new
  edges are `between` their two faces, before the plan disambiguates duplicates.

## References (`naming/reference.rs`)

- References let later features keep hold of generated topology.
- A `FaceReference` is a face's name, origin and the set of its neighbours' names. It resolves:
  - to the one face with that name when it still shares a neighbour (or none were recorded);
  - among fragments of a split face, to the one whose neighbours match best (most shared, then
    fewest differences);
  - when the name is gone (its region key or piece id changed), to the face of the same origin
    sharing at least one neighbour.
- An `EdgeReference` is an edge's name, its two face names and its end vertex names. It resolves by
  name, else among the edges between the same faces by matching ends; an edge sharing no end with
  the reference is never taken for it.
- A tie is `ReferenceError::Ambiguous` with the candidates; no match is `Missing`. Resolution never
  guesses between equals.
- `EdgeNaming` indexes a solid's edges by name, face pair and end vertex names in one pass;
  `capture_in` and `resolve_in` use it, so code handling many references (a blend's edges) builds it
  once per solid.
