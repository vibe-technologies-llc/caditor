---
paths:
  - "crates/caditor/src/fonts.rs"
  - "crates/caditor/src/appearance.rs"
  - "crates/caditor/src/icons.rs"
  - "crates/caditor/src/widgets.rs"
  - "crates/caditor/src/dialog_parts.rs"
  - "crates/caditor/src/canvas.rs"
  - "crates/caditor/src/view_cube.rs"
  - "crates/caditor/src/layout.rs"
  - "crates/caditor/src/menu_bar.rs"
  - "crates/caditor/src/window_frame.rs"
  - "crates/caditor/src/overlay.rs"
  - "crates/caditor/src/toolbar.rs"
  - "crates/caditor/src/sketch_toolbar.rs"
  - "crates/caditor/src/ribbon.rs"
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
- The 3D view keeps a dark canvas in every theme, and `canvas.rs` is its chrome. Colours are
  defined there once: text, muted, dimension, hover amber, selection blue, error, warning, prompt
  and snap, which `scene.rs` converts for the geometry it highlights (`opaque`, `translucent`), so a
  hovered or selected line and its label share one colour. Text uses its type scale (`small` 11.5,
  `body` 13, `title` 15 in Inter medium, `emphasis` for keys and glyph letters, `readout` monospace
  for the cursor position) and sits on the translucent `BACKDROP` with one padding (`PADDING`) and
  radius (`RADIUS`), as `canvas::label` or a `canvas::Label` measured before it is painted; the
  typed-point field uses the opaque `PANEL`. A test holds every canvas colour to 4.5:1 (body 7:1)
  over that backdrop on black, white and the hover amber, key caps and canvas controls in every
  state, and the view cube's labels on each cell state.
- Key hints are `canvas::Hints`: text in the `Key: action` form, items three spaces apart, is laid
  out as key caps (several keys joined by " or ") beside a muted action, plain text where the part
  before ": " is not keys, wrapping in rows within the width given. Each key, "or" and action is its
  own text, so tests check a hint with `shows_hint`.
- Overlay text keeps clear of the view cube: the prompt (`title`, prompt colour) and its key hints
  centre on the view within the band left of `view_cube::area`, wrapping there (at most 720
  points); the hover description starts at the band's top left, moving under the prompt when they
  would meet; the cursor readout sits right of the axis triad; the navigation hints sit bottom
  right, right-aligned, beside room kept for the readout while sketching, and are left out when
  they would meet the cube or the prompt.
- The view cube (`view_cube.rs`) draws its 26 targets as cells with thin dividers, the hovered one
  in hover amber (darker while pressed) with a dark label; it is one focusable button named
  "View cube: View from top, front and right" after the hovered target, else the view it is
  nearest, with that name as its tooltip, and while focused the arrow keys step to the
  neighbouring target on that side. Fit all (Fit selection when something is selected) under it
  is a canvas control (`canvas::button`: backdrop fill, hover and pressed fills, the canvas focus
  ring, an icon and the label, `BUTTON_HEIGHT` tall).

## Widgets

- `widgets.rs` is the kit every panel and dialog is built from: `ToolButton` (icon above label,
  `tool_height` tall, at least two `CONTROL_HEIGHT` rows; `compact` draws the icon alone, named by
  its label, in a square of `compact_tool_side` so two rows match one tool button; selected is an
  accent subtle fill with an accent outline, hovered a `border_strong` outline, pressed the pressed
  fill even when selected), `section`, `properties`/`property`/`error_row`, `removable_row` (text
  wrapping in the room left of its remove button), `card`, `callout` and `pill` with a `Tone`,
  `status_pill` (the tone's icon before the text), `icon_button` (a true `CONTROL_HEIGHT` square,
  so rows reserving that room never overflow and widen a panel), `button` (a plain secondary
  button), `small_button` (icon and text), `primary_button` and `primary_icon_button` (accent
  fills for rest, hover and press, a focus ring outside), `danger_button` (the same in `danger`
  fills, for actions that throw work away; it never registers as the primary), `text_field` (the
  scope every text edit is drawn in: a `field_border` outline, `text_muted` on hover, the focus
  ring), `strong` (semibold text, since egui's `.strong()` only recolours here), `segmented` and
  `segmented_with` (a joined row of 2–4 short choices, the current one raised with an accent
  outline, options disabled with a reason through `Segment::refusal`; when the row is wider than
  the room it falls back to a dropdown captioned like a combo box, so it never widens a panel),
  `key_cap` (each key of "Ctrl+Shift+P" as a small cap, in reading order in either layout
  direction), `empty_state` (a muted icon and sentence with actions under it), `stepper`
  (− value +), `panel_header` (icon, section title, trailing actions), `menu_item`, `menu_choice`
  (one marked as the current choice), `menu_option` (a selectable item in a dropdown),
  `menu_item_with_detail` (a muted detail at the right, such as a recent file's folder),
  `corner_menu_button` (a small caret in another button's top-right corner, opening a popup menu
  without taking width of its own), `link_label`, `choose_in_view`, `dialog`/`footer` (a titled
  modal with a close button and the primary action rightmost), `footer_split` (a footer with a
  destructive action at the far left, away from the primary) and `tabs`. Spacing comes from
  `appearance` (`SPACE_XS` 2, `SPACE_S` 4, `SPACE_M` 8, `SPACE_L` 12, `DIALOG_MARGIN` 20,
  `CONTROL_HEIGHT` 24, `BORDER_WIDTH`, `FOCUS_WIDTH`). Panels and dialogs never add raw egui
  buttons: `conventions_tests.rs` (`buttons_outside_the_widget_kit_come_from_it`) fails on
  `ui.button`, `ui.small_button`, `Button::new`, `Button::selectable` or `selectable_label`
  outside `widgets.rs`. A dialog gives focus to its `primary_button` whenever no widget holds it
  (opening, Escape out of a widget, a click on bare dialog), so Enter runs the primary action
  while a field or another button that has focus keeps Enter for itself. A dialog whose only
  action would be Close has no footer: its title bar's close button is enough.
- `dialog_parts.rs` holds what dialogs share beyond the kit: `confirmation` and `confirm_footer`
  (a warning callout asking, over the destructive action left and a primary Cancel right, used
  before Restore defaults and Reset all shortcuts), `undo_note` (a success callout with Undo after
  either), `titled_modal` (a titled dialog without a close button, for waits that cannot be
  cancelled) and `BodyRoom` (the height a dialog's scrolling middle may take so the whole dialog
  stays within a share of the window, measuring what sits above it and, from the last frame,
  below it). Sentence case everywhere: buttons, titles and commands ("Save as…", "Close without
  saving", "Keyboard shortcuts").
- The side panel's scroll area always keeps its scrollbar's room, so the panel keeps its width
  when its content grows past the window's height.
- Tokens beyond the surfaces and tones: `field_border` (3:1 against every surface a field sits
  on), `accent_hover` and `accent_pressed`, and `danger`, `danger_hover` and `danger_pressed`
  (white text at 4.5:1, 3:1 against panels and dialogs). `tokens_for` finds the token set from
  the visuals' panel fill and theme, and `appearance.rs` tests text pairings, control outlines,
  focus rings and both button kinds in all four themes.
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
  dimension's field is labelled by its constraint's description; the shortcut editor's remove
  crosses, Add… and Reset the command they act on; palette rows and the welcome's sample cards and
  recent files what choosing them does; a tab is an AccessKit `Role::Tab` named by its label and
  marked selected.
  Decorative glyphs are hidden from the AccessKit tree (`icon_label`,
  `decorative`), meaningful ones described (`described_icon`); a property `caption` labels the text
  field, combo box or slider after it in its grid (`tie_to_caption`, via `field::commit_field`).

## Menu bar, ribbon and status bar

- The menu bar holds File, Edit, View, Model, Sketch and Help, built from the commands with their
  icons and shortcuts; items trigger their command and enable from the previous frame's offers
  (`Workspace::last_offers`), reason on hover. Separators group them: View has the camera (Fit,
  Standard views, Camera) | projection | full screen | Measure | visibility | highlight | interface
  size; Model has New sketch, Extrude and Revolve | Fillet, Chamfer and Shell | patterns | datums |
  feature commands | rollback | parameters | recompute; Sketch has Finish sketch | Select and the
  drawing tools | Ways to draw shapes, Reverse the arc, more or fewer sides | Construction, the
  modify tools, Move and Select all | Constraints and Dimensions submenus. Then the model title, the
  Search commands field and, with caditor's title bar, the window buttons; caditor's title bar also
  leads with the logo, as tall as a control, which drags the window like the rest of the bar. A
  `border` hairline (the panel's separator line) parts the menu bar from the ribbon.
- Search commands is drawn as a field: `sunken` fill, a `field_border` outline turning
  `text_muted` on hover or press, the focus ring when focused, the search icon and muted label,
  and the palette's keys as `widgets::key_cap`s at its right end (the field widens to fit them).
  Clicking it opens the palette.
- The model title (file icon, name, Unsaved pill, and "› sketch" while one is edited, name and
  sketch both in the `Button` text style) sits centred on the window when it fits between the menus
  and the search field, else as far towards the centre as it can (`model-title-width`, remembered
  like the bars' trailing widths). It fills with `hover` on hover or while its menu is open,
  `pressed` while pressed, and shows the focus ring when focused. Clicking it opens the model
  details: path, saved state, Save, Save as…, Version history… (once saved) and Copy file location.
  Its text is muted while the window is unfocused.
- Both ribbons are built by `ribbon.rs`: whole groups packed into rows by last frame's natural
  widths (`ribbon::rows`, `remembered_width` per group), `SPACE_L` apart with a 1-point
  `border_strong` divider between neighbours (visible on `accent_surface` too), each captioned in
  small muted text under its buttons while the ribbon fits one row; a ribbon that wraps leaves the
  captions out to stay short. Buttons explain themselves on hover with `ribbon::explained`: the
  name in semibold, then what it does with its shortcut, or why it is unavailable while disabled.
- The main ribbon (`toolbar.rs`) has History (Undo, Redo), Sketch (New sketch), Solid (Extrude,
  Revolve), Modify (Fillet, Chamfer, Shell), Pattern (Linear pattern, Circular pattern), Reference
  (Plane, Axis) and Inspect (Measure, selected while its panel is open), all `ToolButton`s. While a
  plane is being chosen for a new sketch, New sketch shows selected and clicking it stops choosing
  (as Escape does; the prompt is in the view), so the ribbon keeps its height and every group its
  place.
- While a sketch is edited the sketch ribbon (`sketch_toolbar.rs`) sits under it on
  `accent_surface` with a 2-point `accent_text` line along its top. Groups run left to right: a
  header without a caption, vertically centred on the ribbon (a `CONTROL_HEIGHT` sketch badge,
  "Editing <name>" truncating with the full name on hover, the sketch's `status_pill`s; as wide as
  the title between 160 and 240 points of text, never as wide as the pills, so constraining never
  moves the groups), Select, Draw (the drawing tools as `ToolButton`s; rectangle, circle, polygon and
  slot carry a `corner_menu_button`, "Ways to draw a rectangle" and so on, listing their ways of
  drawing as `menu_choice`s with the current one marked), Modify (compact Construction, Trim, Extend
  and Sketch fillet over Offset, Mirror, Move, Select all and Delete, which trigger their commands
  or choose their tool), Constrain (the geometric constraints, six compact buttons a row) and
  Dimension (three a row). Finish sketch is a `CONTROL_HEIGHT` primary button at the right of the
  first row, which takes a row of its own when not even the header fits beside it.
- The three arc tools share one Draw button labelled Arc (`sketch_toolbar::ARC_TOOLS`), showing
  the icon of the arc tool last used or chosen (kept in egui memory while caditor runs) and choosing
  it; its corner menu, "Ways to draw an arc", lists Arc, 3-point arc and Tangent arc with their
  keys. Each keeps its own key and command, which the button handles whichever is shown.
- A group wider than its row (Draw at 200% in a narrow window) wraps inside. Widths never depend
  on the selection or the active tool, so selecting never moves the 3D view; at 1400 points the
  sketch ribbon fits one row.
- The status bar (`status_bar.rs`) has an explicit frame (`SPACE_M` by `SPACE_S`) and runs
  recompute | file activity | notice, then, right-aligned, selection | unit | size, segments parted
  by short `border_strong` dividers (file activity's only when it shows something). Recompute is
  progress with a Cancel button, Up to date, or a failed `status_pill` that is a button (named by
  its text, outlined on hover and press, pointing cursor, focus ring) focusing the first failed
  feature; a cancelled or stopped recompute is a warning or error `status_pill` with the details on
  hover and a Recompute or Restart button (`widgets::button`). The selection reads in `text`, and
  "Nothing selected" muted. The length unit (opening Preferences) is a `small_button` and the
  interface size, when not 100%, a `widgets::button` going back to 100%. An edit clears info and
  refused-edit notices, never a `Notice::failure` from saving, opening, importing, exporting or the
  journal.
- Neither bar clips at large sizes or in narrow windows: the menu bar's search field and model name
  (which truncates, its path on hover) and the status bar's selection, unit and size take their own
  row when last frame's needed width does not fit (`widgets::remembered_width`); the selection has a
  fixed truncating share (all of it on hover), so selecting never moves the 3D view; a notice too
  long for the row takes its own and wraps, its dismiss button measured from last frame.
- Overlap checks in the UI tests compare only the visible part of each text (`Harness::text_clips`),
  since content scrolled or squeezed under a bar is clipped there.

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
  far right are Minimize, Maximize or Restore, and Close, each 40 points by `CONTROL_HEIGHT` and
  `SPACE_XS` apart: `hover` and `pressed` fills, Close solid `danger` on hover and `danger_pressed`
  while pressed with a `text_on_accent` glyph, closing through Quit so unsaved work is asked about,
  and a `FOCUS_WIDTH` focus outline; in full screen Leave full screen replaces the first two.
- While a dialog is open, `window_frame::over_dialogs` redraws the drag area and window buttons in
  a foreground area moved above the modal, so the window can still be moved, maximized or closed.
- When neither maximized nor full screen, a `BORDER_WIDTH` outline marks the window's edge (square:
  the window is opaque, so rounding it would leave its corners showing) and 5-point
  strips along it (foreground areas kept on top, 16-point corners resizing diagonally) set the
  resize cursor and start `BeginResize` on a primary press.
- The compositor takes the pointer during a move or resize and the release never reaches the
  window, so `Overlay` queues a synthetic primary release after `StartDrag` or `BeginResize`.
- The window's minimum size is `layout::MIN_WINDOW_WIDTH` by `MIN_WINDOW_HEIGHT`.
