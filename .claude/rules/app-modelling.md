---
paths:
  - "crates/caditor/src/solid_tools.rs"
  - "crates/caditor/src/solid_panel.rs"
  - "crates/caditor/src/blend_tools.rs"
  - "crates/caditor/src/blend_panel.rs"
  - "crates/caditor/src/shell_tools.rs"
  - "crates/caditor/src/shell_panel.rs"
  - "crates/caditor/src/offset_face_tools.rs"
  - "crates/caditor/src/offset_face_panel.rs"
  - "crates/caditor/src/primitive_tools.rs"
  - "crates/caditor/src/primitive_panel.rs"
  - "crates/caditor/src/combine_tools.rs"
  - "crates/caditor/src/combine_panel.rs"
  - "crates/caditor/src/move_tools.rs"
  - "crates/caditor/src/hole_tools.rs"
  - "crates/caditor/src/hole_panel.rs"
  - "crates/caditor/src/move_panel.rs"
  - "crates/caditor/src/move_manipulator.rs"
  - "crates/caditor/src/mirror_tools.rs"
  - "crates/caditor/src/mirror_panel.rs"
  - "crates/caditor/src/scale_tools.rs"
  - "crates/caditor/src/scale_panel.rs"
  - "crates/caditor/src/datum_tools.rs"
  - "crates/caditor/src/datum_panel.rs"
  - "crates/caditor/src/pattern_tools.rs"
  - "crates/caditor/src/pattern_panel.rs"
  - "crates/caditor/src/feature_fields.rs"
  - "crates/caditor/src/reference_picking.rs"
  - "crates/caditor/src/visibility.rs"
  - "crates/caditor/src/sketch_placement.rs"
  - "crates/caditor/src/reference_rows.rs"
  - "crates/caditor/src/principal_tree.rs"
  - "crates/caditor/src/feature_tree.rs"
---

# Modelling tools in the app

## Preview while open

- While a feature that makes or changes a body is open (its result is a solid), that body is drawn
  see-through at `PREVIEW_ALPHA` in its colour (`scene::previewed_body`), faces still picked, and
  turns solid when the feature closes, so a pattern, hole or extrusion reads as a preview
  until confirmed. It is drawn solid while choosing in the view.
- A feature choosing nothing on the state before it (a move, a combine) draws its result solid
  and the body before it see-through at `GHOST_ALPHA`, never picked (`scene::OpenView::Ghost`), so
  the body is seen where it goes and where it came from.
- An open fillet or chamfer that computed (`FeatureState::UpToDate`, not pending) draws its result
  with unpicked faces and edges, and over it the edges of the body before it, still
  `Pickable::BlendEdge` (`OpenView::Result`, `Builder::open_result`): the chosen ones stand just
  outside the rounded surface, and hidden ones stay hidden since unpicked faces still write depth
  in the pick pass. A failed or pending blend and a shell show the body before them instead
  (`OpenView::Before`).
- Typing in an open fillet's or chamfer's size field or a move's fields previews the value before
  it is entered: `commit_field` reports the text as edited (`FieldResponse::edited`) and
  `feature_fields::expression_row_drafting` turns valid text into `Action::Preview` with the
  transaction it would commit, never applied. `Model` applies it to a copy of the document
  (`DraftPreview`, dropped when the feature closes, the text turns invalid, Escape or leaving the
  field without a change). A blend's copy is computed as a draft (`document-recompute.md`) and its
  body drawn in the result's place once meshed (`BodyMeshes::draft`, `OpenDraft`); a move is not
  recomputed: `Model::draft_placement` (the draft's `Move::placement` after undoing the one shown)
  places the drawn body (`MeshInstance::placement`, its edges and vertices moved on the CPU).
  Entering the value commits as before, and the draft stays shown (held) until the model has
  recomputed and meshed, so the body never jumps back.
- A feature that removes material (an extrusion or revolve removing from a body, a hole) shows
  instead the body solid as cut and only its tools (`Evaluation::cuts`, the swept profile or each
  drill) in `CUT_PREVIEW` over everything (`Scene::overlay_meshes`, edges on the front layer),
  never picked, so the cut reads through the material around it.

## What a command acts on

- A command takes the selection's target or refuses in words; it never swaps in a body, plane,
  sketch or axis the selection does not name. `Selection` keeps the order things were picked in
  beside its sorted set (`Selection::in_pick_order`; a bulk pick such as a box enters in sorted
  order).
- A body is named by selected faces, edges or vertices (`body_selection::bodies_in`) or by rows
  chosen in the tree (`body_selection::tree_bodies`: a body row, or a feature row's body). The
  tree's choice wins, since choosing in the view clears it, so it is always the later of the two
  (`move_tools::chosen_body`). Move, Copy, Mirror, Split, Scale, Pattern and Combine (two tree
  bodies), Remove body, Rename body and Body colour all take it this way, and Measure and
  Interference take every chosen row's body. Offers carry the tree's bodies in their basis.
- Several candidates for one slot (two planes, two axes, curves of two sketches, faces of two
  bodies) are refused naming the conflict (`datum_tools::only_plane`, `only_axis`,
  `chosen_plane`, `SEVERAL_*`), except where the pick order decides: a revolve turns about the
  line or axis picked last (`solid_tools::with_model_axis`), and a pattern's direction ignores
  edges and faces of the patterned body when an axis outside it is selected.
- What a command drops from a mixed selection is said in a notice: a fillet or chamfer keeps
  edges, a shell faces (`body_selection::left_out_words`), and the body card says when selected
  faces of other bodies are not coloured (`body_appearance::other_faces_note`).
- The tree's commands that take the selection (Place sketch, Use selected axis, Up to selected,
  Start at selected, Mirror across selected, Split along selected, the datum and pattern Use
  selected) act on the open feature, else the tree's row (`feature_tree::feature_commands`);
  the rest act on `feature_tree::current_feature`.

## Where new features go

- Every tool creates its feature through `TransactionBuilder::add_feature`, so with the rollback
  bar above the end new features land right above it (`document.md`). Defaults are taken from the
  model as of the bar: the last sketch and body are the last active ones
  (`Document::active_features`), and faces, edges and datums are captured where the bar stands
  (`Document::bar_index`).
- A suppressed or rolled-back feature cannot be opened or edited: `SketchEditing` refuses it,
  `SketchEditing::sync` closes an edited sketch or open feature that becomes one, and the tree's
  Edit says why.
- Starting values are the `DEFAULT_*` constants of each `*_tools.rs`.

## Feature panels

- Every panel runs in one order: its shape switch (Extent, Shape) or, for a shell or datum, a
  muted description; then references, values and options; then Body. Small exclusive choices are
  `segmented`; longer lists stay combo boxes.
- `feature_fields.rs` holds the shared rows and the `Rule` wordings; panels use them rather than
  their own spacing or text. A reference that is not set reads None chosen; a body or sketch that
  no longer exists reads Missing in the warning colour with the warning icon; a refused change
  leaves the notice "<feature name> was not changed: <reason>".
- A reference row offers Use selected when the selection gives a change, else Choose in the view,
  which opens the feature and starts choosing that slot (`reference_picking::Picking`, held by
  `SketchEditing` and published to the panels through egui temp data each frame, since the tree's
  call signatures carry no editing state). While choosing, a click (or the activated keyboard
  highlight) is tried as the selection for that slot: it applies and stops, or leaves an info
  notice with the reason. A datum axis keeps a first plane pending until a second one crosses it.
  Escape stops choosing before anything else; opening another feature or closing this one stops it
  too. The lists of regions, blend edges and shell faces use `choose_in_view`, which opens the
  feature so clicks toggle entries.

## Extrude and Revolve

- Extrude with one flat face of a body selected (no sketch edited, no sketch geometry selected;
  an open feature does not matter) extrudes that face: one transaction creates a hidden sketch on the face holding
  its boundary edges projected (`projecting::face_projections`, so the outline follows the face),
  and an extrusion of it added to that body outward, with a notice naming the sketch
  (`solid_tools::create_on_face`); a curved face is refused in words. A selected face never falls
  through to the last sketch.
- Otherwise they take the edited sketch, else the selection's sketch (curves of two sketches are
  refused, `SEVERAL_SKETCHES`); only with nothing selected (for Revolve, nothing but the axis it
  offers) do they guess the open extrusion's sketch, else the last shown sketch, and only while it
  has an even-depth region no feature sweeps (`solid_tools::has_unswept_regions`; a Hole guesses
  a sketch no hole drills, `Guess::Drill`), so pressing Extrude again never sweeps the same
  profile twice. A selection holding no sketch is refused in words (`NOTHING_TO_EXTRUDE`,
  `NOTHING_TO_REVOLVE`), never swapped for a sketch it does not name. The revolve axis is the
  line of the sketch, sketch axis, principal axis, datum axis, straight edge or round face picked
  last; the panel's axis picker refuses several.
- A new feature adds to the body whose faces are selected with its sketch (`SweepSource::body`),
  else the last body standing (`Document::bodies_standing`, so never one a Combine consumed; a new
  one if none) and opens; an extrusion of a sketch lying on a face of a body still standing
  (`solid_tools::face_body`) instead cuts that body, reversed so it runs into it, unless faces of
  another body are selected; the panel's Body list is `Document::bodies_before`. `SketchEditing` holds at most
  one open solid feature, never together with an edited sketch; `editing::Context` carries both to
  the scene and to availability checks.
- With regions of the sketch selected (`Pickable::SketchRegion`, `app-sketching.md`), a new
  extrusion or revolve chooses them (`solid_tools::selected_regions`), and the button's hover says
  it takes the selected regions; with curves of the sketch selected that close up (every open end
  meets another), it chooses the regions whose anchor lies inside them, even-odd, adding them to any
  selected regions (`solid_tools::with_selected_outline`, a revolve leaving its axis line out), so
  selecting a rectangle overlapping a circle sweeps the rectangle and their overlap; curves that do
  not close and no selected region sweep every region as before. Any area cut out by crossing
  curves therefore sweeps alone without trimming the sketch.
- An open feature's tree row is its property panel; every change is one `SetFeatureKind`, checked
  before it is offered. Regions are fills on the front layer, so the see-through preview of the
  body they sweep (whose faces still pick) never covers them; `Pickable::Region` clicks add or
  leave out, turning `RegionChoice::All` into explicit keys. Clear the chosen regions
  (`Command::ClearChosenRegions`, Alt+Shift+R, the panel's Clear beside the count, the palette;
  `solid_tools::clear_regions`, on the open feature else the tree's row) chooses none in one
  undoable change, so a single region is picked from nothing; the feature then fails saying no
  region is chosen until one is clicked. Double-clicking a face opens the feature that made it.
- Each extrusion side has an end kind; one that cannot apply is offered disabled with the reason
  on hover. Up to face takes the selected face or plane captured where the extrusion sits in the
  tree (`solid_panel::selected_target`, through `datum_tools::plane_reference`), each refusal
  saying why (curved, made later, an axis); with none usable it starts choosing one in the view.
- An extrusion's and a revolve's panel ends with a Start row (`solid_panel::start_rows`): the sketch
  plane with a Start offset field (key `start`, zero clears it), or Face or plane, taken like an
  end (`solid_panel::start_change`, slot `Slot::StartPlane`) and shown as Starts at with a button
  that goes back to the sketch plane.
- A revolve can also take two angles, refusing a pair that turns more than a full turn. Its
  Profile switch (Whole, One side) keeps one side of the axis, starting from the side holding
  more of the chosen regions' area (`solid_panel::larger_side`), with Keep the other side of the
  axis to swap.
- A removal's panel lists under Body the other bodies it Also cuts, each with a button to stop
  cutting it, and an Add another body list of `bodies_before` not yet cut; switching away from
  Remove from body drops them (`solid_tools::with_operation`).

## Fillets, chamfers and shells

- Fillet, Chamfer and Shell take the selected edges (or flat faces) of one body and create a
  feature that opens. The edges and flat faces of the body before the feature
  (`BodyMeshes::body_before`) are `Pickable::BlendEdge` and `Pickable::ShellFace`, drawn as in
  Preview while open, and a click toggles one. The
  panel lists the edges or faces in words (a split edge as its pieces).
- Choose in the view on a fillet or chamfer first adds the edges selected in the view of its
  body (`blend_tools::with_selected_edges`, one undoable change, edges already in a chosen chain
  skipped), then opens it, so those and the edges it held show chosen and a click leaves one out.
- Chains and opened faces stay in `BodyBefore::choice` until the references change; each panel's
  list stays in `PanelState::reference_rows` (`reference_rows.rs`) until the state before it or
  the revision changes.

## Offset face

- Offset face (Alt+Q, Model menu, palette; not on the ribbon, which it would widen past one row) takes the selected faces of one body, of
  any shape (`offset_face_tools::selected_faces`), and creates an `OffsetFace` of 1 mm that opens.
  The panel has the faces in words (Leave this face out per row), a signed Distance (any
  expression, a negative one shrinks; typing previews it as a fillet's size does), the switch
  Move the faces tangent to these too (`tangent`) and Body.
- While it is open and computed, the result is drawn and its faces are `Pickable::ShellFace`s,
  the moved ones in the selected colour (`FaceChoice::Moving`, `Builder::choosable_faces`, the
  tangent chain included), and a click moves a face or leaves it out; faces keep their keys across
  the feature, so the keys of the state before it find them. A failed or pending one shows the
  body before it with every face pickable, so the choice can be mended. Hover texts and the prompt
  say move rather than open.

## Primitives

- Box (Alt+B), Cylinder (Alt+Y), Sphere (Alt+U) and Torus (Alt+Shift+U) are in the Model menu's
  Primitives group and the palette, not on the ribbon, which they would widen past one row. Each
  creates a `Primitive` of the `DEFAULT_*` sizes of `primitive_tools.rs`, a box and a cylinder
  starting at their base's centre, a sphere and a torus at their centre, and opens it, previewed
  like an extrusion (see-through while open, the cut tool over the body for a removal).
- With one plane, datum plane or flat face selected (`datum_tools::chosen_plane`; several are
  refused), it stands there: on a face at its middle (`hole_tools::face_middle`) and added to that
  face's body, on a plane at its origin as a new body. With none it stands on the XY plane at the
  origin as a new body and starts choosing its place in the view (`Slot::PrimitivePlace`, prompt
  saying Escape leaves it there): a click on a plane, datum plane or flat face places it on that
  one where the pointer's ray meets it (`primitive_tools::place_click`, the ray handed to
  `pick_action`); the keyboard highlight places it at the face's middle or the plane's origin.
- The panel has the Shape switch (switching takes the new shape's default sizes), Placed on with
  Use selected or Choose in the view (the same slot), Position X and Y (key `primitive-field`,
  `("at", index)`), Starts at (Corner, Base centre, Centre), the sizes (`("size", index)`),
  Reverse direction (left out for Centre, where it changes nothing), and Result with Body as an
  extrusion's. Switching Result between joining (New body, Add) and cutting (Remove, Intersect)
  on a face reverses the direction to match (`primitive_tools::with_operation`), so a cut goes
  into the body. Placing it again never changes the operation.

## Hole

- Hole (Alt+O, in the Solid group) with one flat face selected (and no sketch edited and no sketch
  geometry selected, which wins over the face) creates in one transaction a hidden sketch on that face holding one point and the hole: the
  point inside the face farthest from its edges and holes (`deepest_point`, a refined grid search
  over the sampled loops, ties going to the middle of its bounds), so an L- or U-shaped or holed face
  is drilled on material, with a notice saying how to move the point
  (`hole_tools::create_on_face`). Otherwise it takes the sketch the way Extrude does (edited,
  selected, or with nothing selected the opened or last one; else `NOTHING_TO_DRILL`) and needs at least one free point or circle in it (`hole_centres`);
  its body is the one the sketch is attached to, else the one whose faces are selected, else the
  last body standing. It creates a plain
  blind hole of 6 mm by 10 mm, hides the sketch and opens the panel: Size (Custom or a metric
  screw), Fit when sized (Close, Normal, Loose, Tapped, Fine, and Insert from M2 to M8, the size's
  `fits`; with the thread named for the tapped ones, a Pitch row of the size's fine pitches when it
  has several, and for an insert a note giving the insert's length and least wall), Style
  (Plain, Counterbore, Countersink, Stepped; switching takes the size's head dimensions, else the
  defaults; Stepped starts from the counterbore and a step midway to the hole, and lists Step n
  diameter and depth with Add a step, a step midway between the last and the hole as deep as the
  last, and Remove the last step), Sized by (Diameter, Circles, which sets `CirclesAndHeads`; shown while the sketch has
  circles or the hole is sized by them, with Scale the counterbore or countersink with each circle
  for a counterbored or countersunk one), Diameter, the style's sizes, Shape (Round, or Slot with its length and angle), Depth
  (Blind with its field, or Through all), for a blind round hole Drill point (a switch giving the
  bottom a 118° cone, with its Drill point angle field, key `drill-point-angle`), Reverse direction, and the Sketch and Body rows (the body
  a list of `bodies_before`). A size sets exact millimetre values; typing any hole, counterbore or
  countersink size makes it Custom again.

## Combine

- Combine (Alt+J) is offered when the selection touches faces, edges or vertices of exactly two
  bodies shown now, or two bodies are chosen in the tree; the earlier body in the tree is the target and the later the tool. It creates a
  Join and opens the panel, whose Operation switch (Join, Cut, Intersect) and Target and Tool
  lists (`Document::bodies_before`, each leaving out the bodies the other rows hold) change it.
  Under the Tool list, Also with lists the further tool bodies, each with a button to drop it, and
  an Add another tool body list offers the standing bodies not yet used; Keep tool leaves the tool
  bodies standing instead of using them up. Nothing is chosen in the view while it is open.

## Move

- Move body (Alt+M) takes the one body whose faces, edges or vertices are selected and creates a `Move`
  of zero turns and distances, then opens it. Copy body (Alt+Shift+C, Model menu, palette, a body's
  right-click menu; not on the ribbon, which it would widen past one row) does the same with
  `copy` set, named Copy N, so the copy is placed from its panel; Make a copy in the panel switches
  between the two. New moves turn about the body's centre; the panel's Turn about switch (Centre,
  Origin, Axis) changes it, its description saying which. The panel has a field for the turn
  about each axis and the distance along each, all expressions (key `move-field`, `offset` or
  `turn`, axis index). Switching to Axis (`move_tools::about_axis`) takes the one axis the
  selection names, else the Z axis, and keeps any angle already set; its Axis row is a combo of
  the principal and earlier datum axes with Use selected and Choose in the view (slot `MoveAxis`,
  the palette's Turn moved body about selected axis, `move_tools::axis_change`) for an edge, round
  face or sketch line, followed by its Angle field (key `angle`); the turns about X, Y and Z are
  then shown only while not zero. Nothing else is chosen in the view while it is open.
- While a move is open (and nothing is being chosen in the view) a manipulator stands at the centre
  of the moved body's box, following its preview (`move_manipulator.rs`): an arrow along each
  world axis in the axis colours (`selection::Axis::rgb`) and a square for each plane, drawn on the
  front layer in the overlay batch at a constant size in points (`ARROW_POINTS`), an arrow pointing
  at the eye or a square seen edge-on left out. Hit-testing is on the UI thread in screen space
  (`Manipulator::hit`, arrows within `HIT_POINTS` winning over squares), so it needs no pick; the
  hovered or dragged handle turns `canvas::HOVERED`, the hover says what a drag does, and a click
  on a handle selects nothing.
- A primary drag on a handle (`PrimaryDrag::Manipulate`, `Manipulating`) follows the point of its
  axis nearest the pointer's ray (`Ray::closest_along_line`) or where the ray meets its plane,
  from where it was grabbed, in steps of 1, 2 or 5 of a power of ten about `STEP_POINTS` on screen
  (Ctrl drags freely). Each change is a preview of the move (`Action::Preview`, drawn as typing
  draws it) with a readout of the distances beside the pointer; release commits one edit setting
  the dragged distances to measured values, Escape or leaving the feature drops it.
- A move turning about its body's centre stands its manipulator at that centre (as moved) and adds
  a ring about each axis (`Handle::Turn`, `RING_REACH` beyond the arrows, left out when seen
  edge-on). Dragging one turns the body by the angle swept round the centre in the ring's plane,
  in steps of `TURN_STEP_DEGREES` (Ctrl drags freely), previewed and committed like an arrow: a
  turn about an axis applied after the others (Z, or Y with no Z turn, or X alone) adds to its own
  field, any other is composed with the turns there and written back as three turns
  (`turns_about_axes`), changing only the fields whose value changed. A move turning about the
  origin has no rings, since a ring at the body would mislead. A move turning about an axis stands
  its manipulator on that axis, at the foot of the body's centre shifted by the distances, with
  one ring about it (`Handle::TurnAbout`, `canvas::SNAP`) whose drag adds the swept angle to the
  Angle field the same way.

## Mirror and scale

- Mirror body (Alt+Shift+M) takes the body of the selection like Move and creates a `Mirror` that
  keeps the original, across the one principal or datum plane selected with it, else the one
  selected flat face (of that body or another), else the YZ plane (`datum_tools::chosen_plane`); a
  selected datum axis or point, a datum after the bar or several planes are refused. The
  panel chooses a principal plane from a combo, or any plane or flat face made before it with Use
  selected, Choose in the view (slot `MirrorPlane`) or the palette's Mirror across selected, and
  has a Keep the original checkbox.
- With rows chosen in the tree that a pattern would repeat (`pattern_tools::repeatable`), Mirror
  body mirrors those features instead (`MirrorSource::mirrored`), named "Mirror features N", the
  hover naming them ("Mirror Hole 1 across the YZ plane"). Its panel's Mirrors rows list them,
  each with Stop mirroring, or read The whole body, and Mirror the chosen features takes the rows
  chosen now, as a pattern's Repeats rows do; Keep the original is hidden while features are
  mirrored. The open mirror shows its body and its reflected cuts like a pattern of holes.
- Split body (Alt+K, Model menu, palette, a body's right-click menu; not on the ribbon, which it
  would widen past one row) takes the body the same way and creates a `Split` along the plane
  chosen as Mirror's is (the Bodies group's Split body too), and opens it; its panel mirrors Mirror's (combo, Use
  selected, Choose in the view with slot `SplitPlane`, the palette's Split along selected), with
  Keep the other side and rows naming the body and the split-off body. While open both bodies show
  as previews.
- Scale body (Alt+Shift+S) takes the body the same way and creates a `Scale` by 1 (the body unchanged until
  a factor is typed) about the origin. The panel has the factor (a plain number above zero) and
  the centre's three coordinates, all expressions (key `scale-field`, `("factor", 0)` or `("center", axis index)`).

## Patterns

- With rows chosen in the tree that are all extrusions, revolves or holes adding to or removing
  from one body (`pattern_tools::repeatable`, from the tree's raw rows, which the offers carry
  beside its bodies), Linear and Circular pattern repeat those features instead of the body, the
  hover naming them ("Repeat Hole 1 along …"), with a default spacing of twice their tools'
  extent along the direction. The panel's Repeats rows list them, each with Stop repeating (the
  last one leaves the whole body repeated), or read The whole body, and Repeat the chosen features
  takes the rows chosen in the tree now (Ctrl+click while the pattern is open), disabled with how
  to choose them otherwise.
- Otherwise Linear and Circular pattern take the body of the selected faces, edges or vertices or of the
  tree (`move_tools::chosen_body`), the last body standing only with nothing selected, and the one
  selected axis, straight edge, round face or sketch line as direction or axis (edges and faces of
  the body itself count only when nothing else is selected), else a principal axis
  (`pattern_tools::source`). While open the patterned body is shown with its directions or axis
  drawn like a revolve's axis.
- Direction, Second direction and Axis are lists of the principal axes and the datum axes made
  before the pattern (`pattern_tools::listed_axes`, set through `with_axis`; Second direction
  also None), above Use selected or Choose in the view for an edge, round face or sketch line.
- Each linear direction has a Measured switch (Each, Overall) above its length, captioned Spacing
  or Total length. Switching keeps the copies in place when the count is a plain number
  (`pattern_panel::respaced`: a literal multiplied or divided, else the expression times or over
  the steps), otherwise it keeps the expression.
- Instances is a grid of `widgets::instance_toggle`s, one per instance at its step along each
  direction (rows for the second), checked when made; clicking one leaves it out or brings it back
  as one undoable change, the original disabled. A muted line under it counts what is left out.
  Switching between linear and circular clears the instances left out.
- While a pattern is open, a click on a face of one of its copies (`FaceOrigin::Copy` of this
  pattern, found through the shown body's result, `pattern_tools::clicked_copy`) leaves that copy
  out as the grid would, and hovering one says so; other clicks select as usual. A copy left out
  is no longer drawn, so the grid or Undo brings it back.

## Visibility

- Sketches, datums and body-making features can be hidden (a body through its feature); hidden
  ones are not drawn, picked, kept selected or counted in fitting, except the edited sketch and
  open datum. Extrude and Revolve hide their sketch in the same transaction. Every hide control
  is also a command. Hide everything but the selection (`visibility::hide_others`) hides in one
  transaction every other shown hideable feature and principal item, except the edited sketch.
- Hide or show every sketch, datum or body (`Command::ToggleSketches`, `ToggleDatums`,
  `ToggleBodies`, View menu, palette, no default key; `visibility::toggle_kind`) is one
  transaction per kind: while any of that kind is shown it hides every one shown, otherwise it
  shows them all; the edited sketch is never touched, and a model with none of the kind refuses
  in words. Body-making features count as bodies, as `can_hide` judges them.
- Principal planes, axes and origin hide the same way through a group row (`principal_tree.rs`)
  with an eye for the group and one per item. Hidden ones are drawn and offered anyway while a
  sketch's plane is chosen (`Context::choosing_plane`).

## Datums

- Points are taken from the selection by `datum_tools::point_reference`: the origin, a datum point,
  a corner, the centre of a round edge, the centre of a spherical or toroidal face or a sketch
  point made before the datum; a sketch line is an axis (`axis_reference`).
- Plane starts from the selected plane or flat face (XY otherwise), turned about the selected
  axis, straight edge or round face if any, else offset; three selected points make a plane
  through them, two planes one midway between them, an axis and a point one through both. Axis
  runs along the selected axis, straight edge, round face or sketch line, where two selected planes
  or flat faces meet, through two points, or square to a plane through a point. Point (Alt+Shift+P)
  sits at the one selected point, else the origin, with zero offsets. A selected face, edge or
  corner giving nothing (curved, made later, not recomputed) refuses them with that reason.
- Selections that give the constructed forms (`document.md`): Plane with two axes or straight
  edges selected passes through both, and with one edge that is not straight (round or curved) and
  nothing else stands square to it at 0 mm along (`PlaneThrough::SquareToCurve`); Point with two
  axes or straight edges selected sits where they cross, with an axis and a plane where they meet,
  with three planes where they meet, with one edge that is not round at 0 mm along it
  (`PointBy::Along`) and with one sphere or torus face at its centre. Combinations keeping an
  older meaning (an axis and a plane turn a plane; one round edge is a centre) keep it.
- A plane through references, a point and the new axis forms show a Defined by (or At) row
  re-chosen from the selection, refused while nothing is selected; a plane through an axis and a
  point switches between Contains it and Square to it, and for a round face's axis also Tangent to
  it (`PlaneThrough::Tangent`, the point picking the side), going back keeping the face and point;
  a plane square to an edge and a point along an edge have a Distance along field; a point at a
  round edge's centre switches to Along it; a point has Offset X, Y and Z fields. Choosing again
  from the selection keeps the form a panel was switched to and its distance (`keeping_plane_mode`,
  `keeping_point_mode`).
  Choosing them in the view holds up to two clicks (`Picking::pending`) until the selection makes
  one, the prompt saying what is still wanted.
- The new feature opens; its panel has reference pickers for base and rotation axis and fields for
  angle and offset. A datum cannot switch between plane and axis, nor an extrusion and a revolve,
  since `SetFeatureKind` refuses a kind change (`document.md`).
- Outside sketch editing datums are picked as `Pickable::Datum` and tinted when failed;
  double-clicking one opens it.

## Sketches on faces and planes

- New sketch starts on the one selected principal plane, datum plane or flat face
  (`sketch_placement::sketch_target`, several refused with `SEVERAL_TO_SKETCH_ON`, as Place on the
  selection does); while choosing a plane a click on any of them does the same.
- The attachment is captured from the body's state where the sketch sits in the tree: a face made
  further down is refused with the reason, and one whose body is not recomputed that far says so
  (`body_state_before` tells the two apart). A sketch's row says what it lies on and offers Detach
  and Place on the selection.
- Every refused New sketch (including a click on a curved face while choosing) and every Fillet,
  Chamfer, Shell or datum that cannot be created leaves a notice with the reason.
- What draws or maps onto a sketch takes its plane from the solved result (`scene::sketch_plane`,
  `Model::displayed_sketch`), since an attached sketch's stored plane is where it was placed.
