---
paths:
  - "crates/caditor/src/field.rs"
  - "crates/caditor/src/commands.rs"
  - "crates/caditor/src/palette.rs"
  - "crates/caditor/src/shortcut_editor.rs"
  - "crates/caditor/src/typed_point.rs"
  - "crates/caditor/src/viewport.rs"
  - "crates/caditor/src/app.rs"
  - "crates/caditor/src/feature_tree.rs"
---

# Input, commands and keyboard operation

## Numeric fields

- Every numeric input is a `field::commit_field`: commits on Enter or blur, reverts on Escape and
  keeps invalid text with its error inline until the stored value changes underneath it (an undo,
  say) while it is not being edited. Expression fields parse, evaluate and check the dimension
  before building a transaction; sketch dimensions go through `field::dimension_transaction`.

## Keyboard focus

- `Situation::accepts` decides what a key press may trigger. Keys widgets use (`WIDGET_KEYS`,
  Escape and Enter included) run commands only when no widget held focus, so Escape or Enter in a
  field never reaches the viewport; other plain keys need only that no text field has focus.
- egui-winit turns Ctrl+C, Ctrl+X and Ctrl+V into `Event::Copy`, `Cut` and `Paste` without a key
  event, so `commands::pressed_shortcut` reads those events as the key presses (a paste event
  arrives only while the system clipboard holds text).
- Ctrl and Alt shortcuts run from a text field too, except on `TEXT_EDITING_KEYS`: the field
  loses focus (committing) and the command runs next frame (`Workspace::deferred_commands`), so
  Save saves a value just typed.

## Commands and keymap

- Every toolbar, menu and sketch action is a `Command` with a stable id, title, `Category`,
  `Scope` (anywhere, only in a sketch, or only outside one, which lets P hide the principal
  geometry outside sketches while it stays the Point tool inside) and default shortcuts. Scope
  only decides where a key reaches the command; the palette and menus offer it as usual. `Command::all()` is built from
  `plain_commands!`, which matches `Command` exhaustively so a missing variant fails to compile,
  and from the `ALL` of each payload type declared by `all_variants!` (`variants.rs`; a test checks
  `ShapeMode::ALL` against its mode enums). Defaults are in `Command::default_shortcuts`; docs do
  not repeat them.
- `Keymap` in `Preferences` stores only overrides, as `keys.<id>` lists of text, so unreadable or
  newer entries survive and a command with no readable binding keeps its defaults. Esc, Enter and
  Tab are reserved and cannot be bound. A binding used in an overlapping scope asks before moving.
- The shortcut editor (`shortcut_editor.rs`; from Preferences, the File menu or the palette) lists
  every command by category, filtered by title, category or keys (`commands::is_named_by`: every
  key named, any order and case). Each binding can be removed, Add records the next key press, and
  Reset restores a command's defaults. Reset all asks first, then offers Undo through
  `Workspace::restored` (`app-files.md`).
- `app::show` runs `commands::dispatch` each frame and hands a `CommandFrame` to toolbars and
  viewport. Exact modifiers win (extra Shift or Alt are ignored only for punctuation keys);
  sketch-scope bindings win while a sketch is edited; Backspace and Delete are left to drawing
  mid-shape; only the keys in `Command::repeats` repeat.
- A command's button calls `CommandFrame::invoke` with its availability, which records an `Offer`
  and says whether it triggered; a triggered unavailable command becomes a notice with the reason.
  Hover texts and menu items show the current binding (`commands::display`).

## Command palette

- `palette.rs` lists this frame's offers, then the sketch commands that do not fit outside a
  sketch (muted, after all offers, `palette::absence`), and once something is typed the features
  (selecting the row and scrolling the tree to it, `Focus::Feature`) and parameters (focusing the
  value field, `Focus::ParameterValue`); the focus reaches the panels the next frame
  (`Palette::take_focus`, `PanelState::request_focus`). Groups (Recent only while nothing is typed,
  Commands, Features, Parameters) are ordered by best match.
- A fixed detail line under the list says what Enter does, or why the highlighted entry is not
  available, distinguishing "unavailable now" from "only works while a sketch is edited". The
  chosen command is triggered on the next frame.

## Keyboard-only operation

- Every command is in the palette, including those with no button: recompute and its cancel,
  going to the first failed feature (selects its row), adding a parameter and deleting the one
  whose field last had focus (`PanelState::parameter`), each recent model (an `Offer`'s detail
  carries the file name), recovering unsaved work, cancelling an export, dismissing the notice and
  dismissing or hiding the current tip (offered while the palette covers it).
- A feature row's and panel's actions are commands on the tree's current feature
  (`feature_tree::current_feature`, the offer's detail; `feature_tree::commands`), sharing the
  button's availability. The selection-taking ones are `sketch_placement::place_on_selection`,
  `solid_panel::selected_axis_change` and `up_to_selected_change`, `datum_panel::base_change` and
  `rotation_change`, and `pattern_tools::selected_change`. Each feature row has a "⋯" menu with
  what its right-click menu holds.
- Tree order and the rollback bar are commands too (move, suppress, update references, roll back
  to here, roll to end, move the bar one row), so nothing needs a drag. Edit feature refuses a
  suppressed or rolled-back feature with the reason.
- Window commands (minimize, maximize or restore, full screen) are egui viewport commands
  (`window_frame::commands`); closing the window is Quit.
- Viewport commands cover measure, standard views, looking straight at the one selected flat face
  (outward normal from `sketch_placement::face_to_look_at`, framed on the face like Fit view),
  projection, orbit, pan and zoom. The view cube
  takes focus like a button and arrows step to the neighbouring view while it has it
  (`app-look.md`). Sketch commands (move, select all, tool keys and the ways of drawing a shape)
  are in `app-sketching.md`.
- Highlight next and previous step a keyboard highlight through the scene's pickables in
  pick-table order (drawn and described like hover; a pointer move or Escape clears it). Activate
  acts as a click would (selection toggle, or the region, blend edge, shell face or sketch plane
  action through `pick_action`); Enter opens what the item belongs to as a double-click would,
  and with nothing highlighted Enter confirms the open feature like its checkmark.
  With Trim, Extend, Mirror or Sketch fillet active they step through that tool's targets instead
  (`app-sketching.md`). With a drawing tool active, Activate on a highlighted point of the edited
  sketch or the origin places the shape's next point there, snapped to it as a click would
  (`place_at_highlight` through `Drawing::type_point`), and on a highlighted curve at its middle
  (`Drawing::type_on_curve`: a line's or arc's with `Midpoint`, a circle's rightmost point or a
  spline's halfway point on it), so keyboard drawing starts from existing geometry; where the shape
  takes only points, or on anything else, it says what to highlight.
- The selection filter (`SelectionFilter`, commands `select.*`, View › Selection filter) makes
  `PickTable::best_hit` and the highlight keys skip every pickable but one kind (faces, edges,
  vertices or sketch geometry), reference geometry included. It applies only while no sketch or
  tool is open and no plane is being chosen (`ViewportState::filter_applies`), so tools keep
  picking what they need; the status bar names an active filter and its button clears it. It is
  kept for the session, not saved.

- Outside sketch editing (no sketch edited, feature open, plane or reference being chosen) a
  primary drag in the 3D view draws a box (`PrimaryDrag::ModelBox`, `box_selection.rs`): left to
  right a window taking what lies wholly inside, right to left a crossing box taking what it
  touches, replacing the selection (Shift or Ctrl adds). It takes faces (Everything or Faces),
  edges, vertices or the curves of shown sketches by the selection filter, of shown bodies only.
  Faces count by their triangles facing the camera, so faces turned away are left out; edges and
  vertices are taken through the body. A box under `SMALLEST_BOX` points takes nothing.
- Select all (`select.all`), Select tangent edges and Select edges around faces
  (`body_selection.rs`) work on the shown bodies outside sketch editing and refuse inside one.
  Select all takes every face, edge or vertex by the selection filter, or by the kind already
  selected while the filter is Everything, and says what to choose when neither names one; seam
  edges, which cannot be picked, are never taken. Tangent edges add what `tangent_chain` reaches
  from the selected edges (smooth or sharp alike, unlike a blend's `blend_chain`); tangent faces
  (Alt+Shift+T) add what `tangent_faces` reaches from the selected faces; the edges around
  faces replace the faces with every loop's edges. Select the whole body (`select.body`, Edit menu,
  palette) replaces the selection with every face, edge or vertex (by the filter or the kind
  selected, faces otherwise) of the bodies the selection touches. An empty result is an info
  notice. Their
  availability is a cheap check on the selection, and the work runs only when triggered.

- The display style (`DisplayStyle`, commands `view.style_*`, View › Display style) is shaded with
  edges, shaded without edges (edges drawn with no alpha, so they stay pickable and appear when
  hovered or selected) wireframe (no faces in the scene, so none is drawn or picked and edges
  show through), hidden lines removed (faces in `Scene::flat_meshes`, unlit in `DRAWING_FACE`
  whatever the body's colour, still pickable and hiding what lies behind them, edges drawn over
  them, so it reads as a drawing) or X-ray (faces in `Scene::translucent_meshes` at
  `XRAY_FACE_ALPHA`, unpickable, edges on top). It applies to bodies outside sketch editing, reaches the scene through
  `Sources::style` and `Revisions::style`, shapes image exports too and is kept for the session.

## Typed-point field

- `typed_point.rs`: with a drawing tool active, typing a digit, sign, point, `(` or `@` opens it.
  It takes two length expressions split at top-level commas, a length and angle split at a
  top-level `<` (from the sketch's x axis; degrees unless a unit is named), `@` for an offset from
  the last placed point, and a lone length from the last point toward the pointer. The point must
  be within `MAX_LENGTH` of the origin. Offset's distance and Sketch fillet's radius reuse it as
  one length expression (`modifying::Value`, `app-sketching.md`), as does Move.
- It is canvas chrome (`canvas::PANEL`, `canvas::Hints`) in the band left of the view cube.
- Enter places the point through `Drawing::type_point` (landing exactly on an existing or the
  pending point snaps to it, like a click); an error keeps it open with the reason; Escape closes
  it without touching the shape. It is handled after the drawing syncs with the displayed sketch
  each frame, so a typed chain carries on from the end the previous segment added.
