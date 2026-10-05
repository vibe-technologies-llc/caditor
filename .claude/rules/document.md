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
  returns the inverse.
- `Editor` keeps undo and redo as stacks of inverses bounded by `MAX_UNDO_STEPS` and
  `MAX_UNDO_BYTES`, measured by `Transaction::approximate_size` (what an inverse keeps alive,
  including heap data through `heap_size` methods; anything new an edit can hold must be counted
  there). Oldest dropped first, newest always kept.
- `Editor::apply` drops a transaction that leaves the content as it was (`same_content`, which
  ignores ID counters and decides whether a model is unsaved): no undo step, redo history kept, no
  revision bump, not journaled, not recomputed.
- Large changes can be prepared off the UI thread: `Editor::base` hands out the document with the
  revision, `Base::prepare` applies a transaction to that copy, `Editor::commit` swaps it in only
  if the revision is unchanged, else `Stale`.
- Edits carry their IDs, so redo restores them; counters never move backwards. Parameter and
  feature IDs stay below `FIRST_UNSTORABLE_ID` (`ReservedId`).
- Edits refuse to break invariants: references to a feature below the one using it, unknown
  parameters, parameter cycles, deleting a parameter in use, moving a feature past one it depends
  on, two features sharing a name (loading renames the second, with a report).
- Deleting a feature others use is allowed: they keep its ID and fail with a reason until the
  deletion is undone. So a feature may be inserted or changed while referring to a feature not in
  the document (undoing a dependent's deletion, loading such a file); the feature ID counter is
  raised past every referenced ID so a dangling reference is never handed to a new feature.
  `dependents_of` gives every user of the given features in tree order; the app asks before
  deleting one that has any (`app-look.md`).
- `Document::check` runs a transaction on a clone so the UI can report an error before committing.
- Parameters live in a `ParameterList` indexed by ID and name, and the `DependencyGraph` survives
  parameter inserts and removals within an `apply`, so restoring a version and cycle checks stay
  near linear in the number of parameters.
- `Document::transaction_to` builds the transaction turning one document into another (bar to the
  end, everything removed, the target's inserted with IDs and flags, its bar restored), never
  lowering a sketch's ID counter; it restores an earlier version as one undoable change.

## Hidden flags

- `hidden` (`Edit::SetFeatureHidden`) is undoable, saved only when set, journaled; recompute
  ignores it.
- Which principal planes, axes and origin (`PrincipalGeometry`) are hidden is document content too
  (`Edit::SetPrincipalHidden`), compared by `same_content` and carried by `transaction_to`, so
  hiding a mixed selection and undoing it are one change and a model reopens as it was left.

## Suppression, the rollback bar and tree order (`tree.rs`)

- `suppressed` (`Edit::SetFeatureSuppressed`) is content like `hidden`, carried by `InsertFeature`
  so undoing the deletion of a suppressed feature restores it suppressed. Recompute skips it as if
  absent.
- The rollback bar (`RollbackBar`, `Edit::SetRollbackBar`) is document state: undoable, journaled,
  saved, compared by `same_content`. It must move with the features inserted at it in the same
  undoable step; a view-only bar would reopen models whole and could not be undone with them.
- `TransactionBuilder::add_feature` inserts at the bar, so every tool puts new features right above
  it and the bar stays under them without an edit of its own.
- The bar is attached to the feature below it: `MoveFeature` of that feature carries it along and
  `RemoveFeature` of it is refused (`RollbackBarAbove`); `Document::deletion` first moves the bar to
  the next surviving feature (or the end).
- `tree_rows` lists the features with the bar as a row between them; `move_row` moves a feature or
  the bar as one `MoveFeature` plus, when the bar's place changes, one `SetRollbackBar`, so a
  feature crossing the bar is rolled back or forward and a refused place returns `AboveDependency`
  or `BelowDependent` naming the feature in the way.

## Sketch edits (`edit/sketch.rs`)

- Sketch content changes only through sketch edits. Removing an entity something still uses is
  refused, not cascaded; `remove_sketch_items` expands a deletion into constraints, then curves,
  then points. `AddSketchEntity` carries the construction flag, and `AddSketchConstraint` the inactive flag, so
  undoing a removal restores them; `SetSketchConstraintActive` changes one constraint's flag and
  `reshape_sketch` emits it for kept constraints whose flag differs.
  Setting an entity changes only its value, never its kind or points.
- `reshape_sketch(feature, before, after)` turns a sketch edited as a whole (trim, extend) into
  edits; an entity whose kind or points changed is removed and added again under its ID with
  everything using it.
- `settle_sketch` moves the definition to a solved shape so the next solve starts from what the user
  sees.

## Feature kinds

- `SetFeatureKind` replaces settings but never the kind (`EditError::KindChange`): extrude stays
  extrude, a plane a plane, an import an import; a blend may switch fillet/chamfer and a pattern
  linear/circular. Sketches change through sketch edits.
- `bodies_used`, `planes_used` and `axes_used` extend `features()`, so dependents, moves and
  deletions account for them. `origin_features` (the features that made the faces and edges a
  feature holds, from the references' `FaceOrigin`s) join them in `dependencies()`, which
  `dependents_of`, `MoveFeature` and the delete dialog use, so deleting a boss a fillet's edge came
  from asks first. Inserts and kind changes check only `features()`, so a model saved with a
  reference to a face made further down still loads and edits. A reference to a patterned copy's
  face depends on both the pattern and the original's feature; `origin_feature`, which a
  double-click opens, is the pattern.
- `complete_origins` (`origins.rs`) fills in the `FaceOrigin`s of references saved without them, as
  a `SetFeatureKind` (`SetSketchPlacement` for a sketch's face) transaction: it evaluates the model
  without display data with the bar moved below the last feature holding such a reference and takes
  each origin from what the reference resolves to by name. Suppressed features, features not
  reached before the cancel token trips, and references no longer found keep what they had.
- Kernel and profile errors become sentences naming the sketch curves, edges or faces involved.

### Solid (`solid.rs`)

- An `Extrude` or `Revolve` of a sketch's regions (`RegionChoice::All`, or chosen
  `RegionReference`s, resolved by `resolve_regions` so a changed region is healed or left out; it
  fails only on a tie or when none is left) with a `BodyOperation`: `NewBody`, or `Add`, `Remove`,
  `Intersect` on the body of the feature that made it, which is named by that feature's ID. The app
  shows a feature's chosen regions as resolved, so a healed region reads as chosen and toggling
  stores it afresh.
- Extents are expressions above zero; a revolve's total angle is at most a full turn. An
  `Extrude::start` is an optional signed length: the profile starts that far along the sketch
  normal from the sketch plane (`offset_plane`), and every end, symmetric extent and up-to search
  works from there. `None` and zero are the same and stored as absent.
- An `ExtrudeEnd` may be `UpToFace(PlaneReference)`, resolved in its body's state at the
  extrusion's place in the tree. Face bodies and datum planes of the ends count as used
  (`end_bodies`, `end_datums`), so recompute reuses the result only while they are unchanged and a
  datum plane stays above the extrusion.
  - Through all only cuts or intersects and reaches past the target's box by `THROUGH_ALL_REACH`
    and `THROUGH_ALL_MARGIN`, so the result does not depend on how far; a body wholly behind fails.
  - Up to next needs a target body and ends on the plane of the first face the profile meets
    (kernel `next_face`); an addition starting inside the body, or a cut first entering it, would
    change nothing and fails.
  - Up to face ends on the face's plane extended past the face, which must lie beyond the whole
    profile on its side.
  - Every failure names the face or plane and the side, with what to do.
- A `RevolveAxis` is a line or axis of its own sketch (a sketch line used as axis cannot be
  deleted; `AxisNotALine`) or an `AxisReference` to a model axis lying in the sketch plane.
- A feature whose body others change keeps making a new body.

### Blend and shell (`blend.rs`, `shell.rs`)

- Both modify a body and resolve their references (`EdgeReference`s, opened `FaceReference`s) in
  the body's state before the feature, each to a `Resolution` (`pieces.rs`; the panel lists them in
  the same terms): one match; the pieces of a split edge on one line or circle (a shell face tied
  between fragments of one surface opens all of them); tied between separate ones; missing. A lost
  or tied reference fails the feature. That state is kept (`Evaluation::body_before`) and meshed,
  for showing while choosing.

### Combine (`combine.rs`)

- `Combine { body, tool, operation }` joins, cuts or intersects two existing bodies (kernel `boolean`)
  and keeps the target's body ID; the tool's body is consumed. `consumed_bodies` tells recompute to
  drop it from the bodies standing after the combine (and not to show it as a stale last-good body),
  so features below cannot use it and suppressing or rolling back the combine brings it back.
  Both bodies count as used (`bodies_used`), so deleting either asks first and neither moves below
  the combine. `bodies_before` lists the bodies standing before a feature, for the panel.
- A body combined with itself, a missing shape and kernel failures fail the combine alone with the
  bodies named; both inputs keep their results.

### Pattern (`pattern.rs`)

- Repeats the whole body as it stands before the pattern and unions the copies (kernel `pattern`).
  Boxed in `FeatureKind`, which it would otherwise double in size.
- Axes are `AxisReference`s resolved like a datum's at the pattern's place in the tree; their body
  and datum count as used.
- Counts are whole and from 1, at most `MAX_PATTERN_INSTANCES` instances in all (directions
  multiplied), since each copy costs a boolean. The reach may not pass `MAX_SIZE`; parallel
  directions are refused.
- It changes its body but is not `modifies_body`: nothing is chosen on the state before it, so the
  app shows the patterned body while it is open.

### Sketch and datum (`attachment.rs`, `datum.rs`)

- A sketch keeps its `Sketch` and optionally a `SketchAttachment`: a datum plane it follows, or on a
  body a `FaceAttachment`. The stored plane is where it was placed; recompute resolves the reference
  in the body's state at the sketch's place (`FeatureKind::body_input`) and gives the solved
  geometry the face's plane, normal and frame. Fragments of a split face are accepted when in one
  plane; a lost, split or curved face fails the sketch alone with a fix pointing at it.
  `SetSketchPlacement` sets plane and attachment together. A body with attached sketches cannot
  stop making a body.
- Datums are planes and axes with a `DatumResult`, referring to model geometry in each body's state
  at the feature's place in the tree. Edits refuse a sketch or plane based on a non-datum-plane
  (`NotAPlane`) or an axis reference to a non-datum-axis (`NotAnAxis`).
- Whether geometry lies on a line or plane, runs along a plane or is parallel is decided in one
  place (`tolerance.rs`) for revolve axes, datums, attachments, patterns and blend pieces, so noisy
  imported geometry is accepted or refused the same way everywhere.

### Import (`import.rs`)

- Keeps the source file's name, the solid and the canonical single-solid STEP text it was read from
  (what the file stores); equality compares the text. The body is `Solid::imported`, so later
  features hold its faces and edges like any body's.
