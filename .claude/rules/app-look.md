---
paths:
  - "crates/caditor/src/fonts.rs"
  - "crates/caditor/src/appearance.rs"
  - "crates/caditor/src/icons.rs"
  - "crates/caditor/src/widgets.rs"
  - "crates/caditor/src/canvas.rs"
  - "crates/caditor/src/layout.rs"
  - "crates/caditor/src/menu_bar.rs"
  - "crates/caditor/src/window_frame.rs"
  - "crates/caditor/src/overlay.rs"
  - "crates/caditor/src/toolbar.rs"
  - "crates/caditor/src/sketch_toolbar.rs"
  - "crates/caditor/src/status_bar.rs"
  - "crates/caditor/src/panels.rs"
  - "crates/caditor/src/feature_tree.rs"
  - "crates/caditor/src/parameter_table.rs"
  - "crates/caditor/src/principal_tree.rs"
  - "crates/caditor/src/tree_row.rs"
---

# Look, layout and panels

## Fonts, icons and theme

- Inter (variable font, `assets/fonts`, OFL) is registered at weights 400, 500 and 600 through its
  `wght` axis as the proportional, `medium` and `semibold` families; Phosphor icons
  (`egui-phosphor`) have their own `icons` family, since Inter's private-use glyphs would shadow
  them. Icons: `icons.rs`, one per command, tool, constraint and feature kind, taken from there by
  panels. Fonts are installed on the first frame, which draws nothing, and the style applies from
  the next.
- `appearance.rs` holds the theme as `Tokens` (surfaces, text, a blue accent, error, warning and
  success with tinted backgrounds, and `accent_surface`, a faint accent tint for a bar marking a
  context such as the sketch ribbon) for dark, light and both high-contrast variants, builds each egui
  `Style` from them (Inter text styles plus a `section` style, spacing, radii, shadows) and tests
  every text pairing against its background. Panels read colours from `appearance::tokens(ui)` or
  the visuals, never fixed values.
- The 3D view keeps a dark canvas in every theme; text on it takes its colours from `canvas.rs` and
  sits on a translucent backdrop (`canvas::label`); a test holds every canvas colour to 4.5:1 (body
  7:1) over that backdrop on black, white and highlight colours.

## Widgets

- `widgets.rs` is the kit every panel and dialog is built from: `ToolButton` (icon above label,
  `tool_height` tall; `compact` draws the icon alone, named by its label, in a square of
  `compact_tool_side` so two rows match one tool button; selected is an accent subtle fill with an
  accent outline), `section`, `properties`/`property`/`error_row`, `removable_row` (text that
  wraps beside a remove button), `card`, `callout` and `pill` with a `Tone`, `status_pill` (the
  tone's icon before the text, for states such as a sketch's), `icon_button`, `small_button`,
  `primary_button` and `primary_icon_button`, `menu_item` and `menu_choice` (one marked as the
  current choice), `corner_menu_button` (a small caret in another button's top-right corner,
  opening a popup menu without taking width of its own), `link_label`, `choose_in_view`,
  `dialog`/`footer` (a titled modal with a close button and the primary action rightmost), and
  `tabs`. A dialog gives focus to its `primary_button` whenever no widget holds it (opening, Escape
  out of a widget, a click on bare dialog), so Enter runs the primary action while a field or
  another button that has focus keeps Enter for itself.
- `tabs` is a wrapping row of `Tab`s (icon and label) over a border line: the selected one tinted
  `accent_subtle` with `accent_text` and a 2-point accent underline, others `text` on hover or
  pressed fills, focus outlined in `focus`. It returns the tab chosen by click, by Left/Right
  (wrapping), Home or End on a focused tab (focus follows, egui's own arrow focus move cancelled),
  or, when `switch_keys` is set, by Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+Page Down and Ctrl+Page Up.
  Tab ids are `tab_id(row, index)`, so tests can focus one.
- Dialog widths and list heights are clamped to the screen (`fitting_width`, `list_height`), so
  nothing clips at 200%. Icons and labels are separate text atoms, so tests find buttons by bare
  label, and compact ones by accessible name.

## Screen readers

- Every button drawn with a glyph takes a name without it (`Named`, `named`): icon buttons their
  hover text; small buttons, menu items and section headers their label; a tree row's chevron and
  "⋯" buttons the action and the feature's name, its edit button "Edit <name>" (or what closes it),
  the row itself a selectable label named by the feature or item and marked selected; a sketch
  dimension's field is labelled by its constraint's description; the shortcut editor's chips, Add…
  and Reset the command they act on; a tab is an AccessKit `Role::Tab` named by its label and marked selected.
  Decorative glyphs are hidden from the AccessKit tree (`icon_label`,
  `decorative`), meaningful ones described (`described_icon`); a property `caption` labels the text
  field, combo box or slider after it in its grid (`tie_to_caption`, via `field::commit_field`).

## Menu bar, ribbon and status bar

- The menu bar holds File, Edit, View, Model, Sketch (with a Ways to draw shapes submenu) and
  Help, built from the commands with their icons and shortcuts; items trigger their command and enable from the previous frame's offers
  (`Workspace::last_offers`), reason on hover. Then the model title, the Search commands field
  and, with caditor's title bar, the window buttons; caditor's title bar also leads with the logo,
  as tall as a control, which drags the window like the rest of the bar.
- The model title (file icon, name, Unsaved pill, and "› sketch" while one is edited) sits centred
  on the window when it fits between the menus and the search field, else as far towards the
  centre as it can (`model-title-width`, remembered like the bars' trailing widths). Clicking it
  opens the model details: path, saved state, Save, Save As…, Version History… (once saved) and
  Copy file location. Its text is muted while the window is unfocused.
- The ribbon groups Undo and Redo, New sketch, Extrude and Revolve, Fillet, Chamfer and Shell,
  Linear pattern and Circular pattern, Plane and Axis, and Measure (selected while its panel is
  open) as `ToolButton`s that wrap.
- While a sketch is edited the sketch ribbon (`sketch_toolbar.rs`) sits under it on
  `accent_surface` with a 2-point `accent_text` line along its top. Groups run left to right,
  split by thin dividers and captioned in small muted text below: a fixed-width header (sketch badge,
  "Editing <name>" truncating with the full name on hover, the sketch's `status_pill`s), Select,
  Draw (the drawing tools as `ToolButton`s; rectangle, circle, polygon and slot carry a
  `corner_menu_button`, "Ways to draw a rectangle" and so on, listing their ways of drawing as
  `menu_choice`s with the current one marked), Modify (compact Construction, Trim, Extend and Sketch
  fillet over Offset, Mirror, Move, Select all and Delete, which trigger their commands or choose
  their tool), Constrain (the geometric constraints, six compact buttons a
  row) and Dimension (three a row). Finish sketch is a primary button at the right of the first
  row. Compact buttons show their name, what they do and the shortcut on hover, and why they are
  unavailable while disabled.
- The ribbon packs whole groups into rows by last frame's natural widths (`remembered_width` per
  group and for Finish sketch), the first row leaving room for Finish sketch, which takes a row of
  its own when not even the header fits beside it; a group wider than its row (Draw at 200% in a
  narrow window) wraps inside. Widths never depend on the selection, so selecting never moves the
  3D view; at 1400 points everything fits one row.
- The status bar shows recompute progress (or Up to date, or a failed pill that focuses the first
  failed feature), saving, importing and exporting, the current notice, the selection, the length
  unit (opening Preferences) and the interface size when not 100%. An edit clears info and
  refused-edit notices, never a `Notice::failure` from saving, opening, importing, exporting or the
  journal.
- Neither bar clips at large sizes or in narrow windows: the menu bar's search field and model name
  (which truncates, its path on hover) and the status bar's selection, unit and size take their own
  row when last frame's needed width does not fit (`widgets::remembered_width`); the selection has a
  fixed truncating share (all of it on hover), so selecting never moves the 3D view; a notice too
  long for the row takes its own and wraps.

## Side panel

- The parameter table's name and expression fields share the panel's width beside a fixed value
  column (`VALUE_WIDTH`, 88 points), since content wider than a side panel widens it the next
  frame. A value that cannot be evaluated is an error icon with the reason on hover, a refused edit
  an `error_row` under the fields. A row's delete button is drawn only while the row is hovered or
  the button holds keyboard focus; it stays in the Tab order. Added parameters are named
  parameter1, parameter2 and so on, the first free number.
- The side panel has collapsible Features and Parameters sections (`widgets::section`, `SPACE_S`
  above and `SPACE_L` between), opening by themselves when a rename or focus request needs
  something inside. Empty, each is an `empty_state`: the tree offers New sketch (choosing a plane
  next) and Open a sample (the welcome dialog), the table Add parameter.
- Tree rows come from `tree_row.rs`, shared by the feature tree and `principal_tree.rs`: a
  20-point chevron, the kind icon, the name and fixed trailing slots, each `CONTROL_HEIGHT` square
  and left empty when the row has nothing for it, so icons line up in columns: status, edit,
  visibility and "⋯" for features, the visibility column alone for principal geometry. Rows are
  `ROW_GAP` apart; the callouts and cards under a row (`tree_row::indented`) keep the normal
  spacing. Selected rows fill with `accent_subtle`, the open row with `accent_surface` and an
  accent bar, a hovered one with `hover`; keyboard focus outlines the row.
- Kind icons are tinted by category: sketches `accent_text`, bodies and what modifies them `text`,
  datums `text_muted`, and any row inactive or hidden `text_muted`. A failure shows once, as the
  status icon and its callout, never by recolouring the icon or the name; an outdated row the same
  with a Recompute button. A suppressed row's name is struck through and muted with the suppress
  icon as its state; a row below the rollback bar is muted with the rolled-back icon; neither offers
  the edit button. A failure caused by a suppressed feature carries an Unsuppress button
  (`FixTarget::Unsuppress`); callout actions (Recompute, Unsuppress, Go to, Edit the dimension,
  Detach) are `small_button`s with icons.
- A click on a row's name only selects it (`PanelState::selected`, cleared when the view selection
  changes), as does tabbing to it; Ctrl+click adds or removes a row and Shift+click takes the rows
  from the selected one (`PanelState::chosen`, the primary first). The chevron alone shows or hides
  the details. A double-click, Enter on the focused row, the edit button or the menu's Edit opens
  the feature: a sketch for editing, any other feature as its open panel, an imported body by
  showing its details; a suppressed or rolled-back one leaves a notice with the reason instead. Edit
  buttons read "Edit <name>", and while open Finish sketch or Finish editing feature. An open
  feature scrolls into view while its card expands (`PanelState::reveal`).
- Rename feature (F2) and the menu's Rename turn the name into a field filling the name's place,
  chevron and icon kept. Rename and Move feature up or down act on the primary, else on the open
  feature (`feature_tree::current_feature`); Suppress or unsuppress feature and Delete feature act on
  every chosen row (a row's menu on the chosen rows when it is one of them); Delete selection
  (Delete) deletes them outside sketch editing.
- The principal group's items are indented rows: hovering one highlights it in the view, a click
  or tabbing to it selects it in the view when shown (clearing the tree's feature selection), and a
  row reads as selected while its pickable is.
- The rollback bar is a row of its own (`feature_tree::rollback_bar`, named "Rollback bar" for
  screen readers): a grip glyph, "N features rolled back" when any are, and an accent line, drawn
  at the end of the tree when nothing is rolled back. It and every row name can be dragged
  (`PanelState::dragging`): the gap under the pointer gets an accent line, or an error-coloured
  line and a tooltip-order popup with the reason when `Document::move_row` refuses it; release
  applies it, Escape or a release outside the tree cancels, and the tree scrolls near its edges.
  Move up and Move down step past the bar when it is next to the row, except past the bar at the
  end.
- Deleting features others depend on opens the "Delete …?" dialog (`feature_tree::delete_dialog`,
  `PanelState::deleting`, counted as a modal): the dependents in tree order with what each uses,
  then Delete with dependents (`danger_button`, leftmost), Keep dependents and Cancel (rightmost);
  Cancel takes focus whenever focus is outside the dialog, so Enter never deletes. Either deletion
  is one transaction. Without dependents Delete acts at once; the menu item reads "Delete…" when it
  will ask.
- A sketch's card gives its entity count, then collapsible Dimensions and Constraints sections with
  their counts (opening when a focus request targets something inside); each dimension's field sits
  under its description. Constraint rows highlight their entities on hover and, clicked, edit the
  sketch and select the constraint (`PanelState::hovered_in_tree`, `chosen_in_tree`, handed to the
  viewport after the panels are drawn).

## Window and panel persistence

- `layout.rs`: `Preferences` holds a `WindowPlacement` (inner size in logical pixels, maximised,
  outer position where the platform reports one, which Wayland never does) and a `PanelLayout` (side
  panel width, open sections) under `window.*` and `panels.*`, read clamped to sane bounds. Startup
  fits the size to the largest monitor and uses a position only when it lies on one; size and
  position are recorded only while the window is not maximised, so unmaximising returns to them.
  `PanelState` restores the sections once and reports the panel as drawn each frame. Changes save
  through `Files::store_settings` a second after they stop, and on exit, where `App::finish` waits
  up to two seconds (`Files::wait_for_jobs`). A preferences file that cannot be read
  (`Settings::load_reporting`) leaves the defaults in use and opens a notice saying so at startup
  (`preferences::unreadable_notice`); a failed save becomes a notice too (`Event::PreferencesNotSaved`)
  and the next change writes everything the failed save held.

## Title bar and window frame

- `Preferences::title_bar` (`appearance.title_bar`: `built_in` by default, or `system`) chooses
  between caditor's title bar (an undecorated window, the menu bar acting as its title bar) and
  the window manager's. The window is created with that decoration and a change is applied at once
  through `ViewportCommand::Decorations` (`window_frame::apply_title_bar`); it is offered in
  Preferences › Appearance and in the title bar's window menu.
- `window_frame.rs` does everything through egui `ViewportCommand`s, which `Overlay::run` hands to
  `egui_winit::process_viewport_commands`; `Overlay` feeds the window's maximized, full-screen and
  focus state back as the root `ViewportInfo` each frame (`WindowState::of`).
- With caditor's title bar, the menu bar's empty space (last frame's rect, registered below its
  widgets) and the model title drag the window (`StartDrag` once a primary drag starts), a
  double-click on empty space maximizes or restores, and a right-click opens the window menu
  (Minimize, Maximize or Restore, Full screen, Use the system title bar, Close). The buttons at the
  far right are Minimize, Maximize or Restore, and Close (red tint on hover, closing through Quit
  so unsaved work is asked about); in full screen Leave full screen replaces the first two.
- While a dialog is open, `window_frame::over_dialogs` redraws the drag area and window buttons in
  a foreground area moved above the modal, so the window can still be moved, maximized or closed.
- When neither maximized nor full screen, a 1-point outline marks the window's edge and 5-point
  strips along it (foreground areas kept on top, 16-point corners resizing diagonally) set the
  resize cursor and start `BeginResize` on a primary press.
- The compositor takes the pointer during a move or resize and the release never reaches the
  window, so `Overlay` queues a synthetic primary release after `StartDrag` or `BeginResize`.
- The window's minimum size is `layout::MIN_WINDOW_WIDTH` by `MIN_WINDOW_HEIGHT`.
