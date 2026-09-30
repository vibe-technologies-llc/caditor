---
paths:
  - "crates/caditor/src/fonts.rs"
  - "crates/caditor/src/appearance.rs"
  - "crates/caditor/src/icons.rs"
  - "crates/caditor/src/widgets.rs"
  - "crates/caditor/src/canvas.rs"
  - "crates/caditor/src/layout.rs"
  - "crates/caditor/src/menu_bar.rs"
  - "crates/caditor/src/toolbar.rs"
  - "crates/caditor/src/status_bar.rs"
  - "crates/caditor/src/panels.rs"
  - "crates/caditor/src/feature_tree.rs"
  - "crates/caditor/src/parameter_table.rs"
  - "crates/caditor/src/principal_tree.rs"
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
  success with tinted backgrounds) for dark, light and both high-contrast variants, builds each egui
  `Style` from them (Inter text styles plus a `section` style, spacing, radii, shadows) and tests
  every text pairing against its background. Panels read colours from `appearance::tokens(ui)` or
  the visuals, never fixed values.
- The 3D view keeps a dark canvas in every theme; text on it takes its colours from `canvas.rs` and
  sits on a translucent backdrop (`canvas::label`); a test holds every canvas colour to 4.5:1 (body
  7:1) over that backdrop on black, white and highlight colours.

## Widgets

- `widgets.rs` is the kit every panel and dialog is built from: `ToolButton` (icon above label),
  `section`, `properties`/`property`/`error_row`, `removable_row` (text that wraps beside a remove
  button), `card`, `callout` and `pill` with a `Tone`, `icon_button`, `small_button`,
  `primary_button`, `menu_item`, `link_label`, `choose_in_view`, `dialog`/`footer` (a titled modal
  with a close button and the primary action rightmost).
- Dialog widths and list heights are clamped to the screen (`fitting_width`, `list_height`), so
  nothing clips at 200%. Icons and labels are separate text atoms, so tests find buttons by bare
  label.

## Screen readers

- Every button drawn with a glyph takes a name without it (`Named`, `named`): icon buttons their
  hover text; small buttons, menu items and section headers their label; a tree row's chevron, edit
  and "⋯" buttons the action and the feature's name; the shortcut editor's chips, Add… and Reset the
  command they act on. Decorative glyphs are hidden from the AccessKit tree (`icon_label`,
  `decorative`), meaningful ones described (`described_icon`); a property `caption` labels the text
  field, combo box or slider after it in its grid (`tie_to_caption`, via `field::commit_field`).

## Menu bar, ribbon and status bar

- The menu bar holds File, Edit, View, Model, Sketch and Help, built from the commands with their
  icons and shortcuts; items trigger their command and enable from the previous frame's offers
  (`Workspace::last_offers`), reason on hover. Its right side shows the document name (with an
  Unsaved pill) and the Search commands field.
- The ribbon groups Undo and Redo, New sketch, Extrude and Revolve, Fillet, Chamfer and Shell, and
  Plane and Axis as `ToolButton`s that wrap. While a sketch is edited a tinted sketch ribbon shows
  its name and state pill, the drawing tools, the constraints in a wrapping grid, Delete and Finish
  sketch.
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
  column, since content wider than a side panel widens it the next frame.
- The side panel has collapsible Features and Parameters sections, opening by themselves when a
  rename or focus request needs something inside.
- A feature row is its kind icon, name, state icon, edit and more buttons, highlighted with an
  accent bar while open, with failures and outdated states as callouts under it and its properties
  in a card; an opening feature scrolls into view while its card expands (`PanelState::reveal`).
- Clicking or tabbing to a row name selects it (`PanelState::selected`, cleared when the view
  selection changes); Rename feature (F2), Move feature up or down and Delete feature act on it,
  else on the open feature (`feature_tree::current_feature`); Delete selection (Delete) deletes it
  outside sketch editing.
- A sketch's constraint rows highlight their entities on hover and, clicked, edit the sketch and
  select the constraint (`PanelState::hovered_in_tree`, `chosen_in_tree`, handed to the viewport
  after the panels are drawn).

## Window and panel persistence

- `layout.rs`: `Preferences` holds a `WindowPlacement` (inner size in logical pixels, maximised,
  outer position where the platform reports one, which Wayland never does) and a `PanelLayout` (side
  panel width, open sections) under `window.*` and `panels.*`, read clamped to sane bounds. Startup
  fits the size to the largest monitor and uses a position only when it lies on one; size and
  position are recorded only while the window is not maximised, so unmaximising returns to them.
  `PanelState` restores the sections once and reports the panel as drawn each frame. Changes save
  through `Files::store_settings` a second after they stop, and on exit, where `App::finish` waits
  up to two seconds (`Files::wait_for_jobs`).
