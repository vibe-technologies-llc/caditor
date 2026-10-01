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
  - "crates/caditor/src/visibility.rs"
  - "crates/caditor/src/sketch_placement.rs"
  - "crates/caditor/src/reference_rows.rs"
  - "crates/caditor/src/feature_tree.rs"
---

# Modelling tools in the app

## Where new features go

- Every tool creates its feature through `TransactionBuilder::add_feature`, so with the rollback bar
  above the end new features go in right above it (`document.md`). What a tool picks by default is
  taken from the model as of the bar: the last sketch and last body are the last active ones
  (`Document::active_features`), and faces, edges and datums are captured where the bar stands
  (`Document::bar_index` as the place in the tree).
- A suppressed or rolled-back feature cannot be opened or edited (`SketchEditing` refuses it and
  `SketchEditing::sync` closes an edited sketch or open feature that becomes one); the tree's Edit
  says why.

## Extrude and Revolve

- Extrude and Revolve take the edited sketch, else the selection's, else the last. The revolve axis
  (also the panel's Use selected axis) is a selected line or sketch axis, else a principal axis,
  datum axis, straight edge or round face (vertical otherwise).
- A new feature is one-sided 10 mm or a full turn, adds to the last body (new if none) and opens;
  `SketchEditing` holds at most one open solid feature, never together with an edited sketch, and
  `editing::Context` carries both to the scene and to availability checks.
- An open feature's tree row is its property panel (sketch, regions, extent, axis, result and target
  body); every change is one `SetFeatureKind`, checked before it is offered. Its regions are fills;
  `Pickable::Region` clicks add or leave out, turning `RegionChoice::All` into explicit keys.
- Double-clicking a face opens the feature that made it; Escape closes an open feature last.
- An extrusion's panel gives each side an end list (End, or Forward end and Backward end): Distance,
  Through all, Up to next, Up to face. An end that cannot apply is offered disabled with the reason
  on hover: Through all until the result removes or intersects, Up to next until it changes a
  body, Up to face until a usable face or plane is selected. A distance end adds its captioned
  field; a face end a row naming the face or plane with Use selected to take another. Switching
  between one side and two sides keeps the end (a reversed one-sided end becomes the backward
  side).
- The face or plane is the selection captured where the extrusion sits in the tree
  (`solid_panel::selected_target`, through `datum_tools::plane_reference`): one principal plane,
  earlier datum plane or flat face, each refusal saying why (curved, made later, an axis). The
  command Extrude up to selected face or plane sets it on the current feature (a one-sided end, or
  the forward side).
- A revolve's list adds Two angles, with Forward and Backward angle fields that refuse a pair
  turning more than 360°; switching to it keeps the current angle forward and adds 30° backward.

## Fillets and chamfers

- Fillet and Chamfer take the selected edges of one body and create a 1 mm feature that opens. The
  body is drawn as before it, edges as `Pickable::BlendEdge` (chosen ones and their chains
  highlighted); a click adds an edge or removes the references whose chain contains it. The panel
  switches between fillet and chamfer, edits the size and lists the edges in words (a split edge as
  its pieces).
- Chains and opened faces stay in `BodyBefore::choice` until the references change; each blend and
  shell panel's list in `PanelState::reference_rows` (`reference_rows.rs`) until the state before it
  or the revision changes.

## Shells

- Shell takes the selected flat faces of one body as the faces to open, creating a 1 mm feature that
  opens; the body is drawn as before it (`BodyMeshes::body_before`), flat faces as
  `Pickable::ShellFace` (shared with blends), opened ones highlighted, a click opening a face or
  closing it again. The panel edits the thickness and lists the open faces.

## Patterns

- Linear pattern and Circular pattern take the body of the selected faces or edges (all of one
  body), else the last body made, and the first selected axis, straight edge or round face as
  direction or axis (the X axis or Z axis otherwise). A linear pattern starts with 3 instances
  spaced about a fifth more than the body's length along the direction, a circular one with 6
  over a full turn; the new feature opens.
- While open the patterned body is shown with its directions or axis drawn like a revolve's axis.
  The panel switches between linear and circular, takes a new direction or axis and a second
  direction from the selection (Use selected, or the commands Pattern along or about selected
  axis and Pattern also along selected direction), removes the second direction and edits count,
  spacing, total angle and Reversed; fields refuse a count that is not whole or above the limit.

## Visibility

- Sketches, datums and features that make a body can be hidden (a body through the feature that made
  it); hidden ones are not drawn, picked, kept selected or counted in fitting, except the edited
  sketch and open datum.
- Each such tree row has an eye button and a Hide or Show menu item (Hide or show feature, on the
  tree's current feature); H hides the bodies, sketches and datums of the selection and Alt+H shows
  everything. Extrude and Revolve hide their sketch in the same transaction.
- Principal planes, axes and origin hide the same way (H on them in the view, Alt+H); a collapsible
  "Principal planes, axes and origin" row at the top of the tree (`principal_tree.rs`) has an eye
  button for the group (also the command Hide or show principal planes, axes and origin) and one per
  item, hovering an item highlighting it in the view. Hidden ones are drawn and offered anyway while a
  sketch's plane is chosen (`Context::choosing_plane`).

## Datums

- Plane starts from the selected plane or flat face (XY otherwise), turned 45° about the selected
  axis, straight edge or round face if any (offset 0 mm), else offset 10 mm. Axis runs along the
  selected axis, straight edge or round face, or where two selected planes or flat faces meet. A
  selected face or edge giving neither (curved, round rim, made later, not recomputed) refuses both
  with that reason.
- The new feature opens; its panel has Use selected for base and rotation axis (or whole axis) and
  fields for the angle and offset.
- Outside sketch editing datums are drawn as translucent squares and lines centred where the world
  origin projects onto them, picked as `Pickable::Datum`, tinted when failed; double-clicking one
  opens it.

## Sketches on faces and planes

- New sketch starts on a selected principal plane, datum plane or flat face; while choosing a plane
  a click on any of them does the same.
- The attachment is captured from the body's state where the sketch sits in the tree: a face made
  further down is refused with the reason, and one whose body is not recomputed that far says so
  (`body_state_before` tells the two apart).
- A sketch's row says which face or plane it lies on and offers Detach, and Place on selected plane
  or Place on selected face when one is selected.
- Every refused New sketch (including a click on a curved face while choosing) and every Fillet,
  Chamfer, Shell or datum that cannot be created leaves a notice with the reason.
- What draws or maps onto a sketch takes its plane from the solved result (`scene::sketch_plane`,
  `Model::displayed_sketch`), since an attached sketch's stored plane is where it was placed.
