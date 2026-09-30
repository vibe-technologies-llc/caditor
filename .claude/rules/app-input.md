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

- Every numeric input is a `field::commit_field`: commits on Enter or blur, reverts on Escape, keeps
  invalid text with its error inline instead of discarding it, until the stored value changes
  underneath it (an undo, say) while it is not being edited. Expression fields parse, evaluate and
  check the dimension before building a transaction. Sketch dimensions use
  `field::dimension_transaction`, applying the constraint's rule (a radius above zero).

## Keyboard focus

- Plain-key shortcuts (and Escape, Enter, Backspace in the viewport) run only when no widget held
  focus at the start of the frame or end of the previous, so Escape or Enter in a field never
  reaches the viewport; plain keys widgets never use (letters, Delete, function keys; not Space,
  Enter, Tab, Escape, arrows, Page Up/Down, Home, End) need only that no text field has focus.
- Ctrl or Alt shortcuts run from a text field too, except on the keys text editing uses
  (`TEXT_EDITING_KEYS`: A, C, V, X, Y, Z, arrows, Home, End, Backspace, Delete): the field loses
  focus (committing) and the command runs next frame (`Workspace::deferred_commands`), so Save saves
  a value just typed.

## Commands and keymap

- Every toolbar, menu and sketch action is a `Command` (`commands.rs`) with a stable id, title,
  category, `Scope` (anywhere, or only in a sketch) and default shortcuts.
- The `Keymap` in `Preferences` holds user bindings as overrides, stored as `keys.<id>` lists of
  text such as `Ctrl+Shift+Z`; only changed commands are written, so unreadable or newer entries
  survive, and a command with no readable binding keeps its defaults. Esc, Enter and Tab are
  reserved and cannot be bound. `shortcut_editor.rs` filters by `commands::is_named_by` (every key
  named, in any order and case, is held by the binding); a binding used in an overlapping scope asks
  before moving. It opens from Preferences, the File menu or the palette and lists every command by
  category, filtered by title, category or keys; Add records the next key press (Esc cancels),
  clicking a binding removes it, Reset and Reset all go back to the defaults.
- `app::show` dispatches key presses to commands each frame and hands a `CommandFrame` to toolbars
  and viewport: exact modifiers first (extra Shift or Alt ignored only for punctuation keys);
  sketch-scope bindings win while a sketch is edited; Backspace and Delete are left to drawing
  mid-shape; only camera, highlight, undo, redo and size keys repeat.
- A command's button calls `invoke` with its availability, recording an `Offer` and saying whether
  it triggered; a triggered unavailable one becomes a notice with the reason. Hover texts and menu
  items show the current binding (`commands::display`).

## Command palette

- `palette.rs` (Ctrl+Shift+P or the menu bar's Search commands) lists this frame's offers, so only
  commands that fit the context appear: title start, word starts, substring, scattered letters;
  available before unavailable, recently used first; arrows move, Enter runs, and an unavailable
  highlighted one shows its reason. The chosen command is triggered on the next frame.

## Keyboard-only operation

- Every command is in the palette: recompute (F5) and its cancel, going to the first failed feature
  (F8, which also selects its row), adding a parameter and deleting the one whose name or expression
  field last had focus (`PanelState::parameter`), opening each recent model (an `Offer` may carry a
  detail, here the file name, shown after the title), recovering unsaved work, cancelling an export or
  an image export, dismissing the notice and dismissing or hiding the current tip (offered while the palette covers
  it).
- A feature row's and panel's actions are commands on the tree's current feature
  (`feature_tree::current_feature`, the offer's detail; `feature_tree::commands`), sharing the
  button's availability: Edit feature (E), Finish editing feature, Detach sketch, Place sketch on
  selected plane or face (`sketch_placement::place_on_selection`), Revolve about selected axis
  (`solid_panel::selected_axis_change`), Extrude up to selected face or plane
  (`solid_panel::up_to_selected_change`), Base datum on selection and Turn datum plane about
  selected axis (`datum_panel::base_change`, `rotation_change`), Pattern along or about selected
  axis and Pattern also along selected direction (`pattern_tools::selected_change`). Tab moves between
  widgets and Escape leaves them; each feature row has a "⋯" menu with what its right-click menu
  holds.
- Tree order and the rollback bar are commands too, so nothing needs a drag: Move feature up and
  down, Suppress or unsuppress feature (the chosen rows), Roll back to here (the bar right below
  the current feature), Roll to end, and Move the rollback bar up or down (Alt+Up, Alt+Down, the
  bar stepping one row). Edit feature refuses a suppressed or rolled-back feature with the
  reason.
- Window commands: Minimize the window, Maximize or restore the window (refused in full screen)
  and Enter or leave full screen (F11), carried out as egui viewport commands
  (`window_frame::commands`); closing the window is Quit.
- Viewport commands: Measure (I), standard views (Alt+0 to Alt+6), the projection (O), orbit (arrows), pan
  (Shift+arrows), zoom (Page Up and Page Down). In a sketch, M moves the selection to a typed
  position and Ctrl+A selects all of it (`app-sketching.md`). Each way of drawing a rectangle,
  circle, polygon or slot is a sketch command without a default key, and a shape's tool key
  pressed again while it is active steps to its next way (`app-sketching.md`).
- N and Shift+N step a keyboard highlight through the scene's pickables in pick-table order, each
  once (drawn and described like hover; a pointer move or Escape clears it); Space acts on it as a
  click would (toggling it in the selection, or a region, blend edge, shell face or sketch plane as
  in those modes, through `pick_action`), Enter opens what it belongs to as a double-click would.
  With Trim or Extend active they step through that tool's targets instead, and Space or Enter
  trims or extends the highlighted one; with Mirror the lines and axes to mirror about, with
  Sketch fillet the corners to round (`app-sketching.md`).

## Typed-point field

- It also takes Offset's distance and Sketch fillet's radius as one length expression (`Offset
  by`, `Fillet radius`, `modifying::Value`), opened the same way (`app-sketching.md`).

- `typed_point.rs`: with a drawing tool active, typing a digit, sign, point, `(` or `@` opens it.
  Accepted: two length expressions (preferred unit) split at top-level commas; a length and angle
  split at a top-level `<` (not `<=`; degrees unless a unit is named, from the sketch's x axis); `@`
  for an offset from the last placed point; a lone length goes from it toward the pointer. The point
  must be within `MAX_LENGTH` of the origin.
- Enter places the point through `Drawing::type_point` (landing exactly on an existing or the
  pending point snaps to it, like a click); an error keeps it open with the reason; Escape closes it
  without touching the shape. It is handled after the drawing syncs with the displayed sketch each
  frame, so the end a typed segment adds is in the sketch by the next sync and a typed chain carries
  on from it.
