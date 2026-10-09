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
  near linear in the number of parameters. Their order is the user's (`MoveParameter`) and changes
  nothing computed; a parameter's `note` (`SetParameterNote`, trimmed, at most
  `MAX_PARAMETER_NOTE_CHARS`) is free text for the user only.
- `Document::inline_parameter` builds the one transaction deleting a parameter in use: its
  expression (not its value, so links to the parameters it uses survive) is substituted into every
  parameter, sketch dimension, feature and body density using it (`Expression::inlining`, which
  round-trips through stored text so precedence is kept), then the parameter is removed. The
  model computes exactly as before. An expression that would grow past what the stored text reads
  back refuses as `InliningTooLong`.
- `Document::transaction_to` builds the transaction turning one document into another (bar to the
  end, everything removed, the target's inserted with IDs and flags, its bar restored), never
  lowering a sketch's ID counter; it restores an earlier version as one undoable change.

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
  or solid (none), which STEP import uses.
  A feature that later stops making a body keeps it, unused.
- The density is a plain number in g/cm³ that may use parameters, so `Feature::parameters` and
  `Feature::uses_parameter` (which parameter deletion, `parameter_users` and loading's stand-ins
  use) include it. It is evaluated where shown (`density_value`, `mass_grams`): above zero and at
  most `MAX_DENSITY`, else a `DensityError` saying why.
- `faces` lists `FaceColour`s, a `FaceReference` with its colour, later entries winning.
  `face_colours` maps a solid's faces to them in one pass: every face of the reference's name (so
  each fragment of a face a later feature splits keeps its colour), else the face the reference
  resolves to; a face found by neither keeps the body's colour.

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
  then points. `AddSketchEntity` carries the construction flag, and `AddSketchConstraint` the inactive flag, so
  undoing a removal restores them; `SetSketchConstraintActive` changes one constraint's flag and
  `reshape_sketch` emits it for kept constraints whose flag differs.
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
  (`start_plane`; a plane that is not parallel fails the feature, naming it). The profile starts
  there, and every end, symmetric extent, up-to search and a revolve's turn (its axis keeps the
  sketch's frame) works from there. `None` and zero length are the same and stored as absent. A
  start plane's body and datum count as used like an end's, and its face is healed like one.
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
- A removal also cuts each body of `other_bodies` (an extrusion's or revolve's, any other
  operation with some fails in words), every body with the same tool, so one feature changes them
  all: they are in `bodies_used` and `Feature::bodies`, their results are `SolidResult::others`
  (like a split's), Through all reaches past all of them, the target and repeats are skipped, and
  a body the tool removes entirely fails the feature naming it.
- A `RevolveAxis` is a line or axis of its own sketch (a sketch line used as axis cannot be
  deleted; `AxisNotALine`) or an `AxisReference` to a model axis lying in the sketch plane.
- A revolve's `side` (`AxisSide`, left or right of the axis's direction in the sketch plane; none
  turns the whole profile) keeps only the part of the chosen regions on that side: the sketch's
  curves are divided again with the axis as a line past the chosen regions' bounds (entity
  `REVOLUTION_AXIS_ENTITY`, worded "the revolution axis"), the regions whose anchor lies on that
  side inside a chosen region are selected as one, and they are revolved, so a profile crossing
  the axis or lying on both sides turns, and a side holding nothing fails in words.
- A feature whose body others change keeps making a new body.

### Blend and shell (`blend.rs`, `shell.rs`)

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

### Hole (`hole.rs`)

- `Hole { sketch, body, diameter, depth, style, reversed }` drills at every free point of its sketch
  (`Sketch::free_points`: no curve uses it, not construction; a point only a constraint uses still
  counts) and at the centre of every circle that is not construction (`hole::centres`), down into
  the sketch plane's normal, or up when reversed. `HoleDepth` is blind or
  through all (the farthest corner of the body past the point plus a margin, as the extrusion's
  through all); `HoleStyle` is plain, counterbore (diameter, depth), countersink (diameter,
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
  It names the thread of a tapped hole for the user and changes nothing computed: the expressions
  alone drive the hole. `MetricSize::offers` says which fits a size has (`fits` for the Fit row,
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

### Move (`movement.rs`)

- `Move { body, offset, turn, copy, about }` places an existing body: turns about the X, Y then Z
  axes through the origin, or with `about: TurnCentre::Body` through the centre of the body's box
  as it stands before the move (`Move::pivot`), then shifts by the three distances, all
  expressions (any sign). With `TurnCentre::Axis` (an `AxisTurn`: an `AxisReference` resolved like
  a pattern's at the move's place, and an angle expression) it first turns by that angle about
  the axis, in the sense of the axis's direction, then by the three turns about the axis's point
  (`Pivot::Axis`), then shifts; the axis's body, datum and sketch count as used and its edge or
  face is healed like any reference. It modifies
  its body like a blend does, keeps every face and edge name (`Solid::transformed`) so references
  held through it survive, and fails alone when a value is not a length or angle or the result is
  not finite. With `copy` it leaves the body alone and makes a new body of its own (`makes_body`,
  `Feature::body` is the move itself) from the placed copy; a copy others use cannot stop being
  one.

### Mirror and scale (`mirror.rs`, `scaling.rs`)

- `Mirror { body, plane, keep_original }` reflects its body across a `PlaneReference` resolved like
  a datum's at its place (its datum and face body count as used, its face is healed). Alone it
  replaces the body by the image (`Solid::mapped`), keeping every name; keeping the original runs
  the kernel `pattern` with the one image as copy `MIRROR_IMAGE`, so the image's faces are
  `FaceOrigin::Copy` of the mirror and `describe_origin` words them "<mirror> image of …".
- `Scale { body, factor, center }` resizes its body about a point (`Solid::mapped`), keeping every
  name. The factor is a plain number from `MIN_SCALE_FACTOR` to `MAX_SCALE_FACTOR`; the centre is
  three lengths. A result past `MAX_SIZE` or with an edge below the resolution fails it alone,
  saying which.
- Both change their body but, like a pattern, are not `modifies_body`: nothing is chosen on the
  state before them, so the app shows the result while one is open.

### Split (`split.rs`)

- `Split { body, plane, flipped }` cuts its body along a `PlaneReference` resolved like a mirror's:
  the part on the side the plane's normal faces (the other side when `flipped`) stays in the body,
  and the rest becomes a body of the split's own (`makes_body`, named after it). Both come from
  one half-space block (a rectangle on the plane past the body's box by `HALF_SPACE_REACH` of its
  diagonal plus `HALF_SPACE_MARGIN`, extruded to past the far side), intersected and subtracted,
  so the two pieces share the cut face's name. A plane that does not pass through the body fails
  it in words.
- It is the one feature whose result holds two bodies: the kept part is the result's own
  `SolidResult` and the split-off part is in `SolidResult::others`. `Feature::bodies` lists both
  (settling counts both), the walk stands each part under its own body (`body_parts`), and
  `Evaluation::body_result` and `body_seen_by` find a body inside its state's result
  (`body_part`), so later features, meshing, stale bodies and the app see two ordinary bodies.

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
- With `repeated` features (`Pattern::repeating`; `repeatable_on` says which: an extrusion or
  revolve adding to or removing from a body, or a hole) it repeats their tools instead: for each,
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

### Sketch and datum (`attachment.rs`, `datum.rs`)

- A sketch keeps its `Sketch` and optionally a `SketchAttachment`: a datum plane it follows, or on a
  body a `FaceAttachment`. The stored plane is where it was placed; recompute resolves the reference
  in the body's state at the sketch's place (`FeatureKind::body_input`) and gives the solved
  geometry the face's plane, normal and frame. Fragments of a split face are accepted when in one
  plane; a lost, split or curved face fails the sketch alone with a fix pointing at it.
  `SetSketchPlacement` sets plane and attachment together. A body with attached sketches cannot
  stop making a body.
- A sketch's `projections` (`projection.rs`) map a projected entity to its `ProjectionSource`: an
  edge of a body (an `EdgeReference`), a corner (its `VertexName`) or an entity of an earlier
  sketch. Before solving, recompute places each in the sketch plane from the source as it stands at
  the sketch (`refreshed`, in the body's state at that point): a line, a circle or arc lying in a
  parallel plane (turned to stay counter-clockwise), a point, or otherwise a spline through
  `PROJECTED_SPLINE_POINTS` samples; another sketch's spline maps its control points exactly. The
  entity keeps the kind and point count it was made with, so a source that now projects to another
  kind, is missing, was split ambiguously (pieces of one line merge) or is unavailable fails the
  sketch alone, naming the entity and the source. Source bodies count in `bodies_used`, source
  sketches in `features()` and edge origins in `origin_features`, so recompute reuses the sketch
  only while they are unchanged and a source stays above it. `TransactionBuilder::add_projection`
  adds an `Outline` (`edge_outline`, `vertex_outline`, `sketch_outline`) as points, a curve and
  its source in one transaction. A sketch's evaluation records every body standing at it
  (`Evaluation::body_result_seen_by`), so the app projects from that state.
- Datums are planes, axes and points with a `DatumResult`, referring to model geometry in each
  body's state at the feature's place in the tree. Edits refuse a sketch or plane based on a
  non-datum-plane (`NotAPlane`), an axis reference to a non-datum-axis (`NotAnAxis`), a point
  reference to a non-datum-point (`NotAPoint`) and a sketch point or line of a feature that is not
  a sketch (`NotASketch`, through `FeatureKind::reference_sketches`). `Datum::kind` (plane, axis,
  point) is what `SetFeatureKind` keeps, so a plane may switch between offset and through forms.
- A `PointReference` is the origin, a datum point, a body corner (`VertexName`, resolved when
  exactly one vertex has it), the centre of a round edge (an `EdgeReference`, pieces of one circle
  accepted), the centre of a spherical or toroidal face (`SurfaceCentre`, a `FaceReference`, pieces
  of one surface accepted) or a sketch point (in its solved plane). An `AxisReference::Sketch` is a line of an
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
  feature's parameters and inlined like any other. Zero everywhere (`is_at_origin`) leaves the
  solid as read; otherwise `Solid::transformed` keeps every face and edge name, so references held
  through the import survive moving it. A value that is not a length or angle, or a body placed
  too far, fails the import alone.
