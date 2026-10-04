---
paths:
  - "crates/caditor/src/solid_tools.rs"
  - "crates/caditor/src/solid_panel.rs"
  - "crates/caditor/src/blend_tools.rs"
  - "crates/caditor/src/blend_panel.rs"
  - "crates/caditor/src/shell_tools.rs"
  - "crates/caditor/src/shell_panel.rs"
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
- A new feature adds to the last body (a new one if none) and opens. `SketchEditing` holds at most
  one open solid feature, never together with an edited sketch; `editing::Context` carries both to
  the scene and to availability checks.
- An open feature's tree row is its property panel; every change is one `SetFeatureKind`, checked
  before it is offered. Regions are fills; `Pickable::Region` clicks add or leave out, turning
  `RegionChoice::All` into explicit keys. Double-clicking a face opens the feature that made it.
- Each extrusion side has an end kind; one that cannot apply is offered disabled with the reason
  on hover. Up to face takes the selected face or plane captured where the extrusion sits in the
  tree (`solid_panel::selected_target`, through `datum_tools::plane_reference`), each refusal
  saying why (curved, made later, an axis); with none usable it starts choosing one in the view.
- A revolve can also take two angles, refusing a pair that turns more than a full turn.

## Fillets, chamfers and shells

- Fillet, Chamfer and Shell take the selected edges (or flat faces) of one body and create a
  feature that opens. The body is drawn as before the feature (`BodyMeshes::body_before`), edges
  and flat faces are `Pickable::BlendEdge` and `Pickable::ShellFace`, and a click toggles one. The
  panel lists the edges or faces in words (a split edge as its pieces).
- Chains and opened faces stay in `BodyBefore::choice` until the references change; each panel's
  list stays in `PanelState::reference_rows` (`reference_rows.rs`) until the state before it or
  the revision changes.

## Patterns

- Linear and Circular pattern take the body of the selected faces or edges (all of one body),
  else the last body, and the first selected axis, straight edge or round face as direction or
  axis, else a principal axis. While open the patterned body is shown with its directions or axis
  drawn like a revolve's axis.

## Visibility

- Sketches, datums and body-making features can be hidden (a body through its feature); hidden
  ones are not drawn, picked, kept selected or counted in fitting, except the edited sketch and
  open datum. Extrude and Revolve hide their sketch in the same transaction. Every hide control
  is also a command.
- Principal planes, axes and origin hide the same way through a group row (`principal_tree.rs`)
  with an eye for the group and one per item. Hidden ones are drawn and offered anyway while a
  sketch's plane is chosen (`Context::choosing_plane`).

## Datums

- Plane starts from the selected plane or flat face (XY otherwise), turned about the selected
  axis, straight edge or round face if any, else offset. Axis runs along the selected axis,
  straight edge or round face, or where two selected planes or flat faces meet. A selected face or
  edge giving neither (curved, round rim, made later, not recomputed) refuses both with that
  reason.
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
