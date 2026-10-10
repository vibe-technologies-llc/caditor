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
- Each undo step carries a serial from a per-editor counter, kept as the step moves between the
  undo and redo stacks. `Editor::undo_mark` notes the newest step, the next serial and how many
  steps the byte and step bounds dropped; `undo_steps_since` gives the undo steps made since the
  mark, newest first, or none once the stack no longer leads back to it (the noted step undone or
  dropped, steps since it dropped). The app's Cancel of an open feature relies on it.
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
  near linear in the number of parameters. Their order is the user's (`MoveParameter`) and changes
  nothing computed; a parameter's `note` (`SetParameterNote`, trimmed, at most
  `MAX_PARAMETER_NOTE_CHARS`) is free text for the user only.
- `Document::inline_parameter` builds the one transaction deleting a parameter in use: its
  expression (not its value, so links to the parameters it uses survive) is substituted into every
  parameter, sketch dimension, feature and body density using it (`Expression::inlining`, which
  round-trips through stored text so precedence is kept), then the parameter is removed. The
  model computes exactly as before. An expression that would grow past what the stored text reads
  back refuses as `InliningTooLong`.
- `Document::scaled` (`model_scale.rs`) builds the one transaction resizing the whole model by a
  `ModelScale` (a factor from `MIN_SCALE_FACTOR` to `MAX_SCALE_FACTOR`, not 1, about a centre),
  rewriting stored values in place so no ID, name or reference changes and undo is one step:
  sketch planes are mapped (`Plane::mapped`), points, radii, `Fix` positions, label offsets and
  projection reaches multiplied, and every length expression of sketches and features scaled;
  angles, counts and plain factors never change. The values a feature holds in the world
  (an unattached sketch's plane, a datum plane offset from a principal plane, a datum point from
  the origin, a primitive's place on a principal plane, a scale's centre, an import's offsets, a
  move turning about the origin) also take the shift a centre off the origin brings, so the
  centre stays put; geometry a feature uses that cannot move (a principal plane, axis or the
  origin not holding the centre, `Anchor`) refuses the scale as `CentreOffPrincipal` naming the
  feature. Saved views follow (target mapped, distance multiplied).
- `ScaledValues::Plain` scales parameter-free values and named values (owned length parameters
  with no parameters of their own); a value using any other parameter is left to follow it and
  listed in `ScaleSummary::kept`. `AndParameters` also rewrites every parameter in evaluation
  order by its length power. Each expression is rewritten by
  `Expression::with_lengths_scaled` and kept only when it evaluates to the factor to its power
  times its old value; otherwise it becomes the exact form (scaled parameters divided back,
  times the factor), so a value never silently changes size. A hole whose diameter changes
  loses its `standard`, and threads keep their size; both are in the summary for the app to say.
- `Document::transaction_to` builds the transaction turning one document into another (bar to the
  end, everything removed, the target's inserted with IDs, flags, notes and owners, its bar
  restored), never
  lowering a sketch's ID counter; it restores an earlier version as one undoable change.

## Model parameters (`model_parameters.rs`)

- A sketch dimension or a feature's value is named by making it a parameter: the parameter holds
  the expression and the field holds `Expression::Parameter` of it. A named value is therefore
  identified by its parameter's own stable `ParameterId`, never by the field's place in the
  feature, and renaming, dependency order, cycle checks, inlining and deletion are those of
  parameters. Measured values are not expressions: parameters evaluate before and apart from
  recompute, so reading geometry into one would make them depend on the model they drive.
- `Parameter::owner` (`ParameterOwner`) says which value it names: `Feature { feature, value }`,
  `value` the field's caption, kept on one trimmed line, past `MAX_VALUE_LABEL_CHARS` refused as
  `ValueLabelTooLong`; or `Dimension { sketch, constraint }`. It is content like a note: compared
  by `same_content`, carried by `InsertParameter` and `transaction_to`, counted by `heap_size`,
  ignored by evaluation and recompute, and never checked against the model (an owner that no
  longer exists reads as deleted). `Edit::SetParameterOwner` sets or clears it.
  `owned_parameter` tells whether a field's expression is the named value of an owner, and
  `owner_text` words the owner ("Extrude 1 · Distance", "Sketch 1 · Radius").
- `TransactionBuilder::add_owned_parameter` inserts one. `Transaction::substituting` replaces an
  exact expression wherever a `SetFeatureKind` (through `inlining::expressions_mut`),
  `SetDimension` or `SetParameterExpression` holds it, and counts the replacements.
- `releasing(transaction, ids)` appends what the named values become once the transaction is
  applied: each one nothing uses any more is removed (users before what they use), any other
  loses its owner and stays an ordinary parameter. `deletion` releases the values owned by the
  deleted features, `remove_sketch_items` those owned by the dimensions it removes; reshaping a
  sketch keeps constraint IDs, so owners survive it.

## Pasting features (`paste.rs`)

- `Document::paste_features` plans one transaction inserting copies of features at the rollback
  bar in the order given (tree order), each under a fresh id from `add_copied_feature` and named
  "<name> copy" (numbered when taken), keeping hidden, suppressed and appearance but not the group.
- `FeatureKind::rename_features` rewrites the plain feature ids a kind holds (sketch, body, tools,
  datum and frame attachments, projections' bodies, sketches and datums). Inputs inside the copy
  are first renamed to placeholders above `PLACEHOLDER_BASE`; any input still among the copied
  ids afterwards is held deeper (a face or edge reference names its feature in its digest), so the
  feature is left out with `PasteRefusal::CopiedGeometry` rather than silently rewired to the
  original, and features using a left-out one are left out with `LeftOutInput`.
- Inputs outside the copy are kept when pasting into the model they came from
  (`PasteOrigin::ThisDocument`) if they exist before the bar, and refused (`OutsideFeature`) when
  the copy came from another model, where the same id names something else.
- Parameters travel as `CarriedParameters` (name and value at copy time). `carry` keeps an id the
  target still has when pasting in the same model, else takes the target's parameter of the same
  name, else writes the copied value as a literal (`literal`: lengths in mm, angles in degrees,
  plain numbers), counted in `inlined`; a value of another kind refuses the feature in words.
  `Expression::substituting` replaces every id in one pass, so swapped ids never collide.

## Importing parameters (`parameter_import.rs`)

- `Document::plan_parameter_import` turns rows of name, expression text and note into a
  `ParameterImport`: an `ImportOutcome` per row (Added, Changed with the expression it replaces,
  Kept when the model's own differs and `replace_existing` is off, Unchanged, or Refused with an
  `ImportRefusal`) and one transaction, or none when nothing changes. Rows merge by name; an
  owned parameter keeps its owner. A note in the file replaces the model's, an empty one leaves it.
- Rows are refused alone, never the whole file: a name `check_name` refuses or seen on an earlier
  row, a note too long, text that does not parse (names resolve to the model's parameters and to
  the file's other rows, so rows may refer to later ones), a cycle on the merged graph
  (`DependencyGraph`, every row in it refused, named as a path), a value that does not evaluate
  once applied (tried on a clone), and a row using a new name that was refused. Refusing repeats
  until nothing more is refused, since one refusal can strand another row.
- New parameters take ids from the counter in row order, gaps left by refused rows included, and
  are inserted at the end of the list with a stand-in value before the expressions are set, so
  rows may refer to each other in any order.

## Hidden flags

- `hidden` (`Edit::SetFeatureHidden`) is undoable, saved only when set, journaled; recompute
  ignores it.
- Which principal planes, axes and origin (`PrincipalGeometry`) are hidden is document content too
  (`Edit::SetPrincipalHidden`), compared by `same_content` and carried by `transaction_to`, so
  hiding a mixed selection and undoing it are one change and a model reopens as it was left.

## Feature groups (`grouping.rs`)

- A feature's `group` is the name of the folder it sits in, or none: content like `hidden`
  (compared by `same_content`, carried by `InsertFeature`, counted by `heap_size`), ignored by
  recompute. `Edit::SetFeatureGroup` sets it, kept on one trimmed line (`group_name`; blank is
  none), refused past `MAX_GROUP_NAME_CHARS` as `GroupNameTooLong`.
- A folder is a run of consecutive features sharing the name (`group_run`), so membership needs no
  table of its own and a folder is always contiguous where it is drawn. `grouping` names the
  chosen features in one transaction, first moving them together under the first of them
  (`move_features`, refusing as a move would) when they are apart; `regrouping` renames or ungroups
  a run; `unused_group_name` gives "Group N".
- Moves keep folders whole: a feature moved (`move_row`, `move_features`) between two members of
  one group joins it, and a member moved where neither neighbour shares its group leaves it
  (`membership_edits`), in the same transaction as the move.

## Model properties (`properties.rs`)

- `ModelProperties` (title, part number, revision, author, organisation, description, notes) is
  document content like the hidden principal geometry: compared by `same_content`, carried by
  `transaction_to`, counted by `heap_size`, ignored by recompute. Empty by default; nothing fills
  them in but the user.
- `Edit::SetModelProperties` sets them whole and its inverse holds the previous set. Each field is
  trimmed, every field but the notes is kept on one line (line breaks become spaces), and a field
  past its `ModelProperty::max_chars` (`MAX_PROPERTY_CHARS`, `MAX_DESCRIPTION_CHARS`,
  `MAX_MODEL_NOTES_CHARS`) is refused as `PropertyTooLong` naming it.

## Saved views (`views.rs`)

- `SavedViews` (named views in the user's order, and the redefined Isometric `home`) is document
  content like the model properties: compared by `same_content`, carried by `transaction_to`,
  counted by `heap_size`, ignored by recompute. A `SavedView` is a camera's target, orientation
  and distance in model units, with no projection, which stays a preference.
- `Edit::SetSavedViews` sets the whole set and its inverse holds the previous one, so saving,
  updating, renaming and deleting a view and changing the Isometric view are each one undoable
  change. Names are trimmed, kept on one line and unique ignoring case; a blank name, one past
  `MAX_VIEW_NAME_CHARS`, a duplicate, more than `MAX_SAVED_VIEWS` views and a view the camera
  cannot show (not finite, no distance) are refused as `ViewNameEmpty`, `ViewNameTooLong`,
  `ViewNameTaken`, `TooManyViews` and `ViewNotUsable`. A unit orientation is stored as given and
  any other is normalised, so setting the same views again is no change.

## Selection sets (`selection_sets.rs`)

- `SelectionSets` (named `SelectionSet`s in the user's order) is document content like the saved
  views: compared by `same_content`, carried by `transaction_to`, counted by `heap_size`, ignored
  by recompute. A set's `SetMember`s are a whole body (its `FeatureId`), or a face or an edge of a
  body as a `FaceReference` or `EdgeReference`, never a position.
- `Edit::SetSelectionSets` sets them whole and its inverse holds the previous ones. Names follow
  the saved views' rules (`set_name`, unique ignoring case, `MAX_SET_NAME_CHARS`); a blank or
  taken name, an empty set, a set past `MAX_SET_MEMBERS` and more than `MAX_SELECTION_SETS` sets
  are refused (`SetNameEmpty`, `SetNameTooLong`, `SetNameTaken`, `SetEmpty`, `SetTooLarge`,
  `TooManySets`). A set may name a body no longer in the model, like a feature using a deleted
  one, so the feature ID counter is raised past every body a set names.
- `SelectionSet::resolve` finds what a set means in an `Evaluation` now, so it heals like any
  reference: a body is every face of its current solid, a face or an edge resolves by its
  reference in its body's shown solid (a tie takes every candidate). What it cannot find is a
  `SetLoss` per body (no solid now, a face or an edge not found), which the app puts in words.

## Configurations (`configurations.rs`)

- `Configurations` is a table kept in the model: columns are `ConfiguredValue`s (a parameter's
  expression, a feature's suppression, a body's colour), rows are named `Configuration`s with a
  stable `ConfigurationId` from the table's own counter (`next_id`, never lowered, ignored by
  `same_content`), each holding a `Setting` per column. It is content like the selection sets:
  compared by `same_content`, carried by `transaction_to` (deactivating first), counted by
  `heap_size`.
- One row may be `active`. The active row's settings are always the model's live values: an edit
  changing a configured value (`SetParameterExpression`, `InsertParameter`,
  `SetFeatureSuppressed`, `SetBodyAppearance`, `InsertFeature`) also writes it into the active
  row, so its inverse writes it back, and `SetConfigurations` overwrites the active row from the
  live values. `configuration_setting` reads a row, the live value for the active one.
- `Edit::SetConfigurations` sets the table whole (names trimmed, on one line, unique ignoring
  case, at most `MAX_CONFIGURATION_NAME_CHARS`; at most `MAX_CONFIGURATIONS` rows and
  `MAX_CONFIGURED_VALUES` columns; settings of the wrong kind or for no column dropped).
  Columns, cells' parameters and feature columns may name things no longer in the model, like a
  selection set naming a deleted body: such a column is skipped when switching, and the ID
  counters are raised past every ID named.
- `Edit::SetActiveConfiguration` changes only which row is active, refused
  (`ConfigurationOutOfStep`) when the row's settings are not the live values. `activating` is the
  one undoable switch: none active, then the row's settings applied as ordinary edits (values that
  use parameters first set to 0, then those using none, then the rest, so values referring to each
  other never pass a cycle), then the row active. Recompute sees only the changed values.
- The builders (`adding_configuration`, `duplicating_configuration`, `renaming_configuration`,
  `moving_configuration`, `deleting_configuration`, `configuring`, `unconfiguring`,
  `setting_configuration`) each make one transaction. A new row copies the live values and is
  active when none is; deleting the active row first switches to the next one; setting a cell
  of the active row is the live edit itself.

## Body appearance (`body_appearance.rs`)

- A feature's `appearance` (`BodyAppearance`: an sRGB `Rgb` colour, a material name, a density
  expression) is content like `hidden`: compared by `same_content`, carried by `InsertFeature`, so
  deleting and restoring a body keeps it, and counted by `heap_size`. Recompute ignores it.
- `Edit::SetBodyAppearance` sets it whole on a feature that `makes_body` (else `NotABody`); a
  material name is trimmed, blank means none, longer than `MAX_MATERIAL_NAME_CHARS` is refused.
  Its `name` is the body's own name, kept on one line the same way (`MAX_BODY_NAME_CHARS`,
  `BodyNameTooLong`); `Document::body_name` gives it, else the making feature's name, and the app
  lists and exports bodies by it. Its `opacity` is a percent from `MIN_OPACITY_PERCENT` (lower is
  `OpacityTooLow`); 100 or more is stored as none, solid. `OPACITY_STEPS` (75, 50, 25) are the
  opacities the app offers, and `nearest_opacity_step` snaps a percent to the nearest of them
  or solid (none), which STEP import uses. `DEFAULT_BODY_COLOUR` is the colour a body without one
  is drawn in; the app's `DEFAULT_COLOUR` is it, and import colours a face see-through but
  coloured by neither itself nor its body with it.
  A feature that later stops making a body keeps it, unused.
- The density is a plain number in g/cm³ that may use parameters, so `Feature::parameters` and
  `Feature::uses_parameter` (which parameter deletion, `parameter_users` and loading's stand-ins
  use) include it. It is evaluated where shown (`density_value`, `mass_grams`): above zero and at
  most `MAX_DENSITY`, else a `DensityError` saying why.
- `faces` lists `FaceColour`s, a `FaceReference` with its colour and an optional opacity (a
  percent, 100 solid on a see-through body, below `MIN_OPACITY_PERCENT` refused like the body's),
  later entries winning. `face_colours` and `face_opacities` map a solid's faces to them in one
  pass: every face of the reference's name (so each fragment of a face a later feature splits
  keeps its colour), else the face the reference resolves to; a face found by neither keeps the
  body's colour and opacity.

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
  or `BelowDependent` naming the feature in the way, or `BelowConsumer` or `AboveConsumedUse` when a
  feature using a body (`bodies_used`) would come below the Combine consuming it.
- `move_features` moves several features to one gap in their tree order, as one transaction of
  `MoveFeature`s ordered so no step passes a feature over one it uses that is moving with it:
  those below the gap rise first, each right under the gap, then those above sink in reverse
  order. Each step is checked like a single move, so the refusals are the same.

## Sketch edits (`edit/sketch.rs`)

- Sketch content changes only through sketch edits. Removing an entity something still uses is
  refused, not cascaded; `remove_sketch_items` expands a deletion into constraints, then curves,
  then points. `AddSketchEntity` carries the construction flag, and `AddSketchConstraint` the inactive flag
  and the label offset, so undoing a removal restores them; `SetSketchConstraintActive` changes one
  constraint's flag and `SetSketchLabel` one dimension's label offset (or none), and
  `reshape_sketch` emits them for kept constraints whose flag or offset differs.
  Setting an entity changes only its value, never its kind or points.
- `Edit::SetSketchProjection` gives an entity a `ProjectionSource` (or none) and sets the projected
  flag on it and its points in one step; its inverse holds the previous source. A projected entity
  is never removed while flagged (`StillProjected`), so `remove_sketch_items` first takes the
  projection off every doomed projected entity, and takes a projected curve's points with it,
  which undo puts back in the opposite order.
- `reshape_sketch(feature, before, after)` turns a sketch edited as a whole (trim, extend) into
  edits; an entity whose kind or points changed is removed and added again under its ID with
  everything using it.
- `settle_sketch` moves the definition to a solved shape so the next solve starts from what the user
  sees.

## Feature kinds

- `SetFeatureKind` replaces settings but never the kind (`EditError::KindChange`): extrude stays
  extrude, a plane a plane, an import an import; a blend may switch fillet/chamfer and a pattern
  linear/circular. An import may take another file's solid and keeps its body for the features
  using it, like an extrusion making a new body. Sketches change through sketch edits.
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
  Boolean failures of extrusions, revolves, holes and combines go through `trouble.rs`: the faces
  of the `BooleanSite` named by origin ("where A meets B" for an intersection), the site's point as
  the error's `place`, and a remedy to make the faces there line up exactly or stay clearly apart,
  since near-coincident faces cause most of them; an invalid result asks only for a slight change.
  Patterns and mirrors, whose operands are intermediate, pass on the point alone.

### Solid (`solid.rs`)

- An `Extrude` or `Revolve` of a sketch's regions (`RegionChoice::All`, or chosen
  `RegionReference`s, resolved by `resolve_regions` so a changed region is healed or left out; it
  fails only on a tie or when none is left) with a `BodyOperation`: `NewBody`, or `Add`, `Remove`,
  `Intersect` on the body of the feature that made it, which is named by that feature's ID. The app
  shows a feature's chosen regions as resolved, so a healed region reads as chosen and toggling
  stores it afresh.
- Extents are expressions above zero; a revolve's total angle is at most a full turn. An
  `start` of an `Extrude` or a `Revolve` is an optional `SolidStart`: a signed length, or a
  `PlaneReference` (resolved like an end's, in its body's state at the feature's place) of a flat
  face or plane parallel to the sketch, whose distance along the normal becomes the length
  (`start_plane`; a plane that is not parallel fails the feature, naming it);
  `displayed_start_offset` gives that length from an evaluation for drawing, a face resolved on
  its body as the feature sees it (`displayed_plane` takes the user, as `displayed_axis` does).
  The profile starts there, and every end, symmetric extent, up-to search and a revolve's turn
  (its axis keeps the sketch's frame) works from there. `None` and zero length are the same and stored as absent. A
  start plane's body and datum count as used like an end's, and its face is healed like one.
- An `ExtrudeEnd` may be `UpToFace(PlaneReference)`, resolved in its body's state at the
  extrusion's place in the tree. Face bodies and datum planes of the ends count as used
  (`end_bodies`, `end_datums`), so recompute reuses the result only while they are unchanged and a
  datum plane stays above the extrusion.
  - Through all only cuts or intersects and reaches past the target's box by `THROUGH_ALL_REACH`
    and `THROUGH_ALL_MARGIN`, so the result does not depend on how far; a body wholly behind fails.
  - Up to next needs a target body and ends on the plane of the first face the profile meets
    (kernel `next_face`); an addition starting inside the body, or a cut first entering it, would
    change nothing and fails. When the first faces met are curved or several, a one-sided
    extrusion with no end offset is swept as far as through all and cut back where it first meets
    the target body (kernel `stop_at_body`, `Stopping` in `solid.rs`), so it follows a cylinder or
    a step; two sides or an offset still need one flat face and fail in words.
  - Up to face ends on the face's plane extended past the face, which must lie beyond the whole
    profile on its side.
  - `UpToSurface` (a boxed `FaceAttachment`, no offset) ends on any face, flat or curved,
    resolved in its body's state at the extrusion's place: a flat one ends on its plane as Up to
    face does; a curved one is stopped where the profile meets that face's body
    (`stop_at_body`, as up to next), and fails naming the other faces of that body met first. Its
    body counts among `end_bodies`, its face's origins among the feature's, and healing keeps it.
  - Either up-to end may carry an `offset` (a signed length expression, boxed like the target to
    keep `FeatureKind` small; zero is stored as absent): the end plane moves along its own normal
    that far past the face, or stops short of it when negative, so a slanted plane stays the same
    slant. The face must still lie ahead of the whole profile, and an offset bringing the end back
    across the profile fails naming it and asking for a smaller offset. Offsets count among the
    feature's expressions (parameters, inlining, model scaling).
  - Every failure names the face or plane and the side, with what to do.
- A removal also cuts each body of `other_bodies` (an extrusion's or revolve's, any other
  operation with some fails in words), every body with the same tool, so one feature changes them
  all: they are in `bodies_used` and `Feature::bodies`, their results are `SolidResult::others`
  (like a split's), Through all reaches past all of them, the target and repeats are skipped, and
  a body the tool removes entirely fails the feature naming it.
- `RevolveExtent::UpTo { target, reversed }` turns up to a flat face or plane (a boxed
  `PlaneReference`, resolved, counted and healed like an extrusion's end) that holds the
  revolution axis: the profile turns, forward or reversed, until its half-plane first lies in the
  target's plane (`Turn` in `solid.rs`, from the chosen regions' middle), so a face is taken as
  its whole plane. A target not holding the axis, or holding the profile itself, fails in words.
- A `RevolveAxis` is a line or axis of its own sketch (a sketch line used as axis cannot be
  deleted; `AxisNotALine`) or an `AxisReference` to a model axis lying in the sketch plane.
- A revolve's `side` (`AxisSide`, left or right of the axis's direction in the sketch plane; none
  turns the whole profile) keeps only the part of the chosen regions on that side: the sketch's
  curves are divided again with the axis as a line past the chosen regions' bounds (entity
  `REVOLUTION_AXIS_ENTITY`, worded "the revolution axis"), the regions whose anchor lies on that
  side inside a chosen region are selected as one, and they are revolved, so a profile crossing
  the axis or lying on both sides turns, and a side holding nothing fails in words.
- An `Extrude`'s optional `direction` (a boxed `AxisReference`: an edge, an axis, a round face's
  axis or a sketch line) runs it along that line instead of square to its sketch (kernel
  `extrude_along`), the way that leaves the sketch on its forward side; a direction in the sketch
  plane fails naming it. Distances are measured along the direction, while up-to planes, their
  offsets and through all keep working by height; up to next, a curved face and a taper run
  square only and fail in words. The axis's body, datum, frame and sketch count as used like a
  revolve's model axis (`SolidFeature::model_axis`), and healing keeps it.
- An `Extrude`'s optional `taper` (an angle, boxed with the wall to keep `FeatureKind` small) goes
  to the kernel's `extrude_tapered`; at or past `MAX_TAPER_DEGREES` it fails before the kernel, and
  every taper refusal (a spline, a slanted end, the profile closing) names the extrusion and says
  what to change. An `Extrude`'s or `Revolve`'s optional `wall` (`Wall`: a thickness expression
  and the kernel's `WallSide`) sweeps `wall_regions` of every non-construction curve of the sketch
  instead of its regions, so the region choice is ignored and an open sketch works; a wall's
  failures are worded per `WallError` (the curves, the feature, what to change), and a revolve
  keeping one side of its axis with a wall fails in words. Both count among the feature's
  expressions (parameters, inlining).
- A feature whose body others change keeps making a new body.

### Blend and shell (`blend.rs`, `shell.rs`)

- A `Blend` is a fillet or chamfer of `size` (radius or first distance). A chamfer's `form` is
  `Equal`, `TwoDistances { second }` or `DistanceAngle { angle }` (degrees, below 180), with
  `flipped` swapping which face of each edge the first distance is measured on (the kernel's
  choice: the face more chosen edges share). A fillet ignores both but keeps them, so switching
  back restores the chamfer; the extra expression counts among the feature's expressions
  (`Blend::expressions`, parameters, inlining) and a second distance scales with the model.

- Both modify a body and resolve their references (`EdgeReference`s, opened `FaceReference`s) in
  the body's state before the feature, each to a `Resolution` (`pieces.rs`; the panel lists them in
  the same terms): one match; the pieces of a split edge on one line or circle (a shell face tied
  between fragments of one surface opens all of them); tied between separate ones; missing. A lost
  or tied reference fails the feature. That state is kept (`Evaluation::body_before`) and meshed,
  for showing while choosing.

### Offset face (`offset_face.rs`)

- `OffsetFace { body, faces, distance, tangent }` resolves its `FaceReference`s in the body's state
  before it, like a shell's open faces (a lost or tied reference fails it), and moves them by a
  signed length expression along their outward normals (kernel `offset_faces`; positive grows the
  body, negative shrinks it, zero fails in words). With `tangent` the chosen faces first grow to
  everything reached across smooth edges (`tangent_faces`), recomputed on every evaluation, so the
  chain follows upstream edits.
- It modifies its body (`modifies_body`), keeps every face and edge name, and keeps the state
  before it for choosing (`Evaluation::body_before`). Kernel refusals become sentences naming the
  faces or edges involved and what to change (`OffsetError`: a face that would vanish, faces that
  would cross, a neighbour it cannot follow, a corner where the faces cannot meet).

### Primitive (`primitive.rs`)

- `Primitive { shape, plane, at, anchor, reversed, operation }` makes a box (length, width,
  height), cylinder (diameter, height), sphere (diameter), torus (diameter across the middle of
  the tube, tube diameter), cone (bottom and top diameters, height), wedge (length, width, height,
  top length) or prism (number of sides, diameter across its corners, height) without a sketch.
  Every size is a length expression above zero (and at most `MAX_SIZE`), as `PrimitiveShape::rules`
  says, except a cone's diameters and a wedge's top length, which may be zero (`SizeRule::ZeroOrMore`)
  but not both diameters, a top no longer than the wedge, and a prism's sides, a plain number
  `MIN_PRISM_SIDES` to `MAX_PRISM_SIDES` (`SizeRule::Sides`); a torus's tube is narrower than
  its diameter; `at` is two lengths of any sign
  along the plane's X and Y axes. `plane` is a `PlaneReference` resolved like a mirror's at the
  feature's place (its datum and face body count as used, its face is healed as "the face it
  stands on"), so a primitive on a face follows that face's own frame.
- The shape's footprint box (a round shape's square around it) has a corner at `at`
  (`PrimitiveAnchor::Corner`), its base's middle there (`BaseCentre`), or its middle there, half
  above the plane (`Centre`); it grows along the plane's normal, or against it when `reversed`.
- A box, a cylinder and a prism are a profile extruded by the kernel (`extrude`), a sphere, a
  torus and a cone one revolved a full turn (`revolve`), with fixed entities so the faces are
  named from the feature: a box's sides are entities 1 to 4 (front at the plane's -Y, then right,
  back, left), a prism's 1 to its count from the flat side at the front, a round shape's curve
  entity 1, a cone's base and top lines 3 and 4 (left out when that diameter is zero, so a point
  has no face), and the caps are the start (bottom, on the plane) and end (top) caps. A wedge is
  its side profile (bottom 1, sloped 2 at +X, top 3, left 4, the top left out when zero)
  extruded along the plane's Y, so its caps are the front and back faces
  (`PrimitiveShape::cap_name`). `describe_origin` words them "Box 1 front face", "Cylinder 1
  wall", "… bottom face", "… top face", "Wedge 1 sloped face", "Sphere 1 surface". Resizing or moving the shape keeps every name, so later references
  survive; switching the shape (`SetFeatureKind` allows it) changes them.
- `operation` is a `BodyOperation` as a solid's: a new body of its own (`makes_body`), or joined
  to, cut from or intersected with the target body (kernel `boolean`), keeping the tool as the
  result's `joins` or `cuts`, so a pattern or mirror of features repeats one that adds or cuts as
  it repeats an extrusion's tool. A failure names the shape and the body in words (`trouble.rs`), the
  feature failing alone.

### Hole (`hole.rs`)

- `Hole { sketch, body, diameter, depth, style, reversed }` drills at every free point of its sketch
  (`Sketch::free_points`: no curve uses it, not construction; a point only a constraint uses still
  counts) and at the centre of every circle that is not construction (`hole::centres`), down into
  the sketch plane's normal, or up when reversed. `HoleDepth` is blind,
  through all (the farthest corner of the body past the point plus a margin, as the extrusion's
  through all), up to next or up to face (a boxed `PlaneReference` resolved like an extrusion's
  end, its body, datum and frame counted as used, its face healed), each up-to end with an
  optional signed `offset` as an extrusion's end has. Each hole's depth is where its axis meets
  the plane, the offset measured along the plane's normal, so a slanted face gives each hole its
  own depth and a flat bottom square to the hole there. Up to next asks kernel `next_face` along
  the hole's own outline (circle or slot) in the body before the feature, and fails in words when
  the hole first enters the body (it would remove nothing), meets several faces, a curved one or
  passes beside it. An up-to hole's counterbore, countersink or steps must stay shallower than
  each hole's depth, and it ends flat whatever its bottom says, like a through hole; `HoleStyle` is plain, counterbore (diameter, depth), countersink (diameter,
  angle) or stepped (`HoleStep`s from the mouth down, 1 to `MAX_HOLE_STEPS`, each a diameter and
  its own depth, so a step's floor lies at the sum of the depths down to it). Each step must be
  narrower than the one above it and wider than the hole, and the steps together shallower than a
  blind hole; a refusal names the step. A counterbore is drilled as one step.
- Each hole is a half-section polygon revolved a full turn about the hole axis (kernel `revolve`),
  starting `MARGIN` above the plane so the cut is clean, then subtracted from the body. Curve
  entities are `point id * 16 + part`, so every hole's faces are named by their sketch point and
  survive adding, moving or removing the other points. The part is the segment's role (`HolePart`:
  wall 1 and bottom 2 as a plain hole always had, counterbore wall 4 and floor 5, countersink 6),
  never its place in the outline, so a style change keeps the wall and bottom and references to
  them; `describe_origin` words them through `Hole::part_name`. A stepped hole's first step uses
  the counterbore's wall and floor parts, so turning a counterbore into steps keeps them; step n
  below it adds `(n - 1) << STEP_SHIFT` (56) to its entities, worded "step n wall" and "step n
  floor" (wrapping like the point id's multiple, so only point ids past 2^52 could collide).
- `shape` is `Round` or `Slot { length, angle }`: each point gets a slot of that centre-to-centre
  length, turned from the sketch's X axis by the angle in its plane, cut as an extruded stadium
  (and a wider, shallower one for a counterbore) rather than a revolve. Its walls are parts 7 to 10
  (`slot side`, `slot end`) and 11 to 14 for the counterbore, so a slot's names never collide with a
  round hole's. A countersunk slot fails alone, saying so.
- `standard` (`hole_standard.rs`) is the ISO metric size and fit the sizes were taken from
  (`HoleStandard`: `MetricSize` M1.6 to M20; `HoleFit` close, normal, loose clearance per ISO 273,
  tapped at the coarse-thread tap drill, tapped fine at one of the size's ISO 261 fine pitches
  (`FinePitch` by its place in `fine_pitches`: the first is the pitch older versions offered, so
  M12 and M20 list 1.25 and 1.5 before the others; drilled at the major diameter less the pitch),
  or a heat-set insert), with counterbore and countersink sizes for socket and countersunk heads.
  It names the thread of a tapped hole and gives each round bore its cosmetic thread (Thread
  below), but changes nothing of the solid: the expressions alone drive the hole. `MetricSize::offers` says which fits a size has (`fits` for the Fit row,
  `fine_fits` for its pitches); `HoleStandard::offered` falls back to the first fine pitch, or to
  normal clearance, when a size lacks the fit asked for.
- Heat-set insert bores (`HeatSetInsert`: hole, insert length, least wall) are those published for
  the common standard brass inserts for prints (CNC Kitchen's standard series, which most others
  match): M2 3.2 mm for a 3 mm insert, M2.5 4.0 for 4, M3 4.0 for 5.7, M4 5.6 for 8.1, M5 6.4 for
  9.5, M6 8.0 for 12.7 and M8 9.7 for 12.7, with walls of 1.3, 1.6, 1.6, 2.1, 2.6, 3.3 and 3.3 mm.
  M1.6 and M10 up offer no insert.
- `sizing` is `Typed` (every hole the typed diameter), `Circles` or `CirclesAndHeads`: a hole at a
  circle's centre takes that circle's diameter (`circle_sizes`, the smallest of several on one
  centre point, construction circles left out) and its style is checked against it, a refusal
  naming the circle; holes at points keep the typed sizes. `CirclesAndHeads`, what the app sets,
  also scales the counterbore's diameter and depth, or the countersink's diameter, by the circle's
  diameter over the typed one, so one counterbore setting suits circles of several sizes;
  `Circles` keeps the typed counterbore or countersink, as holes sized by circles always did.
- `bottom` is `Flat` or `DrillPoint(angle)` (an expression, 118 deg in the app): the bore ends in the
  cone of a drill point, its apex `radius / tan(angle / 2)` below the depth, so the depth stays
  what is counted to the full diameter, as a drill leaves it. The cone is the revolved profile's
  last segment, so it keeps the bottom part and its name. It applies to blind round holes only: a
  through hole is unchanged (the drill point lies past the body) and a slot with one fails alone
  saying it ends flat; the angle follows the countersink's range (above 0° to `MAX_CONE_ANGLE`).
- It fails alone, naming the point, when a size is not positive, the counterbore or countersink is
  not wider than the hole or as deep as it, the angle is outside 0° to `MAX_CONE_ANGLE`
  (179°, also the Hole panel's field rule), the sketch has no
  points, more than `MAX_HOLES`, or a hole does not cut into the body (it adds no face).

### Thread (`thread.rs`, `thread_standard.rs`)

- `Thread { body, face, size, class, hand, length, reversed }` is a cosmetic thread: it refers to
  a cylindrical face by its `FaceReference` (pieces of one split face accepted, like a shell's),
  resolved in the body's state at its place, and changes no solid. Its result is
  `FeatureResult::Thread` (`ThreadResult`: the side, the designation text, the evaluated depth and
  the `ThreadPlacement` at its place). Its body is used (`body_input`, so `bodies_used`) but it is
  not a body state (`Feature::body` is none), like a datum; the face's origin features are its
  `origin_features` and healing matches the face like any reference. It can be hidden.
- `Bore::of` reads the face: internal when the face's sense is reversed (its normal points at the
  axis, a bore), external otherwise (a shaft or boss); the axial extent from its edges; and which
  ends are open (an edge there meets a face whose normal points away from the face's span: a
  hole's mouth or counterbore floor, a shaft's free end or chamfer, never a blind bottom or a
  shoulder). The thread starts at the open end (the upper end along the axis when both or neither
  are), or the other with `reversed`, and runs the whole face or `ThreadLength::Depth` (a length
  expression above zero; one past the face stops at its end).
- It fails alone, in words naming the face or body, when the face is lost or tied, is not
  cylindrical, the class is not one the standard offers for the face's side, or the face's
  diameter does not fit the size (`ThreadSize::fits`: a bore between the minor diameter less a
  pitch and the major diameter, a shaft between the minor and the major plus a pitch), suggesting
  the nearest size of the standard (`ThreadFamily::nearest`) when it fits.
- The standards (`ThreadFamily`): ISO metric coarse and fine (ISO 261, M1.6 to M64, every fine
  pitch of each size), ISO 228 parallel pipe (G 1/16 to 6), ISO 7 taper pipe (R 1/16 to 6) and
  ISO 2901 trapezoidal (Tr 8x1.5 to Tr 100x12 at the preferred pitch). Minor diameters follow the
  standard's profile (metric D1 and d3, the Whitworth form for pipes, ISO 2904's clearance for
  trapezoidal, whose internal major is D4). `ThreadClass` is one of the classes the standard offers
  for a side (metric 6H, 5H, 7H, 4H, 6G and 6g, 6h, 4h, 8g, 6e, 6f; ISO 228 the one internal class
  and A or B; ISO 7 Rc or Rp and R; trapezoidal 7H, 8H and 7e, 8e, 7c, 8c), the first the default.
  `ThreadDesignation::text` writes each standard's notation: `M8-6g`, `M8x1-6H-LH`, `G 1/2 A`,
  `Rc 3/4`, `R 1 LH`, `Tr 20x4 LH-7e`.
- `placed_threads` places every thread for showing and exporting on the bodies as they finally
  stand, not at the feature's place, so a thread follows a later move, mirror or scale of its body
  (names kept) and lands on the body a combine joined it into; a face no longer found there is
  not placed. Failed threads are not placed. A tapped round hole (`hole_thread`: a `standard`
  with a tapped or fine fit, its size found in the metric tables) places one thread per wall face
  (`Hole::is_wall`, grouped by copy and entity so a split bore is one thread), the copies a
  pattern or mirror makes of the hole (`FaceOrigin::Copy` of its walls) included, as its
  `TappedThread` says: the class (none is the family's first internal class, 6H), the hand, and
  the whole wall or a depth (a length expression, a parameter user like the hole's sizes) from the
  bore's open end, one past the wall stopping at its end. The hole fails alone, in words, when the
  class is not an internal one of the family or the depth is not a length above zero.

### Move (`movement.rs`)

- `Move { body, offset, turn, copy, about }` places an existing body: turns about the X, Y then Z
  axes through the origin, or with `about: TurnCentre::Body` through the centre of the body's box
  as it stands before the move (`Move::pivot`), then shifts by the three distances, all
  expressions (any sign). With `TurnCentre::Axis` (an `AxisTurn`: an `AxisReference` resolved like
  a pattern's at the move's place, and an angle expression) it first turns by that angle about
  the axis, in the sense of the axis's direction, then by the three turns about the axis's point
  (`Pivot::Axis`), then shifts; the axis's body, datum and sketch count as used and its edge or
  face is healed like any reference. With `frame` (a coordinate system) the turns are about that
  system's X, Y and Z axes, through its origin when turning about the origin, and the distances
  run along its axes (`MoveAxis::direction_in`); the system counts as used. It modifies
  its body like a blend does, keeps every face and edge name (`Solid::transformed`) so references
  held through it survive, and fails alone when a value is not a length or angle or the result is
  not finite. With `copy` it leaves the body alone and makes a new body of its own (`makes_body`,
  `Feature::body` is the move itself) from the placed copy; a copy others use cannot stop being
  one.

### Mate (`mate.rs`)

- `Mate { body, pair, flipped }` places an existing body by its geometry: `MatePair::Faces` (a
  `FaceReference` of the body, which must resolve to one plane, a `PlaneReference` target and a
  length expression `distance`) or `MatePair::Axes` (two `AxisReference`s, the first normally an
  edge or round face of the body). References are resolved at the mate's place, the moving ones
  on the body as it stands before it, so the mate follows its target on every recompute.
- Faces: the body turns about its box centre by the least turn taking the face's outward normal
  opposite the target's (the same way when `flipped`; a half turn uses the face's x axis), then
  shifts along the target normal until the face lies `distance` beyond the target plane, so it
  keeps its place across the plane. Axes: the body turns about the point of its axis nearest its
  box centre by the least turn onto the target's direction (reversed when `flipped`), then shifts
  square to the target so the lines coincide.
- It modifies its body like a move (`modifies_body`, state before kept), keeps every name
  (`Solid::transformed`), and its target bodies, datums and sketches count as used like a move
  axis's; healing visits the moving face and both references. A lost, split or curved moving face
  fails it naming the face; a placement too far fails it in words.

### Mirror and scale (`mirror.rs`, `scaling.rs`)

- `Mirror { body, plane, keep_original }` reflects its body across a `PlaneReference` resolved like
  a datum's at its place (its datum and face body count as used, its face is healed). Alone it
  replaces the body by the image (`Solid::mapped`), keeping every name; keeping the original runs
  the kernel `pattern` with the one image as copy `MIRROR_IMAGE`, so the image's faces are
  `FaceOrigin::Copy` of the mirror and `describe_origin` words them "<mirror> image of …".
- With `mirrored` features (`Mirror::mirroring`, chosen as a pattern's repeated ones are, by
  `repeatable_on`) it reflects their tools instead of the body, as a pattern of features repeats
  them: each tool is placed as copy `MIRROR_IMAGE` (kernel `pattern_copies`) and cut from or
  joined to the body as it stands, the originals staying; `keep_original` is then ignored. The
  lookup (`pattern::seed`, worded through `SeedWords`) is shared with the pattern, so the same
  failures name the feature with the fix on it, the mirrored features are in `features()` and an
  edit to them recomputes the mirror. The cut images are the mirror's own `cuts`.
- `Scale { body, factor, center, frame }` resizes its body about a point (`Solid::mapped`),
  keeping every name. The factor is a plain number from `MIN_SCALE_FACTOR` to `MAX_SCALE_FACTOR`;
  the centre is three lengths, from the origin along the world's axes or, with `frame` (a
  coordinate system, counted in `frames_used`), from its origin along its axes. A result past
  `MAX_SIZE` or with an edge below the resolution fails it alone, saying which.
- Both change their body but, like a pattern, are not `modifies_body`: nothing is chosen on the
  state before them, so the app shows the result while one is open.

### Split (`split.rs`)

- `Split { body, along, flipped }` cuts its body along a `SplitAlong`: the part on one side stays
  in the body (the other side when `flipped`), and the rest becomes a body of the split's own
  (`makes_body`, named after it). Both come from one tool solid, intersected and subtracted, so
  the two pieces share the cut face's name.
  - `Plane`, a `PlaneReference` resolved like a mirror's: the side its normal faces stays. The tool
    is a half-space block (a rectangle on the plane past the body's box by `HALF_SPACE_REACH` of
    its diagonal plus `HALF_SPACE_MARGIN`, extruded to past the far side).
  - `Sketch`, an earlier sketch whose non-construction curves form one open chain joined end to
    end (no circle, branch, loop or second chain): the chain is carried on straight along its end
    tangents to a rectangle around the body and the curves (margins as the plane's), closed along
    that rectangle on the left of the chain walked from its first end (the end on the curve of
    lowest entity id, that curve's start if both are on it; the right when `flipped`), and the
    region is extruded under the split's id from below the body to above it along the sketch
    normal (`swept_half_space`); the closing lines take entity ids from `CLOSURE_ENTITIES`. A chain
    whose closed outline is not one region (it crosses itself or its extensions cross) fails.
    The sketch is in `reference_sketches`, so an edit refuses a feature that is not a sketch.
  - `Body`, another body as it stands at the split, used whole as the tool: what lies outside it
    stays, so its faces split along their whole surfaces. It counts in `bodies_used` and is left
    as it was; splitting a body along itself fails.
  - A tool that misses the body (an empty side) fails it in words naming the plane, curve or
    body; a sketch's own problems (no curve, closed, not one chain, crossing itself) fail it with
    the fix on the sketch.
- It is the one feature whose result holds two bodies: the kept part is the result's own
  `SolidResult` and the split-off part is in `SolidResult::others`. `Feature::bodies` lists both
  (settling counts both), the walk stands each part under its own body (`body_parts`), and
  `Evaluation::body_result` and `body_seen_by` find a body inside its state's result
  (`body_part`), so later features, meshing, stale bodies and the app see two ordinary bodies.

### Split face (`split_face.rs`)

- `SplitFace { body, faces, along }` divides the chosen faces of its body (`FaceReference`s
  resolved in the body's state before it like an offset face's, pieces of one face accepted, a
  lost or tied one failing it) along a `SplitAlong` (as a split's), without changing the shape
  (kernel `split_faces`): a plane's half-space block (`split::half_space_solid`, the side its
  normal faces being `Inside`), a sketch of one open chain carried on past its ends
  (`swept_half_space`, the left side inside), a sketch of closed outlines extruded through the
  body (`swept_outlines`, the outlines inside), or another body as it stands (inside it). It
  modifies its body (`modifies_body`, state before kept for choosing); its plane's datum and face
  body, the tool body and the sketch count as used like a split's, and healing visits its faces
  and its plane.
- A tool crossing none of the chosen faces fails it in words naming the plane, curve or body;
  splitting along its own body, a sketch's problems and kernel failures (through `trouble.rs`)
  fail it alone.

### Remove (`removal.rs`)

- `Remove { body }` takes a body out of the model from its place on: its result passes the body's
  state through and `consumed_bodies` drops it, as a Combine drops its tool, so suppressing,
  rolling back or deleting the removal brings the body back. A feature still using the body fails
  naming the removal (`Inputs::missing_body`).

### Combine (`combine.rs`)

- `Combine { body, tool, more_tools, keep_tool, operation }` joins, cuts or intersects existing bodies
  (kernel `boolean`) and keeps the target's body ID. `Combine::tools` is `tool` then `more_tools`;
  the target meets them one after another, so one cutter cuts several bodies' worth of tools at
  once and several bodies join in one feature, and a failure names the tool it stopped at. Unless
  `keep_tool` is set the tools' bodies are consumed: `consumed_bodies` tells recompute to drop them
  from the bodies standing after the combine (and not to show them as stale last-good bodies), so
  features below cannot use them and suppressing or rolling back the combine brings them back. A
  kept tool stays a standing body, so a later Combine can use it again on another target. Every
  body counts as used (`bodies_used`), so deleting any asks first and none moves below the combine.
  `bodies_before` lists the bodies standing before a feature, for the panel.
- Every evaluator reports an absent input body through `Inputs::missing_body`: when a Combine already
  evaluated consumed it, the error names that Combine and the body it kept, with the fix target on
  the Combine, never blaming the healthy body.
- A body combined with itself, a tool chosen twice, a missing shape and kernel failures fail the
  combine alone with the bodies named; every input keeps its result.

### Pattern (`pattern.rs`)

- Repeats the whole body as it stands before the pattern and unions the copies (kernel `pattern`).
  Boxed in `FeatureKind`, which it would otherwise double in size.
- With `repeated` features (`Pattern::repeating`; `repeatable_on` says which: an extrusion,
  revolve or primitive adding to or removing from a body, or a hole) it repeats their tools instead: for each,
  in tree order, the tools kept with its result (`SolidResult::cuts` of a removal or a hole,
  `joins` of an addition) are placed at every copy and unioned without the original (kernel
  `pattern_copies`), then cut from or joined to the body as it stands, so the original feature is
  never repeated onto itself. The repeated features are in `features()`, so they stay above the
  pattern and a change to their tools recomputes it (`same_shapes` compares cuts and joins too).
  A repeated feature whose result is missing, that changes another body or that neither adds nor
  removes (a feature making the body) fails the pattern naming it, with the fix on that feature.
  A copy that misses the body changes nothing. The cut copies are the pattern's own `cuts`, so an
  open pattern of holes shows them like a hole's drills; their faces are `FaceOrigin::Copy` of
  the pattern as a body pattern's are.
- Axes are `AxisReference`s resolved like a datum's at the pattern's place in the tree; their body
  and datum count as used.
- Counts are whole and from 1, at most `MAX_PATTERN_INSTANCES` instances in all (directions
  multiplied), since each copy costs a boolean. The reach may not pass `MAX_SIZE`; parallel
  directions are refused.
- A linear direction's length is `measured` between neighbouring copies or, as
  `LinearSpacing::Total`, from the body to the last copy, shared evenly by the steps.
- `skipped` holds the `Instance`s (steps along each direction, as in copy names) left out. The
  original (`ORIGINAL_INSTANCE`) can never be; one past the current counts is kept but changes
  nothing, so lowering and raising a count brings the same copies back. The copies made keep their
  indices and names, so references to them survive leaving others out.
- It changes its body but is not `modifies_body`: nothing is chosen on the state before it, so the
  app shows the patterned body while it is open.
- `PatternKind::Curve` (`CurvePattern`) follows an earlier sketch whose non-construction curves
  form one chain joined end to end (`pattern_path.rs`, `curve_path`), open or closed (a circle, a
  closed spline or a loop of curves); a branch, a second chain or a curve of no length fails the
  pattern with the fix on the sketch. An open chain is walked from its end on the curve of lowest
  entity id (that curve's start if both are), a closed one from the start of its lowest curve, the
  other way when `reversed` (an open one from its far end). Copy `[i, 0]` stands at arc length `i`
  steps along it: `CurveSpacing::Spread` shares the whole length (over `count - 1` steps, or
  `count` round a closed chain), `Distance` is the `spacing` expression, and copies reaching past
  the end (or round onto the original) fail it saying how long the curve is. Each copy is the
  original moved as the walk's start point would move to that station (`along_curve`), turned
  about the sketch normal through the start by the change of tangent when
  `CopyOrientation::Following`. Arc length is exact on lines and arcs and found by bisection on
  splines and ellipses.
- `PatternKind::Points` (`PointsPattern`) places a copy at every lone point of an earlier sketch
  (`Sketch::free_points`, solved), moved from `base`, a `PointReference` resolved like a datum
  point's (its datum, frame, body, sketch and origins count as used, its edge or face healed).
  A point on the base point is the original and makes no copy; a sketch with no lone points fails
  the pattern with the fix on the sketch. A copy's index is its point's id plus one split into two
  words (`point_instance`, `instance_point`; the original stays `[0, 0]`), so adding or deleting
  points never renames the others, and copies are worded "the copy at point N"
  (`Pattern::instance_words`, `describe_origin`). `instances` is none, as they are no grid.
- Both sketches are in `features()` and `reference_sketches` (an edit naming a feature that is no
  sketch is `NotASketch`), so an edit to the sketch recomputes the pattern and it stays below it;
  a sketch that failed fails the pattern naming it. Features are repeated through them as through
  the other kinds, and `SetFeatureKind` may switch a pattern between all four kinds.

### Sketch and datum (`attachment.rs`, `datum.rs`)

- A sketch keeps its `Sketch` and optionally a `SketchAttachment`: a datum plane it follows, a
  plane of a coordinate system (`Frame`), or on a body a `FaceAttachment`. The stored plane is where it was placed; recompute resolves the reference
  in the body's state at the sketch's place (`FeatureKind::body_input`) and gives the solved
  geometry the face's plane, normal and frame. Fragments of a split face are accepted when in one
  plane; a lost, split or curved face fails the sketch alone with a fix pointing at it.
  `SetSketchPlacement` sets plane and attachment together. A body with attached sketches cannot
  stop making a body.
- A sketch's `projections` (`projection.rs`) map a projected entity to its `ProjectionSource`: an
  edge of a body (an `EdgeReference`), a corner (its `VertexName`), an entity of an earlier
  sketch, a section edge or a datum plane's line. Before solving, recompute places each in the sketch plane from the source as it stands at
  the sketch (`refreshed`, in the body's state at that point): a line, a circle or arc lying in a
  parallel plane (turned to stay counter-clockwise), a point, or otherwise a spline through
  `PROJECTED_SPLINE_POINTS` samples; another sketch's spline maps its control points exactly, and
  its ellipse or elliptical arc is always that spline through samples. The
  entity keeps the kind and point count it was made with, so a source that now projects to another
  kind, is missing, was split ambiguously (pieces of one line merge) or is unavailable fails the
  sketch alone, naming the entity and the source. Source bodies count in `bodies_used`, source
  sketches in `features()` and edge origins in `origin_features`, so recompute reuses the sketch
  only while they are unchanged and a source stays above it. `TransactionBuilder::add_projection`
  adds an `Outline` (`edge_outline`, `vertex_outline`, `sketch_outline`) as points, a curve and
  its source in one transaction. A sketch's evaluation records every body standing at it
  (`Evaluation::body_result_seen_by`), so the app projects from that state.
- Intersect sources (`section.rs`): a `Section` is an edge of the body cut by the sketch plane,
  the body intersected with the half-space above the plane (`split::half_space_solid`, the one
  Split uses, extruded under the sketch's own feature id). Its edges between the cut face (a cap
  whose origin is the sketch) and a body face are the section curves, each named by the kernel
  from the body face it crosses (`EdgeName::between` and its disambiguation) and kept as an
  `EdgeReference` with the cap's origin dropped, so `origin_features` never names the sketch.
  `refreshed` cuts each body once per recompute; a reference no longer found means the plane no
  longer cuts that face, a failing cut fails the sketch alone and cancelling stops it. A
  `DatumPlane { datum, reach }` is the line where an earlier datum plane crosses the sketch
  plane, centred where it passes nearest the sketch origin and `reach` long either way;
  `datum_outline` gives none for a parallel plane. Datum sources count in `planes_used`, so an
  edit refuses one that is not a datum plane (`NotAPlane`). A `PrincipalPlane { plane, reach }`
  is the same line for the XY, XZ or YZ plane; it has no source feature (`ProjectionSource::feature`
  is `None`), so only the sketch's own plane moves it and a failure names the plane.
- Datums are planes, axes, points and coordinate systems with a `DatumResult`, referring to model
  geometry in each body's state at the feature's place in the tree. Edits refuse a sketch or plane
  based on a non-datum-plane (`NotAPlane`), an axis reference to a non-datum-axis (`NotAnAxis`), a
  point reference to a non-datum-point (`NotAPoint`), a reference to a coordinate system's axis or
  plane naming another kind of feature (`NotACoordinateSystem`, through
  `FeatureKind::frames_used`, which joins `features()`) and a sketch point or line of a feature
  that is not a sketch (`NotASketch`, through `FeatureKind::reference_sketches`). `Datum::kind`
  (plane, axis, point, frame) is what `SetFeatureKind` keeps, so a plane may switch between offset
  and through forms.
- A coordinate system (`Datum::Frame`, `DatumFrame`, titled "Coordinate system") has an origin (a
  `PointReference`), an X axis (an `AxisReference`, an edge included) and an XY plane (a
  `PlaneReference`); `datum_construction::coordinate_system` takes Z as the plane's normal and X
  as the axis's direction laid into the plane, each reversed by `reverse_x` and `reverse_z`, Y
  completing a right-handed frame, so its XY plane is parallel to the chosen plane through the
  origin. An axis square to the plane fails in words naming both. Its result is
  `DatumResult::Frame(Plane)`, the frame as its XY plane. `AxisReference::Frame` and
  `PlaneReference::Frame` name one of its three axes or planes, placed by
  `PrincipalAxis::in_frame` and `PrincipalPlane::in_frame` (the principal geometry carried by the
  frame), so they work wherever an axis or plane reference does; `displayed_frame` and
  `displayed_plane` read them for drawing. A `PointReference::Datum` cannot name one;
  `PointReference::Frame` names its origin (`frames_used` through `Datum::frames`, so naming
  another kind of feature is `NotACoordinateSystem`), worded "the origin of …".
- A `PointReference` is the origin, a coordinate system's origin, a datum point, a body corner (`VertexName`, resolved when
  exactly one vertex has it), the centre of a round edge (an `EdgeReference`, pieces of one circle
  accepted), the centre of a spherical or toroidal face (`SurfaceCentre`, a `FaceReference`, pieces
  of one surface accepted) or a sketch point (in its solved plane); `PointReference::on_body`
  places the three body kinds on a given solid, for the app. An `AxisReference::Sketch` is a line of an
  earlier sketch. Sketches they use join `features()`, so recompute reuses the datum, pattern or
  revolve only while the sketch is unchanged.
- `Datum::Point` places a `DatumPoint` at a point reference moved by three length offsets.
  `Datum::PlaneThrough` passes through three points (refused when they lie on one line), lies
  `Midway` between two planes (halfway between parallel ones, else on the bisector of the acute
  angle), contains an axis and a point (refused when the point is on the axis) or stands square to
  an axis at a point. `DatumAxis` also runs through two points or stands square to a plane through
  a point. Each refusal names the references and what to choose instead.
- `datum_construction.rs` holds the constructions that need geometry of a body or line crossings.
  `PlaneThrough::Tangent` (`FaceTangent`: body, `FaceReference`, a `PointReference`) touches a
  cylindrical or conical face along the line nearest the point, its normal facing the point's side
  (refused when the point lies on the face's axis, or the face is no longer a cylinder or cone).
  `PlaneThrough::SquareToCurve` stands square to an edge of any curve kind at a `CurveStation`
  (body, `EdgeReference`, a length expression): the distance is arc length from where the edge
  starts, counted back from its end when negative, and a distance past either end fails alone
  naming the edge's length. `PlaneThrough::Lines` holds two lines (`AxisReference`s) that cross or
  run parallel; the same line, or two that neither cross nor run parallel, fail naming both. A
  `Datum::PointBy` is a datum point with no offsets: `LinesCross` (parallel or skew lines fail),
  `AxisAndPlane` (a line parallel to the plane, or lying in it, fails), `ThreePlanes` (two parallel
  or all three sharing a line fail) and `Along` a `CurveStation`. The stations' bodies and edge
  origins count as used like an axis edge's, their distance is a parameter user, and healing
  matches their edges and faces like any reference.
- Curved faces at a point (a `FaceTangent`'s face and `toward` point, `face_foot`): the point of
  the face's surface nearest the point (`Surface::project`) and the face's outward normal there,
  turned to face the point's side. `PlaneThrough::TangentAt` touches the face there and
  `DatumAxis::SquareToFace` stands square to it there, on any curved face (sphere, torus, spline,
  revolution, extrusion, cylinder, cone); a flat face, a point at a sphere's centre or on the axis
  of a round face (every side equally near) fail alone in words. `PointBy::EdgeMiddle` sits halfway
  along an edge by arc length, and `PointBy::FaceCentre` at the area centroid of a face (every
  piece of a split one), taken from the body tessellated at `MeshQuality::SMOOTH`, so a curved
  face's centre may lie off the face, as a cylinder's lies on its axis. Their bodies and the
  origins of their edges and faces count as used and healing matches them.
- Whether geometry lies on a line or plane, runs along a plane or is parallel is decided in one
  place (`tolerance.rs`) for revolve axes, datums, attachments, patterns and blend pieces, so noisy
  imported geometry is accepted or refused the same way everywhere.

### Import (`import.rs`)

- Keeps the source file's name, the path it was read from (`Import::path`, set by
  `Import::from_file` only for a path that is valid UTF-8, so it can be stored), the solid and the
  canonical single-solid STEP text it was read from (what the file stores), both `Arc`s that the
  copies of one part share (`Import::shared`); equality compares the
  name, path, text and placement. The body is `Solid::imported`, so later
  features hold its faces and edges like any body's.
- Its `placement` (`BodyPlacement`, `movement.rs`) turns the body about the X, Y then Z axes
  through the origin, then shifts it, as a `Move` does, all expressions (any sign) counted in the
  feature's parameters and inlined like any other. With `frame` (a coordinate system, in
  `frames_used`) the file's origin and axes are the system's: the body is turned and shifted in
  it, then carried to it (`RigidTransform::from_frame`). Its `scale`, a plain factor expression
  (1 by default, `is_unscaled`), first resizes the solid about the file's origin
  (`Solid::mapped`, from `MIN_SCALE_FACTOR` to `MAX_SCALE_FACTOR`, else the import fails alone),
  so a part in the wrong unit is fixed in place. Zero everywhere, scale 1 and no frame
  (`is_at_origin`) leaves the solid as read; otherwise `Solid::transformed` keeps every face and edge name, so references held
  through the import survive moving it. A value that is not a length or angle, or a body placed
  too far, fails the import alone.
