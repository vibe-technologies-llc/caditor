---
paths:
  - "crates/caditor-document/**"
---

# Document

`caditor-document` is the parametric model: parameters, the ordered feature tree, and everything
that changes or recomputes it. Recompute is in `document-recompute.md`.

## Transactions and undo

- Every mutation is a `Transaction` of `Edit`s passed to `Document::apply`, the only public mutator
  of content (`reserve_ids_below` only raises the ID counters, for loading). `apply` is atomic and
  returns the inverse transaction.
- `Editor` keeps undo and redo as stacks of inverses, each at most `MAX_UNDO_STEPS` (500) and
  `MAX_UNDO_BYTES` (256 MiB, by `Transaction::approximate_size`, which counts what an inverse keeps
  alive: removed features, an import's STEP text, `Solid::approximate_size`); oldest dropped first,
  newest always kept.
- Edits carry their IDs, so redo restores them; ID counters never move backwards. Parameter and
  feature IDs stay below 2^63 (`FIRST_UNSTORABLE_ID`, as in sketches): edits refuse larger ones as
  `ReservedId`, `reserve_ids_below` clamps to it.
- Edits refuse to break invariants: unknown references, parameter cycles, deleting something in
  use, moving a feature past one it depends on, two features sharing a name (names trimmed on
  every insert and rename; loading renames the second, with a report).
- `same_content` compares documents without ID counters; it decides whether a model is unsaved.
- `Document::check` runs a transaction on a clone so the UI can report the error before
  committing; `can_remove_parameter` and `can_remove_feature` answer the common case without one.
- Parameter dependencies are built once per `apply` as a `DependencyGraph` that also knows each
  parameter's users. A cycle check searches back from the edited parameter (nothing while no one
  uses it) and evaluation orders parameters topologically first, so both stay near linear.
- `Document::transaction_to` builds the transaction turning one document into another (every
  feature and parameter removed, then the target's inserted with their IDs), keeping every
  sketch's ID counter at least where it is so IDs are never reused; it restores an earlier version
  as one undoable change.
- Large changes can be applied off the UI thread: `Editor::base` hands out the document with the
  editor's revision, `Base::prepare` applies a transaction to that copy on any thread, and
  `Editor::commit` swaps it in and pushes its inverse only if the revision is unchanged, else
  refuses `Stale`.

## Hidden flags

- A feature's `hidden` flag is changed by `Edit::SetFeatureHidden`: undoable, saved as a `hidden`
  field written only when set, journaled like any edit; recompute ignores it.
- Which principal planes, axes and origin (`PrincipalGeometry`, not features) are hidden is
  document content too (`Edit::SetPrincipalHidden`), compared by `same_content` and carried by
  `transaction_to`; not a view preference, so hiding a mixed selection, showing everything and
  undoing either are one change, and a model reopens as it was left.

## Sketch edits

- Sketch content changes only through sketch edits: add, remove or set an entity, switch a curve
  to or from construction geometry, add or remove a constraint, set a dimension.
- `AddSketchEntity` carries whether the entity is construction geometry, so undoing a removal
  restores it as one.
- Removing an entity something still uses is refused, not cascaded;
  `TransactionBuilder::remove_sketch_items` expands a user's deletion into constraints, then
  curves, then points.
- Setting an entity changes only its value, never its kind or the points it uses.
- `settle_sketch` moves the definition to a solved shape so the next solve starts from what the
  user sees.

## Feature kinds

- `SetFeatureKind` replaces settings but never the kind (an import stays an import, a plane a
  plane, an axis an axis, a solid feature its kind; a pattern may switch between linear and
  circular).
- `FeatureKind::bodies_used`, `planes_used` and `axes_used` extend `features()`, so dependents,
  moves and deletions account for them.

### Import (`import.rs`, `FeatureKind::Import`)

- Makes a body from an imported solid; keeps the source file's name, the solid and the canonical
  single-solid STEP text it was read from (what the file stores). Equality compares the text.
- The body is the solid named by `Solid::imported`, so later features hold its faces and edges like
  any body's. One whose shape cannot be read back fails with a sentence.

### Solid (`solid.rs`, `FeatureKind::Solid`)

- An `Extrude` or `Revolve` of a sketch's regions (`RegionChoice::All` for even depth, or chosen
  `RegionKey`s) with a `BodyOperation`: `NewBody`, or `Add`, `Remove`, `Intersect` on the body of
  the feature that made it. A body is named by that feature's ID.
- Extents are expressions (lengths, or angles in degrees) above zero, both distances of a
  two-sided extrusion included; one-sided extents flip with `reversed`.
- A revolve's axis (`RevolveAxis`) is a line of its sketch, a sketch axis, or an `AxisReference` to
  a model axis lying in the sketch plane. A sketch line used as axis cannot be deleted; edits
  refuse an axis not a line or axis of the revolve's own sketch (`AxisNotALine`).
- Inserting one checks its sketch is a sketch and its target makes a body. A feature whose body
  others change keeps making a new body.
- Profile, sweep and boolean errors become sentences naming the sketch curves involved.

### Blend (`blend.rs`, `FeatureKind::Blend`)

- Keeps a `BlendKind` (fillet or chamfer, switchable through `SetFeatureKind`), the body, the
  chosen `EdgeReference`s and a size expression.
- Recompute resolves references in the body's state before the feature, each to a `Resolution`
  (`pieces.rs`; the panel lists them in the same terms): one edge; the pieces of a split edge lying
  on one line or circle; tied between separate edges; missing. A lost or tied reference fails the
  feature.
- Kernel errors become sentences naming the edge by its faces (`describe.rs`).
- The state each blend starts from is kept (`Evaluation::body_before`) and meshed, for showing
  while choosing edges.

### Shell (`shell.rs`, `FeatureKind::Shell`)

- Keeps the body, the opened faces as `FaceReference`s (possibly none, for a closed hollow body)
  and a thickness expression.
- Like blends: modifies a body, resolves references in the state before it, has that state meshed.
  A reference tied between fragments of one face, on one surface with one sense, opens all of them;
  a lost one, or one tied between separate faces, fails the feature.
- Kernel errors become sentences naming the face or edge involved.

### Pattern (`pattern.rs`, `FeatureKind::Pattern`)

- Repeats the whole body as it stands before the pattern and unions the copies with it (the
  kernel's `pattern`); boxed in `FeatureKind`, which it would otherwise double in size.
- `PatternKind::Linear` has a first `LinearDirection` and optionally a second (axis, count,
  spacing, reversed); `PatternKind::Circular` an axis, count, total angle and reversed. Every axis
  is an `AxisReference` resolved like a datum's (`Resolver::axis`), in the state at the pattern's
  place in the tree; its body and datum count as used, so they cannot be deleted or moved below
  it.
- Counts are plain expressions, whole and from 1 (the body alone) up to `MAX_PATTERN_INSTANCES`
  (100) instances in all, both directions multiplied, since each copy costs a boolean. Spacing
  is a length above zero (Reversed goes the other way) and the reach may not pass `MAX_SIZE`;
  parallel directions are refused. A total angle of 360° divides the turn by the count, a smaller
  one (above 0°) runs from the body to the last copy.
- It changes its body but is not `modifies_body`: nothing is chosen on the state before it, so
  the app shows the patterned body while it is open.
- Kernel failures become sentences: copies meeting only along an edge, touching ambiguously, or
  not joining.

### Sketch (`FeatureKind::Sketch(SketchFeature)`)

- Keeps its `Sketch` and optionally a `SketchAttachment`: a datum plane it follows, or, on a body,
  a `FaceAttachment` (`attachment.rs`: the body's feature ID and a `FaceReference`).
- The stored plane is where it was placed. Recompute resolves the reference in the body's state at
  the sketch's place in the tree (`FeatureKind::body_input`, shared with solid features that change
  a body) and gives the solved geometry the face's plane, outward normal and surface frame.
- Fragments of a split face are accepted when in one plane; a lost, split or curved face fails the
  sketch alone with a fix pointing at it.
- `SetSketchPlacement` sets plane and attachment together (attach, move to another face, or detach
  in place). A body with attached sketches cannot be deleted or stop making a body.

### Datum (`datum.rs`, `FeatureKind::Datum`)

- Planes and axes with a `DatumResult` (`Plane` or `Ray`).
- References to model geometry, resolved in each body's state at the feature's place in the tree
  (pieces of a split edge or face count when on one line): `PlaneReference` (principal plane, datum
  plane, or flat face as a `FaceAttachment`); `AxisReference` (principal axis, datum axis, straight
  edge as an `EdgeReference`, or the axis of a cylindrical, conical, toroidal or revolved face as a
  `FaceReference`).
- Whether geometry lies on a line or plane, runs along a plane or is parallel is decided in one
  place (`tolerance.rs`: 1e-6 in direction, `LINEAR_RESOLUTION` in position) for revolve axes,
  datums, face attachments and blend pieces, so noisy imported geometry is accepted or refused the
  same way everywhere.
- A `DatumPlane` starts from its base plane, optionally moved through an axis and turned about it
  (which must run along the plane) by an angle, then offset along its normal. A `DatumAxis` runs
  along an axis reference or where two planes meet.
- Edits refuse a sketch or plane based on a non-datum-plane (`NotAPlane`) or an axis reference to a
  non-datum-axis (`NotAnAxis`).
