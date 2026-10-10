---
paths:
  - "crates/caditor/src/field.rs"
  - "crates/caditor/src/commands.rs"
  - "crates/caditor/src/palette.rs"
  - "crates/caditor/src/shortcut_editor.rs"
  - "crates/caditor/src/toggles.rs"
  - "crates/caditor/src/menu_bar.rs"
  - "crates/caditor/src/typed_point.rs"
  - "crates/caditor/src/viewport.rs"
  - "crates/caditor/src/pick_list.rs"
  - "crates/caditor/src/view_menu.rs"
  - "crates/caditor/src/saved_views.rs"
  - "crates/caditor/src/selection_sets.rs"
  - "crates/caditor/src/paint_selection.rs"
  - "crates/caditor/src/app.rs"
  - "crates/caditor/src/feature_tree.rs"
---

# Input, commands and keyboard operation

## Numeric fields

- Every numeric input is a `field::commit_field`: commits on Enter or blur, reverts on Escape and
  keeps invalid text with its error inline until the stored value changes underneath it (an undo,
  say) while it is not being edited. Expression fields parse, evaluate and check the dimension
  before building a transaction; sketch dimensions go through `field::dimension_transaction`.
- Focus arriving at a `commit_field` (a click, Tab, the palette or a panel's focus request) selects
  its whole text (`field::select_all`; arrival is tracked per field in egui temp data rather than
  egui's `gained_focus`, which a request made between frames never reports), so typing replaces the
  value. Text left invalid keeps its cursor, so the mistake can be mended.
- Feature values and sketch dimensions are `field::NamedField`s, so `name = expression` names the
  value (`app-modelling.md`).

## Keyboard focus

- `Situation::accepts` decides what a key press may trigger. Keys widgets use (`WIDGET_KEYS`,
  Escape and Enter included) run commands only when no widget held focus, so Escape or Enter in a
  field never reaches the viewport; other plain keys need only that no text field has focus.
- egui-winit turns Ctrl+C, Ctrl+X and Ctrl+V into `Event::Copy`, `Cut` and `Paste` without a key
  event, so `commands::pressed_shortcut` reads those events as the key presses (a paste event
  arrives only while the system clipboard holds text).
- The clipboard reaches commands through `CommandFrame`: `pasted()` is the frame's paste text,
  `copy` puts text on the system clipboard (and keeps it in `Workspace::copied`). A paste command
  run without text (menu, palette, a key press while the clipboard held none) calls
  `ask_for_paste`; the app sends `ViewportCommand::RequestPaste`, which the overlay answers with
  the system clipboard's text as a paste event, and runs the command again next frame
  (`Workspace::awaiting_paste`) with that text, else the last text copied, else nothing. UI tests
  answer the request from the text the harness last saw copied.
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
  every command by category, filtered by the starts of the words of "category: title" as the
  palette matches them (`palette::starts_words`) or by keys (`commands::is_named_by`: every key
  named, any order and case), and with Changed only by the commands whose keys differ from the
  defaults, each of which carries a Changed pill. Each binding can be removed, Add records the next
  key press, and Reset restores a command's defaults. Reset first collects the defaults another
  command now holds (`Keymap::reset_conflicts`) and, when there are any, asks in the same warning
  callout as Add ("F is used by Undo. Reset Fit view and move it there?") before
  `Keymap::reset` takes them. Reset all asks first, then offers Undo through
  `Workspace::restored` (`app-files.md`).
- `app::show` runs `commands::dispatch` each frame and hands a `CommandFrame` to toolbars and
  viewport. Exact modifiers win (extra Shift or Alt are ignored only for punctuation keys);
  sketch-scope bindings win while a sketch is edited; Backspace and Delete are left to drawing
  mid-shape; only the keys in `Command::repeats` repeat.
- A command's button calls `CommandFrame::invoke` with its availability, which records an `Offer`
  and says whether it triggered; a triggered unavailable command becomes a notice with the reason.
  Hover texts and menu items show the current binding (`commands::display`).

## Saved views

- A model keeps named camera views (`document.md`, `saved_views.rs`). Save the current view saves
  where the camera is heading (`Camera::destination`) under the next free "View N" with a notice;
  Saved views… opens a modal (`Workspace::saved_views`, dropped with the session) listing them with
  Show, Replace with the current view, Rename and Delete, a Name field with Save view (Enter
  saves, focus starts there; a refused name is a callout and nothing is applied) and the Isometric
  section. Each change is one undoable `SetSavedViews` built by `saved_views::save`, `update`,
  `rename`, `delete`, `set_home` or `reset_home`, which check the transaction before it is
  offered. The dialog keeps the view it was opened with as the current one until Show moves it.
- A view is restored from View › Saved views, the dialog or a palette search
  (`PreferencesCommand::GoToView`, the camera animating like any view change), so every view is
  reachable from the keyboard.
- The Isometric view (Alt+0, the View menu, the palette) looks from the standard corner at the
  current target and distance until Make the current view the Isometric view redefines it as a
  whole saved view; Reset the Isometric view undoes that. A model with a redefined Isometric view
  opens looking from its orientation, fitted to the model (`ViewportState::start_from_home`, also
  for a new session; the headless PNG does the same), and one without goes back to the default
  orientation after one that had it.

- Go back to the previous view (`Command::PreviousView`, Alt+Left, View menu, palette) steps back
  through `view_history::ViewHistory`, the session's last `KEPT_VIEWS` viewpoints in
  `ViewportState`. `ViewportState::go_to` keeps the camera's destination before every animated
  change the user asks for (standard views, the cube and its Home button, fit, Show where, looking
  at a face or sketch, a saved view, entering a sketch), never the fit after opening. An orbit,
  pan or zoom is a `Gesture` (drag, wheel, keys) kept once, from where it started, when it ends:
  a drag when no button is down, wheel and key gestures when another kind starts or after
  `IDLE_SECONDS` without one, and any pending one before a jump or a step back. A view the same
  as the last kept is not kept again, stepping back skips views the same as the current one, a
  new session clears it, and with nothing earlier the command is unavailable with
  `NO_EARLIER_VIEW`. Stepping back animates without keeping, so it walks back in order; finishing
  a sketch leaves the camera facing it, so one step returns to the view before it was opened.

## Selection sets

- A model keeps named groups of faces, edges and bodies (`document.md`, `selection_sets.rs`).
  `selection_sets::capture` turns the viewport selection into members: a body all of whose faces
  are selected becomes the body, other faces and edges their references; anything else (vertices,
  sketch geometry, datums) is left out and the save says how many. Save the selection as a set
  (Edit › Selection sets, palette; unavailable inside a sketch or with no face or edge selected,
  `NOTHING_TO_KEEP`) saves it under the next free "Set N" with a notice. Selection sets… opens a
  modal like Saved views (`Workspace::selection_sets`, dropped with the session) listing each set
  with what it holds and Select, Replace with the current selection, Rename and Delete, and a
  Name field with Save selection; each change is one undoable `SetSelectionSets` built by
  `selection_sets::save`, `update`, `rename` or `delete`, checked before it is offered.
- A set is selected from Edit › Selection sets, the dialog or a palette search
  (`PreferencesCommand::SelectSet`, `Choice::SelectionSet`), outside a sketch only.
  `selection_sets::choose` resolves it now (`SelectionSet::resolve`), replaces the selection with
  what it finds on shown bodies (seam edges left out) and says in a notice what it could not:
  faces or edges of a body not found, a body with no solid now, a deleted body or a hidden one.
  When nothing can be selected the selection is kept and the notice says why. Tools then take the
  selection as they take any other, so a set feeds a fillet, a hide or a pattern.

## Command palette

- `palette.rs` lists this frame's offers, then the sketch commands that do not fit outside a
  sketch (muted, after all offers, `palette::absence`), and once something is typed the features
  (selecting the row and scrolling the tree to it, `Focus::Feature`) and parameters (focusing the
  value field, `Focus::ParameterValue`), saved views (going to the view, `Choice::View`,
  `Palette::take_view`) and selection sets (selecting the set, `Choice::SelectionSet`,
  `Palette::take_selection_set`) and configurations (switching to it, `Choice::Configuration`,
  `Palette::take_configuration`); the focus reaches the panels the next frame
  (`Palette::take_focus`, `PanelState::request_focus`). Groups (Recent only while nothing is typed,
  Commands, Features, Parameters, Views, Selection sets, Configurations) are ordered by best
  match.
- A fixed detail line under the list says what Enter does, or why the highlighted entry is not
  available, distinguishing "unavailable now" from "only works while a sketch is edited". The
  chosen command is triggered on the next frame.
- Matching ranks a start of the title, then starts of words, then text inside, then other names
  (`Command::keywords`, such as "zoom extents" for Fit view; a feature's kind words from
  `feature_tree::kind_words`), then scattered letters (`palette::fit_with_keywords`), so a synonym
  never outranks a title it also matches.
- `toggles::ToggleStates`, read once from the viewport and section tool when the menus and the
  palette are drawn, says what each on/off command and each choice (filter, display style) is now
  (`ToggleStates::shown`); menus draw it as checkmarks (`Menus::choice`) and the palette as an On or
  Off pill, or Current for a choice, also in the row's accessible name. Toggles with no visible
  effect (snapping, grid snapping, lasso, paint selection, select through, typed dimensions) post
  an info notice with their new state (`toggles::quiet_toggle_notice`) unless a menu, which already
  shows the checkmark, triggered them (`CommandFrame::trigger_showing_state`).
- Recent holds the last `palette::RECENT_LIMIT` commands run from the palette, kept in the
  preferences (`palette.recent`, command ids read through `Command::from_id`, unknown or repeated
  ones skipped), so it outlasts a restart and a failed frame (`Palette::with_recent`); the app
  stores a changed list through `PreferencesCommand::RememberRecent`.

## Keyboard-only operation

- Every command is in the palette, including those with no button: recompute and its cancel,
  going to the next or previous failed feature after the tree's primary row, wrapping (selects its
  row), adding a parameter and deleting the one
  whose field last had focus (`PanelState::parameter`), opening or removing each recent model (an `Offer`'s
  detail carries the file name), recovering unsaved work, cancelling an export, dismissing the notice and
  dismissing or hiding the current tip (offered while the palette covers it).
- A feature row's and panel's actions are commands on the tree's current feature
  (`feature_tree::current_feature`, the offer's detail; `feature_tree::commands`), sharing the
  button's availability. The selection-taking ones are `sketch_placement::place_on_selection`,
  `solid_panel::selected_axis_change` and `up_to_selected_change`, `datum_panel::base_change` and
  `rotation_change`, and `pattern_tools::selected_change`; they act on the open feature before the
  tree's row, so a row only being looked at is never changed. Each feature row has a "⋯" menu with
  what its right-click menu holds.
- The transactions those commands would apply (visibility, move, suppress, group, roll, delete,
  update references, detach and the selection-taking and reverse changes) are built and checked
  once per `OfferBasis` (revision, evaluation, draft, units, the selection's generation, the
  current, open and chosen rows and the editing context) and kept in `PanelState::tree_offers`
  (`feature_tree::TreeOffers`), so a still frame builds and checks none; anything new such an
  offer reads must join the basis.
- Tree order and the rollback bar are commands too (move, suppress, update references, roll back
  to here, roll to end, move the bar one row), so nothing needs a drag. Edit feature takes the
  tree's primary row, else the feature of the one item selected in the view (a face's origin, an
  edge's latest face origin, a datum, coordinate system or sketch curve or region's own;
  `viewport::feature_of`, which double-clicking also uses), else the open feature; it refuses a
  suppressed or rolled-back feature with the reason.
- Window commands (minimize, maximize or restore, full screen) are egui viewport commands
  (`window_frame::commands`); closing the window is Quit.
- Viewport commands cover measure, standard views, looking straight at the one selected flat face
  (outward normal from `sketch_placement::face_to_look_at`, framed on the face like Fit view),
  projection, orbit, pan and zoom. Fit view, Show where, looking at a face or sketch and the
  orbit, pan and zoom keys start from where the camera is heading (`Camera::destination_view`),
  not the view shown mid-turn, so a key pressed during an animation never keeps a half-turned
  view; the keys glide (`Camera::glide_to`) and the orbit step is `KEYBOARD_ORBIT_FRACTION` of the
  view's height times the orbit speed preference. The view cube
  takes focus like a button and arrows step to the neighbouring view while it has it
  (`app-look.md`). Sketch commands (move, select all, tool keys and the ways of drawing a shape)
  are in `app-sketching.md`.
- Highlight next and previous step a keyboard highlight through the scene's pickables in
  pick-table order (drawn and described like hover; a pointer move or Escape clears it), then,
  while a sketch is edited, through its constraints and dimensions in id order
  (`Pickable::SketchConstraint`, coloured as hovered and lighting their entities); Activate
  selects one and Enter opens a highlighted dimension's inline field, so dimensions are edited
  without a pointer. Activate
  acts as a click would (selection toggle, or the region, blend edge, shell face or sketch plane
  action through `pick_action`); Enter opens what the item belongs to as a double-click would,
  and with nothing highlighted Enter confirms the open feature like its checkmark. With Trim,
  Extend, Mirror, Circular pattern or Sketch fillet active they step through that tool's targets
  instead (`app-sketching.md`), except while Mirror or a pattern is gathering what it copies,
  when Activate toggles the highlighted item into the selection. With a drawing tool active, Activate on a highlighted point of the edited
  sketch or the origin places the shape's next point there, snapped to it as a click would
  (`place_at_highlight` through `Drawing::type_point`), and on a highlighted curve at its middle
  (`Drawing::type_on_curve`: a line's or arc's with `Midpoint`, a circle's rightmost point or a
  spline's halfway point on it), so keyboard drawing starts from existing geometry; where the shape
  takes only points, or on anything else, it says what to highlight. With Smart dimension active,
  Activate picks the highlighted item as a click would and Enter adds the dimension of a single
  pick (`app-sketching.md`).
- What lies behind something is reached from the pick list (`pick_list.rs`): a primary press held
  still in the view for `HOLD_TO_LIST` (outside drawing, Trim and the modifying tools, which take
  presses themselves) or List everything under the pointer (`view.list_under_pointer`, View menu,
  palette; at the pointer, else where it last was in the view, else the view's centre) opens a
  popup at that spot listing every pickable under it, from `Scene::hits_through` (`render.md`)
  through `PickTable::listed` (each pick's tolerance and the selection filter; the projectable
  ones while projecting), named as hover names them, one row per body with the Bodies filter.
  Its rows are `widgets::menu_choice`s with `icons::pickable`; the first takes focus, arrows move
  it, and the pointed or focused row is drawn and described as hovered. Choosing one acts as
  Activate does (`ViewportState::choose`) but replaces the selection, adding to it with Shift or
  Ctrl; Escape, a press outside it or a document change closes it. The press that opened it
  never counts as a click, and with nothing under the pointer an info notice says so.
- The 3D view's context menu (`view_menu.rs`, `ViewportState::context_menu`) opens on a secondary
  click there: egui's `secondary_clicked`, released within the click distance and time, so a
  right-drag that orbits or pans in any input mode never opens it and the primary press-and-hold
  of the pick list and the Laptop gestures are untouched. Where a click would select (no drawing,
  modify or dimension tool, plane choice or reference pick; `ViewportState::menu_selects`) it
  waits for a pick for the current cursor and view like a primary click
  (`menu_waits_for_pick`), then an unselected item under the pointer replaces the selection
  (`whole_body_of`, so the Bodies filter takes the body), a selected one or empty space keeps
  it. Show the context menu (`Command::ContextMenu`, Shift+F10, palette; egui has no Menu key)
  opens it at the keyboard highlight, else the selection's middle on screen
  (`screen_centre_of`, `BuiltScene::bounds_of`), else the view's centre, applying the same rule
  to the highlight. What it lists is a `view_menu::Place` fixed when it opens, with what the
  selection holds then (`view_menu::Held`, worked out once at opening: faces, edges, body
  geometry, whole bodies, sketch geometry, a plane or flat face to sketch on, and the one feature
  that made every selected item, `viewport::feature_of`, or a lone whole body's own): the model
  (an item: Edit, then the tools that take the selection (`Held::tools`: New sketch on a plane or
  flat face, Fillet and Chamfer on edges, Extrude, Hole, Offset face, Shell, Split face, Thread
  and Mirror faces on faces, Extrude and Revolve on sketch geometry), then the body tools (Move,
  Copy, Mirror, both patterns, Split, Scale, Combine) inline for whole bodies or when no other
  tool fits, else under Body, then Suppress, Rename and Delete named after the feature that made
  the selection (`view_menu::FEATURE_COMMANDS`; choosing one chooses that feature's tree row
  first, `ViewportState::take_row_to_choose`, so the tree's own command acts on it next frame,
  Delete ending "…" when it will ask), then hide, look, fit, measure, select, list, appearance
  and copy; or empty space: fit, previous view, standard views, show all, paste features,
  selection filter), the edited sketch (the selection's constraint offers inline up to
  `MOST_INLINE_CONSTRAINTS`, else under Constrain, then construction, split, break, delete, the
  sketch tools that take the selection under Modify (Offset, Mirror, both patterns, Sketch
  fillet and chamfer, Tangent circle), move, rotate or scale, cut, copy, paste, select, Smart
  dimension, Finish sketch), or a shape being
  drawn (Take back the last point, Reverse the arc, Finish the shape, Cancel the shape, Type an
  exact value; `Command::TakeBackPoint`, `FinishShape`, `CancelShape`, as Backspace, Enter and
  Escape do mid-shape), with the open feature's Reverse, Cancel and Finish editing on top. Every
  entry is a `Command` drawn by `menu_bar::MenuEntries` from this frame's offers and keymap, as
  the menu bar's are, so a disabled entry says why on hover and shows its keys; entries that only
  apply sometimes (the tools, Look at face, the selection growers, split, break, Reverse) are left
  out rather than disabled, the tools also when the selection holds nothing they take. It opens
  downward from its spot, moved up and left by the size it measured the frame before
  (`widgets::measured_menu`, `menu_opening_down`), so a tall menu stays on screen instead of egui
  flipping it above the spot. It is drawn after the frame's commands finish (`ViewportState::show_menu`),
  first one frame later so the offers follow a selection the click changed, then with its first
  available entry focused so arrows move and Enter runs it; a chosen command runs next frame
  through `Workspace::deferred_commands` (List everything under the pointer at the menu's spot,
  `list_at`). Escape, a click outside it (which does not also act in the view), a dialog, a
  document change or leaving its kind of place closes it.
- The selection filter (`SelectionFilter`, commands `select.*`, View › Selection filter) makes
  `PickTable::best_hit` and the highlight keys skip every pickable but one kind (whole bodies,
  faces, edges, vertices or sketch geometry), reference geometry included. Bodies picks faces
  but acts on the whole body: hover and the keyboard highlight light every face of it, a click,
  Activate or a box (`ViewportState::whole_body_of`) selects, or with Shift or Ctrl toggles, all
  its faces, which is how every tool already reads a body from its faces. Cycle the selection
  priority (`select.priority`, View menu, palette) sets the filter to the next of body, face and
  edge in one step, from any other filter to body. It applies only while no sketch or
  tool is open and no plane is being chosen (`ViewportState::filter_applies`), so tools keep
  picking what they need; the status bar names an active filter and its button clears it. It is
  kept for the session, not saved.

- Outside sketch editing (no sketch edited, feature open, plane or reference being chosen) a
  primary drag in the 3D view draws a box (`PrimaryDrag::ModelBox`, `box_selection.rs`): left to
  right a window taking what lies wholly inside, right to left a crossing box taking what it
  touches, replacing the selection (Shift or Ctrl adds). It takes faces (Everything or Faces),
  edges, vertices or the curves of shown sketches by the selection filter, of shown bodies only.
  Sketch curves go through `sectioned_screen::SectionedScreen`: a point a section plane cuts away
  is left out and a curve counts by the part the planes keep (`Screen::kept`, `kept_span`), so
  a curve cut away entirely is never taken, as picking never takes it.
  Faces count by their triangles facing the camera, so faces turned away are left out, and only
  when one of those triangles is seen (a corner or its middle, `triangle_is_seen`). Faces, edges and
  vertices hidden behind a shown body are left out: `box_selection::Occlusion` rasterises the
  shown bodies' triangles into a depth map over the box (at most `MAX_CELLS` a side), a point
  counting as seen when it lies within `SLACK_CELLS` cells' worth of depth of the nearest surface
  there; an edge is sampled every `SAMPLE_POINTS` on screen and counts only when at least half of
  it is seen, judged by its seen samples alone. A box under `SMALLEST_BOX` points takes nothing.
  Select through (`Command::ToggleSelectThrough`, View menu, palette; kept for the session in
  `ViewportState::select_through`, not saved) swaps the depth map for `Occlusion::open`, which
  shows everything, so boxes and lassos also take the faces, edges, vertices and bodies (with the
  Bodies filter) hidden behind others.
- Select with a lasso (`Command::ToggleLasso`, View menu, palette; kept for the session in
  `ViewportState::lasso`, not saved, since Alt-drag already navigates in the Laptop input mode)
  makes those drags, and the edited sketch's, draw a freehand outline instead
  (`sketch_drag::ScreenArea::Lasso`, a point every `LASSO_STEP` points, closed back to its start)
  that takes what lies wholly inside it, like a window; everything else about box selection holds.
- Select faces by painting over them (`Command::TogglePaintSelection`, View menu, palette; kept for
  the session in `ViewportState::paint`, not saved) makes those model drags a brush
  (`PrimaryDrag::Paint`, `paint_selection.rs`) while the filter takes faces (Everything, Faces or
  Bodies; other filters keep the box): the drag replaces the selection (Shift or Ctrl adds) and
  each pointer move adds the faces under the stroke as it goes, sampled every
  `BRUSH_STEP_POINTS` (at most `MAX_BRUSH_SAMPLES` a move) through `Scene::hits_through` on the
  CPU, so no GPU pick is awaited. Each sample takes the nearest face, or every face under it with
  Select through; the Bodies filter takes the whole body (`whole_body_of`). Escape mid-drag puts
  back the selection the drag started from. Sketch drags keep the box or lasso.
- Select all (`select.all`), Select tangent edges and Select edges around faces
  (`body_selection.rs`) work on the shown bodies outside sketch editing and refuse inside one.
  Select all takes every face, edge or vertex by the selection filter, or by the kind already
  selected while the filter is Everything, and says what to choose when neither names one; seam
  edges, which cannot be picked, are never taken. Tangent edges add what `tangent_chain` reaches
  from the selected edges (smooth or sharp alike, unlike a blend's `blend_chain`); tangent faces
  (Alt+Shift+T) add what `tangent_faces` reaches from the selected faces; Select the whole hole
  (`select.hole`, Edit menu, palette) adds what `hole_faces` reaches from a selected wall; the edges around
  faces replace the faces with every loop's edges. Select the whole body (`select.body`, Edit menu,
  palette) replaces the selection with every face, edge or vertex (by the filter or the kind
  selected, faces otherwise) of the bodies the selection touches. An empty result is an info
  notice. Their
  availability is a cheap check on the selection, and the work runs only when triggered.

- The display style (`DisplayStyle`, commands `view.style_*`, View › Display style) is shaded with
  edges, shaded with hidden edges dashed (the same, with each shown body edge stroked
  `Stroke::DashedWhereHidden`, so the renderer draws the same line again dashed where a face
  covers it, never picked there, with no second copy in the scene), shaded without edges (edges drawn with no alpha, so they stay pickable and appear when
  hovered or selected) wireframe (no faces in the scene, so none is drawn or picked and edges
  show through), hidden lines removed (faces in `Scene::flat_meshes`, unlit in `DRAWING_FACE`
  whatever the body's colour, still pickable and hiding what lies behind them, edges drawn over
  them, so it reads as a drawing) or X-ray (faces in `Scene::translucent_meshes` at
  `XRAY_FACE_ALPHA`, unpickable, edges on top). Every style that draws edges also gives each body
  a `Silhouette` in the edge colour and width (dashed with a troubled body's edges, the dimmed
  edge colour for background bodies, and for the body an opened feature shows), so the outline of
  its curved faces is drawn as the view turns (`render.md`); with hidden edges dashed a coloured
  body's silhouette is `dashed_where_hidden`, drawn again dashed where a face covers it, and like
  the hidden edges never picks. It applies to bodies outside sketch editing, reaches the scene through
  `Sources::style` and `Revisions::style`, shapes image exports too and is kept for the session.

## Typed-point field

- `typed_point.rs`: with a drawing tool active, typing a digit, sign, point, `(`, `@`, `<` or `=`
  opens it. Letters are tool keys, so `=` is the way to start with a parameter's name: a leading
  `=` is left out of what is parsed (`typed_point::value_text`; `Typed::text` is the value,
  `Typed::entered` what was typed, which an error reopens). Type an exact value
  (`Command::TypeValue`, Sketch menu, palette, no default key) opens it empty, or resumes a
  waiting one, while a drawing tool, a modify tool's value field, Move, Rotate or Scale is active
  (`viewport::TYPE_VALUE_UNAVAILABLE` otherwise).
  It takes two length expressions split at top-level commas, a length and angle split at a
  top-level `<` (from the sketch's x axis; degrees unless a unit is named), `@` for an offset from
  the last placed point, and a lone length from the last point toward the pointer. The point must
  be within `MAX_LENGTH` of the origin. Offset's distance and Sketch fillet's radius reuse it as
  one length expression (`modifying::Value`, `app-sketching.md`), as does Move.
- An angle alone (`< 30`, `@< 30`; `typed_point::heading`) locks the direction from the last
  placed point (`Drawing::lock_heading`): `Drawing::place` projects the pointer onto that ray,
  ahead of snapping, Ctrl and the toggles, the preview draws the ray as a dashed guide, the prompt
  says so (`drawing::HEADING_PROMPT`) and a click places the point there, keeping the angle as a
  dimension like a typed one (`typed_hover`); a lone length typed meanwhile runs along the ray and
  keeps the angle too. Placing the point, Backspace, cancelling the shape or Escape
  (`Drawing::release_heading`, before the shape is cancelled) let go of it.
- It is canvas chrome (`canvas::PANEL`, `canvas::Hints`) in the band left of the view cube.
- While it has focus and its text reads as a point (`typed_point::parse_placed`) or a heading, the
  drawing previews it as a placement that is never placed (`Drawing::preview_typed`,
  `preview_heading`): the hover, the arc's sweep and the heading are kept aside (`Pointed`) and put
  back by the next `Drawing::hover`, so the readout and shape follow the text and the pointer
  takes over again when it stops reading. Modify tools preview `TypedPoint::typing_text` the same
  way (`show_text`).
- Losing focus without Enter or Escape (a click in the view, another widget, a shortcut) keeps
  the text, waiting: the label in the chrome's `muted`, the text in the theme's `text_muted` and `typed_point::WAITING_HINT` as its
  hint in place of the prompt, as while typing, and clicks acting at the pointer. Clicking the field, an
  opener typed (appended to the kept text, a leading `=` dropped) or Type an exact value resumes
  it; Escape in the view (first in `ViewportState::escape`), a change of edited sketch or tool
  (`ViewportState::typed_owner`) or leaving the context clears it.
- Enter places the point through `Drawing::type_point` (landing exactly on an existing or the
  pending point snaps to it, like a click); an error keeps it open with the reason; Escape closes
  it without touching the shape. It is handled after the drawing syncs with the displayed sketch
  each frame, so a typed chain carries on from the end the previous segment added.
- A point placed free keeps what was typed as dimensions, as typed (parameters included, a negative
  value as its magnitude, a zero left out): x and y as horizontal and vertical distances from the
  origin, or from the last placed point with `@` (`typed_point::parse_placed`, `TypedDimension`); a
  length as a distance from the origin or last point; and an angle between 0° and 360° as an
  `Angle` from the horizontal axis to the line drawn from that point, when the shape draws one.
  `Drawing::type_dimensioned` keeps them with the placement and `Draft::finish` adds them to the
  shape's transaction against the points at those places, through the shadow check like any
  inferred constraint. Keep typed values as dimensions (`Command::ToggleTypedDimensions`, Sketch
  menu, palette; on by default, kept for the session in `ViewportState::typed_dimensions`) turns
  this off.
