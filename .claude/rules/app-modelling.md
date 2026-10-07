---
paths:
  - "crates/caditor/src/solid_tools.rs"
  - "crates/caditor/src/solid_panel.rs"
  - "crates/caditor/src/blend_tools.rs"
  - "crates/caditor/src/blend_panel.rs"
  - "crates/caditor/src/shell_tools.rs"
  - "crates/caditor/src/shell_panel.rs"
  - "crates/caditor/src/combine_tools.rs"
  - "crates/caditor/src/combine_panel.rs"
  - "crates/caditor/src/move_tools.rs"
  - "crates/caditor/src/hole_tools.rs"
  - "crates/caditor/src/hole_panel.rs"
  - "crates/caditor/src/move_panel.rs"
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

- They take the edited sketch, else the selection's, else the last. The revolve axis (also the
  panel's axis picker) is a selected line or sketch axis, else a principal axis, datum axis,
  straight edge or round face.
- A new feature adds to the last body standing (`Document::bodies_standing`, so never one a Combine
  consumed; a new one if none) and opens; an extrusion of a sketch lying on a face of a body still
  standing (`solid_tools::face_body`) instead cuts that body, reversed so it runs into it; the panel's Body list is `Document::bodies_before`. `SketchEditing` holds at most
  one open solid feature, never together with an edited sketch; `editing::Context` carries both to
  the scene and to availability checks.
- An open feature's tree row is its property panel; every change is one `SetFeatureKind`, checked
  before it is offered. Regions are fills; `Pickable::Region` clicks add or leave out, turning
  `RegionChoice::All` into explicit keys. Double-clicking a face opens the feature that made it.
- Each extrusion side has an end kind; one that cannot apply is offered disabled with the reason
  on hover. Up to face takes the selected face or plane captured where the extrusion sits in the
  tree (`solid_panel::selected_target`, through `datum_tools::plane_reference`), each refusal
  saying why (curved, made later, an axis); with none usable it starts choosing one in the view.
- An extrusion's and a revolve's panel ends with a Start row (`solid_panel::start_rows`): the sketch
  plane with a Start offset field (key `start`, zero clears it), or Face or plane, taken like an
  end (`solid_panel::start_change`, slot `Slot::StartPlane`) and shown as Starts at with a button
  that goes back to the sketch plane.
- A revolve can also take two angles, refusing a pair that turns more than a full turn.

## Fillets, chamfers and shells

- Fillet, Chamfer and Shell take the selected edges (or flat faces) of one body and create a
  feature that opens. The body is drawn as before the feature (`BodyMeshes::body_before`), edges
  and flat faces are `Pickable::BlendEdge` and `Pickable::ShellFace`, and a click toggles one. The
  panel lists the edges or faces in words (a split edge as its pieces).
- Chains and opened faces stay in `BodyBefore::choice` until the references change; each panel's
  list stays in `PanelState::reference_rows` (`reference_rows.rs`) until the state before it or
  the revision changes.

## Hole

- Hole (Alt+O, in the Solid group) with one flat face selected (and no sketch edited or feature
  open) creates in one transaction a hidden sketch on that face holding one point and the hole: the
  point inside the face farthest from its edges and holes (`deepest_point`, a refined grid search
  over the sampled loops, ties going to the middle of its bounds), so an L- or U-shaped or holed face
  is drilled on material, with a notice saying how to move the point
  (`hole_tools::create_on_face`). Otherwise it takes the sketch the way Extrude does (edited,
  selected, opened or last) and needs at least one free point or circle in it (`hole_centres`);
  its body is the one the sketch is attached to, else the last body standing. It creates a plain
  blind hole of 6 mm by 10 mm, hides the sketch and opens the panel: Size (Custom or a metric
  screw), Fit when sized (Close, Normal, Loose, Tapped, Fine, with the thread named for the
  tapped ones), Style
  (Plain, Counterbore, Countersink; switching takes the size's head dimensions, else the
  defaults), Sized by (Diameter, Circles; shown while the sketch has circles or the hole is sized
  by them), Diameter, the style's sizes, Shape (Round, or Slot with its length and angle), Depth
  (Blind with its field, or Through all), Reverse direction, and the Sketch and Body rows (the body
  a list of `bodies_before`). A size sets exact millimetre values; typing any hole, counterbore or
  countersink size makes it Custom again.

## Combine

- Combine (Alt+J) is offered when the selection touches faces, edges or vertices of exactly two
  bodies shown now; the earlier body in the tree is the target and the later the tool. It creates a
  Join and opens the panel, whose Operation switch (Join, Cut, Intersect) and Target and Tool
  lists (`Document::bodies_before`, each leaving out the other) change it. Nothing is chosen in the
  view while it is open.

## Move

- Move body (Alt+M) takes the one body whose faces, edges or vertices are selected and creates a `Move`
  of zero turns and distances, then opens it. Copy body (Alt+Shift+C, Model menu, palette, a body's
  right-click menu; not on the ribbon, which it would widen past one row) does the same with
  `copy` set, named Copy N, so the copy is placed from its panel; Make a copy in the panel switches
  between the two. The panel has a field for the turn about each axis
  and the distance along each, all expressions (key `move-field`, `offset` or `turn`, axis index).
  Nothing is chosen in the view while it is open.

## Mirror and scale

- Mirror body (Alt+Shift+M) takes the body of the selection like Move and creates a `Mirror` that
  keeps the original, across a principal or datum plane selected with it, else the YZ plane. The
  panel chooses a principal plane from a combo, or any plane or flat face made before it with Use
  selected, Choose in the view (slot `MirrorPlane`) or the palette's Mirror across selected, and
  has a Keep the original checkbox.
- Scale body (Alt+Shift+S) takes the body the same way and creates a `Scale` by 2 about the
  origin. The panel has the factor (a plain number above zero) and the centre's three coordinates,
  all expressions (key `scale-field`, `("factor", 0)` or `("center", axis index)`).

## Patterns

- Linear and Circular pattern take the body of the selected faces or edges (all of one body),
  else the last body standing, and the first selected axis, straight edge or round face as direction or
  axis, else a principal axis. While open the patterned body is shown with its directions or axis
  drawn like a revolve's axis.
- Each linear direction has a Measured switch (Each, Overall) above its length, captioned Spacing
  or Total length. Switching keeps the copies in place when the count is a plain number
  (`pattern_panel::respaced`: a literal multiplied or divided, else the expression times or over
  the steps), otherwise it keeps the expression.
- Instances is a grid of `widgets::instance_toggle`s, one per instance at its step along each
  direction (rows for the second), checked when made; clicking one leaves it out or brings it back
  as one undoable change, the original disabled. A muted line under it counts what is left out.
  Switching between linear and circular clears the instances left out.

## Visibility

- Sketches, datums and body-making features can be hidden (a body through its feature); hidden
  ones are not drawn, picked, kept selected or counted in fitting, except the edited sketch and
  open datum. Extrude and Revolve hide their sketch in the same transaction. Every hide control
  is also a command. Hide everything but the selection (`visibility::hide_others`) hides in one
  transaction every other shown hideable feature and principal item, except the edited sketch.
- Principal planes, axes and origin hide the same way through a group row (`principal_tree.rs`)
  with an eye for the group and one per item. Hidden ones are drawn and offered anyway while a
  sketch's plane is chosen (`Context::choosing_plane`).

## Datums

- Points are taken from the selection by `datum_tools::point_reference`: the origin, a datum point,
  a corner, the centre of a round edge or a sketch point made before the datum; a sketch line is an
  axis (`axis_reference`).
- Plane starts from the selected plane or flat face (XY otherwise), turned about the selected
  axis, straight edge or round face if any, else offset; three selected points make a plane
  through them, two planes one midway between them, an axis and a point one through both. Axis
  runs along the selected axis, straight edge, round face or sketch line, where two selected planes
  or flat faces meet, through two points, or square to a plane through a point. Point (Alt+Shift+P)
  sits at the one selected point, else the origin, with zero offsets. A selected face, edge or
  corner giving nothing (curved, made later, not recomputed) refuses them with that reason.
- A plane through references, a point and the new axis forms show a Defined by (or At) row
  re-chosen from the selection, refused while nothing is selected; a plane through an axis and a
  point switches between Contains it and Square to it; a point has Offset X, Y and Z fields.
  Choosing them in the view holds up to two clicks (`Picking::pending`) until the selection makes
  one, the prompt saying what is still wanted.
- The new feature opens; its panel has reference pickers for base and rotation axis and fields for
  angle and offset. A datum cannot switch between plane and axis, nor an extrusion and a revolve,
  since `SetFeatureKind` refuses a kind change (`document.md`).
- Outside sketch editing datums are picked as `Pickable::Datum` and tinted when failed;
  double-clicking one opens it.

## Sketches on faces and planes

- New sketch starts on a selected principal plane, datum plane or flat face; while choosing a plane
  a click on any of them does the same.
- The attachment is captured from the body's state where the sketch sits in the tree: a face made
  further down is refused with the reason, and one whose body is not recomputed that far says so
  (`body_state_before` tells the two apart). A sketch's row says what it lies on and offers Detach
  and Place on the selection.
- Every refused New sketch (including a click on a curved face while choosing) and every Fillet,
  Chamfer, Shell or datum that cannot be created leaves a notice with the reason.
- What draws or maps onto a sketch takes its plane from the solved result (`scene::sketch_plane`,
  `Model::displayed_sketch`), since an attached sketch's stored plane is where it was placed.
