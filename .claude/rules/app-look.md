---
paths:
  - "crates/caditor/src/fonts.rs"
  - "crates/caditor/src/font_fallbacks.rs"
  - "crates/caditor/src/appearance.rs"
  - "crates/caditor/src/icons.rs"
  - "crates/caditor/src/icon_font.rs"
  - "crates/caditor/src/widgets.rs"
  - "crates/caditor/src/dialog_parts.rs"
  - "crates/caditor/src/canvas.rs"
  - "crates/caditor/src/scene_palette.rs"
  - "crates/caditor/src/view_cube.rs"
  - "crates/caditor/src/layout.rs"
  - "crates/caditor/src/menu_bar.rs"
  - "crates/caditor/src/window_frame.rs"
  - "crates/caditor/src/overlay.rs"
  - "crates/caditor/src/toolbar.rs"
  - "crates/caditor/src/sketch_toolbar.rs"
  - "crates/caditor/src/ribbon.rs"
  - "crates/caditor/src/status_bar.rs"
  - "crates/caditor/src/messages.rs"
  - "crates/caditor/src/undo_history.rs"
  - "crates/caditor/src/panels.rs"
  - "crates/caditor/src/feature_tree.rs"
  - "crates/caditor/src/parameter_table.rs"
  - "crates/caditor/src/principal_tree.rs"
  - "crates/caditor/src/bodies_tree.rs"
  - "crates/caditor/src/body_appearance.rs"
  - "crates/caditor/src/tree_row.rs"
  - "crates/caditor/src/conventions_tests.rs"
---

# Look, layout and panels

## Fonts, icons and theme

- Inter (variable, `assets/fonts`) is registered at weights 400, 500 and 600 through its `wght`
  axis as the proportional, `medium` and `semibold` families. Phosphor icons have their own
  `icons` family, since Inter's private-use glyphs would shadow them; `icons.rs` holds one icon
  per command, tool, constraint and feature kind. Fonts install on the first frame, which draws
  nothing.
- Text in other scripts (feature, body, parameter and file names) falls back to fonts installed
  on the system, never bundled ones, which would add tens of megabytes for scripts most people
  never type. `font_fallbacks.rs` looks for them on a thread started when the window opens
  (`FallbackFonts::search`), so startup never waits on it: one file per script from a fixed list
  (`SCRIPTS`: Noto Sans for each script, Noto Sans CJK and its common substitutes, then DejaVu Sans,
  on Linux under the XDG data directories and `~/.fonts`; Segoe UI, Microsoft YaHei, Malgun Gothic,
  Nirmala UI and the like in the Windows and per-user font folders), a `-Regular` name also
  matching its variable font (`Name[wght].ttf`). Each file is read whole, at most `MAX_FONT_BYTES`,
  and kept only when its header is one egui parses (`is_loadable` repeats the checks that would
  otherwise panic inside egui). The fonts found are appended after Inter and egui's own fallbacks
  in every text family (`fonts::definitions_with`) and installed when they arrive; line heights
  stay Inter's, since egui takes metrics from the first font. Finding none, or failing to look,
  leaves the interface as it was. The headless UI tests never search, so they render the same on
  every machine.
- caditor's own icons (fillet, which the sketch fillet shares, chamfer, shell, extrude, revolve and
  both patterns) are glyphs of a font `icon_font.rs` builds in memory at startup from a small
  vector description: strokes, loops, arcs, dots and rings on Phosphor's 256 grid with its 16-unit
  round stroke, outlined into overlapping same-winding TrueType contours (ring holes the other way)
  at Phosphor's 1024 units per em and metrics. The font sits after Phosphor in the `icons` family
  at private-use code points Phosphor leaves free (U+F8E0 onwards), so every consumer keeps taking
  a glyph string and the icons scale, theme and read like the rest. Painted shapes were set aside
  because tool buttons, menus, the palette, tree rows and accessible names all carry icons as text.
  A new icon is a `Mark` list in `ICONS` and a constant; `fonts.rs` tests that each one renders.
- `appearance.rs` holds the theme as `Tokens` for dark, light and both high-contrast variants,
  builds each egui `Style` from them and tests every text pairing, control outline, focus ring
  and button kind against its background in all four. Panels read colours from
  `appearance::tokens(ui)` or the visuals, never fixed values. Design constants (spacing,
  `CONTROL_HEIGHT`, radii, `BORDER_WIDTH`, `FOCUS_WIDTH`) live there too.
- `field_border` meets 3:1 against every surface a field sits on; `danger` fills carry white text
  at 4.5:1 and 3:1 against panels and dialogs. `tokens_for` finds the token set from the visuals'
  panel fill and theme.
- The 3D view keeps a dark canvas in every theme; `canvas.rs` is its chrome and defines its colours
  once (text, hover, selection, error, warning, prompt, snap, ...). `scene.rs` converts them for
  highlighted geometry so a hovered or selected line and its label share one colour. Labels sit on
  the translucent `BACKDROP` (`canvas::label`, `canvas::Label`); the typed-point field uses the
  opaque `PANEL`. A test holds every canvas colour to 4.5:1 (body 7:1) over the backdrop on black,
  white and the hover colour, key caps and canvas controls in every state, and the view cube's
  labels on each cell state.
- Geometry in the 3D view takes its colours and weights from `scene_palette.rs`: `STANDARD`, and
  `HIGH_CONTRAST` while High contrast is on. Its tests hold every high-contrast line and point
  colour to 3:1 against the canvas (`caditor_render::BACKGROUND`) and the dimmed body colour,
  edges and hovered or selected faces to 3:1 on the default body colour (so selected faces are a
  dark blue and hovered ones a dark amber there), and check that the standard palette keeps one
  form for every sketch state. `Highlight::emphasis` widens hovered geometry by one
  `HIGHLIGHT_EXTRA_WIDTH` and selected geometry by the palette's `selection_widening` (1 in
  standard, 2 in high contrast); the high-contrast palette also dashes the edges of a failed or
  outdated body (`scene::body_health`), outside sketch editing.
- `canvas::set_contrast` publishes the contrast to egui temp data each frame (from `app.rs`), and
  `paint_backdrop` draws labels on the opaque `PANEL` in high contrast, where every label colour
  meets 7:1, and on the translucent `BACKDROP` otherwise.
- Key hints (`canvas::Hints`) are `Key: action` text, items three spaces apart, laid out as key
  caps (alternatives joined by " or ") beside a muted action, plain where the part before ": " is
  not keys. Each key, "or" and action is its own text, so tests use `shows_hint`.
- Overlay text keeps clear of the view cube: the prompt and its hints centre in the band left of
  `view_cube::area` (capped width, wrapping), the hover description starts at the band's top left
  and moves under the prompt when they would meet, the grid spacing label (a screen-reader `Label`)
  and cursor readout sit bottom left, the navigation hints bottom right and are left out when
  they would meet the cube or the prompt.
- The view cube is one focusable button named after the hovered target, else the nearest view,
  with that name as tooltip; arrow keys step to the neighbouring target on that side. Fit all (Fit
  selection when something is selected) under it is a canvas control (`canvas::button`).

## Widgets

- `widgets.rs` is the kit every panel and dialog is built from; read it before adding UI
  (`ToolButton` and its compact form, `section`, `properties`, `card`, `callout` and `pill` with a
  `Tone`, the button family, `text_field`, `segmented`, `tabs`, `dialog`/`footer`, menu items).
  Panels and dialogs never add raw egui buttons: `conventions_tests.rs`
  (`buttons_outside_the_widget_kit_come_from_it`) fails on them outside `widgets.rs`.
- Decisions the code does not show:
  - A `ToolButton` is icon above label; `compact` is icon alone, named by its label, in a square
    so two rows match one tool button.
  - `icon_button` is a true `CONTROL_HEIGHT` square so rows reserving that room never overflow and
    widen a panel. `danger_button` is for actions that throw work away and never registers as the
    primary. `strong` is semibold text, since egui's `.strong()` only recolours here.
  - `segmented` falls back to a dropdown when its row is wider than the room, so it never widens
    a panel.
  - `footer_split` puts a destructive action at the far left, away from the primary; the primary
    action is always rightmost.
  - A dialog gives focus to its `primary_button` whenever no widget holds it (opening, Escape
    out of a widget, a click on bare dialog), so Enter runs it while a focused field or button
    keeps Enter for itself. A dialog whose only action would be Close has no footer.
  - `tabs` moves by click, Left/Right (wrapping), Home and End, and with `switch_keys` Ctrl+Tab
    and Ctrl+Page Up/Down; focus follows. `tab_id(row, index)` lets tests focus one.
  - Dialog widths and list heights clamp to the screen (`fitting_width`, `list_height`), so
    nothing clips at 200%.
  - The side panel's scroll area always reserves its scrollbar's room, so the panel keeps its width
    when its content grows past the window's height.
  - Icons and labels are separate text atoms, so tests find buttons by bare label, and compact ones
    by accessible name.
- `dialog_parts.rs` holds what dialogs share beyond the kit: `confirmation` and `confirm_footer`
  (warning callout, destructive action left, primary Cancel right), `undo_note`, `titled_modal`
  (no close button, for waits that cannot be cancelled) and `BodyRoom` (keeps the whole dialog
  within a share of the window).
- Sentence case everywhere: buttons, titles, commands ("Save as…", "Close without saving").

## Screen readers

- Every button drawn with a glyph takes a name without it (`Named`, `named`): icon buttons their
  hover text; small buttons, menu items and section headers their label; tree row buttons the
  action and the feature's name ("Edit <name>"); the row itself a selectable label marked
  selected; a sketch dimension's field its constraint's description; shortcut editor and palette
  rows and the welcome's cards what they do; a tab an AccessKit `Role::Tab` marked selected.
- Decorative glyphs are hidden from the AccessKit tree (`icon_label`, `decorative`), meaningful
  ones described (`described_icon`); a property `caption` labels the field, combo box or slider
  after it in its grid (`tie_to_caption`, via `field::commit_field`).

## Menu bar, ribbon and status bar

- The menu bar (File, Edit, View, Model, Sketch, Help) is built from the commands with their
  icons and shortcuts; items enable from the previous frame's offers (`Workspace::last_offers`)
  with the reason on hover. Then come the model title, the Search commands field and, with
  caditor's title bar, the logo (leading) and the window buttons. Model keeps its creation tools
  at the top level and groups the rest in submenus (Patterns, Datums, Bodies, Features, Rollback,
  Parameters, Recompute).
- Every menu, submenu, context menu and corner menu runs its contents through
  `widgets::fitted_menu`: as tall as the room below its top (at least half the window, which egui
  then moves up), scrolling past that, so no menu runs off the screen at any interface size
  (`every_menu_stays_on_screen_at_the_largest_interface_size`).
- The model title (file icon, name, Unsaved pill, "› sketch" while one is edited) sits centred on
  the window when it fits between menus and search, else as near as it can; clicking it opens the
  model details (path, saved state, Save, Save as…, Version history…, Model properties…, Copy
  file location).
- Search commands looks like a field and opens the palette on click; its keys show as
  `widgets::key_cap`s.
- Both ribbons are built by `ribbon.rs`: whole groups packed into rows by last frame's natural
  widths (`remembered_width`), so widths never depend on the selection or the active tool and
  selecting never moves the 3D view. Captions show only while the ribbon fits one row; a group
  wider than its row wraps inside. Buttons explain themselves on hover with `ribbon::explained`
  (name, what it does with its shortcut, or why it is unavailable).
- The main ribbon (`toolbar.rs`) is `ToolButton`s in the groups of its `Group` enum. While a plane
  is being chosen for a new sketch, New sketch shows selected and clicking it stops choosing (as
  Escape does), so the ribbon keeps its height and every group its place.
- While a sketch is edited the sketch ribbon (`sketch_toolbar.rs`) sits under it on
  `accent_surface` with an accent line along its top: a header without a caption (sketch badge,
  "Editing <name>" truncating with the full name on hover, the sketch's `status_pill`s, its width
  clamped to the title so constraining never moves the groups), Select, Draw, Modify, Constrain
  and Dimension. Finish sketch is a primary button at the right of the first row. Shape tools with
  several ways to draw carry a `corner_menu_button` listing them as `menu_choice`s.
- The three arc tools share one Draw button (`sketch_toolbar::ARC_TOOLS`) showing the arc tool last
  used or chosen; each keeps its own key and command, which the button handles whichever is shown.
  Spline, Ellipse and Elliptical arc share the Curve button (`CURVE_TOOLS`) the same way (a
  `ToolGroup`), so the sketch bar still fits one row at 1400 points. A group's corner menu lists
  its tools, then the ways of the one shown (the spline's), and the group handles the ways'
  commands of all its tools.
- The status bar (`status_bar.rs`) runs recompute | file activity | notice, then right-aligned
  selection | unit | size. Recompute is progress (the feature running, and for how long once past
  two seconds, from `Progress`) with Cancel, Up to date, or a failed
  `status_pill` that is a button focusing the first failed feature; a cancelled or stopped recompute
  is a warning or error pill with a Recompute or Restart button, its text a live region
  (`widgets::announced_status_pill`: polite for a warning, assertive for an error). The unit opens Preferences; the
  interface size, when not 100%, goes back to it.
- Notices: an edit clears info and refused-edit notices, never a `Notice::failure` from saving,
  opening, importing, exporting or the journal. The bar shows only the newest, but
  `Model::set_notice` records recent ones and Help › Recent messages (`messages.rs`) lists them,
  so a failed save outlives the notice that replaced it.
- Edit › Undo history (`undo_history.rs`) lists what Undo and Redo hold (`Editor::undo_steps`,
  `redo_steps`); a click sends that many `Action::Undo` or `Action::Redo`, each an ordinary
  undoable journal step. Hovering a step sums up what it touched (`Transaction::touched`): the
  features by name, how many sketch curves, points, constraints and dimensions, the parameters,
  and the rollback bar, principal geometry, model properties, saved views or selection sets.
- Neither bar clips at large sizes or in narrow windows: the search field, model name, selection,
  unit and size take their own row when last frame's needed width does not fit
  (`widgets::remembered_width`); the selection has a fixed truncating share (all of it on hover);
  a notice too long for the row takes its own and wraps.

## Side panel

- The model panel, Measure, Interference and Analyse faces together take at most `MAX_PANELS_SHARE` (60%) of the
  window, split evenly between those open (`layout::panel_room`, `panel_widths`); each one's
  minimum width yields to that share, so at 200% on a small screen the 3D view keeps a usable
  width. A model panel narrowed by the share keeps the width the user chose for when there is room.
- Features and Parameters are collapsible `widgets::panel_section`s that open by themselves when
  a rename or focus request needs something inside: a header band (card fill and border, the count
  as a pill) and, when open, a muted sentence saying what the section holds
  (`FEATURES_EXPLANATION`, `PARAMETERS_EXPLANATION`), also the header's hover. Nested sections
  (sketch cards, Measure, Preferences) stay plain `widgets::section`s. Empty, each is an
  `empty_state` offering the first step (New sketch, Open a sample, Add parameter).
- The rollback bar always carries a caption: End of model, or how many features it rolls back.
- The parameter table's name and expression fields share the panel's width beside a fixed value
  column (`VALUE_WIDTH`), since content wider than the panel widens it the next frame. Hovering a
  value shows it in full and what uses it (`used_by`, only for the hovered row, since
  `Document::parameter_users` scans every feature). Whether a row is in use comes from
  `Document::used_parameters`, one scan cached per model revision (`ParameterUses`), which also
  puts a muted `icons::UNUSED` before the value of a parameter nothing refers to. Deleting an
  unused parameter removes it; deleting a used one (the row's button, its menu, Delete parameter)
  is `Document::inline_parameter`, built only on hover or click, followed by a notice saying how
  many uses took its expression. A parameter with a note shows `icons::NOTE` before its value,
  described with the note. Right-clicking a row's name or expression opens its menu (Move up,
  Move down, Add or Edit the note…, Delete); the same act on the focused row from the palette and
  the Model menu (`MoveParameterUp`, `MoveParameterDown`, `ParameterNote`, `DeleteParameter`).
  A used parameter's menu also lists under Used by the parameters and features using it
  (`Document::parameter_user_ids`, at most `MAX_LISTED_USERS`, then how many more); choosing one
  focuses that parameter's value or chooses and reveals that feature in the tree. The
  note is edited in a modal dialog (`parameter_table::note_dialog`, `PanelState::noting`). A value that cannot be evaluated is an error
  icon with the reason on hover, a refused edit an `error_row`. A row's delete button shows only
  while the row is hovered or the button has keyboard focus, and stays in the Tab order.
- Model parameters (those with an owner) are a table of their own below the others, under
  `MODEL_PARAMETERS` (its hover says how a value is named), each row followed by a muted line
  naming its owner (`Document::owner_text`, else that it was deleted). Their rows edit, note,
  move and delete like any other; Move up and down stay within the row's table. Deleting a used
  one inlines it, which writes the value back into its field.
- Tree rows (`tree_row.rs`, shared with `principal_tree.rs`) have fixed trailing slots that stay
  empty when a row has nothing for them, so icons line up in columns; callouts and cards under a
  row (`tree_row::indented`) keep the normal spacing.
- A plain feature row (collapsed, no callout, not edited, renamed, revealed, focused or wanted for
  focus; `feature_tree::is_plain`) more than a row beyond the visible part of the panel only
  reserves the height last measured for one (`PanelState::plain_row_height`), so a tree of
  hundreds of features lays out only what is near view. The row next to each edge is laid out, so
  Tab and the keyboard still reach the next one. A sketch card's constraint and dimension rows
  do the same (`is_plain_constraint`: not redundant, not wanted for focus, and for a dimension
  its field neither focused nor holding a draft, looked up once per card with `field::busy`; the last heights measured
  are `PanelState::plain_constraint_height` and `plain_dimension_height`), a run of skipped
  rows reserving its space in one allocation (`feature_tree::Reserved`), so only rows near view
  format a description or a value.
- Kind icons are tinted by category (sketches accent, bodies and modifiers text, datums muted;
  inactive or hidden rows muted). A failure shows once, as the status icon and its callout, never
  by recolouring the icon or name; an outdated row the same with a Recompute button. A row a
  running recompute has not reached yet (`Evaluation::is_pending`) shows the waiting icon over its
  last status. A suppressed
  row is struck through and muted, a rolled-back row muted; neither offers the edit button. A
  failure caused by a suppressed feature offers Unsuppress (`FixTarget::Unsuppress`). A failure
  with a `place` offers Show where, which frames the view around it (`PanelState::shown_place`,
  `ViewportState::show_place`). Callout actions are `small_button`s with icons.
- A feature computed with a healed reference (`FeatureStatus::healing`) shows the warning icon and
  a callout with the reason, the remedy and an Update references button, offered only while the
  evaluation was checked against the feature as it now stands (`Healing::update`).
- A row opened for editing (a sketch entered, a feature opened) collapses again when editing it
  ends, by the checkmark, Enter, Escape or Finish (`PanelState::opened_for_editing`,
  `finished_editing`), so the tree goes back to one line per feature.
- Whatever is selected in the view marks the rows of the features it belongs to as selected, a
  face, edge or vertex its body's feature row and Bodies row, a sketch curve, region or
  constraint its sketch, a datum its own (`Pickable::owner`, `PanelState::follow_view_selection`
  on each change of the selection's generation, `in_view`), and the first newly marked row is
  scrolled into view. The mark is only a look (`reads_selected`): it never chooses the row, so
  the tree's commands and Delete still act on chosen rows only.
- The other way round, the rows chosen in the tree (`PanelState::chosen`, handed to the viewport
  each frame by `Viewport::show_chosen_rows`) highlight what they made in the view as hovered
  geometry is: the faces and edges of a body, the curves of a sketch, a datum
  (`Highlight::chosen_rows`, part of the scene cache's highlight key), never vertices or regions.
  It is only a look: the view's selection stays as it was, so commands keep taking the tree's
  choice, and nothing is highlighted while a sketch or feature is open for editing. Fit view
  (F) with nothing selected in the view frames the chosen rows' bodies, sketches and datums
  (`BuiltScene::bounds_of_features`) and everything when none is chosen.
- A click on a row's name only selects it (`PanelState::selected`, cleared when the view selection
  changes); Ctrl+click toggles and Shift+click extends (`PanelState::chosen`, primary first); the
  chevron alone shows the details. A double-click, Enter, the edit button or the menu's Edit opens
  the feature (a sketch for editing, another feature as its panel, an imported body by showing
  its details); a suppressed or rolled-back one leaves a notice with the reason instead.
- Rename (F2) and Move up or down act on the primary row, else the open feature
  (`feature_tree::current_feature`); Suppress and Delete act on every chosen row; Delete selection
  deletes them outside sketch editing.
- Principal group rows: hover highlights in the view, a click or tab selects in the view (clearing
  the tree's feature selection), and a row reads as selected while its pickable is.
- The Bodies group (`bodies_tree.rs`, above the features, closed at first, absent without a body)
  lists the bodies standing at the end of the model, by their own name or else the name of the
  feature that made each. A click or tab chooses that feature in the tree and the eye hides or
  shows it like the feature's own. A body a Combine or Remove consumed is not listed. Right-clicking
  a row offers Rename body… (the card below with its Body name field focused), Select the whole
  body (its faces in the view, through `PanelState::selected_in_tree`) and Remove body, which adds
  a `Remove` feature at the bar with a notice saying how to bring the body back; the same act from
  the palette and the Model menu on the tree's body, else the selection's (`RenameBody`,
  `RemoveBody`; `app-modelling.md`, "What a command acts on").
- A body row's paint bucket (or the Body colour and material command, for the body chosen in the
  tree, else of the selection, else of the open feature) opens its colour and material card under the row
  (`PanelState::painting`, one at a time, focus on the first swatch until it lands):
  `widgets::swatch`es (default and `body_appearance::SWATCHES`) above a property grid of Colour
  (a hex field), Material (`MATERIALS` presets, which set the density and, for a body without a
  colour, their colour; None clears both), Name and Density (g/cm³, an expression), under a Body
  name field (empty: named after its feature) and above them Opacity (`OPACITIES`: Solid, 75%,
  50%, 25%, a combo box; a see-through body is drawn in the translucent pass like X-ray,
  its faces and edges picked as usual). With faces of the body selected in the view, a second row
  of swatches (named "<colour> for the selected faces", the first the body's own colour, which
  removes theirs) colours those faces (`with_face_colours`), and Clear face colours drops them
  all, keeping a repainted face's own opacity; the scene draws a face's colour, and a face's own
  opacity (from STEP import) by splitting the body between the opaque and translucent passes
  outside the analysis and reflection views, unless the body is failed or outdated. Each change is
  one undoable `SetBodyAppearance`; a refused value is an `error_row`.
- The rollback bar is a row of its own (`feature_tree::rollback_bar`, named "Rollback bar" for
  screen readers), at the end of the tree when nothing is rolled back. It and every row name drag
  (`PanelState::dragging`): the gap under the pointer shows an accent line, or an error one with a
  reason when `Document::move_row` refuses it; release applies, Escape or a release outside the
  tree cancels. Dragging one of several chosen rows moves them all (`Document::move_features`,
  "Move N features").
- A filter field (`PanelState::tree_filter`) heads the feature rows once there are
  `FILTER_FROM_FEATURES` of them, or while it holds text or focus; Filter the feature tree (Ctrl+F,
  Model menu, palette) shows and focuses it at any size. It keeps the features whose name, or one
  of whose kind words (`feature_tree::kind_words`: "extrusion", "fillet", "cut", "datum plane"…),
  contains the text in any case, plus the edited, renamed, revealed or focused one, so going to a feature
  never lands on a hidden row. While it filters, the rollback bar is hidden and rows do not drag,
  since gaps between the shown rows are not the model's; with no match an empty state offers
  Clear the filter.
- Consecutive features of one group (`document.md`) sit under a folder row
  (`feature_groups.rs`): chevron, folder icon, name, member count and a "⋯" menu (Rename group,
  Ungroup), members indented by `GROUP_INDENT`. A click or tab chooses every member; a
  double-click renames in place (blank refused); the chevron folds it, a folded folder's members
  taking zero-height places at its foot so drops still map to tree gaps; a folder opens by itself
  when a member is revealed or wanted for focus. Group features (Ctrl+G, a row's menu, Model ›
  Features, palette) groups the chosen rows as "Group N" and starts renaming it; Ungroup and
  Rename group act on the current feature's folder. While the tree filters, folders are not
  drawn, and a group's name matches the filter like a feature's.
- Deleting features others depend on opens the delete dialog (`feature_tree::delete_dialog`,
  counted as a modal): the dependents in tree order with what each uses, Delete with dependents
  (`danger_button`, leftmost), Keep dependents and Cancel (rightmost). Cancel takes focus whenever
  focus is outside the dialog, so Enter never deletes. Either deletion is one transaction;
  without dependents Delete acts at once and the menu item reads "Delete…" only when it will ask.
- A sketch's card shows its entity count and collapsible Dimensions and Constraints sections;
  constraint rows highlight their entities on hover and, clicked, edit the sketch and select the
  constraint (`PanelState::hovered_in_tree`, `chosen_in_tree`, handed to the viewport after the
  panels are drawn).

## Window and panel persistence

- `layout.rs`: `Preferences` holds a `WindowPlacement` (logical size, maximised, and the outer
  position where the platform reports one, which Wayland never does) and a `PanelLayout` (side
  width, open sections) under `window.*` and `panels.*`, read clamped to sane bounds. Startup fits
  the size to the largest monitor and uses a position only when it lies on one; size and position
  are recorded only while not maximised, so unmaximising returns to them.
- Changes save through `Files::store_settings` shortly after they stop and on exit (`App::finish`
  waits for the jobs). An unreadable preferences file leaves the defaults in use and opens a
  notice at startup; a failed save becomes a notice and the next change writes everything it
  held.

## Title bar and window frame

- `Preferences::title_bar` (`built_in` by default, or `system`) chooses caditor's title bar (an
  undecorated window whose menu bar acts as the title bar) or the window manager's, applied at
  once through `window_frame::apply_title_bar` and offered in Preferences and the window menu.
- `window_frame.rs` works through egui `ViewportCommand`s, which `Overlay::run` hands to
  `egui_winit`; `Overlay` feeds maximized, full-screen and focus state back as the root
  `ViewportInfo`.
- With caditor's title bar, the menu bar's empty space and the model title drag the window, a
  double-click maximizes or restores, a right-click opens the window menu. The window buttons
  close through Quit so unsaved work is asked about; in full screen Leave full screen replaces the
  first two. While a dialog is open, `window_frame::over_dialogs` redraws the drag area and
  buttons above the modal so the window can still be moved, maximized or closed.
- Close takes clicks from its button up to the top and right edges of the window
  (`reaching_the_corner`), so a maximized window closes from the corner of the screen; a framed
  window's resize strips still win along its edge. A gap of `SPACE_S` on each side of the separator
  keeps the window buttons apart from the search field.
- When neither maximized nor full screen, a square outline marks the edge (the window is opaque,
  so rounding would leave corners showing) and thin foreground strips along it, with larger
  corners resizing diagonally, set the resize cursor and start `BeginResize`.
- The compositor takes the pointer during a move or resize and the release never reaches the
  window, so `Overlay` queues a synthetic primary release after `StartDrag` or `BeginResize`. On
  Windows both run the system's modal move and size loops, so dragging to an edge snaps; the
  undecorated window keeps a drop shadow (`with_undecorated_shadow`).
- The window's minimum size is `layout::MIN_WINDOW_WIDTH` by `MIN_WINDOW_HEIGHT`.
