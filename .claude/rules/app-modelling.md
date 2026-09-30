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
  - "crates/caditor/src/visibility.rs"
  - "crates/caditor/src/sketch_placement.rs"
  - "crates/caditor/src/reference_rows.rs"
  - "crates/caditor/src/feature_tree.rs"
---

# Modelling tools in the app

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

## Visibility

- Sketches, datums and features that make a body can be hidden (a body through the feature that made
  it); hidden ones are not drawn, picked, kept selected or counted in fitting, except the edited
  sketch and open datum.
- Each such tree row has an eye button and a Hide or Show menu item (Hide or show feature, on the
  tree's current feature); H hides the bodies, sketches and datums of the selection and Alt+H shows
  everything. Extrude and Revolve hide their sketch in the same transaction.
- Principal planes, axes and origin hide the same way (H on them in the view, Alt+H); a collapsible
  "Principal planes and axes" row at the top of the tree (`principal_tree.rs`) has an eye button for
  the group (also the command Hide or show principal planes, axes and origin) and one per item,
  hovering an item highlighting it in the view. Hidden ones are drawn and offered anyway while a
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
