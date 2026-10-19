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
  returns the inverse. `Document::check` runs a transaction on a clone so the UI can report an
  error first.
- `Editor` keeps undo and redo as stacks of inverses bounded by `MAX_UNDO_STEPS` and
  `MAX_UNDO_BYTES`, measured by `Transaction::approximate_size` (heap data through `heap_size`
  methods; anything new an edit can hold must be counted there). `Sketch::heap_size` destructures
  the sketch exhaustively so a new field fails to compile until counted. Oldest dropped first,
  newest always kept.
- `Editor::apply` drops a transaction that leaves the content as it was (`same_content`, which
  ignores ID counters and decides whether a model is unsaved): no undo step, redo kept, no
  revision bump, not journaled, not recomputed.
- Undo steps carry a serial from a per-editor counter. `Editor::undo_mark` notes the newest step,
  the next serial and how many steps the bounds dropped; `undo_steps_since` gives the steps made
  since the mark, newest first, or none once the stack no longer leads back to it. The app's
  Cancel of an open feature relies on it.
- Off-thread preparation: `Editor::base` hands out the document with the revision, `Base::prepare`
  applies a transaction to that copy, `Editor::commit` swaps it in only if the revision is
  unchanged, else `Stale`.
- Edits carry their IDs, so redo restores them; counters never move backwards. Parameter and
  feature IDs stay below `FIRST_UNSTORABLE_ID` (`ReservedId`); the IDs at or above it name the bodies
  of separate pieces (Separate pieces below), never a feature.
- Edits refuse to break invariants: references to a feature below the one using it or to itself
  (an insert naming its own id too), an insert landing below a feature already using its id
  (`BelowUser`, found through `FeatureList::users_of`), unknown parameters, parameter cycles, deleting a parameter in
  use, moving a feature past one it uses by `features()` or a measurement it reads (what inserts
  and kind changes check, `Measured::features_read_by`), two features sharing a name (loading
  renames the second, with a report). Weaker ties (`origin_features`, a Combine consuming a used
  body) are not invariants, since inserts and kind changes may break them, so `MoveFeature` does not
  refuse them and the inverse of a move always applies; `move_row` and `move_features` do (Rollback
  bar below). A parameter the
  configurations name (`Configurations::parameters_named`: a configured column or a cell of any
  row) is in use: `used_parameters` and `parameter_users` count it and `RemoveParameter` refuses it.
- Deleting a feature others use is allowed: they keep its ID and fail with a reason until the
  deletion is undone. So a feature may be inserted or changed while referring to a feature not in
  the document; the feature ID counter is raised past every referenced ID. `dependents_of` gives
  every user of the given features in tree order; the app asks before deleting one that has any.
- At most `MAX_RECORDS` parameters and features together (what a file is opened with; `load.rs` in
  `caditor-file` leaves the rest out). `apply` refuses a transaction leaving more than that, and
  more than before, as `TooManyRecords`, so a model already past it can still shrink.
- `ParameterList` indexes parameters by ID and name; the `DependencyGraph` survives inserts and
  removals within an `apply`, keeping version restores and cycle checks near linear. Order is the
  user's (`MoveParameter`) and changes nothing computed; `note` (`SetParameterNote`, trimmed, at
  most `MAX_PARAMETER_NOTE_CHARS`) is free text.
- `FeatureList` (`feature_list.rs`) indexes features by ID and names (`name_taken_besides`), kept
  by every insert, removal and move, so `Document::feature` and `feature_index` are lookups (the
  app calls them per body and row; scans were quadratic on imports of thousands of bodies). The
  features making or changing each body (`Feature::body`) are grouped on first ask
  (`Document::features_on_body`, read by `face_splits` and the scene's body health) and forgotten
  by any change to the list or a feature in it. The users of each ID by `features()` (`users_of`,
  present or not) are gathered on first ask, then kept by inserts and removals and forgotten by any
  change to a feature, so loading and `transaction_to`, which insert every feature, stay linear.
- `Document::in_use_order` (`use_order.rs`) puts features about to be inserted in an order every
  insert accepts: a stable topological order by `Measured::features_read_by`, the earliest stored
  ready feature first, returning the list unchanged after one pass when nothing is used from
  above. `UseOrder::lowered` names each feature placed below a feature it was stored above
  (loading reports them, `file-format.md`); features using each other in a circle are taken in
  stored order once nothing else is ready, and are not reported.
- `Document::inline_parameter` builds the one transaction deleting a parameter in use: its
  expression (not value, so links survive) is substituted into every parameter, sketch dimension,
  feature and body density using it (`Expression::inlining`, round-tripping through stored text to
  keep precedence), then the parameter is removed. When configurations name it, one
  `SetConfigurations` first writes its value into the other rows' cells and drops its column and
  cells. An expression growing past what stored text reads back refuses as `InliningTooLong`.
- `Document::transaction_to` builds the transaction turning one document into another (bar to the
  end, everything removed, the target's inserted with IDs, flags, notes and owners, bar restored),
  never lowering a sketch's ID counter; it restores an earlier version as one undoable change. When
  the configuration tables differ, or the current one names a parameter of the model, it first
  deactivates and empties the current one, and sets the target's at the end.

### Model scale (`model_scale.rs`)

- `Document::scaled` builds the one transaction resizing the whole model by a `ModelScale` (factor
  from `MIN_SCALE_FACTOR` to `MAX_SCALE_FACTOR`, not 1, about a centre), rewriting stored values in
  place so no ID, name or reference changes and undo is one step: sketch planes (`Plane::mapped`),
  points, radii, `Fix` positions, label offsets and projection reaches are multiplied, every
  length expression of sketches and features scaled; angles, counts and plain factors never change.
  World values a feature holds (an unattached sketch's plane, a datum plane offset from a principal
  plane, a datum point from the origin, a primitive's place on a principal plane, a scale's centre,
  an import's offsets, a move about the origin) also take the shift a centre off the origin brings.
- Principal geometry a feature places itself by (mirror or split plane, revolve, circular pattern
  or move axis, up-to plane, primitive plane, datum references, a frame's or points pattern's
  origin, mate targets; `anchored`, one mutable walk shared by the check and the rewrite) cannot
  move. When the centre does not hold it (`displaced_by_scale`, shown by the app beforehand) the
  same transaction inserts datums at the top of the tree where it lands and repoints the
  references: a datum point "Origin, scaled", for planes and axes a coordinate system "Axes and
  planes, scaled" (exact images, frame origins included); a datum plane offset from a principal
  plane that something uses is rebased on the frame's plane with only its offset scaled; a sketch
  projecting a principal axis gets a datum axis "<axis>, scaled" along the frame's axis and one
  projecting the origin the "Origin, scaled" point; a sketch
  projecting a principal plane gets a datum plane "<plane>, scaled", since a projection cannot name
  a frame's plane. `ScaleSummary::moved` and `datums` list what moved and was added. Saved views
  follow (target mapped, distance multiplied).
- The transaction ends with one `SetConfigurations` rewriting the length cells of inactive rows
  through the same rescaler (`Rescaler::configurations`: every cell when parameters are scaled too,
  else only cells of named values without parameters of their own), leaving angles, counts,
  suppression, colours and the active row alone.
- `ScaledValues::Plain` scales parameter-free values and named values (owned length parameters
  with none of their own); a value using any other parameter follows it and is listed in
  `ScaleSummary::kept`. `AndParameters` also rewrites every parameter in evaluation order by its
  length power. Each expression goes through `Expression::with_lengths_scaled` and is kept only when
  it evaluates to factor-to-its-power times the old value, else becomes the exact form, so a value
  never silently changes size. A hole whose diameter changes loses its `standard`, and threads keep
  their size; both are in the summary.

## Model parameters (`model_parameters.rs`)

- A sketch dimension or feature value is named by making it a parameter: the parameter holds the
  expression and the field holds `Expression::Parameter` of it, so it is identified by the
  parameter's stable `ParameterId`, never by the field's place. A measured value reaches
  expressions only through a measurement's parameter (Measurement below).
- `Parameter::owner` (`ParameterOwner`): `Feature { feature, value }` (`value` the field's caption,
  one trimmed line, past `MAX_VALUE_LABEL_CHARS` refused as `ValueLabelTooLong`) or
  `Dimension { sketch, constraint }`. It is content like a note (`same_content`, `InsertParameter`,
  `transaction_to`, `heap_size`), ignored by evaluation and recompute, never checked against the
  model (a vanished owner reads as deleted). `Edit::SetParameterOwner` sets or clears it;
  `owned_parameter` and `owner_text` ("Extrude 1 · Distance") read it.
- `carrying_owners(transaction, feature, fields)` keeps named values with their fields across a
  change of kind: an owned parameter in a field of another caption and none of its own takes that
  caption; one the new kind no longer uses is released.
- `TransactionBuilder::add_owned_parameter` inserts one. `Transaction::substituting` replaces an
  exact expression wherever a `SetFeatureKind` (via `inlining::expressions_mut`), `SetDimension` or
  `SetParameterExpression` holds it.
- `releasing(transaction, ids)` appends what named values become once applied: one nothing uses
  (`parameter_user_ids`, counting configurations as `parameter_users` does) is removed, users
  before what they use; any other loses its owner. A removal `RemoveParameter` refuses leaves the
  value pending, so it still loses its owner. `deletion` releases values owned by deleted features,
  `remove_sketch_items` those of removed dimensions; reshaping a sketch keeps constraint IDs.

## Pasting features (`paste.rs`)

- `Document::paste_features` plans one transaction inserting copies at the rollback bar in tree
  order, each under a fresh id (`add_copied_feature`), named "<name> copy", keeping hidden,
  suppressed and appearance.
- A copy keeps its original's folder name, in this model or another, so a copied folder pastes as
  a folder and copies pasted right below the folder they came from join it. When the bar stands
  inside a folder (`group_around`) every copy joins that folder instead, since folders are runs
  and a copy named otherwise would split it. Features copied from no folder take none.
- `FeatureKind::rename_features` rewrites every plain feature id a kind holds, those inside axis,
  plane and point references, measured items, mate targets and datums included. Inputs inside the
  copy are renamed through placeholders above `PLACEHOLDER_BASE`; one still named as a plain id
  afterwards is `PasteRefusal::Unfollowed`, and users of a left-out feature are `LeftOutInput`.
- One still among the origin features (a face or edge reference names its feature in its digest,
  a section projection's edge included), or a body a corner (`VertexName`) is picked on, is a pick
  of copied geometry (`copied_picks` counts such features, and those of the next bullet).
  `paste_features` leaves the feature out as `CopiedGeometry`.
  `paste_features_finding_picks` (`paste_picks.rs`; `PickSearch` carries the evaluator, a
  `CancelToken`, progress and whether threads are shown cosmetic, as the model's recompute shows
  them; the app runs it on a copy of the recompute worker's cache) finds each pick again on the
  copies instead, feature by feature in tree order:
  - It recomputes the document with the copies planned so far on the `Recompute` it is given (the
    worker's cache, so only the copies and what follows them are evaluated, and each later run adds
    only the new copies) and reads each picked body as it stands there.
  - The original's side is the same evaluation when pasting into the same model with every copied
    feature unchanged and before the bar, else a document rebuilt from the copies alone under their
    own ids, parameters as literals (`rebuilt`; a copy using a feature outside it gives none,
    `PicksUnreadable`). The pick resolves there as the original resolves it, in the state the
    original feature saw.
  - Matching is by structure first: faces, edges and corners are keyed by the `FaceOrigin`s around
    them with the copied features renamed (`FaceOrigin::with_features`). One alike on each side is
    the match; else the origins of the neighbours (a face's neighbouring faces, an edge's ends)
    decide; only then the place (`same_places`: a face's edge middles and corners, an edge's ends
    and middle, within `SAME_PLACE`). Several left is `PickAmbiguous`, none `PickNotFound`; a copy
    that did not compute is `CopyUncomputed`. Each match is captured again on the copy and must
    resolve back to itself, so a pick is never moved to a wrong face.
  - A sketch's section projection (`ProjectionSource::Section`, `paste_sections.rs`) is found on
    the cut of the copy. The original's cut is `section_solid` of the body as the sketch saw it
    through the sketch's own plane; the copy's is the cut of the copy body through the plane of a
    trial of the pasted sketch (`Trial`: the pasted kind without its projections inserted at the
    bar on a clone, evaluated on the same cache, so a plane following a copied face or a
    parameter the target names otherwise is the copy's own). The trial's id is the id the copy
    will get, and both cuts are labelled with it (the original's mapped to it like a copied
    feature), because the cut faces' names, and so the edges' between them, hold the sketch's id.
    The edge matches by `choose` over the cut's edges, is captured again by `section_reference`
    (without the cut's own faces) and must resolve back to itself; a cut missing the copy is
    `PickNotFound`.
  - A corner picked on a body outside the copy (same model only) follows the copied faces around
    it: a feature picking one is searched when an earlier copy depends on that body
    (`corner_followers`), and `Mapper::vertex` moves the corner to the match on the copy body when
    the faces around the original corner name a copied feature (`Index::vertex_features`). One
    with no match, or around no copied face, keeps the original corner, since the copies may add
    nothing there.
  - A feature whose picks are not all found is left out with the reason; one still naming a copied
    feature among its origins afterwards is left out as `CopiedGeometry`. A cancel ends the whole
    paste as `PasteError::Cancelled`. The result is the same one transaction as `paste_features`.
- Inputs outside the copy are kept when pasting into the same model (`PasteOrigin::ThisDocument`)
  if they exist before the bar, else refused (`OutsideFeature`), the same id naming something else
  in another model.
- Parameters travel as `CarriedParameters`. `carry` keeps an id the target still has in the same
  model, else the target's parameter of the same name, else writes the copied value as a literal
  (`literal`; counted in `inlined`); a value of another kind refuses the feature in words.
  `Expression::substituting` replaces every id in one pass so swapped ids never collide.

## Importing parameters (`parameter_import.rs`)

- `Document::plan_parameter_import` turns rows of name, expression text and note into a
  `ParameterImport`: an `ImportOutcome` per row (Added, Changed, Kept when the model's own differs
  and `replace_existing` is off, Unchanged, Refused with an `ImportRefusal`) and one transaction, or
  none. Rows merge by name; an owned parameter keeps its owner; a note in the file replaces the
  model's, an empty one leaves it. Each row's text is parsed as typed text, names resolving.
- Rows are refused alone: a name `check_name` refuses or seen earlier, a note too long, text that
  does not parse (names resolve to the model's parameters and the file's other rows, so rows may
  refer to later ones), a cycle on the merged graph (every row in it refused, named as a path), a
  value that does not evaluate once applied (tried on a clone), a row using a refused new name.
  Refusing repeats until nothing more is refused.
- New parameters take ids in row order (gaps for refused rows) and are inserted at the end with a
  stand-in value before expressions are set, so rows may refer to each other in any order.

## Importing configurations (`configuration_import.rs`)

- `Document::plan_configuration_import` turns an `ImportedConfigurations` (columns with their
  headings split into `ColumnKind` and name, rows of a name and one cell per column) into a
  `ConfigurationImport`: a `ColumnOutcome` per column (Known, Added, Refused with a
  `ColumnRefusal`), a `RowOutcome` per row (Added, Changed and Kept with the `CellChange`s,
  Unchanged, Refused with a `RowRefusal`) and one transaction, or none when nothing would change.
  `replace_existing` off makes a row naming a configuration the model has, with other values, Kept.
- A column resolves to a `ConfiguredValue` by exact name (parameter, feature, or a body's
  `body_name`); refused when its heading has no known kind, the parameter, feature or body is not in
  the model (`NotABody` for a feature that makes none), more than one body has the name, an earlier
  column names the same value, or the model would configure more than `MAX_CONFIGURED_VALUES`.
  A resolved value the table does not configure yet becomes a column (`Added`), every existing row
  taking the live value for it, as `configuring` does.
- Rows merge by name ignoring case. A row is refused alone: no name, a name over
  `MAX_CONFIGURATION_NAME_CHARS` or seen on an earlier row, past `MAX_CONFIGURATIONS`, a non-blank
  cell that does not parse (`CellProblem`, naming the column), parameters of the row depending on
  each other in a circle (every parameter cell of the row set in the dependency graph, as
  `check_row_cycle` does one), or a parameter cell that does not evaluate once the row is switched
  to (tried on a clone, the row activated). Refusing repeats until nothing more is refused. A blank
  cell leaves the value as it is (the live value for a new row). A new row is appended and is not
  made active, so the import never changes the model through a row it adds.
- The transaction is one `SetConfigurations` of the merged table, plus, when the active row is
  among the rows written, the ordinary edits turning the model to its new values (`applying`, run
  on a clone that already holds the table), so the active row stays the live values and undo is
  one step.

## Flags and content records

- `hidden` (`Edit::SetFeatureHidden`) and `suppressed` (`SetFeatureSuppressed`) are undoable,
  journaled content carried by `InsertFeature`, so undoing a deletion restores them. Recompute
  ignores `hidden` and skips a suppressed feature as if absent. `SetFeatureHidden` of a piece's body
  sets that piece's flag in its feature's `piece_looks` (Separate pieces below).
- Hidden principal planes, axes and origin (`PrincipalGeometry`, `Edit::SetPrincipalHidden`) are
  content too, so hiding a mixed selection and undoing it are one change. The app tucking them away
  while no plane, axis or point is being chosen is view state outside the document
  (`app-modelling.md`, Visibility).
- Content in the `same_content`/`transaction_to`/`heap_size` family, ignored by recompute: a
  feature's `group`, `appearance`, `piece_looks`, `ModelProperties`, `SavedViews`, `SelectionSets`
  and `Configurations`. Each has one whole-set edit whose inverse holds the previous set. A
  feature's `pieces` is in the family too but is computed (Separate pieces below).

### Feature groups (`grouping.rs`)

- `group` is the name of the folder a feature sits in, or none (`Edit::SetFeatureGroup`, one
  trimmed line via `group_name`, past `MAX_GROUP_NAME_CHARS` refused as `GroupNameTooLong`).
- A folder is a run of consecutive features sharing the name (`group_run`), so membership needs no
  table and a folder is always contiguous. `grouping` names chosen features in one transaction,
  first moving them together under the first (`move_features`) when apart; `regrouping` renames or
  ungroups a run; `unused_group_name` gives "Group N".
- Moves keep folders whole: a feature moved (`move_row`, `move_features`) between two members of
  one group joins it, one moved where neither neighbour shares its group leaves it
  (`membership_edits`), in the same transaction. Inserts too: `add_feature` and
  `add_copied_feature` at a bar between two members of a group join it (`group_around`).

### Body groups (`body_folders.rs`)

- A body's group is `BodyAppearance::folder`, a path of group names from the outermost in, so
  groups nest and a group exists only while some body sits in it or below it; there is no group
  table. Each name is one trimmed line as `group_name` gives it (blank names dropped by `settled`),
  at most `MAX_GROUP_NAME_CHARS` (`GroupNameTooLong`), at most `MAX_BODY_FOLDER_DEPTH` deep
  (`BodyFolderTooDeep`). It travels in `SetBodyAppearance`, so undo, the journal, copying and the
  file need nothing else, and a piece without its own look sits in its feature's group.
- Body groups, unlike feature folders, are not runs: membership is a body's own path, order in the
  list stays the tree's. `body_folder_members(folder)` gives every body ID (a feature's own, and
  pieces with their own look) at or under a folder; `body_folder_names` every name in use, for
  `unused_body_folder_name` ("Group N", unique across all body groups).
- Every change is a set of `SetBodyAppearance` edits built by one `refoldering`, checked first:
  `body_grouping(chosen, listed)` makes "Group N" inside the deepest group the chosen bodies share,
  climbing out of every group whose listed bodies are all chosen (so grouping a whole group wraps
  it), each body keeping its path below that point (`BodyGrouping` gives the transaction and the
  new folder); `body_group_renaming` and `body_ungrouping` rewrite the folder's segment for every
  member (ungrouping moves what it held up one level, a rename onto a sibling's name merges them);
  `body_group_moving` puts a folder inside another (`BodyFolderIntoItself` for itself or one
  inside it); `bodies_moving` sets bodies' folder outright; `bodies_moving_out` takes each up one
  level. `folder_label` joins a path with " › " for the app's words.

### Model properties (`properties.rs`)

- `ModelProperties` (title, part number, revision, author, organisation, description, notes) is
  empty by default; only the user fills it. `Edit::SetModelProperties` sets them whole. Fields are
  trimmed; all but notes are one line; past `ModelProperty::max_chars` (`MAX_PROPERTY_CHARS`,
  `MAX_DESCRIPTION_CHARS`, `MAX_MODEL_NOTES_CHARS`) is `PropertyTooLong`.

### Saved views (`views.rs`)

- `SavedViews` (named views in the user's order, plus the redefined Isometric `home`); a
  `SavedView` is a camera's target, orientation and distance in model units, no projection (a
  preference). `Edit::SetSavedViews`: names trimmed, one line, unique ignoring case; refused as
  `ViewNameEmpty`, `ViewNameTooLong` (`MAX_VIEW_NAME_CHARS`), `ViewNameTaken`, `TooManyViews`
  (`MAX_SAVED_VIEWS`), `ViewNotUsable` (not finite, no distance). A unit orientation is stored as
  given, any other normalised, so setting the same views again is no change.

### Selection sets (`selection_sets.rs`)

- `SelectionSets` holds named `SelectionSet`s whose `SetMember`s are a whole body (`FeatureId`), or
  a `FaceReference` or `EdgeReference`, never a position. `Edit::SetSelectionSets` follows the
  saved views' name rules (`set_name`, `MAX_SET_NAME_CHARS`) and refuses `SetNameEmpty`,
  `SetNameTooLong`, `SetNameTaken`, `SetEmpty`, `SetTooLarge` (`MAX_SET_MEMBERS`), `TooManySets`
  (`MAX_SELECTION_SETS`). A set may name a body no longer in the model, so the feature ID counter
  is raised past every named body.
- `SelectionSet::resolve` finds what a set means in an `Evaluation` (taking the `Document` for the
  sketches' split pieces) and heals like any reference: a body is every face of its solid, a face
  or edge resolves by reference in the body's shown solid (a tie takes every candidate; an edge
  whose sketch curve was split takes every piece, `SplitEdges::follow`, `kernel-naming.md`). What it
  cannot find, a partly traced split edge included, is a `SetLoss` per body, which the app words.

### Configurations (`configurations.rs`)

- A table in the model: columns are `ConfiguredValue`s (a parameter's expression, a feature's
  suppression, a body's colour), rows are named `Configuration`s with a stable `ConfigurationId`
  from the table's own counter (`next_id`, never lowered, ignored by `same_content`), each with a
  `Setting` per column. `transaction_to` deactivates it first.
- One row may be `active`, and its settings are exactly the live values: one for every configured
  value the model has, none for one it lacks (a feature deleted, or one making no body for a
  colour). An edit changing a configured value or whether it exists (`SetParameterExpression`,
  `InsertParameter`, `SetFeatureSuppressed`, `SetBodyAppearance`, `InsertFeature`,
  `RemoveFeature`, `SetFeatureKind`) writes it into or drops it from the active row
  (`settle_active_row`; its inverse writes it back), and `SetConfigurations` settles the active row
  the same way. So undo, `transaction_to` and a saved file all see the row the model holds.
  `configuration_setting` reads a row, the live value for the active one.
- `Edit::SetConfigurations` sets the table whole (names trimmed, one line, unique ignoring case,
  `MAX_CONFIGURATION_NAME_CHARS`, `MAX_CONFIGURATIONS` rows, `MAX_CONFIGURED_VALUES` columns;
  settings of the wrong kind or for no column dropped). Columns may name things no longer in the
  model: such a column is skipped when switching and ID counters are raised past every ID named.
- `Edit::SetActiveConfiguration` changes the active row, refused (`ConfigurationOutOfStep`)
  when the row's settings are not the live values; a column the row has no setting for is not out
  of step and keeps its live value, which the row then records (a setting for a value the model
  lacks is dropped). When that settling changed the row, its inverse is a `SetConfigurations` of
  the table before. `activating` is the one undoable switch: none active, the row's
  settings applied as ordinary edits (values using parameters first set to 0, then those using none,
  then the rest, so values referring to each other never pass a cycle), then the row active.
- Builders each make one transaction: `adding_configuration`, `duplicating_configuration`,
  `renaming_configuration`, `moving_configuration`, `deleting_configuration`, `configuring`,
  `unconfiguring`, `setting_configuration`. A new row copies the live values and is active when none
  is; deleting the active row first switches to the row after it, else before; setting a cell of
  the active row is the live edit itself; a parameter cell of another row is refused as `Cycle`
  when, with that row's other cells in place of live expressions, the parameter would depend on
  itself (`check_row_cycle`), so every row can be switched to.

### Body appearance (`body_appearance.rs`)

- `BodyAppearance`: an sRGB `Rgb` colour, a material name, a density expression, an optional
  `Finish`, `name`, `opacity`, `faces`. `Edit::SetBodyAppearance` sets it whole on a feature that
  `makes_body` (else `NotABody`), or, given a piece's body, on that piece (Separate pieces below); a
  feature that stops making a body keeps it unused. `Document::body_appearance` and `body_hidden`
  read a body's look whichever it is. Recompute
  ignores it except a measurement of a whole body, whose cache key holds the appearance and the
  density's parameters.
- Material name: trimmed, blank is none, past `MAX_MATERIAL_NAME_CHARS` refused. `name` is the
  body's own name, one line (`MAX_BODY_NAME_CHARS`, `BodyNameTooLong`); `Document::body_name`
  gives it, else the making feature's name. `opacity` is a percent from `MIN_OPACITY_PERCENT`
  (lower is `OpacityTooLow`), 100 or more stored as none; `OPACITY_STEPS` (75, 50, 25) are the
  app's offers and `nearest_opacity_step` snaps to them or solid (STEP import uses it).
  `DEFAULT_BODY_COLOUR` is the colour of a body without one (the app's `DEFAULT_COLOUR` is it).
  `Finish::ALL` lists the finishes, each with a stored `key` and shown `name`; none keeps the plain
  look.
- Density is a plain number in g/cm³ that may use parameters, so `Feature::parameters` and
  `uses_parameter` include it. It is evaluated where shown (`density_value`, `mass_grams`): above
  zero and at most `MAX_DENSITY`, else a `DensityError`.
- `faces` lists `FaceColour`s (a `FaceReference`, colour, optional opacity below which
  `MIN_OPACITY_PERCENT` is refused and above 100 held at 100, optional `Finish`), later entries
  winning. `InsertFeature` settles an appearance as `SetBodyAppearance` does (`BodyAppearance::settled`),
  so every appearance the document holds is one a model file reads back. `face_appearances`
  maps a solid's faces to entries in one pass (`face_colours`, `face_opacities`, `face_finishes`
  read it; the scene takes it once per body): every face of the reference's name (so fragments of a
  split face keep their colour) or of a name a split face gives its pieces, else the face the
  reference resolves to; a face found by neither keeps the body's look.
- `folder` is the body's group as a path, outermost group first, empty for none (Body groups
  below).
- Split names come from `Document::face_splits(body)` (`FaceSplits`): for each unsuppressed split
  face of the body in tree order, the names its face references hold; `names_from` adds
  `FaceName::split(feature, name, side)` for both sides (twice for a mixed sketch's two passes) of
  every gathered name the split chose, so a colour given before a split face was inserted above
  reaches every piece, through splits of splits too. Callers (scene, export) pass the body's
  `FaceSplits`.

## Rollback bar and tree order (`tree.rs`)

- The rollback bar (`RollbackBar`, `Edit::SetRollbackBar`) is document state: undoable, journaled,
  saved, compared by `same_content`. It moves with the features inserted at it in the same step.
  `TransactionBuilder::add_feature` inserts at the bar, so tools put new features right above it.
- The bar is attached to the feature below it: `MoveFeature` of that feature carries it and
  `RemoveFeature` of it is refused (`RollbackBarAbove`); `Document::deletion` first moves the bar to
  the next surviving feature (or the end).
- `tree_rows` lists the features with the bar as a row; `move_row` moves a feature or the bar as one
  `MoveFeature` plus, when the bar's place changes, one `SetRollbackBar`, so a feature crossing the
  bar is rolled back or forward. A refused place returns `AboveDependency` or `BelowDependent`
  naming the feature in the way, or `BelowConsumer` or `AboveConsumedUse` when a feature using a
  body (`bodies_used`) would come below the Combine consuming it. `Document::check_moves` holds
  these checks, each `MoveFeature` judged in the state the steps before it leave, then checks the
  transaction as `check` does; the plain edit refuses only the invariant part, so undoing a move
  that lifted a feature above one holding its faces (left there by an insert) puts it back.
- `move_features` moves several to one gap in tree order as `MoveFeature`s ordered so no step
  passes a feature over one it uses that is moving too: those below the gap rise first, each right
  under it, then those above sink in reverse. Each step is checked like a single move.

## Sketch edits (`edit/sketch.rs`)

- Sketch content changes only through sketch edits. Removing an entity something still uses is
  refused, not cascaded; `remove_sketch_items` expands a deletion into constraints, curves, then
  points. A revolve's axis line and a sweep's or pipe's path curves (`FeatureKind::named_curves`)
  are uses from other features that may name a number the sketch has no entity under (a damaged
  file, a fuzzed insert): an `AddSketchEntity` under one is refused (`EntityNumberNamed`), so the
  inverse of every addition, a removal, applies. Every other outside reference to a sketch entity
  (`FeatureKind::named_entities`: those curves plus the sketch points and lines named by datums,
  measurements, mates, moves, coils, a loft's end point, patterns' axes and bases, a split
  face's carry, a revolve's or extrusion's model axis, and the projection sources of other sketches) may
  name a number the sketch lacks the same way, but removing the entity stays allowed (the user fails
  with a reason until the deletion is undone), so its re-adding is not refused; instead inserts and
  kind changes raise the named sketch's ID counter past every such number, so a newly drawn entity
  never takes it and silently becomes the axis, point or source. A points pattern's skipped copies
  are left out of those, since an instance decodes to any number up to 2^64 and raising the counter
  on loading would change how shipped files read (the format corpus holds one skipping copy 2^40);
  `Editor::new` reserves them instead (`Document::reserve_skipped_points`), so a document the app
  opens never draws a point under a number a pattern skips while a loaded file reads as it was. `AddSketchEntity` carries the construction flag and
  the curve a piece was split from (`split_from`), `AddSketchConstraint` the inactive flag and label offset, so undoing
  a removal restores them; `SetSketchConstraintActive` and `SetSketchLabel` change one;
  `reshape_sketch` emits them for kept constraints that differ. Setting an entity changes only its
  value, never its kind or points.
- `Edit::SetSketchProjection` gives an entity a `ProjectionSource` (or none) and sets the projected
  flag on it and its points in one step. The flags always follow `projections`: an entity is
  flagged when it or a curve using it has a source, so taking one projection off leaves a point
  with its own source, or shared with another projected curve, flagged, and undo restores exactly
  the flags before. A projected entity is never removed while flagged
  (`StillProjected`), so `remove_sketch_items` first takes the projection off every doomed
  projected entity and takes a projected curve's points with it (undo restores in opposite order).
- `Edit::AddSketchText`, `RemoveSketchText` and `SetSketchText` (`TransactionBuilder::add_sketch_text`,
  `set_sketch_text`) add, remove and change a sketch text under its id, checking the parameters its
  height and angle use as a dimension's are checked. `remove_sketch_items` takes text ids with the
  entities: a doomed point takes the texts standing on it, and a doomed text takes its anchor when
  no curve and no surviving text uses it (with the constraints on it); texts go after constraints
  and before curves and points. `reshape_sketch` removes a text whose anchor was replaced or
  changed, adds new ones after the entities and sets changed ones. Model scale scales a text's
  height like a length dimension; inlining a parameter rewrites a text's height and angle.
- `Edit::AddSketchGear`, `RemoveSketchGear` and `SetSketchGear` (`TransactionBuilder::add_sketch_gear`,
  `set_sketch_gear`) do the same for a stored gear (`sketch.md`, Gear), checking the parameters any
  of its expressions use (seven for a free gear, the three of a meshing one or of a sprocket). `remove_sketch_items`
  and `reshape_sketch` treat gears as texts, a lone centre shared by texts and gears going only
  when all its users do. A removal also takes the meshing gears of every gear it takes, and gears
  are removed partners first (`partners_first`) because `Sketch::remove_gear` refuses a driver
  with partners; `reshape_sketch` renews a partner whose driver is renewed (a replaced centre) and
  adds drivers before partners, so an undo, a journal and a reshape replay in an order the sketch
  accepts. Model scale scales a gear's module, root fillet and bore like length dimensions and
  leaves its counts and angles (a meshing gear's bore only, the rest follows its driver; a
  sprocket's bore only, since the chain fixes the rest);
  inlining a parameter rewrites every expression it stores. A gear whose values cannot make one
  fails its sketch (`SketchCause::Gear`) naming it, fixing the parameter when one failed; a
  partner that cannot be cut names itself and the meshing problem.
- `reshape_sketch(feature, before, after)` turns a sketch edited as a whole (trim, extend) into
  edits; an entity whose kind or points changed is removed and added again under its ID with
  everything using it, its `split_from` included. `settle_sketch` moves the definition to a solved
  shape so the next solve starts from what the user sees.

## Feature kinds

- An insert or kind change settles a kind as loading would read it back (`settled_kind`): a
  mirror's and a pattern's features each kept once, a pattern's skipped copies settled (Pattern
  below), an angle or point mate or an equal blend never flipped, a split face's carry square
  unless it splits along a sketch, a reference image's opacity and spots held in range. A combine
  keeps its tools as chosen; one chosen twice fails.
- `SetFeatureKind` replaces settings but never the kind (`EditError::KindChange`); a blend may
  switch fillet/chamfer, a pattern between its kinds, a datum within its `Datum::kind`. Sketches
  change through sketch edits. An edit whose new kind still `makes_body` keeps the body for its
  users; one that stops making it is refused as `BodyInUse` naming the users (of its body or of any
  of its pieces), and one that starts or stops making a body of each piece while any of them is used
  as `PiecesInUse`. Update references (`healing.rs`) is not refused by it.
- `bodies_used`, `planes_used` and `axes_used` extend `features()`, so dependents, moves and
  deletions account for them. `origin_features` (features that made the faces and edges a feature
  holds, from the references' `FaceOrigin`s) join them in `dependencies()`, which `dependents_of`,
  the tree's moves (`check_moves`) and the delete dialog use. Inserts and kind changes check only
  `features()`, so a model saved with a reference to a face made further down still loads and
  edits. A reference to a patterned copy's face depends on both the pattern and the original's
  feature; `origin_feature` (which a double-click opens) is the pattern.
- Kernel and profile errors become sentences naming the curves, edges or faces involved. Boolean
  failures of extrusions, revolves, holes and combines go through `trouble.rs`: the faces of the
  `BooleanSite` named by origin ("where A meets B"), the site's point as the error's `place`, and a
  remedy to make faces line up exactly or stay clearly apart; an invalid result asks only for a
  slight change. Patterns and mirrors pass on the point alone.
- Whether geometry lies on a line or plane, runs along a plane or is parallel is decided in one
  place (`tolerance.rs`) for revolve axes, datums, attachments, patterns and blend pieces;
  `along_plane` is public so the app's revolve axis list judges the same way.
- No region chosen, no blend edge, no face to move or split are waiting states, not errors
  (`MissingInput`, `document-recompute.md`).

### Solid (`solid.rs`)

- A sketch's profile curves (`solid::profile_curves`, used by regions, extrude, revolve, walls,
  ribs, splits, emboss) are its non-construction curves and the letters of its texts (`lettering.rs`):
  `placed_text` lays a text out with `caditor-lettering` at its anchor in the solved geometry,
  scaled by its height (cap height) and turned by its angle, each Bézier and line a `ProfileCurve`
  named `letter_curve` (`sketch.md`), so letters become regions and holes like drawn outlines and
  extruded side faces are named from them. A stored gear contributes the same way
  (`gear_curves.rs`): its outline's pieces (a `ToothedOutline`, a sprocket's gaps as arcs) as lines,
  arcs and splines and its bore as a circle, each named `gear_curve`. `PlacedText::polylines` facets them for the app and
  `lettered_at` lays out a text not yet in a sketch, for the preview.
- An `Extrude` or `Revolve` of a sketch's regions (`RegionChoice::All`, or chosen
  `RegionReference`s resolved by `resolve_regions` so a changed region is healed or left out; fails
  only on a tie or when none is left) with a `BodyOperation`: `NewBody`, or `Add`, `Remove`,
  `Intersect` on the body of the feature that made it. The app shows chosen regions as resolved.
- Extents are expressions above zero; a revolve's total angle is at most a full turn. `start` is an
  optional `SolidStart`: a signed length, or a `PlaneReference` of a flat face or plane parallel to
  the sketch (resolved like an end's, in its body's state; not parallel fails naming it; the plane's
  distance along the normal becomes the length, `start_plane`). `displayed_start_offset` gives it
  for drawing (`displayed_plane`, `displayed_axis`). Every end, symmetric extent, up-to search and a
  revolve's turn works from the start. `None` and zero are the same, stored as absent. A start
  plane's body and datum count as used and its face is healed.
- `ExtrudeEnd::UpToFace(PlaneReference)` is resolved in its body's state at the extrusion's place;
  end face bodies and datums count as used (`end_bodies`, `end_datums`).
  - Through all only cuts or intersects and reaches past the target's box by `THROUGH_ALL_REACH`
    and `THROUGH_ALL_MARGIN`; a body wholly behind fails.
  - Up to next needs a target body and ends on the plane of the first face the profile meets
    (kernel `next_face`); an addition starting inside the body, or a cut first entering it, fails.
    When the first faces are curved or several, a one-sided extrusion with no end offset is swept
    through all and cut back where it first meets the target body (kernel `stop_at_body`,
    `Stopping` in `solid.rs`); two sides or an offset need one flat face and fail in words. The
    plane of each flat first face is recorded unoffset in the result (`SolidResult::reached`, a
    `ReachedEnd` per side), where the interface stands its Past the face arrow.
  - Up to face ends on the face's plane extended, which must lie beyond the whole profile.
  - `UpToSurface` (boxed `FaceAttachment`, no offset) ends on any face: flat on its plane, curved
    by `stop_at_body` (failing naming the other faces met first). Its body counts among
    `end_bodies`, its origins among the feature's, healing keeps it.
  - Either up-to end may carry an `offset` (signed length expression, boxed, zero stored as
    absent) along the end plane's normal, so a slanted plane keeps its slant; it fails naming the
    offset if it brings the end back across the profile. Offsets count among the feature's
    expressions.
  - Every failure names the face or plane and the side, with what to do.
- A removal also cuts each body of `other_bodies` (only extrusion and revolve; other operations
  fail in words), with the same tool: they are in `bodies_used` and `Feature::bodies`, their
  results are `SolidResult::others`, through all reaches past all of them, the target and repeats
  are skipped, and a body the tool removes entirely fails the feature naming it.
- `RevolveExtent::UpTo { target, reversed }` turns until its half-plane first lies in the plane of
  a flat face or plane holding the axis (boxed `PlaneReference`, resolved, counted and healed like
  an extrusion's end; `Turn` in `solid.rs`, from the chosen regions' middle). A target not holding
  the axis, or holding the profile itself, fails in words.
- `RevolveAxis` is a line of its own sketch (it cannot then be deleted; `AxisNotALine`) or an
  `AxisReference` to a model axis in the sketch plane.
- A revolve's `side` (`AxisSide`; none turns the whole profile) keeps only the part of the chosen
  regions on that side: the sketch is re-divided with the axis as a line past the regions' bounds
  (entity `REVOLUTION_AXIS_ENTITY`, "the revolution axis"), regions whose anchor lies on that side
  inside a chosen region are selected as one; a side holding nothing fails in words.
- An `Extrude`'s optional `direction` (boxed `AxisReference`: edge, axis, round face's axis or
  sketch line) runs it along that line (kernel `extrude_along`), the way that leaves the sketch on
  its forward side; a direction in the sketch plane fails naming it. Distances are along the
  direction, while up-to planes, offsets and through all work by height; up to next, a curved face
  and a taper run square only and fail in words. The axis's body, datum, frame and sketch count as
  used (`SolidFeature::model_axis`), healing keeps it.
- `taper` (an angle, boxed with the wall) goes to kernel `extrude_tapered`; at or past
  `MAX_TAPER_DEGREES` it fails before the kernel, and every taper refusal names the extrusion and
  what to change. `wall` (`Wall`: a thickness expression and the kernel's `WallSide`) sweeps
  `wall_regions` of every non-construction curve instead of regions (the region choice is ignored,
  an open sketch works); failures are worded per `WallError`, and a revolve keeping one side of its
  axis with a wall fails in words. Both count among the feature's expressions.
- A feature whose body others change keeps making a new body.

### Sweep (`sweep.rs`)

- `SolidFeature::Sweep` (`Sweep { sketch, regions, path, orientation, twist, scale, operation,
  other_bodies }`)
  carries the chosen regions along a `SweepPath`: `Curves { sketch, curves }` of another sketch
  (lifted to its solved plane through `profile_curve`, each keyed by its entity id), `Edges { body,
  edges }` (`EdgeReference`s resolved with `EdgeNaming` in the body as it stands at the sweep, each
  keyed by its edge name's digest), `Helix { helix }` (the turns of a `Helix` feature's result, each
  a kernel `HelixTurn::path_segment` keyed by its turn from 1 and winding about the helix's axis,
  labelled "turn N" of the helix), or `Unchosen`. It shares `solid::evaluate` with extrusion and
  revolve (regions, body operation, other bodies cut, boolean wording) and calls kernel
  `sweep_shaped` with the kernel's `SweepOrientation` and a `SweepShape` (`sweep::shape_of`):
  `twist`, an angle (`untwisted()`, 0 deg, by default) in radians, and `scale`, the end scale, a
  plain number above zero (`unscaled()`, 1, by default; zero or less fails in words, a value of the
  wrong kind asks for a plain number such as 0.5). Both are its expressions (`Sweep::expressions`,
  counted, inlined and pasted with the rest), never scaled with the model; `Sweep::is_shaped` tells
  either from its literal default. It has no start, taper or wall.
- The path's sketch is in `reference_sketches` and `features()`, its body in `bodies_used`, its
  helix in `features()` (`SweepPath::features`), its edges' origins among `origin_features`, and
  healing visits its edges. A curve of the path cannot be removed (`EntityInUse`, as a revolve's
  axis line); a path sketch or helix deleted, or a helix that is not built, fails the sweep.
- Failures are worded per `PathSweepError` (`SweepCause`, `sweep_cause.rs`), naming path curves by label and
  sketch ("Line 3 of Path") or edges by their faces (no path waits instead,
  `MissingInput::Path`): a branching or separate path; a sharp corner (remedy: a fillet in the path's sketch, Parallel, or straight lines so it is mitred); a
  mitre too sharp; a profile too large for a bend (fix on the profile's sketch); a path leaving
  along the profile's plane or, kept parallel, turning along it; a closed path missing the
  profile's plane; a closed path bringing the profile back turned, in degrees with the twist that
  closes it (the current twist less the angle, to three decimals; fixed on the sweep), or back
  tilted (a path in one plane, or the profile square to the path); a closed path with an end
  scale; a twisted or scaled sweep at a sharp corner between lines; a twist or scale out of range;
  a sweep running into itself. The kernel's profile
  errors go through `sweep_failure` as "sweep". A path curve no longer in its sketch, or an edge
  lost or tied, asks to choose the path again.

### Pipe (`pipe.rs`)

- `Pipe { path, shape, size, wall, bend, operation }` runs a section of the kernel's `PipeShape`
  (round of a diameter, square of a width, equilateral triangle of a side length) along a
  `SweepPath` chosen and resolved as a sweep's (`sweep::path_of`, shared, which also gives the
  path's `up`: the path sketch's normal, else Z), solid or hollow with `wall` (a thickness), its
  sharp corners between any two path curves bent to the `bend` radius (zero leaves corners between
  lines mitred and fails any other). Size and wall
  are lengths above zero, the bend zero or more (`primitive::Context`, shared, worded "The diameter
  of the pipe must be more than zero."); all three are its expressions (parameters, inlining, model
  scale). It calls kernel `pipe` and joins, cuts or intersects like a primitive
  (`primitive::combined`, keeping its tool as `joins` or `cuts`, so `repeatable_on` takes it).
- `pipe::stand` (exported as `pipe_stand`) resolves the path from an `Evaluation` the way the
  recompute does (the path sketch's result, the path body as the pipe sees it) and asks kernel
  `pipe_stand` for the start section's plane and first corner, for the app's value arrows.
- The path's sketch is in `reference_sketches` and `features()`, its body in `bodies_used`, its
  edges' origins among `origin_features`; healing visits its edges (`healing::visit_path`), a path
  curve cannot be removed (`EntityInUse`) and no path waits (`MissingInput::Path`).
- Failures: a wall too thick names the limit (half the diameter or width, a third of the triangle's
  height); a bend too tight for the section or one needing more than the curves at a corner names
  the two path curves, fixed on the pipe; a corner too sharp to turn; a sharp corner touching an
  arc, spline or edge curve with no bend radius, fixed on the pipe by giving it one; the rest per
  `PathSweepError` in
  `sweep::path_failure` (a `SweepCause` shared by any path follower: branches, gaps, a curve too short, a
  sharp corner where an arc meets another curve, a bend of the path too tight, running into itself).
- Faces are worded "<pipe> outside" and "<pipe> inside" (`pipe::side_name`: entities past the
  section's `PipeShape::curves` are the inner outline), caps as a sweep's.

### Loft (`loft.rs`)

- `Loft { profiles, end, skin, closed, square_start, square_end, operation }` passes through
  `LoftSection`s in order (a sketch and an
  optional `RegionReference`; none takes the sketch's even-depth shapes, which must be one), ending
  at an optional `PointReference` resolved like a datum point's, with the kernel's `LoftSkin`. Each
  section's region resolves on the sketch's result as an extrusion's does (`resolve_regions`), the
  plane is the solved sketch plane, and kernel `loft` builds the tool, joined, cut or intersected
  like a primitive's (`primitive::combined`, so patterns and mirrors repeat it, `repeatable_on`).
  Kernel `loft_through` takes `LoftEnds`: `closed` runs back into the first profile, and
  `square_start` and `square_end`, weights (plain numbers above zero, "start weight" and "end
  weight"; zero or less fails in words) present only when that end leaves its profile square.
  The weights are its expressions (parameters, inlining, pasting; never scaled);
  `Loft::has_ends_shaped` tells any of the three apart from a plain loft.
- The sketches and the end point's sketch are `reference_sketches` and `features()`, the end's
  body, datum and frame `bodies_used`, `points_used` and `frames_used`, its origins
  `origin_features`; healing visits the end point, pasting renames all of them and model scale
  holds an end at the origin as principal geometry.
- Fewer than two rows (profiles plus the end point) waits (`MissingInput::Profiles`). Failures name
  the sketch: one not available, one with no closed shape or several (asking to choose a region),
  a shape with a hole, a profile lying along the way the loft runs, two profiles in one place, sides
  running into each other, too many curves; a closed loft with an end point or under three
  profiles, a square end at an end point, on a ruled or closed loft, or of a weight not above zero;
  profile problems go through `solid::profile_failure`.
- Side faces are worded "<loft> side from <curve>" of the first profile's sketch
  (`describe_origin`), caps as a sweep's.
### Coil (`coil.rs`)

- `Coil { axis, reversed, diameter, length, shape, size, placement, taper, turning, operation }`
  winds a section of the kernel's `CoilShape` (round of a section diameter, square of a section
  width) around an `AxisReference` resolved like a move's (`Resolver::axis`; its body, datum,
  frame and sketch count as used, its origins among `origin_features`, healing visits it as "axis
  it winds around"), from the axis's point along its direction (against it when `reversed`), the
  frame's x axis from `Plane::new`. `length` is a `CoilLength` holding two of height, pitch and
  turns (`TurnsAndPitch`, `HeightAndTurns`, `HeightAndPitch`; `CoilSizing` names which), the
  third derived (`coil::span`); turns are a plain number from `LEAST_COIL_TURNS` to
  `MAX_COIL_TURNS`, lengths above zero (`primitive::Context`). `placement` is the kernel's
  `CoilPlacement` against the diameter, `taper` an angle below `MAX_COIL_TAPER_DEGREES` either way,
  `turning` `CoilTurning` (`CounterClockwise` is the kernel's right hand). Every value is an
  expression (parameters, inlining; model scale scales the lengths, not turns or taper,
  `Coil::lengths_mut`); the principal axis it may wind round is anchored for model scale.
- It calls kernel `coil` and joins, cuts or intersects like a primitive (`primitive::combined`,
  keeping its tool as `joins` or `cuts`, so `repeatable_on` takes it). Kernel refusals become
  sentences: turns that would touch (with the least pitch and a remedy naming pitch, height, turns
  and section), the section reaching the axis (from a taper drawing it in, or the diameter and
  position), too few or many turns, too far from the origin; the rest are logged and worded as
  sizes it cannot be built with.
- Faces are worded "<coil> start" and "end", a round section's sides "<coil> surface" and a
  square's "inside", "outside", "top" and "underside" (`coil::part_name`).
### Helix (`helix.rs`)

- `Helix { axis, reversed, diameter, length, taper, turning, start, start_point }` is a curve feature: the axis,
  sizing (`CoilLength`, `coil::span`), taper (`coil::taper`) and turning are a coil's, resolved,
  counted, healed ("axis it winds around"), pasted, inlined and anchored for model scale the same
  way (`Helix::lengths_mut` scales the diameter and lengths), with the frame from `Helix::frame` (x
  axis from `Plane::new`, so it starts toward X, toward Y about the X axis). `start` is an angle
  expression of any value (0 deg by default, an expression like the others, never scaled) turning
  the start counter-clockwise about the axis whichever way it turns (kernel
  `HelixSpec::start_angle`); one that is not an angle fails in words. `start_point` (an optional
  `PointReference`: a sketch point, a body corner by vertex name, a round edge's centre, a datum
  point or frame origin) replaces the typed angle: each recompute resolves it and takes the angle
  of its position in the helix's frame (`Plane::to_local`, `atan2`), so the start follows the
  point; the typed `start` stays stored, unevaluated, for when the point is cleared. A point on
  the axis (within `LINEAR_RESOLUTION`) fails the helix (`CoilCause::StartOnAxis`). The point's
  features count among `Helix::features`, its sketch among `reference_sketches` and
  `named_entities`, its body among `bodies_used`, its datum among `points_used`, its frame among
  `frames_used`; it is healed ("point it starts at"), pasted (`rename_point`) and anchored for
  model scale (`Anchored::Point`) like a loft's end point. It calls kernel `helix` and
  gives `FeatureResult::Helix` (`HelixResult`: the frame and one `HelixTurn` per piece of at most a
  turn; dependents compare it whole). It makes no body (`Feature::body` none, not `makes_body`) and
  can be hidden. Refusals are sentences: a taper drawing it into its axis, too few or many turns, a
  taper too steep, too far from the origin; the rest are logged and worded as sizes it cannot be
  built with. A sweep or pipe follows it (`SweepPath::Helix`).
### Rib (`rib.rs`, `rib_tool.rs`)

- `Rib { sketch, body, thickness, draft, side, direction, ends, reversed }` joins thin walls along the
  non-construction curves of a whole sketch to a body (`Feature::body` is the body; not
  `makes_body`, not `modifies_body`). `RibDirection::Parallel` (a rib) puts the material in the
  sketch plane on the left of each chain walked from its first end (the end on the curve of lowest
  entity id, as a split's; the right when `reversed`) and the thickness across the plane
  (`RibSide::OneSide` in front, along the normal, `OtherSide` behind, `Symmetric` half each);
  `Square` (a web) thickens each chain in the plane (`OneSide` left of the walk, `OtherSide` right,
  `Symmetric` centred; `wall_regions`, `Inside` or `Outside` picked by a probe point, since an open
  chain's inside is its bend; lines, arcs, splines and elliptical arcs, a spline's or ellipse's
  wall edges being fitted splines) and runs it along the sketch normal (against it when
  `reversed`).
  The thickness and the draft (an angle, 0 when made, below `MAX_RIB_DRAFT_DEGREES`; negative or
  steeper fails on the rib) are its expressions (parameters, inlining; model scale reaches the
  thickness, a length). A web's draft
  tapers its wall outward away from the sketch (`extrude_tapered` by minus the draft), so it widens
  toward the body; lines, arcs, splines and ellipses all take it (a wall's edges along a spline
  are fitted splines the taper sweeps as ruled surfaces, two splines meeting at a corner are
  mitred, `kernel-profile.md`). A rib parallel to its sketch takes it too, as flanks: its
  thickness is its width at the free edge (the sketch's curves) and its broad faces lean the draft
  from the sketch plane, widening toward the body (`Rib::is_drafted` says whether the draft is a
  literal zero); a drafted rib's region is closed by one big circle round the whole sketch and
  body instead of the bounding rectangle (so no short rim edge is drawn in to nothing), the flat
  slab keeps its thickness and a flank (`extrude_flared`, from the slab's face up to the depth the
  farthest corner of the body's box lies from the curves, so it is cut by the body) is united to
  each face that leans: both for `Symmetric`, the front for `OneSide`, the back for `OtherSide`
  (a one-sided rib's other face stays in the sketch plane). The flank's faces carry the curve's
  entity marked `FRONT_FLANK` or `BACK_FLANK` (`is_flank`, read as "drafted face along"), so they
  are named apart from the edge face. A flank whose bend closes up (`TaperCloses`) is tried again
  at half, a quarter and an eighth of the depth; a kept lump reaching the flank's far cap is
  `RunsPastFlank`, as is `DraftCloses` in words. A rib's draft above 0 and under
  `MIN_FLANK_DEGREES` is `DraftShallow`. `RibEnds` says whether the free arc ends of a rib or a web run on
  straight (`Straight`) or round their circles (`RoundArcs`). The sketch is in `reference_sketches` and `features()`.
- `rib_tool::rib_tools` builds a generous tool per chain and keeps only what lies between the drawn
  curves and the body. Chains (`rib_tool::chains`) come from the curve ends shared at each point
  (`split::curve_links`): at a point where several ends meet, the two running straightest through
  it (`straightest`, most opposite outward tangents) are paired again and again, so two crossing
  pairs make two chains and the odd end of a T starts a chain of its own; walks from unpaired ends
  give the chains (`split::chain` each). A closed outline, a circle or curves left in a loop fail
  `Closed`, a walk coming back to a point it passed `Branches`, both fixed on the sketch. A chain
  whose end lies on another chain (`RibChain::passes_by`: a T, at a shared point or mid-curve) is
  built after it (`attachments`, `build_order`, a cycle broken in index order), and the tools
  already built for the chains it meets are taken out of its tool with the body, so it runs on
  only up to that rib. Free ends are carried on along their end tangents past the body's box: an
  end line is lengthened in place (one face, keeping its name), an arc end gets a line keyed
  `START_RUN` or `END_RUN` with the arc's entity. With `RibEnds::RoundArcs` a web's free arc end
  is instead carried round its circle (`bent_round`, the arc's own end moved, sharing what is left
  of the circle with its other end less `ARC_GAP_DEGREES`), a line end still straight. A rib's
  free arc end is carried round its circle only as far as the body (`rolled_chain`, once per
  rib before the flank retries): the circle is stepped by `ROUND_STEP_DEGREES` from the end, up to
  the same share of the circle, until a point at the slab's middle height (`slab_middle`) is no
  longer outside the body, where the arc's own end is put; from there a straight run along the
  circle's tangent closes the outline to the rim as for any end, and the rim's box grows to the
  moved arc. An end already inside the body stays, a circle that never meets the body is
  `RunsPastEnd` naming the arc. A rib's region is closed round the box like a split's half
  space (perimeter lines `PERIMETER_ENTITIES`) and extruded by the thickness; a web's wall is
  extruded from the sketch plane past the box. The tool less the body (`boolean` difference) is
  taken apart (`Solid::lumps`) and the lumps holding seed points (just off the curves on the
  material side, outside the body) are kept. A kept lump with no face of the body is
  `MeetsNothing`, one holding the far cap is `RunsPastDepth`, a carried end's cap `RunsPastEnd`
  (naming the curve), a perimeter face `RunsPast`; so a rib never floats or pokes out. A body wholly
  behind the sketch is `Behind`, seeds all inside the body `Inside`. The kept lumps of every chain
  are joined to the body one by one (crossing webs overlap there) and kept as the result's `joins`.
- Failures are worded per `RibError` (`RibCause`): sketch problems fixed on the sketch, the rest
  on the rib with what to do (reverse it, draw the curve so it runs into a wall, carry its ends on
  straight when carried round its arcs, make the draft smaller when the drafted bend closes,
  `SweepError::TaperCloses`); a join failure goes through `trouble.rs`.
- Faces are named by extrusion naming from the rib's feature id: worded (`rib::face_words`)
  "<rib> side from <curve>" (a web's side), "edge face along <curve>" (a rib's drawn edge), "side
  carried on from <curve>", "top face" (a web's cap at the sketch) and "face in front of" or
  "behind the sketch" (a rib's flat sides). A face flush with a body face merges into the body's.
- `rib::facing` (exported `rib_facing`) says whether the rib should be `reversed` to meet the body:
  a ray from the first chain (a web: square to the sketch from a point on a curve; a rib: in the
  plane from the chord's middle, square to it) both ways, the nearer crossing winning; none is
  none. The app sets the default from it.

### Blend and shell (`blend.rs`, `shell.rs`)

- A `Blend` is a fillet or chamfer of `size`. A chamfer's `form` is `Equal`,
  `TwoDistances { second }` or `DistanceAngle { angle }` (below 180), with `flipped` swapping which
  face of each edge the first distance is on (the kernel's choice: the face more chosen edges
  share). A fillet ignores both but keeps them; the extra expression counts among the feature's
  expressions whatever the kind (`Blend::expressions`, `parameters`, `uses_parameter`, inlining,
  pasting), while recompute evaluates only what the kind needs (`chamfer_form`).
- Besides `edges`, a blend holds `groups` (`EdgeGroup`): a face (`FaceReference`, pieces of one
  split face accepted) or the whole body, expanded each recompute to the edges it has then
  (`GroupResolution::edges`: boundary edges, or every edge, seams and edges between pieces of one
  face left out, each once). Resolved edges and group edges are joined once (`Blend::resolve`). A
  lost or tied face fails with `BlendUnresolved::Faces`; a face group's origins count among
  `origin_features` and healing visits it. `entry_count`, `remove_entry`, `entry_edges` treat edges
  then groups as one list, the panel's.
- A `Shell` holds the opened faces, a `thickness`, a `ShellSide` (`Inside`, `Outside`,
  `BothSides`, handed to the kernel's `Walls`) and `faces`, a list of `FaceThickness` (a
  `FaceReference` and its own thickness expression, every piece of a split face taking it).
  `Shell::expressions` covers the thickness and every face's (parameters, inlining, model scale);
  healing visits the faces as well as the opened ones, and their origins count among
  `origin_features`. Recompute evaluates each face's thickness in words naming the face ("The
  thickness of Base start face must be more than zero."), fails a lost or tied face ("… given its
  own thickness is no longer part of the body of …"), and words the kernel's
  `OpenedFaceThickness`, `CurvedOpening` and `InvalidFaceThickness` with the face and a fix.
  `openable_face` says which faces a shell can open (flat, cylinder, cone, sphere, torus).
- A blend refused as `UnsupportedEnd` at a vertex whose faces an earlier blend made
  (`Context::earlier_blend_at`) names that feature and says to move this one above it.
- Blend and shell resolve their references (`EdgeReference`s, opened `FaceReference`s) in the body's
  state before the feature, each to a `Resolution` (`pieces.rs`; the panel uses the same terms): one
  match; the pieces of a split edge on one line or circle (a shell face tied between fragments of
  one surface opens all of them); tied between separate ones; missing. A lost or tied reference
  fails the feature. A blend's edges go through `SplitEdges::follow` (`split_edges.rs`,
  `kernel-naming.md`): an edge whose sketch curve a later trim, split or break cut into pieces
  resolves to the pieces along it (`Resolution::Pieces`), and one only partly traced fails as
  `BlendUnresolved::PartlyFollowed` (the panel shows it as tied). So `Blend::resolve`, `resolutions`
  and `entry_edges` take the `Document`, and the evaluation reads the extruding feature's sketch
  through `Inputs::document`: a piece's `split_from` is set only when the piece is added, so it
  changes only with the entities whose side faces the body's key already covers. That state is kept
  (`Evaluation::body_before`) and meshed for showing while choosing.

### Offset face (`offset_face.rs`)

- `OffsetFace { body, faces, distance, tangent }` resolves its `FaceReference`s in the body's state
  before it (lost or tied fails) and moves them along their outward normals by a signed length
  (kernel `offset_faces`; positive grows, negative shrinks, zero fails in words). With `tangent` the
  faces first grow to everything reached across smooth edges (`tangent_faces`), recomputed each
  evaluation.
- It modifies its body (`modifies_body`), keeps every face and edge name and the state before it.
  Kernel refusals become sentences (`OffsetError`: a vanishing face, crossing faces, a neighbour it
  cannot follow, a corner where faces cannot meet).

### Move face (`move_face.rs`)

- `MoveFace { body, faces, distance, direction, angle, axis }` resolves its `FaceReference`s in the
  body's state before it (lost or tied fails, none waits as `MissingInput::FacesToMove`) and carries
  them by one rigid motion (kernel `move_faces`): a tilt by `angle` about `axis`, then a shift by the
  signed `distance` along `direction`. No `direction` means square to the first flat face among the
  chosen, in reference order, out of the body (`flat_normal`; none flat with a distance is
  `NoNormal`); a
  non-zero angle with no `axis` is `NoAxis`; both nought is `Unmoved`. Both references are
  `AxisReference`s resolved through `Resolver::axis` (an edge's or datum's direction, its origin
  for the tilt), counted in `bodies_used`, `axes_used`, `frames_used`, `reference_sketches`,
  `named_entities`, healed (`direction`, `tilt axis`) and renamed when pasted; the tilt axis is
  anchored under model scaling and the distance scales as a length.
- It modifies its body (`modifies_body`), keeps every face and edge name and the state before it.
  Kernel refusals become sentences naming the face, edge or corner (`MoveError`), with what to do:
  move the fillet along an edge that cannot follow too, or move less.

### Draft (`draft.rs`)

- `Draft { body, faces, neutral, angle, flipped, tangent, beyond, own }` tilts flat faces of its
  body, and turns cylinders and cones coaxial with the pull into cones (kernel `draft_faces`), about
  where each meets the neutral plane (a `PlaneReference`: a
  principal, datum or coordinate system plane or a flat face, resolved through `Resolver::plane`;
  its datum, frame and body count as used, its face is healed as "its neutral plane", it is
  renamed when pasted and anchored for model scale), so each makes `angle` with the pull: the
  plane's normal, reversed when `flipped`. `faces` are `FaceReference`s resolved in the body's state
  before it (pieces of one split face accepted; lost or tied fails, none waits as
  `MissingInput::FacesToDraft`); with `tangent` they first grow to everything reached across smooth
  edges (`tangent_faces`), so a slot's or rounded outline's wall drafts as one chain. Before the
  kernel, `check_draftable` refuses a face that is neither flat nor round (`NotDraftable`) or round
  about an axis off the pull (`OffPull`, a fillet along a top edge), saying when the chain brought
  it.
- `own` lists `FaceAngle`s: faces drafted at an angle of their own instead of `angle` (beyond the
  neutral plane they keep `beyond`), resolved like `faces` (lost or tied fails as a face given its
  own draft angle); a face in both takes its own. Either list alone is enough.
- `beyond` (an angle, absent for none) is a parting line: every drafted face whose boundary lies on
  both sides of the neutral plane (by `LINEAR_RESOLUTION`, sampled along its edges) is first split
  there (`split_faces` with `split::half_space_solid` of the neutral plane under the draft's id),
  the pieces found again by the names `FaceName::split` gives them; a piece lying beyond the plane
  is drafted by minus `beyond`, so the body narrows both ways from the parting line. Without
  `beyond` a face crossing the plane is drafted whole.
- Every angle is an expression (`Draft::expressions`: parameters, inlining, pasting); model scale
  changes none. An angle steeper than `STEEPEST_DRAFT` fails before the kernel, naming which angle.
  Kernel refusals become sentences naming the face, edge or corner as a move's do (a face parallel
  to the neutral plane, a face that cannot be drafted, a round face narrowing to a point
  (`NarrowsToPoint`), every face already at its angle, a face that would vanish,
  faces that would cross, a neighbour that cannot follow), each with what to change.
- It modifies its body (`modifies_body`), keeps every face and edge name (split pieces take split
  names) and the state before it. `default_neutral` gives the app's starting plane: of the flat
  faces beside the first flat chosen face and square to it, the one whose outward normal points
  lowest (a floor), else the first principal plane across it; with only round faces chosen, of the
  flat faces beside the first one square to its axis, one facing the face (a boss's plate, a blind
  hole's floor) before the one pointing lowest, else the first principal plane square to the axis;
  `flipped_toward` then points the pull at the faces (the mean height of their edges above the
  plane).

### Replace face (`replace_face.rs`)

- `ReplaceFace { body, faces, target }` puts flat faces of its body onto the target plane (kernel
  `replace_faces`). `target` is an `Option<PlaneReference>` (a principal, datum or coordinate
  system plane or a flat face of any body made before it, its own included, resolved through
  `Resolver::plane`; its datum, frame and body count as used, its face is healed as "its target
  plane", it is renamed when pasted and anchored for model scale); none waits as
  `MissingInput::TargetPlane`, which the app answers by picking. `faces` are `FaceReference`s
  resolved in the body's state before it (pieces of one split face accepted; lost or tied fails,
  none waits as `MissingInput::FacesToReplace`).
- It has no values, so no expressions or parameters, and model scale changes nothing but the
  anchored target. Kernel refusals become sentences naming the face, edge or corner, each with
  what to change: a curved face, a target square to a face, a target on or past the far side of the
  body (turning it inside out), two faces along an edge that would both lie on the target, every
  face already on it, and a move's (a face that would vanish, faces that would cross, a neighbour
  that cannot follow).
- It modifies its body (`modifies_body`) and keeps every face and edge name and the state before it.

### Delete face (`delete_face.rs`)

- `DeleteFace { body, faces }` resolves its `FaceReference`s in the body's state before it (lost or
  tied fails, no face waits as `MissingInput::FacesToDelete`) and removes them, the faces around
  closing the gap (kernel `delete_faces`). It modifies its body (`modifies_body`), keeps the state
  before it and every name the kernel keeps; it has no expressions. Kernel refusals become sentences
  naming the face (`DeleteError`: neighbours that cannot close the gap, with what to choose too; a
  face that would vanish; faces that would cross; a result that would not stay valid).

### Primitive (`primitive.rs`)

- `Primitive { shape, plane, at, anchor, reversed, operation }` makes a box, cylinder, sphere,
  torus (diameter across the tube's middle, tube diameter), cone (bottom and top diameters,
  height), wedge (length, width, height, top length) or prism (sides, diameter across corners,
  height) without a sketch. Every size is a length expression above zero and at most `MAX_SIZE`
  (`PrimitiveShape::rules`), except a cone's diameters and a wedge's top length (may be zero,
  `SizeRule::ZeroOrMore`, but not both cone diameters, and a top no longer than the wedge) and a
  prism's sides (plain number `MIN_PRISM_SIDES` to `MAX_PRISM_SIDES`, `SizeRule::Sides`); a
  torus's tube is narrower than its diameter. `at` is two lengths of any sign along the plane's X
  and Y. `plane` is a `PlaneReference` resolved like a mirror's at the feature's place (datum and
  face body count as used, the face is healed as "the face it stands on").
- The footprint box (a round shape's square) has a corner at `at` (`PrimitiveAnchor::Corner`), its
  base's middle (`BaseCentre`) or its middle with half above the plane (`Centre`); it grows along
  the plane's normal, or against it when `reversed`.
- A box, cylinder and prism are a profile extruded (`extrude`), a sphere, torus and cone one
  revolved a full turn (`revolve`), with fixed entities so faces are named from the feature: a
  box's sides are entities 1 to 4 (front at -Y, then right, back, left), a prism's 1 to its count
  from the flat side at the front, a round shape's curve entity 1, a cone's base and top lines 3
  and 4 (left out at diameter zero), caps are the start (bottom) and end (top). A wedge is its side
  profile (bottom 1, sloped 2 at +X, top 3, left 4) extruded along Y, caps the front and back
  (`PrimitiveShape::cap_name`). `describe_origin` words them ("Box 1 front face", "Cylinder 1
  wall"). Resizing or moving keeps every name; switching shape changes them.
- `operation` is a `BodyOperation`: a new body (`makes_body`), or joined to, cut from or
  intersected with the target body (kernel `boolean`), keeping the tool as the result's `joins` or
  `cuts` so patterns and mirrors of features repeat it. A failure names shape and body
  (`trouble.rs`) and fails the feature alone.

### Hole (`hole.rs`)

- `Hole { sketch, body, diameter, depth, style, reversed }` drills at every layout point of its
  sketch (`Sketch::layout_points`, `sketch.md`: free points and the far ends of a circular
  pattern's rays, never their hub) and at the centre of every non-construction circle
  (`hole::centres`), down the sketch plane's normal, or up when reversed.
- `HoleDepth`: blind, through all (as an extrusion's), up to next or up to face (boxed
  `PlaneReference` resolved like an extrusion's end; body, datum, frame counted, face healed),
  each up-to with an optional signed `offset`. Each hole's depth is where its axis meets the plane
  (offset along the plane's normal), so a slanted face gives each hole its own depth and a flat
  bottom. Up to next asks kernel `next_face` along the hole's outline (circle or slot) in the body
  before the feature and fails in words when the hole first enters the body, meets several faces,
  a curved one, or passes beside it; the plane it found, per hole in drilling order, is recorded
  in the result's `reached` as an extrusion's is. An up-to hole's counterbore, countersink or steps must stay
  shallower than each depth, and it ends flat whatever its bottom says.
- `HoleStyle`: plain, counterbore (diameter, depth), countersink (diameter, angle) or stepped
  (`HoleStep`s from the mouth down, 1 to `MAX_HOLE_STEPS`, each a diameter and own depth; each
  narrower than the one above and wider than the hole, together shallower than a blind hole; a
  refusal names the step). A counterbore is drilled as one step.
- Each hole is a half-section polygon revolved a full turn about its axis (kernel `revolve`),
  starting `MARGIN` above the plane, then subtracted. Curve entities are `point id * 16 + part`, so
  faces are named by sketch point and survive changes to other points. The part is the segment's
  role (`HolePart`: wall 1 and bottom 2, counterbore wall 4 and floor 5, countersink 6), never its
  place in the outline, so style changes keep references; `describe_origin` words them through
  `Hole::part_name`. A stepped hole's first step uses the counterbore's parts; step n adds
  `(n - 1) << STEP_SHIFT` (56) to its entities ("step n wall", "step n floor"; only point ids past
  2^52 could collide).
- `shape` is `Round` or `Slot { length, angle }` (centre-to-centre length, turned from the sketch's
  X axis), cut as an extruded stadium (a wider, shallower one for a counterbore) rather than a
  revolve. Slot walls are parts 7 to 10 (`slot side`, `slot end`) and 11 to 14 for the
  counterbore. A countersunk slot fails alone.
- `standard` (`hole_standard.rs`) is the ISO metric size and fit the sizes came from
  (`HoleStandard`: `MetricSize` M1.6 to M20; `HoleFit` close, normal, loose clearance per ISO 273,
  tapped at the coarse tap drill, tapped fine at an ISO 261 fine pitch (`FinePitch` by its place in
  `fine_pitches`, so their order is part of the file format: M12 and M20 list 1.25 and 1.5 first;
  drilled at major diameter less the pitch), or a heat-set insert), with counterbore and
  countersink sizes for socket and countersunk heads. It names a tapped hole's thread and gives
  each round bore its thread (Thread below); the bore's size comes from the expressions, which drive
  the hole. `MetricSize::offers` says which fits a size has (`fits`,
  `fine_fits`); `HoleStandard::offered` falls back to the first fine pitch, or to normal
  clearance, when a size lacks the fit asked for.
- `HeatSetInsert` (hole, insert length, least wall) follows CNC Kitchen's standard brass inserts:
  M2 3.2 mm for a 3 mm insert, M2.5 4.0 for 4, M3 4.0 for 5.7, M4 5.6 for 8.1, M5 6.4 for 9.5, M6
  8.0 for 12.7, M8 9.7 for 12.7, walls 1.3, 1.6, 1.6, 2.1, 2.6, 3.3, 3.3 mm. M1.6 and M10 up offer
  none.
- `sizing` is `Typed`, `Circles` or `CirclesAndHeads`: a hole at a circle's centre takes that
  circle's diameter (`circle_sizes`, the smallest of several on one point, construction left out)
  and its style is checked against it, a refusal naming the circle; holes at points keep the typed
  sizes. `CirclesAndHeads` (what the app sets) also scales the counterbore's diameter and depth, or
  the countersink's diameter, by circle over typed diameter; `Circles` keeps them as typed.
- `bottom` is `Flat` or `DrillPoint(angle)` (expression, 118 deg in the app): the bore ends in a
  drill-point cone, apex `radius / tan(angle / 2)` below the depth, as the last segment of the
  revolved profile so it keeps the bottom part and name. Blind round holes only: a through hole is
  unchanged and a slot with one fails alone; the angle is above 0 to `MAX_CONE_ANGLE`.
- It fails alone, naming the point, when a size is not positive, a counterbore or countersink is
  not wider than the hole or as deep as it, the angle is outside 0 to `MAX_CONE_ANGLE` (179 deg,
  also the panel's field rule), the sketch has no points, more than `MAX_HOLES`, or a hole does not
  cut into the body: no face named after its tool is left on the result
  (`hole::leaves_a_face_of`). Counting faces would not do, since the cut may also merge faces of
  the body, as two coplanar sides split where a circle touches the outline.

### Thread (`thread.rs`, `thread_standard.rs`)

- `Thread { body, face, size, class, hand, length, reversed, form }` threads a cylindrical face
  (`FaceReference`, pieces of one split face accepted), resolved in the body's state at its place.
  `ThreadForm::Cosmetic` changes no solid: result `FeatureResult::Thread` (`ThreadResult`: side,
  designation text, evaluated depth, `ThreadPlacement`); its body is used (`body_input`,
  `bodies_used`) but it is not a body state (`Feature::body` is none), like a datum, and it can be
  hidden. The face's origin features are its `origin_features`.
- `ThreadForm::Modelled` (`Thread::is_modelled`: not for taper pipe, which stays cosmetic whatever
  the form says) modifies its body (`Feature::body`, `modifies_body`, state before kept) after the
  same checks: `Bore::modelled_spec` builds the kernel `ThreadSpec` (frame at the start along the
  thread, a start at an open end runs out, the far end runs out only when the whole face is threaded
  and that end is open, else both stop square) with `ThreadSize::modelled_profile`, and
  `cut_thread` subtracts the kernel's `thread_tools` one by one, keeping them as the result's
  `cuts` (so patterns and mirrors of features repeat the thread). Profiles: ISO metric is the basic
  ISO 68-1 profile, crest and root at D (d) and D1 = d1 = D - 1.0825 P for both sides, flats P/8 and
  P/4, whatever the class; ISO 228 Whitworth truncated flat at 2/3 H (flats P/6); trapezoidal 15°
  flanks between the table's major and minor diameters with crest flats of the basic profile
  (0.366 P). Kernel refusals become sentences naming the size and the face (`ThreadCause::Modelled`), each
  ending "or make the thread cosmetic"; a failing boolean gives its point. Thread faces are worded
  "<feature> thread root/flank/crest/groove/end" (`thread::part_name`).
- `Document::threads_as_cosmetic` is a copy with every modelled thread and tapped hole cosmetic (the
  app recomputes it while the session shows modelled threads as cosmetic); `has_modelled_threads`
  says whether there is any. Feature content differs, so recompute caches both evaluations.
- `Bore::of` reads the face: internal when the face's sense is reversed (normal points at the
  axis), external otherwise; the axial extent from its edges; and which ends are open (an edge
  meets a face whose normal points away from the face's span: a mouth, counterbore floor, free end
  or chamfer; never a blind bottom or shoulder). The thread starts at the open end (the upper end
  when both or neither), the other with `reversed`, and runs the whole face or `ThreadLength::Depth`
  (a length above zero; past the face stops at its end).
- It fails alone naming the face or body when the face is lost, tied, not cylindrical, the class is
  not one the standard offers for the face's side, or the diameter does not fit the size
  (`ThreadSize::fits`: a bore between minor diameter less a pitch and major, a shaft between minor
  and major plus a pitch), suggesting the nearest size (`ThreadFamily::nearest`) when it fits.
- `ThreadFamily`: ISO metric coarse and fine (ISO 261, M1.6 to M64, every fine pitch), ISO 228
  parallel pipe (G 1/16 to 6), ISO 7 taper pipe (R 1/16 to 6), ISO 2901 trapezoidal (Tr 8x1.5 to
  Tr 100x12 at the preferred pitch). Minor diameters follow the standard's profile (metric D1 and
  d3, Whitworth for pipes, ISO 2904 clearance for trapezoidal, whose internal major is D4).
  `ThreadClass` is one the standard offers for a side (metric 6H, 5H, 7H, 4H, 6G and 6g, 6h, 4h,
  8g, 6e, 6f; ISO 228 one internal class and A or B; ISO 7 Rc or Rp and R; trapezoidal 7H, 8H and
  7e, 8e, 7c, 8c), the first the default. `ThreadDesignation::text` writes the notation (`M8-6g`,
  `M8x1-6H-LH`, `G 1/2 A`, `Rc 3/4`, `R 1 LH`, `Tr 20x4 LH-7e`).
- `placed_threads` places every thread for showing and exporting on the bodies as they finally
  stand, so a thread follows a later move, mirror or scale of its body (names kept) and lands on the
  body a combine joined it into; a face no longer found there, and failed threads, are not placed.
  A tapped round hole (`hole_thread`: a `standard` with tapped or fine fit, size in the metric
  tables) places one thread per wall face (`Hole::is_wall`, grouped by copy and entity so a split
  bore is one thread; copies from patterns and mirrors included, `FaceOrigin::Copy`), as its
  `TappedThread` says: class (none is the family's first internal class, 6H), hand, form, and the
  whole wall or a depth (a length expression, a parameter user) from the bore's open end. The hole
  fails alone when the class is not an internal one of the family or the depth is not a length above
  zero. A modelled tapped hole (`Hole::is_modelled_thread`) cuts, after drilling, a thread into each
  wall (grouped by entity) as a Thread does, with the tools from `thread_tools_numbered`: each part's
  entity becomes `Hole::thread_entity(wall, part)` (bit 55 set, the wall entity above 16 bits of
  part, the ring's parts marked by bit 15), so bores keep their threads apart and `is_wall` and
  `part_name` tell them from the hole's own faces; the tools join the hole's `cuts`.
- Placing reads the result: a modelled Thread's (a `Solid` result) and a hole's are found on the
  final bodies by their thread faces (`thread_faces`, grouped by copy and wall): the axis and side
  from a root face's cylinder, the extent along it from every face's edges, the start at the end of
  the lowest turn (`modelled_placement`), marked `PlacedThread::modelled`; a hole wall with no thread
  faces places a cosmetic thread as before. So an evaluation of `threads_as_cosmetic` places them
  cosmetic for the same document.

### Move (`movement.rs`)

- `Move { body, offset, turn, copy, about }` places an existing body: turns about X, Y then Z
  through the origin, or with `about: TurnCentre::Body` through the centre of the body's box before
  the move (`Move::pivot`), then shifts by three distances (all expressions, any sign). With
  `TurnCentre::Axis` (an `AxisTurn`: an `AxisReference` resolved at the move's place, and an angle)
  it first turns by that angle about the axis in the axis's sense, then by the three turns about
  the axis's point (`Pivot::Axis`), then shifts; the axis's body, datum and sketch count as used and
  its edge or face is healed. With `frame` (a coordinate system, counted as used) the turns are
  about its X, Y, Z axes (through its origin when turning about the origin) and the distances run
  along its axes (`MoveAxis::direction_in`).
- It modifies its body like a blend, keeps every face and edge name (`Solid::transformed`), and
  fails alone when a value is not a length or angle or the result is not finite. With `copy` it
  leaves the body alone and makes a new body of its own (`makes_body`, `Feature::body` is the move)
  from the placed copy; a copy others use cannot stop being one.

### Mate (`mate.rs`, `mate_placement.rs`)

- `Mate { body, pair, flipped }` places an existing body by its geometry. `MatePair`:
  `Faces` (`FaceMate`: a `FaceReference` of the body resolving to one plane, a `PlaneReference`
  target, a length `distance`); `Axes` (`AxisMate`: two `AxisReference`s, the first normally an
  edge or round face of the body); `FaceAxis` (both, flush and concentric); `Angle` (`AngleMate`:
  `AngleSides::Faces`, a `FacePair` of a flat face and a plane, or `AngleSides::Axes`, and an angle
  from 0 to `MAX_MATE_ANGLE_DEGREES`); `Tangent` (a `FacePair` whose face is a cylindrical or
  spherical face of the body, `round_face`); `FaceOnRound` (a flat face of the body and `round`, a
  `FaceAttachment` to a cylindrical or spherical face of another body, `Mate::round_targets`);
  `Point` (`PointMate`: a `PointReference` on the body and a `PointTarget`, a point or a plane).
  Pieces of one surface are accepted. References resolve at the mate's place, the moving ones on
  the body as it stands before it, so the mate follows its target on every recompute.
- Placement (`mate_placement.rs`): Faces turn the body about its box centre by the least turn
  taking the face's outward normal opposite the target's (the same way when `flipped`; a half turn
  uses the face's x axis), then shift along the target normal until the face lies `distance` beyond
  the plane. Axes turn about the axis point nearest the box centre by the least turn onto the
  target direction (reversed when `flipped`), then shift square to the target so lines coincide.
  Face and axis line the axes first, direction chosen so the face ends opposite the plane, then
  slide along the target axis until the face lies `distance` beyond the plane; an axis along the
  plane fails (`Unplaced::AxisAlongPlane`). It is one feature because two mates in a row would let
  the second undo the first.
- Angle is only a turn, by the least turn bringing the angle between the face's outward normal and
  the reverse of the target's normal (0 is facing) or between the two axes to the angle; faces turn
  about the line where the planes meet nearest the box centre (box centre when parallel), axes about
  the point of the moving axis nearest the target axis. Outside 0 to 180 deg fails.
- Tangent: a cylinder first turns, about the axis point nearest the box centre, laying its axis
  along the plane; then the body shifts along the plane's normal until the axis or sphere centre
  lies its radius from the plane on the side the normal faces (other side when `flipped`). Flat face
  on a round face: for a cylinder the body first turns laying the face's normal square to the axis;
  then it shifts along that normal until the centre lies the radius in front of the face when the
  round face is convex (`Convexity`, from its sense), behind when concave (a bore), other way when
  `flipped`. Point: the body shifts so the point lands on the target point, or along the plane's
  normal onto the plane. Angle and point mates have no side (`Mate::flips`).
- It modifies its body like a move (`modifies_body`, state before kept), keeps every name, and its
  target bodies, datums, frames and sketches count as used; healing visits the moving face and every
  reference. A lost, split or curved moving face (lost or flat round face for a tangent), or a lost
  or not round face to rest on, fails it naming the face; the round face's body counts among
  `bodies_used`. A placement too far fails in words. Only the distance scales with the model.

### Mirror and scale (`mirror.rs`, `scaling.rs`)

- `Mirror { body, plane, keep_original }` reflects its body across a `PlaneReference` resolved like
  a datum's at its place (datum and face body counted, face healed). Alone it replaces the body by
  the image (`Solid::mapped`), keeping every name; keeping the original runs kernel `pattern` with
  the image as copy `MIRROR_IMAGE`, so image faces are `FaceOrigin::Copy` of the mirror
  (`describe_origin`: "<mirror> image of …").
- With `mirrored` features (`Mirror::mirroring`, chosen as a pattern's repeated ones by
  `repeatable_on`) it reflects their tools instead: each placed as copy `MIRROR_IMAGE` (kernel
  `pattern_copies`) and cut from or joined to the body as it stands, a fillet or chamfer blended on
  the image as a pattern repeats it (Repeated blends, Pattern below), originals staying;
  `keep_original` is then ignored. The lookup (`pattern::seed`, the `Repeater` that words it) is shared with the
  pattern, the mirrored features are in `features()`, and the cut images are the mirror's `cuts`.
- With `faces` (`Mirror::mirroring_faces`, `FaceReference`s resolved on the body at the mirror,
  pieces accepted, lost or tied fails) and no mirrored features (`mirrors_faces`) it reflects the
  region the faces bound: kernel `enclose_faces` closes them across their openings (flat, or with a
  patch of the round face a pocket was cut into or a boss stands on) into a tool, placed as copy
  `MIRROR_IMAGE` and cut where the faces bounded a cavity (`Bounds::Cavity`, kept as `cuts`) or
  joined where they bounded material (`joins`); `keep_original` is ignored. Openings neither in one
  plane nor on a neighbouring face, freeform faces, faces around a round face, faces not in
  separate loops, or ones closing into no valid solid fail alone in words asking for every face
  around the pocket or boss. The faces' origin features are its `origin_features`; healing visits
  each face.
- `Scale { body, factor, center, frame }` resizes its body about a point (`Solid::mapped`), keeping
  names. Factor is a plain number `MIN_SCALE_FACTOR` to `MAX_SCALE_FACTOR`; the centre is three
  lengths from the origin along the world's axes or, with `frame` (counted in `frames_used`), its
  origin and axes. A result past `MAX_SIZE` or with an edge below the resolution fails alone.
- Both change their body but, like a pattern, are not `modifies_body`: nothing is chosen on the
  state before them, so the app shows the result while one is open.

### Split (`split.rs`)

- `Split { body, along, flipped }` cuts its body along a `SplitAlong`: the part on one side stays
  (the other when `flipped`), the rest becomes a body of the split's own (`makes_body`). Both come
  from one tool solid, intersected and subtracted, so the pieces share the cut face's name.
  - `Plane`: a `PlaneReference` resolved like a mirror's; the side its normal faces stays. The tool
    is a half-space block past the body's box by `HALF_SPACE_REACH` of its diagonal plus
    `HALF_SPACE_MARGIN`.
  - `Sketch`: an earlier sketch whose non-construction curves form one open chain joined end to end
    (no circle, branch, loop or second chain), carried on along its end tangents to a rectangle
    around the body and curves, closed along it on the left of the chain walked from its first end
    (the end on the curve of lowest entity id; the right when `flipped`), and the region extruded
    under the split's id from below the body to above along the sketch normal
    (`swept_half_space`); closing lines take entity ids from `CLOSURE_ENTITIES`. A closed outline
    that is not one region fails. The sketch is in `reference_sketches`.
  - `Body`: another body as it stands at the split, used whole as the tool (what lies outside it
    stays). It counts in `bodies_used` and is left as it was; splitting a body along itself fails.
  - `Surface`: a `FaceAttachment` to one face of a body as it stands (its own included; pieces of
    one surface accepted), carried past its edges (`surface_tool.rs`): a flat face is its plane's
    half-space, a cylinder a solid cylinder extruded along its axis past the box, a cone the solid
    cone of its nappe, a sphere and torus their whole solids, with entities `SURFACE_ENTITY`,
    `CAP_ENTITY`, `AXIS_ENTITY`. The side the outward normal faces stays (outside a shaft, inside a
    bore), the other when `flipped`. A freeform face, a lost face and pieces no longer on one surface
    fail in words. Body counted in `bodies_used`, origins among the feature's, healing visits.
  - A tool that misses the body fails naming the plane, curve or body; a sketch's own problems fail
    with the fix on the sketch.
- Its result holds two bodies: the kept part is the result's `SolidResult` and the split-off part
  is in `SolidResult::others`. `Feature::bodies` lists both (settling counts both), the walk stands
  each part under its own body (`body_parts`), and `Evaluation::body_result` and `body_seen_by` find a
  body inside its state's result (`body_part`). A feature making a body of each piece holds its
  pieces the same way (Separate pieces below).

### Separate pieces (`body_pieces.rs`)

- A feature's `pieces` (`Pieces::OneBody`, the default and what every older model holds, or
  `BodyEach`) says whether a new body of several pieces that do not touch stays one body of several
  lumps or becomes a body per piece. It lives on the `Feature`, beside `hidden` and `appearance`, so
  every body-making kind can take it; only a `SolidFeature` (extrusion, revolve, sweep) making a
  `NewBody` acts on it (`Feature::can_separate_pieces`, `separates_pieces`). A pipe follows one path
  without gaps and so is always one piece; a primitive, import or copy is one solid as made.
  `Edit::SetFeaturePieces` sets it (undoable, journaled); the recompute cache compares it with the
  kind.
- A piece's body ID is a `FeatureId` at or above `FIRST_UNSTORABLE_ID`: the mark bit, the making
  feature's ID (below 2^31) and a 32-bit `PieceKey` (`FeatureId::piece`, `piece_key`, `maker`,
  `is_piece`). So every field naming a body (a target, a tool, a blend's or reference's body, a
  selection set's or configuration's colour) names a piece unchanged, and the file stores it as any
  body ID. `FeatureKind::features` maps every ID through `maker`, so dependencies, deletion
  questions, moves, inserts' checks and the ID counter see the making feature; `bodies_used` and
  `consumed_bodies` keep the piece IDs. A piece of a feature with a larger ID, or keyed past 2^32,
  fails the feature in words.
- The key is the sketch curve that drew the piece, never its place in the result:
  `body_pieces::separated` takes the built solid apart (`Solid::lumps`), gathers the entities of each
  lump's side faces made by the feature (`FaceOrigin::Side`), and keys the lump by the lowest of
  them; lumps sharing their lowest curve (a long line under two shapes) each take their lowest curve
  no other lump has, and lumps drawn wholly by shared curves fail the feature saying they cannot be
  told apart. Moving shapes, drawing new ones (new bodies) and holes inside a shape keep every key;
  pieces that grow together into one keep the key of the one whose curve was drawn first (the lowest
  entity ID), and the other's body is gone. Each curve counts as the lowest deleted curve it stands
  in for (`PieceStandIns::standing_for`, the feature's `piece_stand_ins`), so deleting and redrawing
  one curve of a shape keeps its key: `Transaction::remove_sketch_items` and `reshape_sketch`
  (`keeping_piece_keys`, for each separating feature on the sketch) hand a deleted curve's key, or
  the deleted key it carried, to the lowest-standing curve meeting one of its ends
  (`Entity::ends` within `LINEAR_RESOLUTION`, construction left out) when that stands lower than
  the curve, in one `Edit::SetPieceStandIns` (whole set, inverse the previous) per feature, keeping
  one entry per carrier; a curve with no curve at its ends (a circle, a whole shape deleted) hands
  nothing on, so a shape drawn again is a new body. The recompute cache compares the stand-ins with
  `pieces`, pasting carries them. Pieces sort by key: the first is the result's own
  `SolidResult`, the rest its `others`, all under the making feature's state, so `body_part`,
  `Evaluation::body_result`, `bodies` and the stale last good bodies (`last_good_bodies`, by maker)
  read them like any body; `Evaluation::pieces_made_by` lists a feature's pieces. Settling at a
  separating feature's index settles each of its pieces no later feature changes
  (`settled_bodies`).
- A feature using a piece that no longer stands fails through `Inputs::missing_body`
  (`missing_piece`): with the maker separating, "The piece of <feature> drawn by <curve> is no
  longer there" ("a curve since deleted" when the curve is gone), fixed on the sketch; with the maker
  no longer separating, saying to set it back to a body each.
- `piece_looks` (`PieceLooks`) holds each piece's own `BodyAppearance` and hidden flag by key. A piece
  without its own look takes the feature's appearance without its name (`Feature::piece_appearance`,
  `inherited_piece_appearance`); `SetBodyAppearance` of an appearance equal to that stores none, so
  the inverse restores exactly what was there. `InsertFeature` settles them and checks each
  appearance; their densities count among the feature's parameters, inlining rewrites them per
  piece and pasting carries them. Looks of pieces gone are kept, so a piece drawn again gets its
  look back.
- `Document::body_name` (a `Cow`) gives a piece's own name, else "<feature> (<curve label>)"
  (`piece_curve`, the key entity's label in the sketch while it exists, else its carrier's); `body_label` gives a piece that
  name and any other body its feature's name, which every failure naming a body uses. `body_maker`
  is the making feature. `features_on_body` of a piece includes the features on its maker, so the
  scene's health and `face_splits` see them. `bodies_before_in` and `bodies_standing_in` list the
  bodies with each separating feature's pieces, read from an `Evaluation`; `bodies_before` and
  `bodies_standing` stay by making feature.
- Pasting maps a piece of a copied feature through a placeholder of its own
  (`PIECE_PLACEHOLDER_BASE`, below `FIRST_UNSTORABLE_ID` like the plain placeholders), so a copy
  using a piece of a copy uses the matching piece of the new feature.

### Split face (`split_face.rs`)

- `SplitFace { body, faces, along, carry }` divides the chosen faces of its body (`FaceReference`s
  resolved in the state before it, pieces accepted, lost or tied fails) along a `SplitAlong`,
  without changing the shape (kernel `split_faces`): a plane's half-space (`split::half_space_solid`,
  the normal side `Inside`), a sketch of one open chain (`Sweep::half_space`, left side inside), a
  sketch of closed outlines extruded through the body (`Sweep::outlines`, outlines inside), another
  body as it stands (inside it), or a body face's surface carried past its edges (`surface_tool`). A
  sketch with one open chain plus closed outlines (`split::mixed_curves`: curves grouped by shared
  ends, exactly one group with free ends, whose bridges, curves on no cycle, form the chain; its
  other curves join the outlines; a chain end at an outline's corner is carried straight on) splits
  twice under the same feature: along the outlines, then the resulting pieces along the chain, named
  `split(feature, split(feature, original, outline side), chain side)`; a pass dividing nothing is
  skipped, both skipped is the miss below. It modifies its body (`modifies_body`, state before kept);
  its plane's datum and face body, tool body and sketch count as used like a split's, healing visits
  its faces and plane.
- `carry` (`SplitCarry`) says how a sketch's curves reach the faces: `Square`, `Along` a boxed
  `AxisReference` or `Wrapped`. It applies only to a sketch; `SplitFace::with_along` sets it square
  when the split switches to a plane, body or surface.
- `Along` (edge, axis, round face's axis or sketch line) carries the curves along that line (kernel
  `extrude_along`, turned to rise along the sketch normal); the block's outline covers the body's
  box projected along it onto the sketch plane. A direction in the sketch plane fails naming axis and
  sketch. Body, datum, frame, sketch count as used (`direction_body` etc.), origins among the
  feature's, healing keeps it.
- `Wrapped` wraps the curves round the cylinder or cone the chosen faces lie on: every face on one
  such surface (cylinder axis line and radius, cone apex, opening and half angle, within
  `tolerance.rs`; `on_one_unrolled_surface`, which the app asks too), the sketch plane running along
  its axis. Closed outlines (even depth) go to kernel `wrap_regions`, one open chain to `wrap_chain`
  (a cut along it), a mixed sketch takes both passes. Fails in words for a face neither cylindrical
  nor conical (a sphere said to have no flat unrolling), faces of several surfaces, a plane across
  the axis, faces with no common seam, outlines farther round than once (mm round a cylinder,
  degrees round a cone), curves past a cone's apex, a chain end running straight round, a chain
  crossing itself or closing round to the same edge; the sketch's with the fix on the sketch.
- A tool crossing none of the chosen faces fails naming the plane, curve or body; splitting along
  its own body, a sketch's problems and kernel failures (`trouble.rs`) fail it alone.

### Emboss (`emboss.rs`)

- `Emboss { body, faces, sketch, regions, depth, form }` raises (`EmbossForm::Raised`) or sinks
  (`Recessed`) every closed outline of its sketch (`Selection::EvenDepth` of `solid::profile_curves`,
  so a letter's counter stays open and curves enclosing nothing are ignored), or only the chosen
  `regions` (a `RegionChoice` as an extrusion's, `RegionReference`s resolved against the sketch's
  cached profile so a changed region is healed by the healing visitor or left out; none chosen waits
  as `MissingInput::RegionsToEmboss`, every chosen one gone or a tie fails with `RegionGone` or
  `RegionAmbiguous` naming the sketch) on the chosen faces by `depth`, a length above zero (an
  expression, scaled with the model). `faces` are `FaceReference`s
  resolved as a split face's (pieces accepted, lost or tied fails); none waits as
  `MissingInput::FacesToEmboss`, no `sketch` as `SketchToEmboss`. It modifies its body
  (`modifies_body`, state before kept); its sketch is in `features()` and `reference_sketches`, its
  faces' origins among `origin_features`, healing visits the faces ("its embossed face").
- The material is one tool body, joined to the body (raised) or cut from it (recessed), and a union or
  difference of it is the whole result; its faces are named from the feature as an extrusion's are
  (`FaceName::side` of the sketch entity, a top `end_cap`/`start_cap` of the region), so later
  features hold the letters' tops and floors. `describe_origin` words them "<emboss> top" (raised) or
  "floor" (recessed) and "<emboss> side from <curve>". The tool spans from `EMBED` (0.01 mm) on the
  other side of the face to `depth` on the chosen one, so it crosses the face rather than lying on it.
  The tool is also kept as the result's `joins` (raised) or `cuts` (recessed), which patterns and
  mirrors of features repeat as a rigid copy (`repeatable_on`), never wrapped again.
  - Flat faces (all `face_plane`s in one plane, `tolerance::same_plane`): the sketch must be parallel
    to the plane (`NotParallel`); the tool is kernel `extrude` of the regions between the heights
    that put the face plane at 0, whatever plane the sketch lies in.
  - Faces of one cylinder or cone (`wrapping_surface`, shared with split face, so a sphere, faces of
    several surfaces and `NotAlongAxis` fail as its wrap does): the tool is `wrap_regions_within` with a
    `WrapBand` of `depth` towards the side the material goes and `EMBED` the other, `inward` and
    `outward` swapped for a concave face (a bore, sense reversed) so Raise always leaves the body. A
    depth leaving no radius (`WrapError::ReachesAxis`) is `TooDeep`.
- Before joining, kernel `split_faces` of the chosen faces along the tool must divide them, and no
  piece inside the tool (`FaceName::split(feature, name, Inside)`) may border a face that is not
  chosen (`reaching_edge`): an outline reaching or crossing the edge of a face, or a hole in it, is
  `RunsOff` naming sketch and face, and one touching none is `Misses`. Boolean failures go through
  `trouble.rs` ("Move its curves"); a recess leaving nothing is `LeavesNothing`.
- Wrapped outlines and their errors are those of split face (`document.md`, Split face): outlines
  reaching round more than once, past a cone's apex and a sketch plane across the axis are worded
  with the fix on the sketch.

### Remove (`removal.rs`)

- `Remove { body }` takes a body out of the model from its place on: its result passes the body's
  state through and `consumed_bodies` drops it, as a Combine drops its tool, so suppressing, rolling
  back or deleting it brings the body back. A feature still using the body fails naming the removal
  (`Inputs::missing_body`).

### Combine (`combine.rs`)

- `Combine { body, tool, more_tools, keep_tool, operation }` joins, cuts or intersects existing
  bodies (kernel `boolean`) and keeps the target's body ID. `Combine::tools` is `tool` then
  `more_tools`, met one after another; a failure names the tool it stopped at. Unless `keep_tool`,
  the tools' bodies are consumed (`consumed_bodies` tells recompute to drop them and not show them
  as stale last-good bodies), so later features cannot use them and suppressing or rolling back
  brings them back; a kept tool stays standing. Every body counts as used (`bodies_used`), so
  deleting any asks first and none moves below the combine. `bodies_before` lists the bodies
  standing before a feature, for the panel.
- Every evaluator reports an absent input body through `Inputs::missing_body`: when a Combine
  consumed it, the error names that Combine and the body it kept, with the fix target on the
  Combine, never blaming the healthy body.
- A body combined with itself, a tool chosen twice, a missing shape and kernel failures fail the
  combine alone with bodies named; every input keeps its result.

### Pattern (`pattern.rs`)

- Repeats the whole body as it stands before the pattern and unions the copies (kernel `pattern`).
  Boxed in `FeatureKind`. It changes its body but is not `modifies_body` (the app shows the
  patterned body while open).
- With `repeated` features (`Pattern::repeating`; `repeatable_on`: an extrusion, revolve,
  primitive, pipe, coil or loft adding to or removing from a body, a hole, an emboss, a fillet or
  chamfer, or a shell) it repeats their tools instead: for each, in tree order, the tools kept in its result
  (`SolidResult::cuts` of a removal or hole, `joins` of an addition) are placed at every copy and
  unioned without the original (kernel `pattern_copies`), then cut from or joined to the body as it
  stands. A blend is repeated as Repeated blends below, a shell as Repeated shells. The repeated features are in `features()` and a
  change to their tools recomputes it (`same_shapes` compares cuts and joins). A repeated feature
  with a missing result, that changes another body, or that neither adds nor removes fails the
  pattern naming it. A copy that misses the body changes nothing. The cut copies are the pattern's
  `cuts`; their faces are `FaceOrigin::Copy` of the pattern.
- Repeated blends (`repeat_blend.rs`, shared with the mirror): a blend's result keeps a
  `BlendRecipe` (`SolidResult::blending`): its `BlendShape` and, per edge it blended, the
  `EdgeReference` and the faces (names and origins) around each end, captured on the body before
  it. `pattern::seed` returns it as `Repeated::Blend`. `CopiedNames` follows each face name onto a
  copy: a face made by a feature repeated earlier in the list (origin not itself a copy) is
  `FaceName::pattern(pattern, copy, name)`, a face of an earlier repeated blend is the name that
  blend gave on that copy (recorded as `blend(blend, edge)` to `blend(pattern, copied edge)`), any
  other face of the body keeps its name (a plate the boss stands on). A corner face of an earlier
  repeated blend (`FaceName::corner` of the blend and the vertex) is followed too: for each end of
  each edge that blend repeated on a copy, the corner of the original's faces around the end is
  recorded as `corner(pattern, ...)` of the followed faces, which is what `blend_copy` names it. An
  edge none of whose faces is followed is not repeated; a blend with no such edge fails (`NoRepeatedEdge`, asking to repeat the
  feature it blends too). For each copy the edges are rebuilt as references (`between` of the
  followed faces, ends `VertexName::of_faces` of the followed faces around) and resolved by name
  alone in the body as it stands, then blended on that copy (kernel `blend_copy`, faces `Copy` of
  the pattern with the blend's feature). A copy where none is found changes nothing (the copy
  missed the body); some missing or one tied fails the pattern naming the blend and the copy, as
  does a kernel refusal (`repeat_blend::failure`). Edge references are rebuilt per copy, so a
  raised count blends its new copies.
- Repeated shells (`repeat_shell.rs`, shared with the mirror): a shell's result keeps a
  `ShellRecipe` (`SolidResult::shelled`, compared by `same_shapes`): the body state it hollowed
  (the `Arc` of the state before it, counted in the history's measure with cuts and joins), the
  thickness, side, and the names and origins of the opened faces and of the faces given their own
  thickness. `pattern::seed` returns it as `Repeated::Shell` only when the shell is the body's
  state at the pattern; otherwise it fails naming the feature changing the body after the shell
  and asking to move the pattern right below it. `Seeds::of` takes the shell out of the list: the
  pattern starts from the body before the shell, repeats every other feature as above, then
  hollows the result once (kernel `shell_copies`) with the original's openings and each copy's,
  followed by `CopiedNames` like a blend's faces; a face kept by name (the plate's bottom) stays
  one opening. A copy none of whose followed openings is found changes nothing (it missed the
  body); some missing, an original opening no longer found, or a kernel refusal fail the pattern
  in words (`repeat_shell::failure`).
- Axes are `AxisReference`s resolved like a datum's at the pattern's place; body and datum counted.
  A circular pattern's axis is optional (`CircularPattern::axis`): none is a pattern not yet given
  one, which waits for it (`MissingInput::Axis`, `document-recompute.md`), as a sweep with
  `SweepPath::Unchosen` waits for its path.
- Counts are whole and from 1, at most `MAX_PATTERN_INSTANCES` in all (directions multiplied), each
  copy costing a boolean. The reach may not pass `MAX_SIZE`; parallel directions are refused. A
  linear direction's length is `measured` between neighbouring copies or, as
  `LinearSpacing::Total`, from the body to the last copy, shared evenly.
- `skipped` holds the `Instance`s left out. The original (`ORIGINAL_INSTANCE`) can never be: an
  insert or kind change drops it and, but for a points pattern, any step past
  `MAX_PATTERN_INSTANCES` (`Pattern::settle_skipped`), as loading does. One
  past the current counts is kept but changes nothing, so lowering and raising a count brings the
  same copies back. Copies made keep their indices and names.
- `PatternKind::Curve` (`CurvePattern`) follows an earlier sketch whose non-construction curves form
  one chain, open or closed (`pattern_path.rs`, `curve_path`); a branch, second chain or zero-length
  curve fails with the fix on the sketch. An open chain is walked from its end on the curve of
  lowest entity id (that curve's start if both are), a closed one from the start of its lowest
  curve, the other way when `reversed` (an open one from its far end). Copy `[i, 0]` stands at arc
  length `i` steps along it: `CurveSpacing::Spread` shares the whole length (over `count - 1` steps,
  `count` round a closed chain), `Distance` is the `spacing` expression; copies reaching past the
  end fail saying how long the curve is. Each copy is the original moved as the walk's start point
  would move to that station (`along_curve`), turned about the sketch normal through the start by
  the change of tangent when `CopyOrientation::Following`. Arc length is exact on lines and arcs
  and bisected on splines and ellipses. `PatternPath` hands the walk to the interface: the length,
  whether it is closed, and `copy_at` for the placement, point and tangent of the copy at an arc
  length, so the pattern's arrows stand where its copies do.
- `PatternKind::Points` (`PointsPattern`) places a copy at every lone point of an earlier sketch
  (`Sketch::layout_points`, solved), moved from `base`, a `PointReference` resolved like a datum
  point's (datum, frame, body, sketch and origins counted, edge or face healed). A point on the base
  is the original and makes no copy; no lone points fails with the fix on the sketch. A copy's index
  is its point's id plus one split into two words (`point_instance`, `instance_point`; the original
  stays `[0, 0]`), so adding or deleting points never renames others; copies are worded "the copy at
  point N" (`Pattern::instance_words`, `describe_origin`). `instances` is none.
- Both sketches are in `features()` and `reference_sketches` (an edit naming a non-sketch is
  `NotASketch`), so an edit to the sketch recomputes the pattern, which stays below it; a failed
  sketch fails the pattern naming it. `SetFeatureKind` may switch a pattern between all four kinds.

### Sketch and datum (`attachment.rs`, `datum.rs`)

- A sketch keeps its `Sketch` and optionally a `SketchAttachment`: a datum plane it follows, a plane
  of a coordinate system (`Frame`), or on a body a `FaceAttachment`. The stored plane is where it
  was placed; recompute resolves the reference in the body's state at the sketch's place
  (`FeatureKind::body_input`) and gives the solved geometry the face's plane, normal and frame.
  Fragments of a split face are accepted when in one plane; a lost, split or curved face fails the
  sketch alone with a fix. `SetSketchPlacement` sets plane and attachment together. A body with
  attached sketches cannot stop making a body. A face attachment is boxed
  (`SketchAttachment::Face`), keeping `FeatureKind` and so `Edit` small;
  `SketchAttachment::heap_size` counts the box and its neighbour names, `SketchFeature::heap_size`
  the projections.
- `projections` (`projection.rs`) map a projected entity to its `ProjectionSource`: an edge of a body
  (`EdgeReference`), a corner (`VertexName`), an entity of an earlier sketch, a section edge, a
  datum plane's line, an axis or a point (Datum sources below). Before solving, recompute places
  each in the sketch plane from the source as it stands at the sketch (`refreshed`): a line, circle
  or arc in a parallel plane (turned to stay counter-clockwise), a point, or otherwise a spline through `PROJECTED_SPLINE_POINTS` samples;
  another sketch's spline maps its points and keeps its kind (exact except a centripetal fit spline
  on a non-parallel plane, which passes the projected fit points but spaces them as in the new
  plane); an ellipse or elliptical arc (another sketch's, or a body's elliptical edge) in a parallel
  plane projects as an ellipse or elliptical arc (ends swapped when the planes face opposite ways),
  else as a sampled spline. The entity keeps the kind and point count it was made with, so a source
  that now projects to another kind, is missing, split ambiguously or unavailable fails the sketch
  alone naming entity and source. An edge an upstream trim, split or break cut into pieces is
  followed as the pieces it became (`SplitEdges::run`, `kernel-naming.md`): while they run on end
  to end the one projected curve spans all of them (a line, an arc or an elliptical arc from the
  run's start to its end, else a spline sampled along the run), keeping its entity, points and so
  every constraint and dimension on them. When a gap opens between the pieces, or one left the
  line, no single curve follows the edge: the sketch fails (`ProjectionError::Unjoined`,
  `PartlyTraced`) saying the edge was cut into parts, with the fix to delete the projection and
  project the parts wanted (which deletes the constraints on it, never silently), and never
  projects one piece or bridges the gap. A section edge's fragments are joined the same way. Source bodies count in `bodies_used`,
  source sketches in `features()`, edge origins in `origin_features`.
- `TransactionBuilder::add_projection` adds an `Outline` (`edge_outline`, `vertex_outline`,
  `sketch_outline`) as points, a curve and its source in one transaction; `add_projected` does the
  same and returns every entity added, so a caller can check constraints against a shadow sketch. A
  sketch's evaluation records every body standing at it (`Evaluation::body_result_seen_by`;
  `body_state_seen_by` for the result's `Arc`, which a cache keys on), so the app projects from it.
- Intersect sources (`section.rs`): a `Section` is an edge of the body cut by the sketch plane, the
  body intersected with the half-space above the plane (`split::half_space_solid`, extruded under
  the sketch's own id). Edges between the cut face (a cap whose origin is the sketch) and a body
  face are the section curves, named by the kernel from the face they cross (`EdgeName::between`)
  and kept as an `EdgeReference` with the cap's origin dropped, so `origin_features` never names the
  sketch. `refreshed` cuts each body once per recompute; a reference no longer found means the plane
  no longer cuts that face, a failing cut fails the sketch alone, cancelling stops it.
  `DatumPlane { datum, reach }` is the line where an earlier datum plane crosses the sketch plane,
  centred nearest the sketch origin and `reach` long either way (a reach not finite and above zero
  is refused, `ProjectionReach`) (`datum_outline` gives none for a
  parallel plane); datum sources count in `planes_used`, so an edit refuses a non-datum-plane
  (`NotAPlane`). `PrincipalPlane { plane, reach }` is the same line for XY, XZ or YZ; it has no
  source feature (`ProjectionSource::feature` is `None`), so only the sketch's own plane moves it.
- Datum sources: `DatumAxis { datum, reach }` and `PrincipalAxis { axis, reach }` project an axis
  onto the sketch plane (`axis_outline`): a line through the foot nearest the sketch origin,
  `reach` long either way (refused like a plane's reach), or a point where the axis stands square
  to the plane, so an axis turned end-on (or back) fails the sketch as a changed shape naming it.
  `DatumPoint { datum }` and `Origin` project a point (`point_outline`). A datum source that is gone
  or not of its kind fails the sketch alone (a deleted datum through `missing_upstream`); datum
  axes count in `axes_used` and datum points in `points_used` and `features()`, so the sketch
  depends on them and recomputes with them, and an edit refuses a non-datum-axis (`NotAnAxis`) or
  non-datum-point (`NotAPoint`). Principal axes and the origin have no source feature.
  `ProjectionSource::is_same_source` says two sources name the same geometry (edges by name,
  datums by id, principal geometry by kind), which the app uses to refuse projecting twice.
- Datums are planes, axes, points and coordinate systems with a `DatumResult`, referring to model
  geometry in each body's state at the feature's place. Edits refuse a sketch or plane based on a
  non-datum-plane (`NotAPlane`), an axis reference to a non-datum-axis (`NotAnAxis`), a point
  reference to a non-datum-point (`NotAPoint`), a coordinate-system axis or plane naming another
  kind of feature (`NotACoordinateSystem`, via `FeatureKind::frames_used`, which joins `features()`)
  and a sketch point or line of a non-sketch (`NotASketch`, via `reference_sketches`).
- A coordinate system (`Datum::Frame`, `DatumFrame`, "Coordinate system") has an origin
  (`PointReference`), an X axis (`AxisReference`, an edge included) and an XY plane
  (`PlaneReference`); `datum_construction::coordinate_system` takes Z as the plane's normal and X as
  the axis laid into the plane, reversed by `reverse_x` and `reverse_z`, Y completing a right-handed
  frame. An axis square to the plane fails naming both. Result `DatumResult::Frame(Plane)`.
  `AxisReference::Frame` and `PlaneReference::Frame` name one of its three axes or planes
  (`PrincipalAxis::in_frame`, `PrincipalPlane::in_frame`), working wherever an axis or plane
  reference does; `displayed_frame` and `displayed_plane` read them. `PointReference::Datum` cannot
  name one; `PointReference::Frame` names its origin (`Datum::frames`, "the origin of …").
- A `PointReference` is the origin, a coordinate system's origin, a datum point, a body corner
  (`VertexName`, resolved when exactly one vertex has it), the centre of a round edge (an
  `EdgeReference`, pieces of one circle accepted), the centre of a spherical or toroidal face
  (`SurfaceCentre`, a `FaceReference`) or a sketch point (in its solved plane);
  `PointReference::on_body` places the three body kinds on a given solid, for the app.
  `AxisReference::Sketch` is a line of an earlier sketch. Sketches they use join `features()`.
- `Datum::Point` places a `DatumPoint` at a point reference moved by three length offsets.
  `Datum::PlaneThrough` passes through three points (not collinear), lies `Midway` between two
  planes (halfway between parallel ones, else on the bisector of the acute angle), contains an axis
  and a point (not on the axis) or stands square to an axis at a point. `DatumAxis` also runs
  through two points or stands square to a plane through a point. Each refusal names the references
  and what to choose instead.
- `datum_construction.rs` holds constructions needing body geometry or line crossings.
  `PlaneThrough::Tangent` (`FaceTangent`: body, `FaceReference`, a `PointReference`) touches a
  cylindrical or conical face along the line nearest the point, its normal facing the point's side
  (refused when the point is on the axis, or the face is no longer a cylinder or cone).
  `PlaneThrough::SquareToCurve` stands square to an edge of any curve kind at a `CurveStation`
  (body, `EdgeReference`, length expression: arc length from the edge's start, back from its end
  when negative; past either end fails naming the edge's length). A station's edge and an edge
  middle's resolve through `SplitEdges::run` (`kernel-naming.md`): an edge a sketch edit split into
  pieces still running end to end is walked as one `EdgeRun`; pieces with a gap or only partly
  traced fail in words asking to choose the piece or edge again. `PlaneThrough::Lines` holds two
  lines (`AxisReference`s) that cross or are parallel (the same line, or neither, fails naming both).
  `Datum::PointBy` is a datum point with no offsets: `LinesCross`, `AxisAndPlane` (parallel or
  lying in it fails), `ThreePlanes` (two parallel or all three sharing a line fail), `Along` a
  `CurveStation`. Stations' bodies and edge origins count as used, their distance is a parameter
  user, healing matches their edges and faces.
- Curved faces at a point (a `FaceTangent`'s face and `toward` point, `face_foot`): the point of the
  surface nearest the point (`Surface::project`) and the outward normal there, turned to face the
  point's side. `PlaneThrough::TangentAt` touches the face there and `DatumAxis::SquareToFace`
  stands square to it, on any curved face; a flat face, a point at a sphere's centre or on a round
  face's axis fail alone in words. `PointBy::EdgeMiddle` sits halfway along an edge by arc length,
  `PointBy::FaceCentre` at the area centroid of a face (every piece of a split one), from the body
  tessellated at `MeshQuality::SMOOTH`, so a curved face's centre may lie off the face. Their bodies
  and edge and face origins count as used and healing matches them.

### Measurement (`measurement.rs`)

- `Measurement { reading, parameter }` keeps a reading in the tree: `Reading::Between`
  (`Between::Distance` or `Angle`) of two `MeasuredItem`s, `Reading::Along` (offset between two
  items projected on a third, an axis item, as Measure's Along X reads it), `Reading::Of`
  (`Of::Length`, `Radius`, `Sweep`, `Area`, `Perimeter`, `Volume`, `Mass`) of one, or
  `Reading::Position` (a point's signed offset along an axis item from the point the axis passes
  through; on a principal axis the world coordinate, on a coordinate system's axis the coordinate
  in it). An item is a `PointReference`, `AxisReference`, `PlaneReference`, an edge or face of a
  body, a sketch entity (on the sketch's solved plane) or a whole body (`MeasuredItem::Body`, as it
  stands at the measurement), resolved at the measurement's place (`Resolver`; an edge through
  `SplitEdges::follow`, its split pieces read together, `kernel-naming.md`; the app asks the same
  through `trace_edge`), so its bodies,
  datums, frames and sketches count as used (`MeasuredItem::features`) and healing visits its edges
  and faces. It changes no body (`Feature::body` is none) and can be hidden.
- Result `FeatureResult::Measurement` (`MeasurementResult`: the value as a `Quantity` in mm,
  degrees or mm squared, the closest points as `line`, an `anchor` to label at, the kernel's
  `Accuracy`); dependents compare only the value. The kernel measures it (`distance`, `angle`,
  `edge_measure`, `curve_measure`, `face_form`, `face_area`); a sweep is a circular edge's or
  curve's turn in degrees, a perimeter the length of a face's outer loop, seams left out
  (`face_perimeter`, which Measure's face cards also read). A body item stands for its centre of
  mass wherever a point would, and reads volume (mm cubed), surface area and mass, a plain number of
  grams (`MASS`, `Dimension::NONE`) from its density (`BodyAppearance::mass_grams`).
- These come from `SolidResult::exact_mass` (`ExactMass`, kernel `mass_properties_tessellating`:
  exact by quadrature; a face that does not converge is taken from a `MeshQuality::SMOOTH`
  tessellation and the reading is approximate), worked out in recompute only when a measurement
  locates a body and kept inside the body result (a `OnceLock`), so an unchanged result `Arc` is
  never integrated again; it runs under the kernel interrupt the evaluation installs and a
  cancelled one is not kept. A body with no density, or one that does not evaluate, fails a mass
  reading with the fix on the body. A lost or tied reference, two items with no angle, an axis
  giving no direction, an edge with no length or an item with no radius, sweep, area, perimeter,
  volume, mass or position fails alone in words.
- `parameter` (optional) is the measured parameter its reading feeds. This version never evaluates
  its own expression (`ParameterError::Unmeasured` until recompute reads it, `values.rs`); the
  expression is kept as the latest reading for the ordinary parameter deleting the measurement
  leaves. `Document::following_readings` turns each up-to-date reading that differs into a
  `SetParameterExpression` of `reading_literal`, which `Editor::follow_readings` applies outside the
  undo history and without a new revision (the app runs it on every evaluation of the current
  revision, `app.md`); a prepared commit or undo that drops it is followed again on the next
  evaluation. `Document::deletion` (`keeping_readings`) also writes the stored reading into each
  surviving measured parameter in its own transaction, so a journal replayed onto a base that never
  followed still leaves the last reading. A failed or suppressed measurement's users still fail; the
  stored value is no fallback.
- `Measured` maps measured parameters to their measurements and every parameter reading one,
  directly or through others, to the measurements it reads (`measurements_read`, from evaluation
  order). A feature using a measured or derived parameter depends on its measurements
  (`dependencies_of`, which `dependents_of`, the tree's moves and the app's Uses and Used by take).
  Edits refuse a feature at or above a measurement it reads (a sketch's `SetDimension` and
  `AddSketchConstraint` too), an expression change making a parameter read a measurement some
  feature using it (`reading_users`) does not stand below, and a measurement placed below a feature
  reading its parameter, each as `MeasurementBelowUser` naming the chain as a path ("Base → half →
  clearance → Measurement 1 → Base", `Document::parameter_path`); a second measurement feeding the
  same parameter (`ParameterMeasuredTwice`); and inlining a measured parameter
  (`MeasuredParameterInlined`). A pasted measurement feeds nothing; its references follow a datum's
  paste rules.

### Reference image (`reference_image.rs`, `picture.rs`)

- A picture to trace over is a feature of its own, `FeatureKind::ReferenceImage` (boxed
  `ReferenceImage`), not an item of a sketch: it is placed like a datum or primitive (on a
  `PlaneReference`, or with `ImagePlane::Sketch` on a sketch's solved plane), it can stand on any
  plane without a sketch there, hides with the datums, and one feature row is what the user opens,
  hides, deletes and undoes. Keeping it out of `Sketch` also keeps the solver, sketch records and
  clipboard free of picture bytes. It makes no body (`Feature::body` none) and can be hidden.
- `ReferenceImage { picture, plane, at, turn, flipped, size, opacity, in_front, locked }`: `at` is two
  lengths along the plane's X and Y to the picture's middle, `turn` an angle about the normal,
  `flipped` mirrors it left to right, `size` is `ImageSize::Width` (a length above zero) or
  `Calibrated` (`Calibration`: two `ImageSpot`s, fractions of the picture's width and height from
  its bottom left as unflipped, and the real `distance` between them; the width is the distance over
  their gap with the picture's aspect, `Calibration::spot_gap`). Every value is an expression
  (parameters, inlining, model scale). `opacity` is a percent, settled by inserts and kind changes
  into `MIN_IMAGE_OPACITY_PERCENT` to `IMAGE_OPAQUE_PERCENT` (`ReferenceImage::settle`, spots into
  the picture); `in_front` and `locked` only change how the app shows and picks it.
- Recompute resolves the plane at its place (a sketch's from its result; a face healed as "the face
  it lies on"; datum, frame, body and sketch counted in `features()`, so a sketch-plane image stays
  below its sketch) and gives `FeatureResult::Image` (`PlacedImage`: middle, the unit `across` and
  `up` directions, width, height, flip; `point_at`, `spot_at`, `corners`, `placed_on`). A width not
  above zero, two calibration spots at one place, a distance not above zero or a picture past
  `MAX_SIZE` fails it alone in words.
- `Picture` holds the file's bytes as they were read (never re-encoded), their blake3
  `PictureDigest`, the `PictureFormat` sniffed from the bytes (`sniff_picture`: magic numbers for
  PNG, JPEG, WebP, JPEG XL as a codestream or container, GIF, BMP, TIFF, QOI, a plausible header for
  TGA; AVIF and HEIC named as `UnsupportedPicture`), and the pixel size the file decodes to, which
  the shape of the placed picture follows. `Picture::new` refuses (`PictureError`) empty or unknown
  bytes, more than `MAX_PICTURE_BYTES`, sides past `MAX_PICTURE_SIDE` and more than
  `MAX_PICTURE_PIXELS`; it never decodes (`caditor-file` does). Kinds share it behind an `Arc`, so a
  change of opacity copies no bytes; equality is by digest and size. `Picture::placeholder` is a
  grey pixel standing in for a picture a file lost. `ReferenceImage::heap_size` counts the bytes, as
  an import counts its STEP text, so undo bounds hold when a deleted image is the last holder.

### Import (`import.rs`)

- Keeps the source file's name, its path (`Import::path`, set by `Import::from_file` only for valid
  UTF-8), the solid and the canonical single-solid STEP text it was read from (what the file
  stores), both shared by copies of one part (`Import::shared`); equality compares name, path,
  text and placement.
- The text is a `StepText` (`step_text.rs`): zstd at `Level::FAST` behind an `Arc`, plain when
  packing fails or does not shrink it, unpacked only to save (`text`, a `StepTextError` when it will
  not). The text is needed only to write the file, and caditor's STEP text is about six times its
  packed size (the VZ330 kept 984 MiB of it unpacked). Packing is deterministic, so two packed texts
  are equal exactly when their frames are; `len` is the text's length (what a file record holds),
  `held_bytes` what memory holds (what undo counts), `address` the shared buffer's identity. The body is `Solid::imported`, so later features hold its faces and edges like
  any body's.
- `placement` (`BodyPlacement`, `movement.rs`) turns the body about X, Y then Z through the origin,
  then shifts it, as a `Move` does, all expressions (any sign) counted in the feature's parameters
  and inlined. With `frame` (counted in `frames_used`) the file's origin and axes are the system's:
  turned and shifted in it, then carried to it (`RigidTransform::from_frame`). `scale`, a plain
  factor expression (1 by default, `is_unscaled`), first resizes the solid about the file's origin
  (`Solid::mapped`, `MIN_SCALE_FACTOR` to `MAX_SCALE_FACTOR`, else the import fails alone). Zero
  everywhere, scale 1 and no frame (`is_at_origin`) leaves the solid as read; otherwise
  `Solid::transformed` keeps every face and edge name. A value that is not a length or angle, or a
  body placed too far, fails the import alone.
