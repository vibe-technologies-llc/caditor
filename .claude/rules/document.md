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
  alive: removed features, an import's STEP text, `Solid::approximate_size`, and what each owns on
  the heap through `heap_size` methods: expression trees (`Expression::heap_size`), a spline's
  control list, chosen regions, face and axis references and their neighbour sets, a sketch's
  entities and dimensions; set and map overhead is counted per element at the element's size, not
  exactly); oldest dropped first, newest always kept.
- `Editor::apply` returns whether the model changed: a transaction that leaves the content as it was
  (`same_content`: a parameter set to its own expression, a flag to its value, edits that cancel
  out) is dropped, so it is no undo step, keeps the redo history, does not bump the revision and
  is neither journaled nor recomputed. `Base::prepare` decides this off the UI thread and
  `Editor::commit` then returns an empty transaction.
- Edits carry their IDs, so redo restores them; ID counters never move backwards. Parameter and
  feature IDs stay below 2^63 (`FIRST_UNSTORABLE_ID`, as in sketches): edits refuse larger ones as
  `ReservedId`, `reserve_ids_below` clamps to it.
- Edits refuse to break invariants: references to a feature below the one using it, unknown
  parameters, parameter cycles, deleting a parameter in use, moving a feature past one it depends
  on, two features sharing a name (names trimmed on every insert and rename; loading renames the
  second, with a report).
- Deleting a feature others use is allowed: they keep referring to its ID and fail with a reason
  until the deletion is undone. A feature may be inserted or changed while referring to a feature
  that is not in the document (undoing the deletion of a dependent, loading a file saved that
  way); the feature ID counter is raised past every referenced ID, so a dangling reference is
  never handed to a new feature. `dependents_of` gives every feature using the given ones,
  directly or through others, in tree order; the app asks before deleting a feature that has any
  (`app-look.md`).
- The parameters live in a `ParameterList` that indexes them by ID and by name, so
  `Document::parameter`, `parameter_named` and the position lookups of edits cost a map lookup,
  not a scan; appending and removing the last one is O(1), an insert or removal in the middle
  renumbers the ones after it. `Document::apply` keeps its `DependencyGraph` across inserts and
  removals of parameters (a removal asks the graph whether anyone uses the parameter), and
  `transaction_to` removes parameters last to first, so restoring a version stays linear in the
  number of parameters.
- `same_content` compares documents without ID counters; it decides whether a model is unsaved.
- `Document::check` runs a transaction on a clone so the UI can report the error before
  committing; `can_remove_parameter` answers the common case without one.
- Parameter dependencies are built once per `apply` as a `DependencyGraph` that also knows each
  parameter's users. A cycle check searches back from the edited parameter (nothing while no one
  uses it) and evaluation orders parameters topologically first, so both stay near linear.
- `Document::transaction_to` builds the transaction turning one document into another (the
  rollback bar moved to the end, every feature and parameter removed, then the target's inserted
  with their IDs and suppressed flags, then the target's rollback bar), keeping every sketch's ID
  counter at least where it is so IDs are never reused; it restores an earlier version as one
  undoable change.
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

## Suppression, the rollback bar and tree order (`tree.rs`)

- A feature's `suppressed` flag is changed by `Edit::SetFeatureSuppressed` (`Document::suppression`
  builds one transaction for several features). It is content like `hidden`, compared by
  `same_content`, carried by `InsertFeature`, so undoing the deletion of a suppressed feature
  brings it back suppressed. Recompute skips a suppressed feature as if it were absent
  (`document-recompute.md`).
- The rollback bar is document state, `RollbackBar::AtEnd` or `Before(first feature below it)`,
  changed by `Edit::SetRollbackBar` (`Document::roll_to`): undoable, journaled, saved, compared by
  `same_content`. SolidWorks, Fusion 360 and Onshape all keep it with the model, and it has to
  move with the features inserted at it in the same undoable step; a view-only bar would reopen
  models whole and could not be undone with the insertions it takes part in.
- Features at or after the bar are rolled back (`is_rolled_back`, `bar_index`); `is_active` is
  neither rolled back nor suppressed, and `active_features` lists those in tree order.
- Insert here: `TransactionBuilder::add_feature` inserts at the bar, so every tool puts new
  features right above it and the bar, naming the first feature below, stays under them without
  an edit of its own.
- The bar is attached to the feature below it: `MoveFeature` of that feature carries it along, and
  `RemoveFeature` of it is refused (`RollbackBarAbove`); `Document::deletion` first moves the bar
  to the next surviving feature (or the end), then removes the features last to first.
- `tree_rows` lists the features with the bar as a row between them; `move_row(row, gap)` moves a
  feature or the bar to a gap between rows as one `MoveFeature` and, when the bar's place
  changes, one `SetRollbackBar`, checked, so a feature crossing the bar is rolled back or forward
  and a refused place returns `AboveDependency` or `BelowDependent` naming the feature in the way.

## Sketch edits

- Sketch content changes only through sketch edits: add, remove or set an entity, switch a curve
  to or from construction geometry, add or remove a constraint, set a dimension.
- `AddSketchEntity` carries whether the entity is construction geometry, so undoing a removal
  restores it as one.
- Removing an entity something still uses is refused, not cascaded;
  `TransactionBuilder::remove_sketch_items` expands a user's deletion into constraints, then
  curves, then points.
- Setting an entity changes only its value, never its kind or the points it uses.
- `reshape_sketch(feature, before, after)` turns a sketch edited as a whole (trim, extend) into
  edits: constraints removed, curves then points removed, points then curves added, values and
  construction flags set, constraints added. An entity whose kind or points changed is removed and
  added again under its ID, with everything using it; the builder's counter moves past `after`'s.
- `settle_sketch` moves the definition to a solved shape so the next solve starts from what the
  user sees.

## Feature kinds

- `SetFeatureKind` replaces settings but never the kind (an import stays an import, a plane a
  plane, an axis an axis, a solid feature its kind; a pattern may switch between linear and
  circular).
- `FeatureKind::bodies_used`, `planes_used` and `axes_used` extend `features()`, so dependents,
  moves and deletions account for them.
- `FeatureKind::origin_features` are the features that made the faces and edges a feature holds
  (blend edges, shell faces, sketch and datum faces, axes along edges or faces, extrusion end
  faces), from the references' `FaceOrigin`s; `dependencies()` is `features()` with them.
  `dependents_of`, `MoveFeature` and the delete dialog use `dependencies()`, so deleting a boss a
  fillet's edge came from asks first and a fillet cannot move above it. Inserts and kind changes
  check only `features()`, so a model saved with a reference to a face made further down still
  loads and edits. A reference to a patterned copy's face depends on both the pattern and the
  original's feature (`FaceOrigin::features`), and is described as "Pattern 1 copy 2 of Base end
  face" (`describe_origin`; `origin_feature`, which a double-click opens, is the pattern).
- `complete_origins` (`origins.rs`) fills in the `FaceOrigin`s of face and edge references
  saved without them, as a transaction of `SetFeatureKind` (`SetSketchPlacement` for a sketch's
  face): it evaluates the model without display data (`Recompute::run_without_display`), with
  the rollback bar moved below the last feature holding such a reference, and takes each origin
  from the face or edge the reference resolves to by name in the body that feature sees (an edge
  only when it lies between the same faces). Suppressed features, features not reached before
  the cancel token trips, and references no longer found keep what they had. It walks a
  feature's references with the same `ReferenceVisitor` as the healing check (`healing.rs`).

### Import (`import.rs`, `FeatureKind::Import`)

- Makes a body from an imported solid; keeps the source file's name, the solid and the canonical
  single-solid STEP text it was read from (what the file stores). Equality compares the text.
- The body is the solid named by `Solid::imported`, so later features hold its faces and edges like
  any body's. One whose shape cannot be read back fails with a sentence.

### Solid (`solid.rs`, `FeatureKind::Solid`)

- An `Extrude` or `Revolve` of a sketch's regions (`RegionChoice::All` for even depth, or chosen
  `RegionReference`s, resolved by `resolve_regions` so a region the sketch changed is healed or
  left out, and the feature fails only on a tie or when none is left) with a `BodyOperation`:
  `NewBody`, or `Add`, `Remove`, `Intersect` on the body of the feature that made it. A body is
  named by that feature's ID. The app captures references from the shown regions
  (`SketchRegion::reference`, reusing their triangulation for the anchor) and shows a feature's
  chosen regions as resolved, so a healed region reads as chosen and toggling stores it afresh.
- Extents are expressions (lengths, or angles in degrees) above zero, both distances of a
  two-sided extrusion included; one-sided extents flip with `reversed`. A revolve turns a full
  turn, one angle, a symmetric angle, or two angles (`RevolveExtent::TwoSides`, forward and
  backward from the sketch plane) that together may turn at most 360°.
- An extrusion is one-sided or two-sided with an `ExtrudeEnd` per side, or symmetric by a
  distance. An end is a `Distance`, `ThroughAll`, `UpToNext` or `UpToFace(PlaneReference)`
  (a principal plane, a datum plane, or a flat face as a `FaceAttachment` resolved in its body's
  state at the extrusion's place in the tree, like a datum's). Face bodies and datum planes of
  the ends count as used (`SolidFeature::end_bodies`, `end_datums`), so recompute reuses the
  result only while they are unchanged and a datum plane stays above the extrusion.
  - Through all only cuts or intersects: it reaches past the target body's box along the side's
    direction (by a twentieth of its diagonal and 1 mm), so the result does not depend on how
    far; a body lying wholly behind fails.
  - Up to next needs a target body and ends on the plane of the first face its profile meets
    (the kernel's `next_face`); an addition whose profile starts inside the body, or a cut whose
    profile first enters it, would change nothing and fails instead.
  - Up to face ends on the face's plane, extended past the face, so a face that covers only part
    of the profile still ends all of it. The plane must lie beyond the whole profile on its side:
    one behind, across the profile or along the direction fails.
  - Every failure names the face or plane and the side, with what to do: a lost face points back
    at the extrusion.
- A revolve's axis (`RevolveAxis`) is a line of its sketch, a sketch axis, or an `AxisReference` to
  a model axis lying in the sketch plane. A sketch line used as axis cannot be deleted; edits
  refuse an axis not a line or axis of the revolve's own sketch (`AxisNotALine`).
- Inserting one checks its sketch, when present, is a sketch and its target makes a body. A
  feature whose body others change keeps making a new body.
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
- Kernel errors become sentences naming the face or edge involved, or the faces around a corner
  the walls cannot meet at.

### Pattern (`pattern.rs`, `FeatureKind::Pattern`)

- Repeats the whole body as it stands before the pattern and unions the copies with it (the
  kernel's `pattern`); boxed in `FeatureKind`, which it would otherwise double in size.
- `PatternKind::Linear` has a first `LinearDirection` and optionally a second (axis, count,
  spacing, reversed); `PatternKind::Circular` an axis, count, total angle and reversed. Every axis
  is an `AxisReference` resolved like a datum's (`Resolver::axis`), in the state at the pattern's
  place in the tree; its body and datum count as used, so they are its dependencies (deleting
  them lists the pattern first) and cannot be moved below it.
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
  in place). A body with attached sketches cannot stop making a body; they depend on it, so
  deleting it lists them first.

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
