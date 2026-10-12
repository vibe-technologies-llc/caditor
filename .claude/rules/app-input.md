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
  - "crates/caditor/src/box_selection.rs"
  - "crates/caditor/src/completion.rs"
  - "crates/caditor/src/look_at.rs"
  - "crates/caditor/src/repeating.rs"
  - "crates/caditor/src/stepping.rs"
  - "crates/caditor/src/view_history.rs"
  - "crates/caditor/src/sectioned_screen.rs"
  - "crates/caditor/src/clipboard.rs"
  - "crates/caditor/src/display_style.rs"
  - "crates/caditor/src/canvas_guidance.rs"
  - "crates/caditor/src/queued.rs"
---

# Input, commands and keyboard operation

## Guidance in the 3D view

- Key hints are data, not text: a `canvas::HintList` of `KeyHint`s (keys, or none for a plain
  note, an action and a `Rank`: `Main`, `Useful`, `Extra`). Sources still written as
  `Key: action` text (modify tools, Smart dimension, reference picking, the input modes) go
  through `HintList::parsed`. The prompt's list (`ViewportState::prompt`, a drawing tool's from
  `drawing::Prompt::hints` and `drawing_hints`) folds past `MOST_UNFOLDED` items: the most wanted
  by rank show in their own order and a `canvas::MORE_KEYS` chip (a focusable button, expanded
  state for AccessKit) shows the rest until Fewer keys folds them, kept while caditor runs
  (`ViewportState::more_keys`). The prompt is a polite live region reading what is shown and how
  many keys are folded (`HintList::spoken_folded`), all of them once expanded. Navigation and
  keyboard-highlight hints never fold.
- Prompts (`canvas_guidance::show_prompt`): an open feature's says what a click does
  (`viewport::feature_prompt`, "Click edges to fillet…") with `Esc: done` only where clicks act
  (blend, shell, offset and split faces, extrude, revolve and sweep regions, pattern copies),
  else where in its panel to look, per datum and pattern kind; Select in an edited sketch has
  `SELECT_PROMPT` with Esc finishing or clearing.
- An empty model (no feature, nothing open, no plane being chosen) shows a card at the prompt's
  place (`canvas_guidance::show_empty_state`): Start with a sketch, Start with a box, Import a
  file…, canvas buttons whose commands reach the app through `ViewportState::show_menu`
  (`chosen_commands`), so they run next frame like a context menu's.
- Beside the pointer, stacked without overlap and kept inside the view
  (`canvas_guidance::paint_beside_pointer`, flipping left or above near an edge): a refused click's
  note (any warning or error notice the click produced, `RefusalNote`, fading over its last
  `NOTE_FADE_SECONDS` of `NOTE_SECONDS`), the snap label, the size readout, the drag-blocked cue and
  the hover description. A tool's words (trim, modify, project, Smart dimension, handles, pattern
  copies) show at once; a plain hover's after `HOVER_DELAY` on the same words (`HoverDelay`,
  repainting when it is due). A keyboard highlight's description stays at the band's top left
  with the highlight keys.
- The fixed chrome is placed after the prompt and the pointer labels and keeps clear of them and of
  the view cube (`canvas_guidance::first_clear`): the grid label, the cursor readout (above the
  grid label, else beside it, else lifted up to `READOUT_LIFTS` rows, each also clear of the edited
  sketch's dimension labels, `Annotations::label_rects`, so it never covers one) and the
  navigation hints, each left out when nothing fits inside the view.

## Numeric fields

- Every numeric input is a `field::commit_field`: commits on Enter or blur, reverts on Escape, keeps
  invalid text with its inline error until the stored value changes underneath it while not edited.
  Expression fields parse, evaluate and check the dimension before building a transaction; sketch
  dimensions go through `field::dimension_transaction`. Feature values and sketch dimensions are
  `field::NamedField`s, so `name = expression` names the value (`app-modelling.md`).
- Focus arriving at a `commit_field` selects its whole text (`field::select_all`; arrival is tracked
  per field in egui temp data, since `gained_focus` never reports a request made between frames).
  Invalid text keeps its cursor.
- Value fields (`field::value_field`: feature values, sketch dimensions, parameter expressions,
  section and analysis values, configuration cells; names, colours and exports keep `commit_field`)
  step with Up and Down: `stepping::step` adds or takes one unit of the last digit of a plain number
  (Shift ten), in integers so 0.1 steps never drift, keeping the typed unit and crossing zero by
  sign. The stepped text is the draft and reaches the caller as `edited`, so an open feature
  previews it. Anything else (expression, name, `name = value`, unknown suffix, empty) shows
  `stepping::Refusal::message` inline until the text changes. Arrows are taken from the frame's
  events (`field::take_step`) ahead of the text edit; Ctrl and Alt with them go to the text edit.
- Value fields complete parameter names (`completion.rs`). `completion::publish` builds the
  candidates and the `DependencyGraph` once per revision, evaluation and unit while a widget has
  focus, in egui temp data. The identifier before the caret (`completion::token_at`: starts with a
  letter or `_`, so units glued to a number and the name in `name = …` are never completed) is
  matched case-insensitively, at most `MAX_SHOWN`, leaving out an exact match and, for a field
  editing a parameter (`completion::editing_parameter`), any name that is that parameter or would
  depend on it (`DependencyGraph::cycle`). The list is an `Area` (`completion::list`), a `ListBox`
  named `LIST_NAME` of `ListBoxOption`s; the field keeps focus and carries `has_popup`, `expanded`
  and the active descendant. While open (known from last frame's `Suggesting`) Up and Down choose a
  row instead of stepping, Tab inserts the first or chosen one, Enter only one chosen with the
  arrows (else it commits), Escape closes until the identifier changes; keys are taken before the
  text edit, whose `EventFilter` (`completion::filter`) claims Tab and Escape while open.
- egui drops focus at a click's release, so a press on a list row or a sketch dimension in the view
  makes the field hold focus until the pointer is up (`pointer_holds`, focus requested again). While
  a value field has focus, the annotation under the pointer offers its dimension's text
  (`completion::offer_insertion`: the parameter's name, else the value expression, bracketed unless
  a name, literal or call; offers expire after two passes, so the pointer must rest on the label
  first); the press inserts it and marks the press taken (`completion::mark_taken`) so the label's
  click does not select (`take_click`); the app forgets an unused mark once the button is up
  (`forget_taken_click`). A Parameters panel row's value column inserts the parameter's name the
  same way (`field::focused_value_field`, hover `parameter_table::INSERT_HINT`), offering nothing to
  the row's own field or one it would make depend on itself (`Completions::forms_cycle`,
  `parameter_table::insertable_into_focused`).

## Keyboard focus

- `Situation::accepts` decides what a key press may trigger. Keys widgets use (`WIDGET_KEYS`,
  Escape and Enter included) run commands only when no widget held focus, so Escape or Enter in a
  field never reaches the viewport; other plain keys need only that no text field has focus.
- egui-winit turns Ctrl+C, Ctrl+X and Ctrl+V into `Event::Copy`, `Cut` and `Paste` with no key
  event, so `commands::pressed_shortcut` reads them as key presses (a paste event arrives only while
  the system clipboard holds text).
- Commands reach the clipboard through `CommandFrame`: `pasted()` is the frame's paste text, `copy`
  puts text on the system clipboard (and in `Workspace::copied`). A paste command run without text
  calls `ask_for_paste`; the app sends `ViewportCommand::RequestPaste`, the overlay answers with the
  clipboard text as a paste event, and the command runs again next frame
  (`Workspace::awaiting_paste`) with that text, else the last text copied. UI tests answer from the
  text the harness last saw copied.
- Ctrl and Alt shortcuts run from a text field too, except on `TEXT_EDITING_KEYS`: the field loses
  focus (committing) and the command runs next frame (`Workspace::deferred_commands`), so Save saves
  a value just typed.

## Commands and keymap

- Every toolbar, menu and sketch action is a `Command` with a stable id, title, `Category`, `Scope`
  (anywhere, only in a sketch, or only outside one, which lets P hide the principal geometry outside
  sketches while it stays the Point tool inside) and default shortcuts. Scope only decides where a
  key reaches the command; palette and menus offer it as usual. `Command::all()` is built from
  `plain_commands!`, which matches `Command` exhaustively so a missing variant fails to compile, and
  from the `ALL` of each payload type declared by `all_variants!` (`variants.rs`). Defaults are in
  `Command::default_shortcuts`; docs do not repeat them. In a sketch a plain letter is a tool or
  sketch command and Shift with a letter belongs to the constraints alone
  (`shift_and_a_letter_is_kept_for_the_constraints`); the guide's keyboard page states the pattern.
- `Keymap` in `Preferences` stores only overrides, as `keys.<id>` lists of text, so unreadable or
  newer entries survive and a command with no readable binding keeps its defaults. Esc, Enter and
  Tab are reserved. A binding used in an overlapping scope asks before moving.
- The shortcut editor (`shortcut_editor.rs`; Preferences, Help menu, palette) filters by word starts
  of "category: title" (`palette::starts_words`) or by keys (`commands::is_named_by`: every key
  named, any order and case); Changed only shows commands differing from defaults. Add records the
  next key press. Reset first collects defaults another command now holds
  (`Keymap::reset_conflicts`) and, if any, asks in the same warning callout as Add before
  `Keymap::reset`. Reset all asks first, then offers Undo through `Workspace::restored`
  (`app-files.md`).
- Repeat the last command (`Command::RepeatLast`, F4, Edit menu, palette, top of the view's context
  menu) runs the last command that ran and `Command::is_repeatable` accepts: modelling tools,
  datums, sketch tools, constraints and sketch edits, never a view, file or selection command.
  `CommandFrame::invoke` records each triggered, available command (`CommandFrame::ran`);
  `repeating::Repetition::settle`, run in `app::show` after the frame's commands, keeps the newest
  repeatable one in `Workspace::repetition` and adds the offer for `RepeatLast` itself ("Repeat
  Fillet") with the last command's availability, so it reads the selection and context of the frame
  it runs in. Triggering it puts the last command into `Workspace::deferred_commands`, where its own
  `invoke` decides and refuses with its own reason; with nothing to repeat, or the command not
  offered here, it is refused at once (`repeating::NOTHING_TO_REPEAT`).
- `app::show` runs `commands::dispatch` each frame and hands a `CommandFrame` to toolbars and
  viewport. Exact modifiers win (extra Shift or Alt ignored only for punctuation keys); sketch-scope
  bindings win while a sketch is edited; Backspace and Delete are left to drawing mid-shape; only
  keys in `Command::repeats` repeat.
- A command's button calls `CommandFrame::invoke` with its availability, which records an `Offer`
  and says whether it triggered; a triggered unavailable command becomes a notice with the reason.
  Hover texts and menu items show the current binding (`commands::display`).

## Queued actions

- A refusal that only says the model is not ready is never "try again": its reason is one of
  `queued.rs`'s constants (`BODY_SHAPE`, `BODIES_SHAPE`, `MODEL_UP_TO_THERE`, `SKETCH_SHOWN`,
  `CONSTRAINT_CHECK`), which `Awaited::of` recognises, and the action is queued instead. The app's
  refusal sink (`queued::refused`, also used by the tools' `create_actions` and the sketch bar for a
  constraint while one is checked), a palette entry unavailable for such a reason (chosen anyway,
  so it is refused and queued next frame), New sketch on a face not recomputed
  (`Rerun::Editing`) and the sketch's Move, Rotate and Scale field (`Rerun::Typed`) all send
  `Action::Queue`.
- `Model` keeps one `queued::Queue`: a newer queued action replaces it. It is held only while what
  it waits for runs (`Model::awaits`: a recompute, or a constraint trial); otherwise the refusal is
  a warning notice saying what to do first (`Queued::refusal`). It carries the session and
  revision it was queued on and is dropped, never run, once either changes; the one change it
  survives is the verdict of the trial it waited for (`Queue::follow`). `app::perform` takes it
  once nothing it waits for runs (`Model::take_due`) and reruns it: a command through
  `Workspace::deferred_commands` (so its own `invoke` reads the selection and model afresh), an
  `EditingCommand`, or the typed text; `perform` then asks for a frame.
- While held, the status bar's activity shows `queued::note`, a `widgets::progress` ("<title>:
  waiting for the model to recompute…") with Cancel; Cancel and Escape (`ViewportState::escape`,
  right after closing the pick list) send `Action::DropQueued`.

## Saved views

- A model keeps named camera views (`document.md`, `saved_views.rs`). Save the current view saves
  `Camera::destination` as the next free "View N". Saved views… opens a modal
  (`Workspace::saved_views`, dropped with the session) with Show, Replace, Rename and Delete per
  view, a Name field with Save view, and the Isometric section. Each change is one undoable
  `SetSavedViews` built by `saved_views::save`, `update`, `rename`, `delete`, `set_home` or
  `reset_home`, checked before it is offered; a refused name is a callout and nothing is applied.
- A view is restored from View › Saved views, the dialog or a palette search
  (`PreferencesCommand::GoToView`, animated like any view change).
- The Isometric view (Alt+0) looks from the standard corner at the current target and distance until
  Make the current view the Isometric view redefines it as a whole saved view; Reset the Isometric
  view undoes that. A model with a redefined Isometric view opens looking from its orientation,
  fitted to the model (`ViewportState::start_from_home`, also for a new session and the headless
  PNG); one without goes back to the default orientation after one that had it.
- Go back to the previous view (`Command::PreviousView`, Alt+Left) steps back through
  `view_history::ViewHistory`, the session's last `KEPT_VIEWS` viewpoints in `ViewportState`.
  `ViewportState::go_to` keeps the camera's destination before every animated change the user asks
  for (standard views, the cube, fit, Show where, looking at a face or sketch, a saved view,
  entering a sketch), never the fit after opening. An orbit, pan or zoom is a `Gesture` kept once,
  from where it started, when it ends: a drag when no button is down, wheel and key gestures when
  another kind starts or after `IDLE_SECONDS`, and any pending one before a jump or a step back.
  Views equal to the last kept are not kept, stepping back skips views equal to the current one, a
  new session clears it, and with nothing earlier the command is unavailable with `NO_EARLIER_VIEW`.
  Stepping back animates without keeping, so it walks back in order.

## Selection sets

- A model keeps named groups of faces, edges and bodies (`document.md`, `selection_sets.rs`).
  `selection_sets::capture` turns the viewport selection into members: a body all of whose faces are
  selected becomes the body, other faces and edges their references; anything else (vertices, sketch
  geometry, datums) is left out and the save says how many. Saving (Edit › Selection sets, palette;
  unavailable inside a sketch or with no face or edge selected, `NOTHING_TO_KEEP`) uses the next free
  "Set N". Selection sets… opens a modal like Saved views (`Workspace::selection_sets`); each change
  is one undoable `SetSelectionSets` built by `selection_sets::save`, `update`, `rename` or
  `delete`, checked before it is offered.
- Selecting a set (Edit menu, the dialog or a palette search: `PreferencesCommand::SelectSet`,
  `Choice::SelectionSet`; outside a sketch only) goes through `selection_sets::choose`: it resolves
  now (`SelectionSet::resolve`), replaces the selection with what it finds on shown bodies (seam
  edges left out) and says in a notice what it could not (faces or edges not found, a body with no
  solid now, a deleted or hidden body). When nothing can be selected the selection is kept and the
  notice says why.

## Command palette

- `palette.rs` lists this frame's offers, then the sketch commands that do not fit outside a sketch
  (muted, after all offers, `palette::absence`), and once something is typed: features
  (`Focus::Feature`), parameters (`Focus::ParameterValue`), saved views (`Choice::View`,
  `Palette::take_view`), selection sets (`Choice::SelectionSet`, `Palette::take_selection_set`) and
  configurations (`Choice::Configuration`, `Palette::take_configuration`); focus reaches the panels
  next frame (`Palette::take_focus`, `PanelState::request_focus`). Groups (Recent only while nothing
  is typed) are ordered by best match.
- A fixed detail line under the list says what Enter does, or why the highlighted entry is not
  available ("unavailable now" versus "only works while a sketch is edited"). The chosen command is
  triggered next frame. Enter or a click on an entry that is not ready closes the palette and posts
  its detail line as a warning notice (`Palette::take_refusal`), so the next key reaches the view or
  panel it was meant for.
- Matching ranks a start of the title, then starts of words, then text inside, then other names
  (`Command::keywords`; a feature's kind words from `feature_tree::kind_words`), then scattered
  letters (`palette::fit_with_keywords`), so a synonym never outranks a title it also matches.
- `toggles::ToggleStates`, read once from the viewport and section tool when menus and palette are
  drawn, says what each on/off command and each choice is now (`ToggleStates::shown`); menus draw
  checkmarks (`Menus::choice`), the palette an On or Off pill (Current for a choice), also in the
  row's accessible name. Toggles with no visible effect (snapping, grid snapping, lasso, paint
  selection, select through, typed dimensions) post an info notice with their new state
  (`toggles::quiet_toggle_notice`) unless a menu triggered them
  (`CommandFrame::trigger_showing_state`).
- Recent holds the last `palette::RECENT_LIMIT` commands run from the palette, kept in preferences
  (`palette.recent`, ids read through `Command::from_id`, unknown or repeated ones skipped), so it
  outlasts a restart and a failed frame (`Palette::with_recent`); changes are stored through
  `PreferencesCommand::RememberRecent`.

## Keyboard-only operation

- Every command is in the palette, including those with no button: recompute and its cancel,
  next or previous failed feature, adding a parameter and deleting the one whose field last had
  focus (`PanelState::parameter`), opening or removing each recent model (the `Offer`'s detail
  carries the file name), recovering unsaved work, cancelling an export, dismissing the notice and
  dismissing or hiding the current tip.
- A feature row's and panel's actions are commands on the tree's current feature
  (`feature_tree::current_feature`, the offer's detail; `feature_tree::commands`), sharing the
  button's availability. The selection-taking ones are `sketch_placement::place_on_selection`,
  `solid_panel::selected_axis_change` and `up_to_selected_change`, `datum_panel::base_change` and
  `rotation_change`, and `pattern_tools::selected_change`; they act on the open feature before the
  tree's row, so a row only being looked at is never changed. Each feature row has a "⋯" menu with
  what its right-click menu holds.
- The transactions those commands would apply are built and checked once per `OfferBasis` (revision,
  evaluation, draft, units, the selection's generation, current, open and chosen rows, editing
  context) and kept in `PanelState::tree_offers` (`feature_tree::TreeOffers`), so a still frame
  builds none; anything new such an offer reads must join the basis.
- Tree order and the rollback bar are commands too, so nothing needs a drag. Edit feature takes the
  tree's primary row, else the feature of the one item selected in the view (a face's origin, an
  edge's latest face origin, a datum, coordinate system or sketch curve or region's own;
  `viewport::feature_of`, which double-clicking also uses), else the open feature; it refuses a
  suppressed or rolled-back feature with the reason. Edit the sketch (`Command::EditSketch`,
  `feature_tree::sketch_change`) takes the same target and enters its sketch (the feature's own, an
  extrusion's, revolve's or hole's, so a selected face opens the sketch whose profile made it); any
  other feature refuses in words, and the context menu offers it only when it applies. The extrusion
  and revolve panels' Sketch row has the button too (`solid_panel::sketch_row`).
- Window commands are egui viewport commands (`window_frame::commands`); closing the window is Quit.
- Viewport commands cover measure, standard views, projection, orbit, pan, zoom and looking straight
  at the selection (`look_at::look_target`, framed like Fit view). Repeating it while the view
  already looks that way turns a quarter turn about that direction (`look_at::quarter_turned`);
  entering a sketch only faces it, from the side of its plane the camera is on (`viewport::entering`, `Viewpoint::facing_from_behind`). Fit view, Show where, looking at a face or sketch and the orbit,
  pan and zoom keys start from where the camera is heading (`Camera::destination_view`), not the
  view shown mid-turn; keys glide (`Camera::glide_to`) and the orbit step is
  `KEYBOARD_ORBIT_FRACTION` of the view's height times the orbit speed preference. The view cube
  takes focus like a button and arrows step to the neighbouring view (`app-look.md`). Sketch
  commands are in `app-sketching.md`.
- Highlight next and previous step a keyboard highlight through the scene's pickables in pick-table
  order (drawn and described like hover; a pointer move or Escape clears it), then, while a sketch
  is edited, through its constraints and dimensions in id order (`Pickable::SketchConstraint`).
  Outside sketch editing, while dimensions are shown on the model, they step through its dimension
  and value labels. Activate acts as a click would (selection toggle, or the region, blend edge,
  shell face or sketch plane action through `pick_action`); Enter opens what the item belongs to as
  a double-click would (a highlighted dimension's inline field), and with nothing highlighted
  confirms the open feature. With Trim, Extend, Mirror, Circular pattern or Sketch fillet active
  they step through that tool's targets instead (`app-sketching.md`), except while Mirror or a
  pattern gathers what it copies, when Activate toggles the highlighted item into the selection.
  With a drawing tool active, Activate on a highlighted point of the edited sketch or the origin
  places the next point there, snapped as a click would (`place_at_highlight` through
  `Drawing::type_point`), and on a highlighted curve at its middle (`Drawing::type_on_curve`); where
  the shape takes only points, or on anything else, it says what to highlight. With Smart dimension
  active, Activate picks as a click would and Enter adds the dimension of a single pick.
- What lies behind something is reached from the pick list (`pick_list.rs`): a primary press held
  still for `HOLD_TO_LIST` (not while drawing, Trim or the modifying tools, which take presses
  themselves) or List everything under the pointer (`view.list_under_pointer`; at the pointer, else
  where it last was in the view, else the view's centre) opens a popup listing every pickable under
  it, from `Scene::hits_through` (`render.md`) through `PickTable::listed` (each pick's tolerance
  and the selection filter), named as hover names them, one row per body with the Bodies filter.
  Rows are `widgets::menu_choice`s with `icons::pickable`; the first takes focus, and the pointed or
  focused row is drawn as hovered and described at once at the band's top left, as a keyboard
  highlight is (`described_in_band`), never beside the pointer under the list. Choosing one acts as Activate does
  (`ViewportState::choose`) but replaces the selection, adding with Shift or Ctrl. The press that
  opened it never counts as a click; with nothing under the pointer an info notice says so.
- The 3D view's context menu (`view_menu.rs`, `ViewportState::context_menu`) opens on egui's
  `secondary_clicked`, released within the click distance and time, so a right-drag that orbits or
  pans in any input mode never opens it. Where a click would select (`ViewportState::menu_selects`)
  it waits for the pick at the click's cursor and view like a primary click (`menu_waits_for_pick`,
  resolved on the CPU once the pointer's newer pick replaced it, `app.md`), then an unselected item
  under the pointer replaces the selection (`whole_body_of`), a selected one or empty space keeps it.
  Show the context menu (`Command::ContextMenu`, Shift+F10; egui has no Menu key) opens it at the
  keyboard highlight, else the selection's middle on screen (`screen_centre_of`,
  `BuiltScene::bounds_of`), else the view's centre.
- What it lists is a `view_menu::Place` fixed when it opens, with what the selection holds then
  (`view_menu::Held`, worked out once, including the one feature that made every selected item,
  `viewport::feature_of`, or a lone whole body's own). Tools taking the selection come from
  `Held::tools`; Suppress, Rename and Delete are named after that feature
  (`view_menu::FEATURE_COMMANDS`) and choosing one chooses its tree row first
  (`ViewportState::take_row_to_choose`) so the tree's own command acts next frame. A sketch's
  constraint offers go inline up to `MOST_INLINE_CONSTRAINTS`, else under Constrain. While a shape
  is drawn it offers `Command::TakeBackPoint`, `FinishShape` and `CancelShape`, as Backspace, Enter
  and Escape do mid-shape. Every entry is a `Command` drawn by `menu_bar::MenuEntries` from this
  frame's offers and keymap, so a disabled entry says why on hover and shows its keys; entries that
  apply only sometimes are left out rather than disabled.
- It opens downward from its spot, moved up and left by the size it measured the frame before
  (`widgets::measured_menu`, `menu_opening_down`). It is drawn after the frame's commands
  (`ViewportState::show_menu`), first one frame later so the offers follow a selection the click
  changed, then with its first available entry focused; a chosen command runs next frame through
  `Workspace::deferred_commands` (List everything under the pointer at the menu's spot, `list_at`).
  Escape, a click outside it (which does not also act in the view), a dialog, a document change or
  leaving its kind of place closes it.
- The selection filter (`SelectionFilter`, commands `select.*`, View › Selection filter) makes
  `PickTable::best_hit` and the highlight keys skip every pickable but one kind (whole bodies,
  faces, edges, vertices or sketch geometry), reference geometry included. Bodies picks faces but
  acts on the whole body: hover and highlight light every face of it, and a click, Activate or a box
  (`ViewportState::whole_body_of`) selects, or with Shift or Ctrl toggles, all its faces, which is
  how every tool reads a body. Cycle the selection priority (`select.priority`) sets the filter to
  the next of body, face and edge, from any other filter to body. It applies only while no sketch or
  tool is open and no plane is being chosen (`ViewportState::filter_applies`); the status bar names
  an active filter and its button clears it. Kept for the session, not saved.
- Outside sketch editing a primary drag in the 3D view draws a box (`PrimaryDrag::ModelBox`,
  `box_selection.rs`): left to right a window taking what lies wholly inside, right to left a
  crossing box taking what it touches, replacing the selection (Shift or Ctrl adds). It takes faces,
  edges, vertices or the curves of shown sketches by the selection filter, of shown bodies only.
  Where the display style picks no faces (Wireframe and X-ray, `DisplayStyle::picks_faces`),
  Everything takes edges (`Catch::of`) and nothing hides anything (`Occlusion::open`). Sketch curves
  go through `sectioned_screen::SectionedScreen`: a point a section plane cuts away is left out and a
  curve counts by the part the planes keep (`Screen::kept`, `kept_span`), so a curve cut away
  entirely is never taken, as picking never takes it. A shown body whose bounds project clear of
  the box (by `BOUNDS_SLACK`, `box_selection::may_reach`; kept when a corner is behind the eye) is
  passed over before anything else, so a box costs by what lies under it, not by the model.
  Faces count by their camera-facing triangles, and only when one that meets the box is seen
  (`triangle_is_seen`). Anything hidden behind a shown body is left out:
  `box_selection::Occlusion` rasterises the shown bodies' triangles into a depth map over the box
  (at most `MAX_CELLS` a side), a point counting as seen within `SLACK_CELLS` cells of depth of the
  nearest surface. A triangle is judged by the cells it covers (`Occlusion::sees_triangle`), its
  corners and middle only when it covers none; an edge is sampled every `SAMPLE_POINTS` on screen
  and counts when at least half its samples inside the map are seen (all of them when none is), so
  a face or edge reaching out of the box is never taken through its part outside it. A box under
  `SMALLEST_BOX` points takes nothing.
- Reaching through: Select through (`Command::ToggleSelectThrough`; `ViewportState::select_through`,
  session only) or Alt held during a model box, lasso or paint drag (`ViewportState::through_held`,
  read each frame, so letting go before the release takes only what is seen) swaps the depth map
  for `Occlusion::open`, so they also take what is hidden, faces turned away included (bodies too
  with the Bodies filter). Alt is the one modifier left: Shift and Ctrl add, and Alt pressed after
  the drag began reaches through in every input mode and past desktops that move windows on an
  Alt-drag, while Alt held at the press navigates in the Laptop mode (`render.md`). While a box,
  lasso or paint drag can hide anything (the style picks faces and the filter is not sketch
  geometry), the prompt says which it takes (`ViewportState::reach_prompt`: `TAKING_SEEN_PROMPT`
  with `HOLD_ALT_HINT`, or `TAKING_HIDDEN_PROMPT` with `LET_GO_OF_ALT_HINT` or how to turn Select
  through off) and a box or lasso reaching through is drawn with a double outline
  (`THROUGH_STROKE_WIDTH` in its colour, a `Chrome::panel` line inside), told by shape, not
  colour.
- Select with a lasso (`Command::ToggleLasso`; `ViewportState::lasso`, session only, since Alt-drag
  already navigates in the Laptop input mode) makes those drags, and the edited sketch's, draw a
  freehand outline (`sketch_drag::ScreenArea::Lasso`, a point every `LASSO_STEP` points, closed back
  to its start) taking what lies wholly inside, like a window.
- Select faces by painting (`Command::TogglePaintSelection`; `ViewportState::paint`, session only)
  makes model drags a brush (`PrimaryDrag::Paint`, `paint_selection.rs`) while the filter takes
  faces (Everything, Faces or Bodies) and the display style picks them (otherwise the box stays):
  the drag replaces the selection (Shift or Ctrl adds) and each pointer move adds the faces under the
  stroke, sampled every `BRUSH_STEP_POINTS` (at most `MAX_BRUSH_SAMPLES` a move) through
  `Scene::hits_through` on the CPU, so no GPU pick is awaited. Each sample takes the nearest face, or
  every face under it with Select through; the Bodies filter takes the whole body. Escape mid-drag
  restores the selection the drag started from. Sketch drags keep the box or lasso.
- Select all (`select.all`), Select tangent edges and Select edges around faces
  (`body_selection.rs`) work on shown bodies outside sketch editing and refuse inside one. Select all
  takes every face, edge or vertex by the filter, or by the kind already selected while the filter
  is Everything, and says what to choose when neither names one; seam edges are never taken.
  - Tangent edges add what `tangent_chain` reaches (smooth or sharp alike, unlike a blend's
    `blend_chain`); tangent faces (Alt+Shift+T) add what `tangent_faces` reaches; Select the whole
    hole (`select.hole`) adds what `hole_faces` reaches from a selected wall.
  - Select the whole body (`select.body`) and Invert the selection (`select.inverse`,
    `body_selection::invert`) work on every face, edge or vertex of the kind selected (or the
    filter's), inverting by whole bodies with the Bodies filter.
  - Select the faces of the same feature (`select.feature_faces`) adds, per body, every face whose
    `FaceOrigin` names the feature that made a selected one (`bodies::origin_feature`). Select the
    loop of the selected edge (`select.loop`) needs an edge and a face of its body selected
    (`loops_of`).
  - Select faces or edges like the selected ones (`select.similar`, Alt+Shift+A) adds, from the shown
    bodies, every face and edge matching a selected one's `similar::Shape`
    (`body_selection::similar_to`): a flat face by area (`measure::face_area_of`: exact once the
    body's mass properties arrive, the mesh's before, with the looser `MESH_SLACK`), a cylinder by
    radius and whether it is a hole wall (`hole_faces`), cone, sphere, torus, line, circle, arc,
    ellipse and other curves by their sizes; a free-form face matches nothing. Sizes match within a
    relative `EXACT_SLACK`. The notice counts what matches in all, selected ones included. Their
    availability is a cheap check on the selection; the work runs only when triggered.
- The display style (`DisplayStyle`, commands `view.style_*`) is shaded with edges, shaded with
  hidden edges dashed (each shown body edge stroked `Stroke::DashedWhereHidden`, so the renderer
  draws it again dashed where a face covers it, never picked there, with no second copy in the
  scene), shaded without edges (edges drawn with no alpha, so they stay pickable), wireframe (no
  faces in the scene, so none is drawn or picked), hidden lines removed (faces in
  `Scene::flat_meshes`, unlit in `ScenePalette::drawing_face`, still pickable and hiding what lies
  behind) or X-ray (faces in `Scene::translucent_meshes` at `XRAY_FACE_ALPHA`, unpickable). Every
  style that draws edges also gives each body a `Silhouette` in the edge colour and width (dashed for
  a troubled body, dimmed for background bodies), so the outline of curved faces is drawn as the
  view turns (`render.md`); with hidden edges dashed a coloured body's silhouette is
  `dashed_where_hidden` and never picks. It applies to bodies outside sketch editing, reaches the
  scene through `Sources::style` and `Revisions::style`, shapes image exports too and is kept for
  the session. X-ray also turns off: choosing it while it is the style returns to the style it
  replaced (`ViewportState::choose_style`; `set_style` remembers the last opaque one), so the View
  menu and the palette switch it both ways like the other toggles.

## Typed-point field

- `typed_point.rs`: with a drawing tool active, typing a digit, sign, point, `(`, `@`, `<` or `=`
  opens it. Letters are tool keys, so `=` is the way to start with a parameter's name: a leading `=`
  is left out of what is parsed (`typed_point::value_text`; `Typed::text` is the value,
  `Typed::entered` what was typed, which an error reopens). Type an exact value
  (`Command::TypeValue`, no default key) opens it empty, or resumes a waiting one, while a drawing
  tool, a modify tool's value field, Move, Rotate or Scale is active
  (`viewport::TYPE_VALUE_UNAVAILABLE` otherwise).
- It takes two length expressions split at top-level commas, a length and angle split at a top-level
  `<` (from the sketch's x axis; degrees unless a unit is named), `@` for an offset from the last
  placed point, and a lone length from the last point toward the pointer. The point must be within
  `MAX_LENGTH` of the origin. Offset's distance, Sketch fillet's radius and Move reuse it as one
  length expression (`modifying::Value`, `app-sketching.md`). Outside a sketch it opens the same way
  over a hovered or held handle of the open feature (`ViewportState::type_handle`,
  `app-modelling.md`, Value handles).
- An angle alone (`< 30`, `@< 30`; `typed_point::heading`) locks the direction from the last placed
  point (`Drawing::lock_heading`): `Drawing::place` projects the pointer onto that ray, ahead of
  snapping, Ctrl and the toggles; the preview draws a dashed guide, the prompt says so
  (`drawing::HEADING_PROMPT`) and a click places the point there, keeping the angle as a dimension
  like a typed one (`typed_hover`); a lone length typed meanwhile runs along the ray. Placing the
  point, Backspace, cancelling the shape or Escape (`Drawing::release_heading`) let go of it.
- A length followed by `<` with no angle (`12 <`, `@12<`; `typed_point::length_lock`) holds that
  distance from the last placed point (`Drawing::lock_length`, a `Lock::Length` of the same
  `Heading`): `Drawing::place` puts the point that far from it toward the pointer, ahead of snapping;
  the prompt says the length (the radius for round shapes) is locked (`held_noun`) and a click keeps
  it as a dimension. It is held only while the shape has just its first point (a spline from its
  first on), else `LENGTH_LOCK_REFUSED` in the field; zero, negative and out-of-range lengths and a
  missing last point are errors in the field. Release is as for the direction, and the field
  previews it (`Drawing::preview_length`).
- It is canvas chrome (`Chrome::panel`, `canvas::Hints`) in the band left of the view cube.
- While it has focus and its text reads as a point (`typed_point::parse_placed`) or a heading, the
  drawing previews it as a placement that is never placed (`Drawing::preview_typed`,
  `preview_heading`): the hover, the arc's sweep and the heading are kept aside (`Pointed`) and put
  back by the next `Drawing::hover`. Modify tools preview `TypedPoint::typing_text` the same way
  (`show_text`).
- Losing focus without Enter or Escape keeps the text, waiting: the chrome label in `muted`, the text
  in the theme's `text_muted`, `typed_point::WAITING_HINT` as hint in place of the prompt, and clicks
  acting at the pointer. Clicking the field, an opener typed (appended to the kept text, a leading
  `=` dropped) or Type an exact value resumes it; Escape in the view (first in
  `ViewportState::escape`), a change of edited sketch or tool (`ViewportState::typed_owner`) or
  leaving the context clears it.
- Enter places the point through `Drawing::type_point` (landing exactly on an existing or the
  pending point snaps to it, like a click); an error keeps it open with the reason; Escape closes it
  without touching the shape. It is handled after the drawing syncs with the displayed sketch each
  frame, so a typed chain carries on from the end the previous segment added.
- A point placed free keeps what was typed as dimensions, as typed (parameters included, a negative
  value as its magnitude, a zero left out): x and y as distances from the origin, or from the last
  placed point with `@` (`typed_point::parse_placed`, `TypedDimension`); a length as a distance; and
  an angle between 0° and 360° as an `Angle` from the horizontal axis to the line drawn from that
  point, when the shape draws one. `Drawing::type_dimensioned` keeps them with the placement and
  `Draft::finish` adds them to the shape's transaction against the points at those places, through
  the shadow check like any inferred constraint. Keep typed values as dimensions
  (`Command::ToggleTypedDimensions`; on by default, `ViewportState::typed_dimensions`, session only)
  turns this off.
