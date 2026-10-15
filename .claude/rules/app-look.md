---
paths:
  - "crates/caditor/src/fonts.rs"
  - "crates/caditor/src/font_fallbacks.rs"
  - "crates/caditor/src/appearance.rs"
  - "crates/caditor/src/themes.rs"
  - "crates/caditor/src/files/themes.rs"
  - "crates/caditor/src/icons.rs"
  - "crates/caditor/src/icon_font.rs"
  - "crates/caditor/src/widgets.rs"
  - "crates/caditor/src/dialog_parts.rs"
  - "crates/caditor/src/canvas.rs"
  - "crates/caditor/src/start_card.rs"
  - "crates/caditor/src/scene_palette.rs"
  - "crates/caditor/src/view_cube.rs"
  - "crates/caditor/src/layout.rs"
  - "crates/caditor/src/menu_bar.rs"
  - "crates/caditor/src/view_menu.rs"
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
  - "crates/caditor/src/feature_clipboard.rs"
  - "crates/caditor/src/pasting.rs"
  - "crates/caditor/src/parameter_table.rs"
  - "crates/caditor/src/principal_tree.rs"
  - "crates/caditor/src/bodies_tree.rs"
  - "crates/caditor/src/body_appearance.rs"
  - "crates/caditor/src/colour_selector.rs"
  - "crates/caditor/src/tree_row.rs"
  - "crates/caditor/src/conventions_tests.rs"
  - "crates/caditor/src/feature_groups.rs"
  - "crates/caditor/src/upload_badge.rs"
---

# Look, layout and panels

## Fonts, icons and theme

- Inter (variable, `assets/fonts`) is registered at weights 400, 500 and 600 through its `wght` axis
  as the proportional, `medium` and `semibold` families. Its italic face is embedded only for SVG
  text import (`file-import-export.md`); the interface has no italic. Phosphor icons have their own
  `icons` family, since Inter's private-use glyphs would shadow them; `icons.rs` holds one icon per
  command, tool, constraint and feature kind. Fonts install on the first frame, which draws nothing.
- Text in other scripts falls back to fonts installed on the system, never bundled ones.
  `font_fallbacks.rs` searches on a thread started when the window opens (`FallbackFonts::search`,
  one file per script from `SCRIPTS`; Noto, DejaVu on Linux, Segoe UI and friends on Windows). Each
  file is read whole (at most `MAX_FONT_BYTES`), leaked as `'static` so egui borrows instead of
  copying at every `set_fonts`, and kept only when `is_loadable` passes (it repeats the checks that
  would panic inside egui). Found fonts are appended after Inter and egui's fallbacks in every text
  family (`fonts::definitions_with`); line heights stay Inter's. Headless UI tests never search.
- caditor's own icons (fillet, which the sketch fillet shares, chamfer, shell, extrude, revolve, both
  patterns) are glyphs of a font `icon_font.rs` builds in memory at startup from a vector
  description on Phosphor's 256 grid and 16-unit stroke, 1024 units per em. It sits after Phosphor
  in the `icons` family at private-use code points from U+F8E0, so every consumer keeps taking a
  glyph string (buttons, menus, the palette, tree rows and accessible names all carry icons as
  text). A new icon is a `Mark` list in `ICONS` and a constant; `fonts.rs` tests that each renders.
- Themes are data. `appearance.rs` holds the `Tokens` sets (`DARK`, `LIGHT`, `MIDNIGHT`, `GRAPHITE`,
  `PAPER`, `SLATE`, `STUDIO` and two high-contrast sets), each colour a `Token` whose `key` a theme
  file uses, and builds each egui `Style` from a `Skin` (tokens, lightness, contrast, the 3D view's
  `Canvas`, the chosen grid colour). Contrast rules are one table run at runtime (`appearance::check`,
  `required_pairs`): 4.5:1 for text, 7:1 for body text in high contrast, 3:1 for outlines,
  `field_border`, `focus` and `danger`, `text_on_accent` 4.5:1 on every accent and danger fill. A
  failure is a `ContrastFailure` naming the pair in words. Tests run it on every shipped set.
- The design scale lives in `appearance.rs`, and a size anywhere else is a step of it, never a bare
  number: spacing `SPACE_XS` 2, `SPACE_S` 4, `SPACE_SM` 6 (the rows' vertical rhythm), `SPACE_M` 8,
  `SPACE_L` 12, `SPACE_XL` 16 (`appearance::margin` makes an egui `Margin` of two steps); type
  `SMALL_SIZE` 11.5, `LABEL_SIZE` 12.5 (section titles, monospace, the 3D view's key hints),
  `BODY_SIZE` 13.5, `TITLE_SIZE` 15 (titles in the 3D view, side panel headers), `HEADING_SIZE` 17, `PAGE_TITLE_SIZE`
  20 (a guide page's title); radii `SMALL_RADIUS` (key caps), `WIDGET_RADIUS`, `CARD_RADIUS`,
  `WINDOW_RADIUS`; `CONTROL_HEIGHT`, `CONTROL_MIN_WIDTH`, `BORDER_WIDTH`, `FOCUS_WIDTH` and
  `FOCUS_GAP`. The egui `Style` is built from named values only (`ITEM_SPACING`, `BUTTON_PADDING`,
  `TOOLTIP_WIDTH`, `MENU_WIDTH`, `COMBO_WIDTH`, …). `TOOLTIP_DELAY` is the Standard hover help
  delay; `app::apply_appearance` replaces it in every installed style with the Hover help
  preference (`TooltipDelay::seconds`) whenever the styles or the preference change.
- `app::apply_appearance` resolves the preference (`themes::Appearance::resolve`) into `Skins`, one
  per egui theme (Follow the system fills dark and light; a named theme both, and sets egui's
  preference to its lightness), installs the styles and publishes the skins in egui temp data
  (`appearance::publish`), where `appearance::tokens(ui)`, `tokens_of(ctx)`, `skin` and
  `is_high_contrast` find them by the visuals' `dark_mode`; without one (tests setting a style
  directly) they fall back to the built-in set recognised by its panel fill. Panels read colours
  from these or the visuals, never fixed values.
- `themes.rs` holds `Theme` (System, the shipped ones, `User(key)`), `Accent` (the theme's own, or a
  hue from which `with_accent` derives the accent family by stepping each colour until its pairs
  stay readable; offered on a theme only when the derived set passes `check`), `ViewChoice` (with
  the theme, or one `Canvas`) and `GridColour` (automatic, or a custom `Rgb`). High contrast
  replaces the theme's colours and accent with the high-contrast set of its lightness and keeps the
  view and grid choices.
- User themes are JSON files in the config directory's `themes` folder (`caditor_file::read_themes`,
  at most `MAX_THEME_FILES` of `MAX_THEME_FILE` bytes): `name`, `base` (dark or light, filling what
  the file leaves out), `view` (a `Canvas::key`), `grid` (`#rrggbb`, optional) and `colours` by token
  key as `#rrggbb` (`#rrggbbaa` only for the shadow). `themes::sort` builds and checks each on the
  files worker (`Files::list_themes`: startup, Preferences opening, Reload themes) into
  `UserThemes`; an unreadable file, unknown colour, base or view, bad colour, repeated name, failed
  contrast pair or grid its view hides (`ThemeProblem::UnseenGrid`) is a `Refusal` shown in
  Preferences in words. A chosen user theme that is missing or refused once listed keeps the last
  look applied (`Workspace::usable_look`) and posts one warning (`Unusable::words`); before the
  first listing it waits silently on the default.
- The theme picker (`preferences.rs`, Appearance › Theme) is a wrapping grid of `widgets::theme_card`s
  (a `ThemeSample` miniature drawn with the current accent, contrast and view; each card a button
  named by the theme and toggled when chosen, `theme_card_id`); choosing applies at once, so the
  window is the live preview. Accents are `widgets::swatch`es named "Accent: …", disabled with the
  reason when high contrast is on or the accent would not stay readable. The 3D view is a row of
  swatches named "3D view: …" (`preferences::view_name`); the Grid row has automatic
  (`AUTOMATIC_GRID`, filled with `scene_palette::shown_grid`), the custom colour
  (`custom_grid_name`) and `CUSTOM_GRID`, which opens `colour_selector.rs` (`GRID_SELECTOR`). A
  colour the selector applies goes through `scene_palette::seen_grid`: below `Contrast::grid_seen_from`
  (`GRID_SEEN` 1.5:1; `HIGH_CONTRAST_GRID_SEEN` 3:1) on the canvas in use it is refused in an error
  callout (`UnseenGrid::sentence`) and the preference stays. A custom colour the canvas now hides
  (view or contrast changed) is not drawn: `Appearance::skin_of` resolves the grid (custom colour,
  else the theme file's, each only while seen, else the palette's) and a warning callout says so
  (`Appearance::unseen_grid`, `UnseenGrid::drawn_instead`).
- The Bodies row of the same section sets the colour of bodies without one of their own
  (`themes::BodyColour`, `appearance.body`: `default` or `#rrggbb`, reset with the Colours
  section): the default swatch (`DEFAULT_BODY`), the fixed `SWATCHES` named "Body colour: …"
  (`body_swatch_name`), the custom colour and `CUSTOM_BODY`, which opens the colour selector
  (`BODY_SELECTOR`). Every choice goes through `scene_palette::seen_body`: the colour as drawn
  (`ScenePalette::body`, so lightened in high contrast) below `BODY_SEEN` (1.5:1) on the canvas in
  use is refused in an error callout (`UnseenBody::sentence`), a swatch that would be is disabled
  with that sentence on hover, and a chosen colour the canvas now hides is drawn as the default
  with a warning callout (`Appearance::unseen_body`, `UnseenBody::drawn_instead`). The default
  colour holds on every canvas in both contrasts
  (`the_default_body_colour_is_seen_on_every_canvas_and_one_too_close_is_refused_in_words`); the
  body card's Default colour swatch and the theme cards show the colour in use.
- Six canvases (`Canvas::ALL`: Dark, Grey, Studio grey, Blue-grey, Light, White) give the 3D view its
  background; `Canvas::is_dark` (Dark and Grey) picks `DARK_CHROME` for labels, the others
  `LIGHT_CHROME`. The renderer clears to a solid colour, so a gradient would need a pass in
  `caditor-render`. The canvas follows the resolved skin (`Skin::canvas`): `apply_appearance`
  publishes it per frame (`canvas::set_canvas`), the viewport takes it (`ViewportState::set_canvas`,
  part of the scene cache's revisions) and the session hands `Canvas::background` to the renderer
  after each frame's actions (`Renderer::set_background`). The grid colour travels the same way
  (`canvas::set_grid`, `ViewportState::set_grid`); `SceneCache::set_grid` recolours the cached
  `Scene::grid` in place without a rebuild.
- `canvas.rs` is the canvas chrome: a `Chrome` of every colour (backdrop, panel, text, hover,
  selection, error, warning, prompt, snap, key caps, controls, view cube), `DARK_CHROME` and
  `LIGHT_CHROME`, read through `canvas::chrome(ctx)`. Its sizes are `appearance`'s scale (text and
  icon sizes, `WIDGET_RADIUS` for labels, controls and hints, `SMALL_RADIUS` for key caps, gaps and
  padding on the spacing steps), so a label in the view matches the panels' text; `HINT_SIZE` is the
  `LABEL_SIZE` step. Labels sit on the translucent backdrop
  (`canvas::label`, `canvas::Label`); the typed-point field uses the opaque panel; in high contrast
  `paint_backdrop` uses the opaque panel too (`canvas::set_contrast` publishes the contrast each
  frame from `app.rs`; `Chrome::backdrop`). Tests hold every chrome colour to 4.5:1 (body 7:1) over
  the backdrop on black, white and hover, 7:1 on the opaque panel, and key caps, canvas controls and
  the view cube's labels in every state.
- Geometry in the 3D view takes colours and weights from `scene_palette.rs`, one palette per contrast
  and canvas (`Contrast::palette(canvas)`: `STANDARD`, `HIGH_CONTRAST`, `LIGHT_STANDARD`,
  `LIGHT_HIGH_CONTRAST`; the other canvases' palettes are struct updates of the one of their
  lightness: `GREY_*` lightens reds and bands, `STUDIO_*` and `BLUE_GREY_*` darken the redundant
  orange, comb teeth and hovered selection, `WHITE_*` change only the hole). A hollow point's `hole`
  is the canvas; the halo of the centre of mass, comb and isocurves is `outline` (near-black on dark
  canvases, black on light ones, which no light colour could hold to 3:1 on a body). The light
  canvas's scene highlights are a little lighter than its chrome label colours (7:1 on the white
  label panel) so those halos can still frame them.
- Palette tests: on every canvas the standard sketch states but Background, highlights, origin,
  preview and axes hold 3:1 and the grid `GRID_SEEN`; every high-contrast line, point, marker, band
  and comb colour holds 3:1 on the canvas (`Canvas::colour`) and the dimmed body colour, edges,
  threads and hovered or selected faces 3:1 on every body as drawn. That goes through
  `ScenePalette::body`, which in high contrast (`lightens_bodies`) draws a body or face colour
  lightened toward white by the fewest of `LIGHTENING_STEPS` after which those marks hold
  `MARKS_ON_A_BODY` (`marks_stand_out_on`), so a black body keeps its hue as a light tint; the
  default colour is drawn as it is (`scene::body_color` and `face_looks` take it). The standard
  palettes keep one form for every sketch state. A render `Color` becomes an egui colour only through
  `scene_palette::color32` (canvases, axis colours, analysis swatches, the palette tests), a
  document `Rgb` only through `body_appearance::color32`.
- The axis triad (`view_cube::show_axis_triad`) draws in the palette's `axes` with each letter on the
  label backdrop, every letter over every line (the letter of an axis pointing away sits on the
  origin, where the other two lines meet); axis colours hold 4.5:1 (7:1 in high contrast) over it on black, white and hover,
  which is why the standard dark red and blue are lighter and the light ones darker than a plain
  hue. `Highlight::emphasis` widens hovered geometry by `HIGHLIGHT_EXTRA_WIDTH` and selected by the
  palette's `selection_widening` (1 standard, 2 high contrast); the high-contrast palette dashes the
  edges of a failed or outdated body (`scene::body_health`), outside sketch editing.
- Key hints (`canvas::Hints`) lay out a `canvas::HintList` (`app-input.md`, Guidance in the 3D
  view) as key caps (alternatives joined by " or ") beside a muted action, plain for a note, on a
  bordered panel filled with the chrome's opaque `panel` colour on every canvas and contrast, never
  the translucent label backdrop, so a dimension label beneath cannot show through
  (`the_key_hint_panel_is_opaque_on_every_canvas_so_no_label_shows_through`); a folded list ends in
  the More keys chip, a bordered chip resting on that panel colour (control-coloured when hovered or
  pressed, with the focus ring). Each key, "or" and action is its own text, so tests use `shows_hint`.
- The display picker (`display_picker.rs`, `canvas::menu_button`: the style's icon, its
  `DisplayStyle::short_title` and a caret) sits at the top of the 3D view in the corner across from
  a top-left cube, else at the top left, named "Display style: <title>" for screen readers. A click
  opens the style menu as the ribbon's group menus do (`ribbon::group_menu` from
  `ViewportState::show_menu`, so a choice runs next frame like the context menu's): every display
  style checked as `ToggleStates` says, then Light the rendered view. The prompt band
  (`top_band`) and the pointer labels keep clear of it as of the cube (`display_area`).
- Overlay text keeps clear of the view cube wherever it sits (`ViewportState::cube_area`, the
  `CubePlacement`'s area): a failure or interference label covering the cube or its buttons moves
  to the side of them away from the view's edge (`viewport::clear_of`); the prompt and its hints
  centre in the band beside a top cube, or across the top under a bottom one (`top_band`), its key
  hints left out when they would reach the bottom strip of triad and readouts (at least
  `view_cube::TRIAD_WIDTH`, the cube's height when it sits at the bottom); the axis triad and
  readouts start right of a bottom-left cube and the navigation hints end left of a bottom-right one
  (`bottom_strip_left`, `bottom_strip_right`); labels beside the pointer, the grid spacing label (a
  screen-reader `Label`), cursor readout and navigation hints avoid each other as `app-input.md`
  says (`the_view_cube_takes_any_size_and_corner_with_the_overlays_clear_of_it_and_is_remembered`
  checks every size and corner).
- The start card (`start_card.rs`, behaviour in `app-input.md`) is a panel, not canvas chrome:
  the dialog frame (`widgets::dialog_frame`: opaque window fill, border, shadow, `DIALOG_MARGIN`)
  holding the kit's widgets, so the theme's checked tokens hold whatever the canvas. A heading
  with a close icon button, a muted sentence laid out by `widgets::balanced_label` (the narrowest
  width keeping its line count, so no word sits alone on the last line), Start with a sketch as a
  full-width `primary_icon_button`, the other two equal `sized_icon_button`s side by side (stacked
  when a label would not fit half the width), and Don't show this again as a `widgets::checkbox`.
  `start_card::placement`: `WIDTH` wide, centred in the view `TOP_MARGIN` below its top when that
  clears the view cube's area (`ViewportState::cube_area`, wherever the cube sits); else as near
  the centre as the band beside a top cube allows while it is at least `MIN_WIDTH`; else under the
  cube, centred, and without the sentence. A bottom cube shortens its area while the card, as tall
  as it was drawn the frame before (`StartCard::drawn_height`), ends above it, and otherwise is
  treated as a top cube is, its fallback being the top without the sentence
  (`the_card_keeps_clear_of_the_view_cube_in_every_corner_and_size`,
  `the_card_moves_clear_of_the_view_cube_at_every_size_and_corner`; at 200% a large cube in a bottom
  corner leaves the view too little room and may meet the card), so at 200% it fits the view
  (`the_card_fits_in_the_view_clear_of_the_view_cube_at_every_interface_size`). Its child `Ui`
  senses clicks and drags over the card, so a click on its background never reaches the view.
- The view cube is one focusable button named after the hovered target, else the nearest view, with
  that name as tooltip followed by the bound keys of a standard view's direction
  (`CubeTexts::view_keys`; Isometric only while the model keeps no redefined Isometric view); arrow
  keys step to the neighbouring target. A primary drag orbits like a right-drag
  (`CubeAction::Orbit`). Fit all (Fit selection when something is selected) under it is a canvas
  control (`canvas::button`) and left of it `canvas::icon_button` "Isometric view" goes to
  Isometric as Alt+0 does (right of it in a left corner); `CubePlacement::area` covers both.
  Preferences › Appearance › In the 3D view can hide the cube with both buttons
  (`ViewDetails::view_cube`, `ViewportState::view_cube_shown`), leaving their commands; the
  overlays then keep clear of an empty corner (`ViewportState::cube_area`). The axis triad stays.
  The same section sizes it (`view.view_cube_size`: `CubeSize` small 80, standard 104, large 136
  points; Fit all keeps the standard width) and puts it in any corner (`view.view_cube_corner`:
  `Corner`), both in `ViewDetails::cube`, read leniently; the default is the standard size at the
  top right, laid out exactly as before.

## Widgets

- `widgets.rs` is the kit every panel and dialog is built from; read it before adding UI
  (`ToolButton`, `section`, `properties`, `card`, `callout` and `pill` with a `Tone`, the button
  family, `checkbox`, `unlabelled_checkbox` and `unlabelled_radio` (no visible text, named for
  screen readers with their role), `combo`, `text_field`, `segmented`, `tabs`, `dialog`,
  `dialog_body` and `footer`, `side_panel`, menu items, `error_row` and `error_text`). Panels and
  dialogs never add raw egui buttons, checkboxes, radio buttons or combo boxes:
  `conventions_tests.rs` (`buttons_outside_the_widget_kit_come_from_it`) fails on them outside
  `widgets.rs`, a call split over lines (`ui` then `.button(…)` on the next) included.
- Surfaces step `sunken` (fields, check boxes) < `panel` < `raised` (cards, section bands, dialogs,
  menus); every shipped set but high contrast holds `raised` 1.1:1 off its `panel`
  (`every_shipped_set_outside_high_contrast_lifts_cards_off_the_panel_by_fill_alone`). A `card` stands one step off what it
  sits on: `raised` on a panel, `panel` on a dialog or popup (any layer above `Order::Background`).
  Outside high contrast cards and section bands separate by fill alone; strong outlines stay on
  what takes input (fields, check boxes and radio buttons through `field_border`, hovered buttons
  through `border_strong`) and on focus. High contrast, whose surfaces are one colour, outlines
  cards, bands, selected tools and the segmented thumb (`surface_edge`).
- Accent fills mark only the primary action, the current selection (selected rows, the active
  tool's `accent_subtle` tint) and focus; `accent_surface` (the sketch ribbon, an open tree row) is
  a faint tint. A segmented control's choice is a thumb (`raised` on light, `pressed` on dark, ringed
  in `border_strong`) with full text beside muted others; the selected tab is full text with an
  accent glyph and the accent underline, unselected tabs muted; `danger_button` is outlined in
  `danger` with `error` text at rest and fills on hover and press, so it never competes with the
  primary. Secondary text (captions, counts, unselected choices) is `text_muted`.
- Decisions the code does not show:
  - A `ToolButton` is icon above label; `compact` is icon alone, named by its label, in a square so
    two rows match one tool button. `icon_button` and `framed_icon_button` are true
    `CONTROL_HEIGHT` squares (`square_icon_padding`) so rows and columns reserving that room never
    overflow, as Look at sketch's compact column beside Finish sketch does
    (`the_buttons_beside_finish_sketch_keep_a_gap_from_it_at_every_interface_size`).
    `danger_button` is for actions that throw work away and never registers as the primary.
    `strong` is semibold, since egui's `.strong()` only recolours here.
  - `segmented` falls back to a dropdown when wider than the room, so it never widens a panel.
    Every `widgets::combo` (that dropdown and `feature_fields::combo` included) truncates its chosen
    text rather than wrap it, is `COMBO_WIDTH` wide unless given a width, ties itself to the property
    caption before it (`tie_to_caption`) and can take its hover through `Combo::hover`. Its list is
    as tall as its entries up to `ListHeight::Tall` (`list_height`, so never over the window's share)
    and scrolls beyond that, rather than egui's 200 points that cut a long list off.
  - Progress is one widget, `widgets::progress(label)`: a spinner, the label (a polite live region,
    the spinner hidden), an optional `count(shown, total)` in muted text, and a Cancel button only
    when the caller passes `cancel(hover)`; `show` returns whether Cancel was clicked. `wrapping`
    lets it wrap in a panel or dialog; the status bar keeps it on one line. Every wait in the
    interface uses it (recompute, files, export, paste, tidy, isocurves, interference, analysis,
    mass properties, bodies being prepared for display, measuring, previews), so a wait without
    Cancel is a worker with no cancel token (`docs/PERFORMANCE.md`). A cancelled tidy, isocurve or
    interference worker remembers the basis it was stopped on (`Tidying::stop`,
    `IsocurveTool::stop`, `Interference::stop`) and does not restart until the inputs change or the
    user asks again, saying so in an info callout. Cancelled face analysis, mass properties and
    body preparing keep what they stopped as stopped (`Analyses::stop`, `BodyMesh::stop_mass`,
    `BodyMeshing::stop`) until the user asks again with the button beside the info callout or pill
    that says so (Analyse again, Work them out, Show the rest). Each of these Cancels, and each of
    those resuming buttons (Check again included), is also a command (`Command::CancelAnalysis`,
    `CancelMassProperties`, `CancelPreparingBodies`, `CancelUploading`, `CancelIsocurves`,
    `CancelInterference`; `AnalyseAgain`, `WorkOutMassProperties`, `ShowRestOfBodies`,
    `ContinueUploading`, `CheckInterferenceAgain`), invoked
    by the panel or status bar that draws the wait, so the button and the command share one path
    (`app-input.md`).
  - One focus ring: `paint_focus_ring` in the panels' focus colour, `paint_focus_ring_in` with a
    colour and radius of the caller's; the 3D view's controls and the view cube pass the chrome's
    `focus`, which the canvas tests hold over the backdrop.
  - An inline error is the failure icon and the error colour, never colour alone: `error_row` as a
    property grid row, `error_text` anywhere else (feature tree fields, the 3D view's dimension
    fields).
  - `small_button` marks its label, not its icon, as the part that shrinks when the button is wider
    than a whole row of its container: in a narrower non-wrapping column the label wraps; in a
    wrapping row the button moves to the next row whole. `label_before_icon_buttons` wraps a value
    leaving room for the icon buttons after it; `slider` narrows the track to the room left beside
    its value (`MIN_SLIDER_WIDTH`).
  - A `properties` grid wraps values and captions once they would leave the value column narrower
    than `FIELD_WIDTH`, never below `CAPTION_WIDTH`; in a grid narrower than a caption and a field
    the caption column is as wide as its widest caption. A field's value preview wraps to the next
    line rather than being squeezed. `panel_header` puts its actions on their own row once title and
    actions no longer fit one row (`remembered_width`).
  - `pill` and `status_pill` measure their text and reserve that room before drawing, so a wrapped
    row moves one that does not fit to the next line instead of widening the side panel, which would
    shift the 3D view whenever a sketch's status changes
    (`a_pill_that_does_not_fit_beside_another_wraps_instead_of_widening_the_panel`).
  - `glyph_pill` and `count_pill` take a `PillRole`: `Shown` reads its text, `Named` hides its text
    from screen readers behind a name, `Button` is a button named by what it does (outlined at half
    colour, fully on hover, thicker while pressed, focus ring), so a pill that acts never looks like
    one that reads.
  - `footer_split` puts a destructive action at the far left; the primary action is always rightmost.
    A dialog gives focus to its `primary_button` whenever no widget holds it (opening, Escape out of
    a widget, a click on bare dialog), so Enter runs it while a focused field or button keeps Enter.
    A dialog whose only action would be Close has no footer: its title bar's close button (named
    `CLOSE_DIALOG`) and Escape are enough (the load report, recent messages, undo history, saved
    views, selection sets, version history).
  - `tabs` moves by click, Left/Right (wrapping), Home and End, and with `switch_keys` Ctrl+Tab and
    Ctrl+Page Up/Down; focus follows. `tab_id(row, index)` lets tests focus one.
  - Dialog widths and list heights clamp to the screen (`fitting_width`, `list_height`), so nothing
    clips at 200%. A list's height is a `ListHeight` step: `Short` 160 (import layers, export
    bodies, replaced files), `Medium` 240 (saved views, selection sets, delete dependents, the open
    report), `Tall` 360 (palette, messages, undo history, version history, shortcuts, parameter and
    configuration files). `widgets::dialog` centres below the window buttons
    (`window_frame::controls_bottom`), which are drawn over dialogs, and lays out what its closure
    adds under the title without scrolling it: the dialog's scrolling part is its
    `widgets::dialog_body` (or `DialogBody` with an id, a least height or a `ListHeight` cap), as
    tall as the window leaves once the title, what sits above the body and what came after it the
    frame before are counted (`arrange_dialog`, kept per dialog; a change redoes the pass). So the
    footer, a confirmation or an undo note after the body always stay on screen and the body scrolls
    under them. A dialog whose content is a list makes the list its body (the load report, replaced
    files, parameter and configuration imports, messages, undo history, delete dependents,
    shortcuts); every other dialog wraps everything above the footer.
  - `widgets::waiting_dialog` is the same frame without a close button, Escape doing nothing, for
    a wait nothing can cancel ("Closing…" while the storage worker finishes). `search_dialog` is the
    frame with no title, anchored at the top (`TOP_DIALOG_OFFSET`, below the window buttons), for
    the command palette.
  - Side panels scroll through `panel_scroll`, which always reserves its scrollbar's room, so a panel
    keeps its width when content grows past the window's height.
  - Icons and labels are separate text atoms, so tests find buttons by bare label, compact ones by
    accessible name.
  - Animated widgets repaint on a timer: `widgets::spinner` asks for the next frame after
    `SPINNER_FRAME` (about 15 a second), since egui's own `Spinner` repaints every frame
    (`conventions_tests.rs` refuses the raw one). The text caret is solid (`text_cursor.blink` off
    in `appearance::visuals`), so a focused field never wakes an idle app.
- `dialog_parts.rs` holds what dialogs share beyond the kit: `confirmation` and `confirm_footer`
  (warning callout, destructive action left, primary Cancel right) and `undo_note`.
- Sentence case everywhere: buttons, titles, commands ("Save as…", "Close without saving").

## Screen readers

- Every button drawn with a glyph takes a name without it (`Named`, `named`): icon buttons their
  hover text; small buttons, menu items and section headers their label; tree row buttons the action
  and the feature's name ("Edit <name>"); the row itself a selectable label marked selected; a
  sketch dimension's field its constraint's description; shortcut editor and palette rows and the
  welcome's cards what they do; a tab an AccessKit `Role::Tab` marked selected.
- Decorative glyphs are hidden from the AccessKit tree (`icon_label`, `decorative`), meaningful ones
  described (`described_icon`); a property `caption` labels the field, combo box or slider after it
  in its grid (`tie_to_caption`, via `field::commit_field`).

## Menu bar, ribbon and status bar

- The menu bar, the main ribbon and the status bar share one frame (`ribbon::bar_frame`,
  `ribbon::BAR_MARGIN`); the sketch ribbon adds its accent line to the top margin.
- The menu bar (File, Edit, View, Model, Sketch, Help) is built from the commands with their icons
  and shortcuts; every menu, File's included (`files::menu`, `templates::menu`), draws its command
  entries through `menu_bar::MenuEntries` (`entry`, `hinted`, `choice_with` for a hover hint or a
  check the `ToggleStates` do not hold, `availability_or` for a command not offered every frame),
  enabled from the previous frame's offers (`Workspace::last_offers`) with the reason on hover.
  Menus keep few entries: related commands sit in submenus (`menu_bar::submenu`), e.g. File's
  New from template, Open recent, Open sample, Versions and details, Import and Export, Edit's
  Select, View's Projection and snapping, Selection tools, Show and hide, Inspect and Window and
  interface, Model's Modify shape and Sketch's Drawing tools, Modify geometry, Select geometry and
  Constrain automatically. Then come the model title, a Preferences icon button (the gear; no menu
  entry) and the Search commands field (opens the palette on click; keys as `widgets::key_cap`s)
  and, with caditor's title bar, the logo (leading) and window buttons. Model keeps its creation
  tools (New sketch, Extrude, Revolve, Sweep, Hole, Thread, Primitives) at the top level and groups
  the rest in submenus (Patterns, Datums, Bodies, Features, Rollback, Parameters, Recompute).
- The model title (file icon, name, Unsaved pill, "› sketch" while one is edited) sits centred on the
  window when it fits between menus and search, else as near as it can; clicking opens the model
  details (path, saved state, Save, Save as…, Version history…, Model properties…, Copy file
  location).
- Every menu, submenu, context menu and corner menu runs through `widgets::fitted_menu`: as tall as
  the room below its top (at least half the window), scrolling past that
  (`every_menu_stays_on_screen_at_the_largest_interface_size`). The 3D view's context menu, which
  egui would flip above its spot, measures itself (`widgets::measured_menu`) and opens downward from
  a spot moved to fit (`menu_opening_down`). Menus of commands draw entries through
  `menu_bar::MenuEntries` (availability from the offers, the reason on hover of a disabled entry,
  bound keys, checkmarks from `ToggleStates`), shared by the menu bar and that context menu
  (`app-input.md`), an egui `Popup` of `PopupKind::Menu` at the pointer so its submenus
  (`menu_bar::submenu`) work as the menu bar's do.
- Both ribbons are built by `ribbon.rs`: whole groups packed into rows by last frame's natural widths
  (`remembered_width`), so widths never depend on the selection or active tool and selecting never
  moves the 3D view. Groups keep one rhythm on both: `DIVIDER_SPACE` with a divider between groups,
  `TOOL_GAP` between buttons, `ROW_GAP` between rows, captions `CAPTION_GAP` under their buttons,
  shown only while the ribbon fits one labelled row. A labelled group's natural width counts its
  caption whether shown or not (`ribbon::captioned`), so hiding captions never narrows the groups
  into fitting one row again and the layout never flips between frames
  (`the_ribbons_settle_on_one_layout_at_every_interface_size`). Each ribbon turns compact while its labelled
  rows would take more than `MOST_LABELLED_SHARE` of the window's height
  (`ribbon::labelled_too_tall`), judged from labelled widths remembered apart from compact ones (a
  second ribbon name). Compact buttons are icon-only `ToolButton`s named by their label for screen
  readers. Buttons explain themselves on hover with `ribbon::explained` (name, what it does with its
  shortcut, or why it is unavailable).
- The main ribbon (`toolbar.rs`) is `ToolButton`s in the groups of its `Group` enum: History, Sketch,
  Solid (Extrude, Revolve, Sweep, Hole), Modify, Pattern, Reference, Analyse (Measure,
  Interference); compact through `toolbar::tool_button`. While a plane is being chosen for a new
  sketch, New sketch shows selected and clicking it stops choosing (as Escape does), so the ribbon
  keeps its height.
- Every captioned group of both ribbons opens a menu of all its commands, buttonless ones included,
  through one implementation in `ribbon.rs`: `ribbon::menu_group` draws the group with its trigger,
  `ribbon::group_menu` shows the menu and `ribbon::menu_sections` its paragraphs split by separators.
  The trigger is the group's caption while captions show (`widgets::group_title`: the caption and a
  caret, a strip as wide as the group, drawn in the caption's place by `ribbon::captioned_by`), else
  a corner mark over one tool the group names (`ribbon::GroupTools::host`): the caret
  (`widgets::corner_menu_button`) on a full button, the notch (`compact_corner_menu_button`) on a
  compact one. The title with its caret (`widgets::group_title_width`) counts in the group's natural
  width like the caption alone did; that widens only a group narrower than it (the sketch ribbon's
  History column), so the main ribbon's widths and every layout's packing stay as
  they were (`the_ribbons_settle_on_one_layout_at_every_interface_size`, `interface_fit`,
  `large_interface`). Both triggers are buttons whose screen-reader name is the menu's and open an
  egui `Popup` of `PopupKind::Menu` through `widgets::fitted_menu`, so Escape or a click outside
  closes it and it scrolls when taller than the room. Group menus hold no submenus: scrolling one
  with the wheel would pass a submenu's button under the pointer, open it and send the wheel to it.
  Entries are `menu_bar::MenuEntries` from the last frame's offers, keymap and `ToggleStates`
  (disabled with the reason on hover, bound keys, checkmarks through `MenuEntries::marked`, Undo and
  Redo named for the step and Repeat for the command through `MenuEntries::history_item`); opening
  it by click, Enter or Space focuses the first available entry. A chosen command closes the menu
  and is returned by `toolbar::show` or `sketch_toolbar::show`; `app::show` runs it next frame
  through `Workspace::deferred_commands` as the context menu's are, so the same `Command` path as
  the button, palette and keys decides what happens (`ui_tests/group_menus.rs`).
- The main ribbon's menus are named "<group> commands" (`Group::menu_name`) and listed by
  `Group::sections` (Thread, the primitives, Offset face, Delete face, Split face, Copy body, Mirror faces,
  Split, Mates, Scale model, the face analyses, the comb, isocurves, Add a section plane, centres of
  mass without buttons); their trigger's host is the whole row of tools, so the corner sits over the
  last tool. `ToolbarContext::analysing`, `combing`, `isocurving` and the measure and interference
  flags mark the open panels. Analyse is where the analysis and inspection commands live: Measure,
  Check interference, the six face analyses, comb, isocurves, section view and centres of mass; no
  command of another group belongs to it, and it keeps two buttons because the ribbon already fills
  a 1400-point window, so another button would wrap it into two rows and move the view of every
  test.
- The sketch ribbon's menus are named "Sketch <group> commands" (`sketch_toolbar::group_menu_name`,
  apart from the main ribbon's History and Modify) and listed by `sketch_toolbar::menu_sections`:
  History (Undo, Redo, Repeat, Undo history; Cancel sketch), Select (the Select tool; Select all,
  what is still free, the open ends, the redundant constraints), Draw (every drawing tool and Spur
  gear; each shape's ways, one paragraph a shape; the drawing aids; typed dimensions, drawing
  symmetrically, slicing the bodies), Modify (Construction and every modify and projecting tool;
  Offset's and Blend curve's ways; Split, Break, Respace; Move, Rotate, Scale; Copy, Cut, Paste,
  Delete; Project every curve of another sketch, Intersect the whole body), Constrain (every geometric constraint, Curvature included; Add
  the relations the drawing shows, Check the sketch for flaws) and Dimension (Smart dimension and
  every dimension; Disable, Dimension fully from a datum point; Scale the whole sketch on its first
  dimension). The active tool, Construction while drawing construction and each shape's current way
  (`ShapeModes::of`) are marked; choosing the active tool again is dropped, so the menu never steps
  its way. Corner hosts: Redo, Select, Line (`DRAW_MENU_HOST`, the last Draw button without a corner
  menu of its own), Delete, the last constraint button and Disable.
- While a sketch is edited the sketch ribbon (`sketch_toolbar.rs`) sits under it on `accent_surface`
  with an accent line along its top: a header without a caption (sketch badge, "Editing <name>"
  truncating with the full name on hover, the sketch's counts as `count_pill`s at the right and its
  status pill under it, two lines whatever the status, width clamped to the title and at least the
  widest status so constraining never moves the groups or the view; `app-sketching.md`, Status
  pills), then History (compact Undo over Redo, named for the step as the main ribbon's are, sending
  `Action::Undo` and `Redo` without invoking the commands the main ribbon invokes), Select, Draw,
  Modify, Constrain and Dimension, each with its group menu. Compact (`Bar::compact`, at 200% in the screenshot window) the
  header drops its badge, Select and the Draw buttons go icon-only and Draw folds into
  `COMPACT_DRAW_ROWS` rows, so the bar packs into two rows; every button keeps its accessible name.
  Finish sketch is a primary button at the right of the first row, `CONTROL_HEIGHT` tall and
  centred on the whole bar, captions included. Left of it Cancel sketch, a compact `ToolButton` (a
  cross, named `sketch_toolbar::CANCEL_LABEL`, disabled with the reason when the session cannot be
  cancelled), sits over Look at sketch, a framed `framed_icon_button`, the pair one compact column
  (`Beside::Stacked`) so the bar keeps one row at 1400 points; on a row of its own (the header
  leaving Finish no room beside the groups) the three sit side by side (`Beside::InRow`).
- Corner menus: shape tools with several ways to draw carry a `widgets::corner_menu_button` (a caret
  at the top right, `compact_corner_menu_button` on a compact ribbon) listing them as
  `menu_choice`s; the compact Offset, Mirror and Project buttons a `compact_corner_menu_button` (a
  notch at the bottom right) listing the tools without a button of their own, then the host's ways
  when it has some (Offset's). Both marks are drawn in the text colour (accent while one of their
  tools is active), and each takes a `widgets::CornerMenu` whose hover names the menu and lists
  what is inside (`Partners::contents`, `sketch_toolbar::menu_contents`).
- Several tools share one Draw button showing the one last used or chosen, each keeping its own key
  and command, which the button handles whichever is shown: the three arcs (`ARC_TOOLS`), Spline,
  Ellipse, Elliptical arc and Conic (`CURVE_TOOLS`, a `ToolGroup`), Polygon and Spur gear
  (`POLYGON_TOOLS`, labelled Polygon whichever is shown), so the sketch bar fits one row at 1400
  points. A group's corner menu lists its tools, then the ways of the one shown.
- While the renderer is still uploading (`Renderer::is_uploading`, set each frame on
  `ViewportState::set_uploading` before the interface runs), the status bar's file
  activity ends with a `widgets::progress` of `upload_badge::TEXT` (`StatusContext::uploading`),
  the only place it is shown, with a Cancel (`Action::StopUploading`). The spinner shows only in
  frames the uploads already ask for, so the last upload frame is followed by one with neither and
  no repaint request. Cancelled, the bodies not on the graphics card yet stay hidden and the same
  place shows an info pill `upload_badge::STOPPED` (`StatusContext::upload_stopped`, from
  `Renderer::is_upload_stopped`) with Continue loading (`Action::ResumeUploading`), both also
  commands (below); `ViewportState::request_upload` holds the `UploadRequest` that `app.rs` hands
  the renderer after `perform` (`ui_tests/upload_indicator.rs`). Saving, the file dialog wait and
  the closing waits are not cancellable by design.
- The status bar (`status_bar.rs`) runs recompute | file activity | notice, then right-aligned
  selection | its size or pair reading | selection filter (while not Everything) | unit | interface
  size (while not 100%); the filter, unit and interface size are `small_button`s with an icon.
  Recompute is progress (the feature running, and how long once past two
  seconds, from `Progress`) with Cancel, Up to date, or a failed `status_pill` that is a button
  focusing the next failed feature after the tree's primary row (`status_bar::failed_after`,
  wrapping, as Go to the next failed feature, F8, does; Shift+F8 the previous). A feature waiting
  for something to be chosen (`FeatureState::Waiting`) is counted by an info pill beside it, "N
  waiting", the same button focusing the next waiting feature (`status_bar::waiting_after`; Go to
  the next feature waiting for input, F9, does the same, wrapping, and Shift+F9 the previous), and
  Up to date shows only when nothing has failed or waits; a cancelled or
  stopped recompute is a warning or error pill with a Recompute or Restart button, its text a live
  region (`widgets::announced_status_pill`: polite for a warning, assertive for an error). Those
  hovers name their command's keys (`status_bar::Hovers`, `CommandFrame::with_keys`). The unit opens
  Preferences; the interface size, when not 100%, goes back to it. A notice too wide for the row
  wraps onto a row of its own; the bar is a bottom panel sized by the frame before, so a change of
  its height discards the pass (`widgets::remember_height`) and the new row shows in the same frame,
  never drawn below the window for a frame or until the next repaint.
- Notices have a `NoticeKind` drawn through its `Tone` (`NoticeKind::tone`): `Notice::info` for what
  simply happened or found nothing, `Notice::success` for a completed operation the user asked for,
  `Notice::warning` for a refusal, something not done or done in part, `Notice::error` or
  `Notice::failure` for errors. Only an error's text takes its colour; the notice is a live region,
  assertive only for an error. An edit clears every notice not made to outlast edits, never a
  `Notice::failure` from saving, opening, importing, exporting or the journal. The bar shows only
  the newest, with a Recent messages button (`status_bar::SHOW_MESSAGES`) beside Dismiss;
  `Model::set_notice` records recent ones and Help › Recent messages (`messages.rs`) lists them,
  each icon described by `NoticeKind::label`.
- Edit › Undo and Redo name the step they would take ("Undo Extrude 1 distance", from
  `Model::undo_label` and `redo_label`, as the ribbon's hover does) while available.
- Edit › Undo history (`undo_history.rs`) lists what Undo and Redo hold (`Editor::undo_steps`,
  `redo_steps`); a click sends that many `Action::Undo` or `Action::Redo`, each an ordinary undoable
  journal step. The current step is a label beside Now, not a button; a last row, Before these
  changes (`undo_history::BEFORE`), undoes every step listed. Hovering a step sums up what it touched
  (`Transaction::touched`).
- Neither bar clips at large sizes or in narrow windows: the search field, model name, selection,
  unit and size take their own row when last frame's needed width does not fit
  (`widgets::remembered_width`); the selection has a fixed truncating share (all of it on hover); a
  notice too long for the row takes its own and wraps.

## Side panel

- The model panel, Measure, Interference, Analyse faces, the curvature comb, Constrain automatically,
  Spur gear and the user guide (`app.md`) together take at most `MAX_PANELS_SHARE` (60%) of the
  window, split evenly between those open (`layout::panel_room`, `panel_widths`); each one's minimum
  width yields to that share. A model panel narrowed by the share keeps the width the user chose for
  when there is room.
- The right-hand panels are each a `layout::RightPanel` (an id; all open `RIGHT_WIDTH` 320 wide and
  narrow to `MIN_RIGHT_WIDTH` 240 at least), drawn through `widgets::side_panel`: a `PanelTitle`
  (icon, title, Close's name, the guide page its help button opens; the guide has none) as the
  header with Close, help and the panel's own actions, then the contents in `panel_scroll`.
  `side_panel_parts` leaves the area under the header to the panel, for Measure's Relative to, the
  guide's search and Constrain automatically's task above the scroll, and for the primary action
  that Constrain automatically and Spur gear keep in a `panel_footer`. `RightPanel::show` lays the
  contents out in a child bounded to the panel
  (`layout::within_width`): egui clamps an overflowing right-hand panel's rect by keeping its far
  edge, so wider content would shift the rect off the window and reserve less than the panel paints.
  They share one width the user last gave one of them (`PanelLayout::right_width`, `None` until
  then): `app.rs` hands it over through egui temp data (`layout::share_right_width`) and reads it
  back. A width differing from the one egui stored the frame before (`PanelState`) counts as a
  resize, unless the share caps it, and becomes the shared width.
- Features and Parameters are collapsible `widgets::panel_section`s that open by themselves when a
  rename or focus request needs something inside: a header band (the count as a pill) and, when
  open, nothing but its content: what the section holds is the header's hover only
  (`FEATURES_EXPLANATION`, `PARAMETERS_EXPLANATION`). Nested sections (sketch cards, Measure,
  Preferences) stay plain `widgets::section`s. Empty, each is an `empty_state` offering the first
  step (New sketch, Open a sample, Add parameter). The rollback bar always carries a caption: End of
  model, or how many features it rolls back.

### Parameters

- The parameter table's name and expression fields share the panel's width beside a value column
  that is as wide as its widest value and icons up to `VALUE_WIDTH`, since wider content widens the
  panel the next frame. `Content::measure` lays out every name, expression and value once per
  frame (all parameters, not only the filtered ones, so a filter never moves a column) and
  `FieldWidths::fitting` gives the name its longest name's width whenever that exceeds its
  `NAME_SHARE`, capped at `LONGEST_NAME_SHARE` of the room and so that the expression keeps its
  widest text down to `LEAST_EXPRESSION_WIDTH`. A name longer than that ends in an ellipsis painted
  over the field's end (`ellipsize_clipped_name`) with its full text on hover, until the field is
  edited
  (`parameter_names_of_normal_length_show_whole_from_75_to_200_percent`,
  `a_parameter_name_longer_than_its_column_ends_in_an_ellipsis_and_shows_whole_on_hover`). Hovering a value shows
  it in full and what uses it (`used_by`, only for the hovered row, since
  `Document::parameter_users` scans every feature). Whether a row is in use comes from
  `Document::used_parameters`, one scan cached per model revision (`ParameterUses`; it counts a
  feature's appearance too, a body's density), which also puts a muted `icons::UNUSED` before the
  value of an unreferenced parameter. A parameter with a note shows `icons::NOTE`, described with
  the note; the note is edited in a modal (`parameter_table::note_dialog`, `PanelState::noting`). A
  value that cannot be evaluated is an error icon with the reason on hover, a refused edit an
  `error_row`. A row's delete button shows only while the row is hovered or has keyboard focus, and
  stays in the Tab order.
- Deleting an unused parameter removes it; deleting a used one is `Document::inline_parameter`, built
  only on hover or click, followed by a notice saying how many uses took its expression. The row
  menu (right-click on name or expression) has Move up, Move down, Add or Edit the note…, Delete,
  also from the palette and Model menu on the focused row (`MoveParameterUp`, `MoveParameterDown`,
  `ParameterNote`, `DeleteParameter`), and for a used parameter a Used by list of the parameters,
  features and configurations using it (`Document::parameter_user_ids`, at most `MAX_LISTED_USERS`,
  then how many more); choosing one focuses that parameter's value, chooses and reveals that
  feature, or opens Configurations (`PreferencesCommand::ShowConfigurations`).
- Model parameters (those with an owner) are a table of their own below the others, under
  `MODEL_PARAMETERS` (its hover says how a value is named), each row followed by a line naming its
  owner (`Document::owner_text`), a `widgets::truncated_link` that chooses and reveals the owning
  feature and focuses it, or the owning dimension's field (`Focus::Feature`, `Focus::Dimension`;
  `parameter_table::go_to_owner`), else a muted line saying it was deleted. Their rows edit, note,
  move (within the row's table) and delete like any other; deleting a used one inlines it, writing
  the value back into its field.
- A filter field (`PanelState::parameter_filter`) heads both tables once there are
  `FILTER_FROM_PARAMETERS` parameters, or while it holds text or focus, keeping rows whose name,
  expression text or owner text contains it in any case, plus a row wanted for focus or holding it;
  with no match an empty state offers Clear the filter.
- Delete N unused parameters (`Command::DeleteUnusedParameters`, the button under the tables while
  there are any, Model › Parameters, palette; `parameter_table::UnusedDeletion`) removes in one
  transaction every parameter `ParameterUses` finds unreferenced, owned ones included, with a notice
  naming them; unavailable with the reason when there are none. A parameter used only by one of them
  stays, as it is still in use when the change is built. The transaction is checked first (`perform`
  takes the document), and a refusal is the warning notice instead of the success one.

### Feature tree

- Tree rows (`tree_row.rs`, shared with `principal_tree.rs`) have fixed trailing slots that stay
  empty when a row has nothing for them, so icons line up in columns (the open feature's row alone
  adds one, its Cancel cross left of the checkmark, `app-modelling.md`); callouts and cards under a
  row (`tree_row::indented`) keep the normal spacing.
- Rows far out of view only reserve the height last measured for one, so a tree of hundreds of
  features, or thousands of imported bodies, lays out only what is near view; the row next to each
  edge is laid out so Tab still reaches the next one. A plain feature row (collapsed, no callout,
  not edited, renamed, revealed, focused or wanted for focus; `feature_tree::is_plain`) uses
  `PanelState::plain_row_height`; the Bodies group (`bodies_tree::off_screen_height`: no colour card
  open, not focused) `plain_body_row_height`; a sketch card's constraint and dimension rows
  (`feature_tree::card_row`) `plain_constraint_height` and `plain_dimension_height`. `CardRow::Plain`
  is a row not redundant, not wanted for focus, and for a dimension with its field neither focused
  nor holding a draft (looked up once per card with `field::busy`); a redundant row
  (`CardRow::Varied`, its callout makes its height its own) reserves the height last measured for
  that constraint (`PanelState::varied_row_heights`, forgotten when the revision, evaluation or card
  width changes); a row wanted for focus or with a busy field (`CardRow::Live`) is always laid out.
  A run of skipped rows reserves its space in one allocation (`feature_tree::Reserved`), so only
  rows near view format a description or a value.
- Revealing a row or its edited card (`scroll_into_view`) scrolls it in by the least distance, except
  a card taller than the visible panel, which aligns its top so the status and count pills above its
  sections are never scrolled away.
- Kind icons are tinted by category (sketches accent, bodies and modifiers text, datums muted;
  inactive or hidden rows muted). A failure shows once, as the status icon and its callout, never by
  recolouring the icon or name; an outdated row the same with a Recompute button. A row a running
  recompute has not reached (`Evaluation::is_pending`) shows the waiting icon over its last status.
  A feature waiting for an input (`FeatureState::Waiting`) shows `icons::AWAITING_INPUT` in the info
  tone and an info callout, never the failure icon (`app-modelling.md`).
  A suppressed row is struck through and muted, a rolled-back row muted; neither offers the edit
  button, and the card under either is drawn disabled below a muted line giving `editable`'s reason,
  with Unsuppress or Roll forward to here (`feature_tree::inactive_note`). A failure caused by a
  suppressed feature offers Unsuppress (`FixTarget::Unsuppress`); one whose fix is the failing
  feature itself (`FixTarget::Feature` of its own row) offers Edit <name> (`edit_itself`), hidden
  while it is open. A failure with a `place` offers Show where, which frames the view around it
  (`PanelState::shown_place`, `ViewportState::show_place`). A row asked to take focus
  (`Focus::Feature`: F8, the failed pill, the palette) scrolls so its callouts show with it. Callout
  actions are `small_button`s with icons.
- A feature computed with a healed reference (`FeatureStatus::healing`) shows the warning icon and a
  callout with the reason, the remedy and an Update references button, offered only while the
  evaluation was checked against the feature as it now stands (`Healing::update`) and is of the
  current revision with the feature not pending (`Model::current_healing`), so an undo or edit
  meanwhile never writes a reference captured on another model; the row menu and palette take the
  same healing, refusing with `HEALING_AWAITS_RECOMPUTE` until the recompute reports.
- A row opened for editing (a sketch entered, a feature opened) collapses again when editing ends, by
  the checkmark, Enter, Escape or Finish (`PanelState::opened_for_editing`, `finished_editing`).
- Whatever is selected in the view marks the rows of the features it belongs to as selected: a face
  the feature row that made it (`viewport::tree_row_of`, the double-click's `feature_of`: a hole's
  bore, a fillet's or chamfer's surface, a shell's wall or a pattern's copy marks that feature, not
  the extrusion whose body it changed) and its body's Bodies row, an edge or vertex its body's feature
  row and Bodies row, a sketch curve, region or constraint its sketch, a datum its own
  (`Pickable::owner`, `PanelState::follow_view_selection` on each change of the selection's generation
  or the evaluation, `in_view` for feature rows, `bodies_in_view` for Bodies rows); the first newly
  marked feature row is scrolled into view. The mark is only a look (`reads_selected`,
  `reads_body_selected`): it never chooses the row, so the tree's commands and Delete act on chosen
  rows only.
- The other way round, the rows chosen in the tree (`PanelState::chosen`, handed to the viewport each
  frame by `Viewport::show_chosen_rows`) highlight what they made as hovered geometry is: faces and
  edges of a body, curves of a sketch, a datum (`Highlight::chosen_rows`, part of the scene cache's
  highlight key), never vertices or regions. A row of a feature that changes a body it did not make
  (fillet, shell, hole, cut extrusion) lights only the faces whose `FaceOrigin` names it on that
  body's shown result and the edges bounding them (`bodies::made_by`, joined to the hovered items),
  so two holes of one body light apart; one that made no face there (combine, move, remove) lights
  the whole body (`viewport::LitRows`, kept until the rows, revision or evaluation change). Fit view
  still frames the whole body of such a row: `viewport::with_their_bodies` adds each row's
  `Feature::body`, as `visibility::hideable_rows` does.
  The view's selection stays as it was, and nothing is highlighted while a sketch or feature is open
  for editing. A feature or body row under the pointer (`PanelState::hovered_row`,
  `ViewportState::hover_row`) is lit the same way while hovered, without counting as chosen.
- Escape with the view's selection already empty and nothing open, or a click on empty space in the
  view with nothing selected (`ViewportState::take_tree_dismissal`), lets go of the chosen rows
  (`PanelState::choose_nothing`), so Move, Pattern or Measure never act on a row left chosen unseen.
  Whenever the document's revision changes, `PanelState::forget_missing` drops chosen rows the
  document no longer holds. Fit view (F) with nothing selected in the view frames the chosen rows'
  bodies (a modifying feature's the body it changes), sketches and datums
  (`BuiltScene::bounds_of_features`), and everything when none is chosen.
- A click on a row's name only selects it (`PanelState::selected`, cleared when the view selection
  changes); Ctrl+click toggles and Shift+click extends (`PanelState::chosen`, primary first) over the
  rows drawn this frame, so a filtered tree or folded folder never adds rows it hides
  (`feature_tree::choose_among`, resolved after layout); the chevron alone shows the details. A
  double-click, Enter, the edit button or the menu's Edit opens the feature (a sketch for editing,
  another feature as its panel, an imported body by showing its details); a suppressed or
  rolled-back one leaves a notice with the reason instead. With Preferences › General › Feature
  tree set to rename (`PanelState::double_click`, `DoubleClick::Rename`) a double-click on the
  name starts renaming instead, Enter and the rest still opening.
- Rename (F2) and Move up or down act on the primary row, else the open feature
  (`feature_tree::current_feature`); Suppress, Hide or show feature and Delete act on every chosen
  row; Delete selection deletes them outside sketch editing. The row menu's Hide or Show
  (`visibility::toggle_rows`) acts on every chosen row when the row itself can be hidden. The menu
  also has Copy features (asked of the command next frame through `PanelState::requested`, choosing
  the row first when it was not chosen), Update references, and Uses and Used by submenus listing the
  features the row's kind depends on (`FeatureKind::dependencies`) and those depending on it
  directly, in tree order, at most `MAX_LISTED_RELATIONS` then how many more, disabled with the
  reason when empty; choosing one chooses and reveals its row (`Focus::Feature`).
- Copy features and Paste features (`feature_clipboard.rs`, Ctrl+C and Ctrl+V outside a sketch, the
  Model menu and palette) copy every chosen row in tree order to the system clipboard as text
  (`file-format.md`) and paste the clipboard's features as one change at the rollback bar
  (`Document::paste_features`, `document.md`), choosing and revealing the copies. A notice names each
  feature left out with the reason (a pick not found again on the copies, or a feature not in this
  model) and how many values became numbers; foreign or damaged text, or sketch geometry, is
  refused in words, and Paste features inside a sketch says to finish it.
- When a copied feature picks faces, edges or corners of another copied one (`copied_picks`), the
  paste goes to `Action::Paste` and `Model` runs `Document::paste_features_finding_picks` on a
  copy of the document (`pasting.rs`, `Pastes`, `PasteTarget`) as a task on the recompute worker's
  cache (`Recomputer::submit_on_cache`, `document-recompute.md`), so only the copies are computed,
  with the threads shown as cosmetic when the model is; without a worker it runs on a `paste`
  thread from an empty cache. The status bar's activity shows a spinner, "Pasting N features:
  finding picked faces and edges, i of n…" and Cancel (`Command::CancelPaste`, palette too;
  "Stopped pasting. Nothing was added."). Paste features is refused while one runs. A result for
  an older revision, or a search a newer model submission stopped (`Outcome::Interrupted`), is
  planned again on the current model; a finished paste is applied as one change with its notice,
  and the tree chooses and reveals the copies through `Model::pasted` (`PanelState::paste_shown`).
  A panic in the search pastes nothing and says so. A model switch cancels it.
- Principal group rows: hover highlights in the view, a click or tab selects in the view (clearing
  the tree's feature selection), and a row reads as selected while its pickable is. Rows and eyes
  read what the view shows (`PrincipalView::shows`), so geometry tucked away while not needed reads
  hidden, yet its row still selects it, which shows it (`app-modelling.md`, Visibility).
- The rollback bar is a row of its own (`feature_tree::rollback_bar`, named "Rollback bar"), at the
  end of the tree when nothing is rolled back. It and every row name drag (`PanelState::dragging`):
  the gap under the pointer shows an accent line, or an error one with a reason when
  `Document::move_row` refuses it; release applies, Escape or a release outside the tree cancels.
  Dragging one of several chosen rows moves them all (`Document::move_features`, "Move N features").
- A filter field (`PanelState::tree_filter`) heads the feature rows once there are
  `FILTER_FROM_FEATURES` of them, or while it holds text or focus; Filter the feature tree (Ctrl+F,
  Model menu, palette) shows and focuses it at any size. It keeps the features whose name, kind
  words (`feature_tree::kind_words`) or state words (`state_words`: "failed", "outdated",
  "suppressed", "hidden") contain the text in any case, plus the edited, renamed, revealed or focused
  one, so going to a feature never lands on a hidden row. While it filters, the rollback bar is
  hidden, rows do not drag (gaps between shown rows are not the model's) and folders are not drawn;
  with no match an empty state offers Clear the filter.
- Consecutive features of one group (`document.md`) sit under a folder row (`feature_groups.rs`):
  chevron, folder icon, name, member count and a "⋯" menu (Rename group, Ungroup), members indented by
  `GROUP_INDENT`. A click or tab chooses every member; a double-click renames in place (blank
  refused); the chevron folds it, a folded folder's members taking zero-height places at its foot so
  drops still map to tree gaps; a folder opens by itself when a member is revealed or wanted for
  focus. Group features (Ctrl+G, a row's menu, Model › Features, palette) groups the chosen rows as
  "Group N" and starts renaming it; Ungroup and Rename group act on the current feature's folder. A
  group's name matches the filter like a feature's.
- Deleting features others depend on opens the delete dialog (`feature_tree::delete_dialog`, counted
  as a modal): the dependents in tree order with what each uses, Delete with dependents
  (`danger_button`, leftmost), Keep dependents and Cancel (rightmost). Cancel takes focus whenever
  focus is outside the dialog, so Enter never deletes. Either deletion is one transaction; without
  dependents Delete acts at once and the menu item reads "Delete…" only when it will ask.
- A sketch's card shows its entity count and collapsible Dimensions and Constraints sections;
  constraint rows highlight their entities on hover and, clicked, edit the sketch and select the
  constraint (`PanelState::hovered_in_tree`, `chosen_in_tree`, handed to the viewport after the
  panels are drawn).

### Bodies

- The Bodies group (`bodies_tree.rs`, above the features, closed at first, absent without a body)
  lists the bodies standing at the end of the model, by their own name or else the name of the
  feature that made each; a body a Combine or Remove consumed is not listed. A click or tab chooses
  that feature (Ctrl+click toggles and Shift+click extends over the listed bodies, through
  `feature_tree::choose_among`) and the eye hides or shows it like the feature's own.
  A separating feature lists each piece as a row of its own (`Document::body_name`: its own name,
  else "<feature> (<curve>)"), whose eye toggles only that piece (`visibility::toggle_body`, which
  also shows the feature when it was hidden) and whose card edits only its look; choosing a piece row
  chooses that body for the body commands (`body_selection::tree_bodies` keeps piece ids). Hide
  selection hides a picked piece alone and Show everything shows hidden pieces too; Hide others
  on a piece hides the feature's other pieces (`visibility::hide_others`).
  Right-clicking a row offers Rename body… (the card below with its Body name field focused), Select
  the whole body (its faces in the view, through `PanelState::selected_in_tree`) and Remove body,
  which adds a `Remove` feature at the bar with a notice saying how to bring the body back; the same
  act from the palette and Model menu on the tree's body, else the selection's (`RenameBody`,
  `RemoveBody`; `app-modelling.md`, "What a command acts on").
- A body row's paint bucket (or the Body colour and material command, for the body chosen in the
  tree, else of the selection, else of the open feature) opens its colour and material card under
  the row (`PanelState::painting`, one at a time, focus on the first swatch until it lands):
  `widgets::swatch`es (default, `body_appearance::SWATCHES`, the recent custom colours
  (`Model::recent_colours`, named "Recent colour, <name>, <hex>") and the Custom colour
  `widgets::icon_swatch`, always last) above a property grid of Colour (a hex field), Material
  (`MATERIALS` presets, which set the density, their `Finish` and, for a body without a colour,
  their colour; None clears density and material and keeps the finish), Name (empty: named after its
  feature), Density (g/cm³, an expression), Opacity (`OPACITIES`; a see-through body is drawn in the
  translucent pass like X-ray, its faces and edges picked as usual) and Finish (`FINISHES`: Plain,
  then every `caditor_document::Finish`; `body_appearance::surface` gives each its metalness and
  roughness for the renderer, `render.md`). Each change is one undoable `SetBodyAppearance`; a
  refused value is an `error_row`.
- With faces of the body selected in the view, a second row of swatches (named "<colour> for the
  selected faces", the first the body's own colour, which removes theirs) colours those faces
  (`with_face_colours`), and Clear face colours (`without_face_colours`) drops them all, keeping a
  repainted face's own opacity and finish. A face that came with an opacity or finish (STEP import)
  is not dropped by either, since a `FaceColour` always holds a colour: it keeps its entry with the
  body's colour at that moment, and Clear face colours is offered only while it would change
  something. The scene draws a face's colour and its own opacity by splitting the body between the
  opaque and translucent passes outside the analysis and reflection views, unless the body is failed
  or outdated. A face's finish is its own, else the body's (`FaceLooks::finish`), applied only when
  the body is sound, the palette `shows_finishes` (the standard ones; high contrast draws every body
  plainly so its tested colours hold) and the face is neither hovered nor selected
  (`Builder::finish_of`), never in X-ray, the drawing style, an analysis or a reflection view. The
  rendered style gives every face a finish in high contrast too, falling back to the material
  preset's, and outside high contrast keeps it on hovered and selected faces (`app.md`, Rendered
  view).
- The Custom colour swatch, on the body row and the selected-faces row alike, opens
  `colour_selector.rs` in the card (state in egui temp data keyed by `body_appearance::selector_id`,
  so a body and its faces each have their own): a saturation and brightness square, a hue strip,
  Current and New previews and a Hex colour field. The working colour is an `Hsv` kept apart from the
  model, which keeps a grey's hue; nothing is applied while the pointer is down. It applies as one
  `SetBodyAppearance` (`with_colour` or `with_face_colours`) when the pointer is released or a click
  lands, when a keyboard edit ends and when a hex value is committed; the body's Colour field
  follows the working colour meanwhile (`colour_selector::working`). The square and strip are
  focusable, lock the arrow keys while focused (egui hands the lock over one frame after focus
  lands) and step 1% or 1 degree, 10% or 15 degrees with Shift. Screen readers get the square
  (`SQUARE_NAME`), the strip (a slider in degrees) and both previews as images named with a colour
  word and the hex (`colour_selector::describe`); chrome colours are `Tokens`, with the handles
  ringed in text over panel colour so they hold over any colour. Every applied custom colour that is
  not a fixed swatch joins the recent row (`PreferenceChange::RecentColour`, `app-files.md`). Tests:
  `ui_tests/custom_colours.rs`.

## Window and panel persistence

- `layout.rs`: `Preferences` holds a `WindowPlacement` (logical size, maximised, and the outer
  position where the platform reports one, which Wayland never does) and a `PanelLayout` (side width,
  the right-hand panels' width `panels.right_width` once one was resized, open sections) under
  `window.*` and `panels.*`, read clamped to sane bounds. Startup fits the size to the largest
  monitor and uses a position only when it lies on one; size and position are recorded only while
  not maximised, so unmaximising returns to them. The window's minimum size is
  `layout::MIN_WINDOW_WIDTH` by `MIN_WINDOW_HEIGHT`.
- Changes save through `Files::store_settings` shortly after they stop and on exit (`App::finish`
  waits for the jobs). An unreadable preferences file leaves the defaults in use and opens a notice
  at startup; a failed save becomes a notice and the next change writes everything it held.

## Title bar and window frame

- `Preferences::title_bar` (`built_in` by default, or `system`) chooses caditor's title bar (an
  undecorated window whose menu bar acts as the title bar) or the window manager's, applied at once
  through `window_frame::apply_title_bar` and offered in Preferences and the window menu.
- `window_frame.rs` works through egui `ViewportCommand`s, which `Overlay::run` hands to
  `egui_winit`; `Overlay` feeds maximized, full-screen and focus state back as the root
  `ViewportInfo`.
- With caditor's title bar, the menu bar's empty space and the model title drag the window, a
  double-click maximizes or restores, a right-click opens the window menu. The window buttons close
  through Quit so unsaved work is asked about; in full screen Leave full screen replaces the first
  two. While a dialog is open, `window_frame::over_dialogs` redraws the drag area and buttons above
  the modal. Close takes clicks from its button up to the top and right edges of the window
  (`reaching_the_corner`); a framed window's resize strips still win along its edge. A gap of
  `SPACE_S` on each side of the separator keeps the window buttons apart from the search field.
- On Windows the maximize button's rectangle goes to `caditor-windows` each frame it changes, so
  hovering it offers Snap Layouts (`windows.md`); the non-client hover and press that replace egui's
  pointer there come back as `window_frame::NonClientPointer`.
- When neither maximized nor full screen, a square outline marks the edge (the window is opaque, so
  rounding would leave corners showing) and thin foreground strips along it, with larger corners
  resizing diagonally, set the resize cursor and start `BeginResize`.
- The compositor takes the pointer during a move or resize and the release never reaches the window,
  so `Overlay` queues a synthetic primary release after `StartDrag` or `BeginResize`. On Windows both
  run the system's modal move and size loops, so dragging to an edge snaps; the undecorated window
  keeps a drop shadow (`with_undecorated_shadow`).
