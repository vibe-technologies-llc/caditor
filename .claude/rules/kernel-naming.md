---
paths:
  - "crates/caditor-kernel/src/naming/**"
  - "crates/caditor-kernel/src/build/**"
  - "crates/caditor-kernel/src/boolean/**"
  - "crates/caditor-kernel/src/blend/**"
  - "crates/caditor-kernel/src/shell/**"
  - "crates/caditor-kernel/src/pattern/**"
  - "crates/caditor-document/**"
  - "crates/caditor-step/src/read/**"
---

# Naming and references

## Names (`naming/`)

- `FaceName`, `EdgeName` and `VertexName` are 128-bit FNV-1a digests over a canonical little-endian
  encoding with a tag byte per constructor.
- Stored in files: the encoding, the tags and the pinned digests in `naming/tests.rs` never change.
  Later generators are new constructors with new tags.
- A face is named by its feature and what made it. A pattern copy's index is its step along each
  direction (row 0 for a circular pattern), so changing a count never renames the copies it keeps.
- A tapered extrusion spanning its sketch plane has two side faces per piece: the one towards the
  extent's end is `side`, the other `side_behind`, so adding a taper keeps the end side's names.
- An edge is `between` an unordered face pair. When several edges share that name, `between_at`
  adds the end vertex names (faces oriented by the edge, in a fixed order when both ends have the
  same name, so reversing a closed edge keeps the name); the last resort is `occurrence`, ordered
  by position on a grid coarse enough that rounding noise cannot swap them. `seam` names the
  profile seam of a full revolution.
- A vertex is `VertexName::of_faces` the set of faces around it, so it survives whatever keeps its
  faces' names.
- `FaceOrigin` says in words what a face came from. A patterned copy's face is `Copy`: the
  `FaceCopy` (pattern feature, copy index) with the original's feature and `Made` kind, so
  `original()` gives the origin it copies and `features()` both features that made it. A copy of a
  copy keeps only the latest pattern.
- `Solid::imported(feature)` names faces by position in the solid (its order in the stored STEP
  text, which never changes) and edges `between` their faces, disambiguated like sweeps.
- Boolean fragments of a split face keep its name and origin; pieces keep their edge's name and new
  edges are `between` their two faces, before the plan disambiguates duplicates. A pattern renames
  a copy's faces, then its edges from those faces.

## References (`naming/reference.rs`)

- References let later features keep hold of generated topology. Resolution never guesses between
  equals: a tie is `ReferenceError::Ambiguous` with the candidates, no match is `Missing`.
- A `FaceReference` is a face's name, origin and the set of its neighbours' names. It resolves to
  the one face with that name when it still shares a neighbour (or none were recorded); among
  several faces of that name (fragments of a split face), to the one whose neighbours match best;
  when the name is gone (its region key or piece id changed), to the face of the same origin
  sharing at least one neighbour.
- An `EdgeReference` is an edge's name, its two face names with their `FaceOrigin`s (`None` in
  references read from files saved before origins were kept, until the document's
  `complete_origins` fills them on opening) and its end vertex names. It resolves by name, else
  among the edges between the same faces, several candidates told apart by matching ends (an edge
  sharing no end is never taken). When no edge lies between the same faces (a cap renamed because a
  hole was added inside its outline), it falls back to edges whose faces each keep the name or,
  failing that, the origin; `resolve_by_names_in` skips that fallback.
- `EdgeNaming` indexes a solid's edges in one pass; code handling many references (a blend's edges)
  builds it once per solid and uses `capture_in` and `resolve_in`.
