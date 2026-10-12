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
  encoding with a tag byte per constructor. They are stored in files: the encoding, the tags and
  the digests pinned in `naming/tests.rs` never change; later generators get new tags.
- A face is named by its feature and what made it. A pattern copy's index is its step along each
  direction (row 0 for circular and curve patterns), so changing a count never renames kept copies;
  a point pattern's is the sketch point's id plus one, so copies keep names as points come and go.
- A tapered extrusion spanning its sketch plane has two side faces per piece: `side` towards the
  extent's end, `side_behind` the other, so adding a taper keeps the end side's names.
- An edge is `between` an unordered face pair. Shared names are split by `between_at` (adds the end
  vertex names, faces oriented by the edge, fixed order when both ends match so reversing a closed
  edge keeps the name), last by `occurrence` (position on a grid coarse enough that rounding cannot
  swap them). `seam` names a full revolution's profile seam.
- A vertex is `VertexName::of_faces` the faces around it.
- `FaceOrigin` says what a face came from. A patterned copy is `Copy`: a `FaceCopy` (pattern
  feature, copy index) with the original's feature and `Made` kind; `original()` gives the origin
  copied, `features()` both features. A copy of a copy keeps only the latest pattern.
- `Solid::imported(feature)` names faces by position in the stored STEP text (never changes) and
  edges `between` their faces, disambiguated like sweeps.
- Split pieces (`split_faces`, `kernel-operations.md`) are `FaceName::split` of the split feature,
  the original name and the tool side (`SplitPiece`), keeping the original's origin; pieces on one
  side share a name and are told apart by neighbours. A face colour finds its pieces by computing
  these names forward from the split features (`FaceSplits`, `document.md`), since a digest cannot
  be reversed.
- A thread groove face is `FaceName::thread` of the feature and its part (`kernel-operations.md`).
  A closing face of `enclose_faces` is `FaceName::closure` of the feature and the sorted, deduped
  names of its edges.
- Boolean fragments of a split face keep its name and origin; pieces keep their edge's name and new
  edges are `between` their two faces, before the plan disambiguates duplicates. A pattern renames
  a copy's faces, then its edges from those faces.

## References (`naming/reference.rs`)

- References keep later features attached to generated topology. Resolution never guesses between
  equals: a tie is `ReferenceError::Ambiguous` with the candidates, no match is `Missing`.
- A `FaceReference` is a face's name, origin and neighbour names. It resolves to the one face of
  that name still sharing a neighbour (or none recorded); among several of that name (split
  fragments), to the best neighbour match; when the name is gone, to the face of the same origin
  sharing a neighbour.
- An `EdgeReference` is an edge's name, its two face names with `FaceOrigin`s (`None` in files from
  before origins were kept, until the document's `complete_origins` fills them on opening) and end
  vertex names. It resolves by name, else among edges between the same faces, told apart by
  matching ends (an edge sharing no end is never taken). With no edge between those faces (a cap
  renamed because a hole was added) it falls back to edges whose faces each keep the name, then the
  origin; `resolve_by_names_in` skips the fallback.
- `EdgeNaming` indexes a solid's edges in one pass; code handling many references builds it once
  per solid and uses `capture_in` and `resolve_in`.
