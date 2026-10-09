---
paths:
  - "crates/caditor/src/files.rs"
  - "crates/caditor/src/files/**"
  - "crates/caditor/src/portal.rs"
  - "crates/caditor/src/portal/**"
  - "crates/caditor/src/onboarding.rs"
  - "crates/caditor/src/export.rs"
  - "crates/caditor/src/configuration_export.rs"
  - "crates/caditor/src/image_export.rs"
  - "crates/caditor/src/drawing_export.rs"
  - "crates/caditor/src/import.rs"
  - "crates/caditor/src/import_panel.rs"
  - "crates/caditor/src/history.rs"
  - "crates/caditor/src/preferences.rs"
  - "crates/caditor/src/graphics.rs"
  - "crates/caditor/src/units.rs"
  - "crates/caditor/src/model.rs"
---

# Files, import, export, history, preferences

## File workflow

- `files.rs` owns the File menu and shortcuts, the recovery offer and the load report. Native
  dialogs run on their own thread; recovery scans and the other file checks run on the files
  worker, each job under `catch_unwind` so a panic becomes its failure. Opening and imports never
  use the files worker: each runs on a thread of its own (`Files::spawn_own`, named `open` or
  `import`, on the UI thread when none can be started), so a save, another open or an import is
  not queued behind a slow one.
- `portal.rs` holds the request and error types; `portal/xdg.rs` shows file dialogs on Unix
  through the XDG desktop portal over `zbus` (pure Rust, no `libdbus`) and falls back to `zenity`,
  `portal/windows.rs` through rfd, owned by the main window. On X11 the portal request names the
  main window as its parent (`x11:<id>`, `portal::own_dialogs` at startup) so the dialog stays
  above it; on Wayland it passes none, since exporting an `xdg-foreign` handle needs the raw
  Wayland connection, which only `unsafe` code could reach. A dialog that cannot be shown is a
  `DialogError` whose `notice` says what to install, never a silent Cancel.
- While a native dialog is open (`Files::picking`, a ticket and its purpose) the window is blocked
  and the status bar says it is waiting, with Stop waiting (and Esc) abandoning the pick as a
  cancel would; whatever that dialog answers later carries an old ticket and is ignored.
- Filters on Unix match either case (`*.[sS][tT][eE][pP]`), since GTK's portal matches globs
  case-sensitively, and Open and Import end with All files (`Filter::any`), leaving the decision
  to the content check. Every save dialog confirms replacing a file itself: the portals and rfd
  do, and zenity is passed `--confirm-overwrite` (needed by zenity 3, a no-op in zenity 4).
- The unsaved-changes prompt precedes every action that replaces or ends the document: Save
  primary and rightmost, Cancel beside it, the discarding choice a `danger_button` at the far left
  (`widgets::footer_split`). Quit waits for the storage worker in a "Closing…" modal without
  blocking the UI and without a close button, since quitting cannot be taken back.
- Opening shows a cancellable dialog. Each open counts an attempt (`Files::open_attempt`) and the
  result of an abandoned one is ignored. Cancelling, or starting another open, raises the open's
  `CancelToken`, which `caditor_file::load_cancellable` checks after reading the file, between
  records and while completing reference origins, ending in `LoadError::Cancelled`. Closing the window while a file is opening abandons the
  open the same way (with a notice saying so) before the unsaved-changes prompt is raised, so the
  prompt is never hidden behind the opening dialog and a load finishing later cannot replace the
  model the prompt is about.
- Save As appends `.caditor`, checks the target on the worker (refused while open in another
  window or holding unrecovered changes) and asks before replacing a file the dialog did not name.
- A save over a file another program changed (`Report::ChangedOnDisk`) asks: Save a copy primary,
  Cancel, Replace at the far left, not a danger button, since the outside contents become a
  version. A save started from the unsaved-changes prompt continues after Replace or Save a copy
  and is dropped on Cancel.
- A load or import report is one card listing each issue.
- Files dragged over the window (`drop_target.rs`, from egui's `hovered_files`, so only where the
  platform reports them: X11 and Windows) outline the view and say in a card what dropping them
  does, mirroring `Files::dropped`: open one model, import drawings and STEP files (into the sketch
  being edited for a DXF or SVG drawing), or a warning naming why the drop would be refused.
  Hovered files are judged by extension only (`has_importable_extension`), never read on the UI
  thread, since a stalled mount would freeze the window; an unknown extension warns that dropping
  reads it, and the import worker decides by content.

## Templates

- Templates are ordinary model files in the `templates` folder of the config directory
  (`files/templates.rs`, `Templates`). Its listing is read on the files worker at startup, when
  the File menu or Preferences opens and after a template is saved; nothing lists it on the UI
  thread, and the folder is made only before a dialog that needs it.
- File › New from template lists the templates by name, then New from template… (a dialog opened
  in the folder, accepting any model) and Save as template… (`Command::NewFromTemplate`,
  `Command::SaveAsTemplate`, both in the palette, no default keys). Starting from one passes the
  unsaved-changes prompt first, reads the file on an `open` thread behind the cancellable opening
  dialog ("Starting from …", `Origin::Template`) and replaces the model with an untitled, unchanged
  copy: no path, no lock or journal on the template, not added to recent files, so the template
  file is never written by editing. Parts that could not be read go in the report card; a template
  that cannot be read at all leaves a new empty model with a notice saying why.
- Save as template writes a copy of the document without its versions (`templates::save`, the
  files worker), the dialog suggesting the model's name in the folder; an appended extension asks
  before replacing as the other outputs do, and naming the open model's own file is refused in
  words. The model keeps its own path.
- Preferences › General › New models › Start from (key `files.default_template`, a file name in
  the folder, empty by default) makes New model start from that template (`Origin::DefaultTemplate`);
  `Files` reads it from every `Settings` it is handed. A default that is missing or unreadable
  starts an empty model and the notice points to Preferences; Preferences shows it "(not found)".
  The welcome's Start with an empty model (`FileCommand::NewEmpty`) and the model at startup stay
  empty.

## Onboarding

- The welcome dialog shows once (`onboarding.welcomed`) and from the Help menu. It offers an empty
  model (primary, through the unsaved-changes prompt unless the model is already empty and
  untitled), Open, the latest recent files and the samples. Its body scrolls when the window is
  short.
- Tips show one at a time at the bottom centre of the viewport while no dialog is open: the first
  `Hint` that applies to the model's state and was not dismissed. Dismissed ids are kept even when
  unknown (`onboarding.dismissed_hints`); Hide tips is `onboarding.hints`; Preferences restores
  both. Tip text uses the names the UI uses (the ribbon, Parameters).

## Export

- Export waits for a running recompute and warns when features failed (bodies export as last
  good) or a stopped recompute left features outdated; warnings and blockers are callouts shared
  with the image export (`export::warnings`).
- The dialog stays open behind the save dialog so cancelling the picker keeps the choices, and
  closes when the export starts. The export then runs on its own thread, cancellable from the
  status bar and the palette.
- STL adds an Encoding choice (binary by default, text keeping each body a named solid), each
  option saying what it costs on hover. A binary STL moved near the origin says by how much and
  how to move it back, in a notice that outlasts edits (`export::moved_note`; the command line
  prints it as a warning).
- A 3MF export first has its thumbnail drawn: the `Exporter` holds the job (`thumbnail_job`)
  until the session draws the bodies chosen, alone, framed from the initial view on a transparent
  background at `THUMBNAIL_SIZE` (`ViewportState::thumbnail` from the meshes already shown, through
  `snapshot::of_bodies`), with `Renderer::render_image`; the export thread reads the bands before
  writing. When the renderer is busy with an image export or drawing fails, the 3MF goes without
  one. The command line draws it offscreen when a graphics adapter exists.
- With two or more configurations (`document.md`) the dialog adds a Configurations choice: This
  configuration, or Every configuration (`Exporter::every_configuration`), which writes each
  configuration to a file of its own beside the chosen one, its name added to the stem with
  characters no file name may hold replaced (`configuration_export::file_for`: "bracket M4.stl").
  That export runs a `ConfigurationJob` on the export thread: for each configuration a copy of
  the document is switched to it (`Document::activating`), recomputed without display data (one
  `Recompute` for all, so unchanged features are reused) and its bodies not left out exported,
  the status bar saying which configuration of how many and Cancel stopping between and inside
  them, keeping the files already written. The model itself never switches. One notice lists the
  files and, per configuration, a refused switch, failed features or bodies left out. A 3MF
  written this way has no thumbnail, since the view shows only the active configuration.
- A path lacking the format's extension gets it appended, so an export never replaces a model
  file; when the appended name exists, `files.rs` asks "Replace …?" as Save As does, and Cancel
  returns to the export dialog.

## Drawing export

- Export sketch (File menu, palette; `Command::ExportSketch`) is offered by
  `feature_tree::commands` for the sketches among the features chosen in the tree, else the one it
  has current or that is edited (`drawing_export::exportable_sketches`; other kinds are ignored),
  and only once every one has solved geometry. Export face (`Command::ExportFace`,
  `drawing_export::face_commands`) is offered while one or more faces are selected, all flat (a
  curved one refuses in words). Export sketch also takes the flat faces selected in the view
  (`DrawingSource::with_faces`, `DrawingSource::Both`, titled "Export sketches and faces"), and
  Export face the sketches among the features chosen in the tree (`drawing_export::face_source`,
  refusing as Export sketch does while one has no solved geometry); the dialog offers the faces as
  a checkbox, on each time it opens, and the files worker writes both into
  one drawing (`caditor_file::export_drawing`) and announces it with `drawing_export::both_finished`.
- Both send `FileCommand::ExportDrawing` with a `DrawingSource`, which opens the drawing export
  dialog (`drawing_export::dialog`, `DrawingExporter` in `Files`, `FileCommand::DrawingExport`):
  Layout side by side (the default, a single sketch keeping its coordinates) or nested on a sheet
  with a sheet width and spacing (length fields taking units and expressions, 600 mm and 5 mm by
  default) and turns (on by default: quarter turns and the 15° step that boxes a part smallest);
  Dimensions and names (Names for faces, Dimensions and names whenever a sketch is included), off by
  default; and, for sketches, the construction choice below. The choices last for the session.
  Export… picks a `.dxf` or `.svg` path (`Purpose::Drawing`, titled for the source, `.dxf`
  appended to any other and replacing asked as for the other outputs); the dialog stays open behind
  the save dialog and a refused replacement, and closes when the export starts. The files worker
  writes a clone of each displayed sketch, or each face found again by its key in the shown
  body's result (an `Arc`, not a copy), named after the feature or face.
- The sketch notice counts what was written, the dimensions and the construction curves left out
  or kept; the face notice counts curves and loops, says side by side or nested and how many
  became polylines; both say how many parts were wider than the sheet (`drawing_export.rs`).
- Keep construction geometry in drawings (`Command::KeepDrawingConstruction`, a File menu choice
  under the exports, palette, and the dialog's checkbox) makes Export sketch write construction
  curves on their own dashed layer instead of leaving them out; it is kept for the session in
  `Files`, not saved.

## Image export

- Choices (size, scale, background) are kept for the session, not saved. A side beyond
  `MAX_IMAGE_SIDE` disables Export with the reason on hover.
- The image is the scene drawn again at the image's size (`ViewportState::image`) with the camera
  and graphics settings in use, without hover, selection, keyboard highlights, grid or drawing
  preview; egui overlays (labels, annotations, view cube) are never in it. A wider image shows
  more at the sides, and sketch curves are faceted for the image's size (`app.md`).
- After the save dialog the job waits in `Files` (`image_job`); the session starts it with
  `Renderer::render_image` before the frame's own drawing and hands its `ImageBands` (as
  `RenderedRows`) to the PNG writer, then calls `advance_image` every frame (UI tests stand in
  with rows of their own). The PNG is streamed band by band on its own thread under
  `catch_unwind`, cancellable from the status bar and the palette; cancelling drops the bands,
  which stops the tiles still to draw.
- Failures say what to change (size, scale, anti-aliasing, folder).

## Import

- Import picks a DXF or SVG drawing (`DRAWING_IMPORT_EXTENSIONS`; `read_drawing` tells them apart
  by extension, else by content), a STEP model or a mesh, and reads it on its own thread. A drawing
  becomes one change to the edited sketch if the command was given there, else
  to a new sketch named after the file, which is entered. A STEP model becomes one change adding
  an import feature per body.
- A drawing that was read opens the import options dialog (`import_options.rs`, `Files::arranging`)
  before anything is planned: the unit to read its numbers in (`DrawingUnit`, as the file says
  by default, SI only), a scale, centring the outline on the origin and, for a new sketch, the
  plane (`Arrangement`), and, for a drawing of several layers, which layers to import (all by
  default, Import disabled while none is chosen). It shows the size the drawing will have, and a
  warning callout when the chosen layers hold more than `MAX_DRAWING_CURVES`
  (`Drawing::chosen_curve_count`); Cancel adds nothing, and a queue of dropped files waits behind
  it. A drawing that fails to read skips it and reports as
  before.
- An import's details (`import_panel.rs`) end with its placement: Placed in (World, or a
  coordinate system above it, `feature_fields::frame_row`, the description then saying the file's
  origin and axes are the system's), Turn about X, Y and Z, Move along X, Y and Z, then Scale (a
  plain factor above zero), all expressions (key `import-field`, `turn` or `offset` and the axis
  index, or `("scale", 0)`), each entered value one undoable `SetFeatureKind` named "Place
  <name>".
- Replace from file (`Command::ReplaceImport`, an import's details, its right-click menu, the
  palette on the tree's current import) picks a STEP or mesh file, reads it on an import thread and
  applies one `SetFeatureKind` putting its body in place of the import, keeping its placement, so
  features using the body keep it and find its faces again by name. A file of several bodies gives the one named like the
  feature (or like it before a " 2" suffix), else nothing with the reason; a drawing is refused.
- Reload import from its file (`Command::ReloadImport`, the import's details, its right-click menu,
  the palette) does the same with the path the import kept (`import::kept_source`, made absolute
  by `import::read_model`) without a dialog; a body imported before paths were kept, or whose file
  has gone, says so and points to Replace from file.
- Every import step (reading a drawing, a model, replanning after `Placement::Stale`, Replace and
  Reload) is one job on a thread of its own (`Files::start_import`, named `import`), spawned
  through `run_where_possible`: when the thread cannot be started the job runs on the UI thread,
  logged. Each job counts an attempt (`Files::import_attempt`) and carries it in its event; the
  result of a cancelled job is ignored, so a late result never clears or fills a newer one.
- Importing shows in the status bar with a Cancel button and the palette command Cancel the import
  (`Command::CancelImport`, offered only while a file is being read). Cancelling raises the job's
  `CancelToken` (the readers poll it, `file-import-export.md`), forgets the job at once, drops
  its result even if the reader ignores the token, clears dropped files still queued and says so
  in a notice. A pick still waiting on the file dialog is stopped with Stop waiting instead.
  Opening another model meanwhile is allowed: the import's result is dropped as from another
  session.
- The worker plans the change on the model as it was at the start (`import::plan_drawing` with
  the `Arrangement`, `Model::base`); the UI thread only commits it (`Model::commit`) and replans
  on `Placement::Stale`. Results arriving after another document opened are dropped.
- The file's notes (units, left-out objects, fitted curves, repaired edges) go in the report
  dialog, also when nothing imported.
- Dropped files (`FileCommand::Drop`): one `.caditor` model opens; otherwise they queue as imports
  into what was edited at the drop, each after the previous report closed. Mixed or multi-model
  drops, or drops during an import or dialog, are refused with a notice.

## Parameter files

- File › Import parameters… and Export parameters… (`Command::ImportParameters`,
  `ExportParameters`, also in the palette, no default keys; `files/parameters.rs`) move the
  model's parameters through a CSV file (`file-import-export.md`). Export is offered only while
  the model has parameters; the text is made on the UI thread from the document and written on
  the files worker, `.csv` appended to a name without it and replacing asked as for the other
  outputs, and a notice counts what was written.
- Import picks a file and reads it on an `import` thread; a result arriving after another model
  opened is dropped. The rows open a preview (`ParameterImportDraft`, a modal counted by
  `is_blocking`) planned by `Document::plan_parameter_import`: a row per parameter with a pill
  (New, Replaces, Kept, Same, Left out) and the expression, what it replaces or the reason it is
  left out, a warning callout counting values the model already has differently and an error
  callout counting rows left out. "Replace values the model already has" (on by default) replans
  with conflicts kept instead. Import replans against the model as it is then and applies one
  transaction ("Import parameters from <file>", undone in one step) with a notice summing up what
  was added, changed, kept and left out; Import is disabled with the reason when nothing would
  change. A file that cannot be read at all is a notice saying why.

## Model properties

- Model properties… (File menu, the model details under the title, palette) opens a modal
  (`model_properties.rs`, `Workspace::model_properties`, dropped with the session): a property
  grid of the one-line fields, focus on Title, and a multi-line Notes field. Save properties applies
  one `SetModelProperties` only when something changed; Cancel and Escape drop the draft.

## Version history

- File › Version history lists the file's versions newest first, read on the files worker, each
  with Restore and Keep; a damaged one shows a "Damaged" `status_pill` with the reason on hover
  and neither button. It has no footer.
- Keep / Stop keeping marks a version so the age-based thinning never removes it
  (`file-format.md`); a kept version shows the `icons::KEPT_VERSION` icon and the word "Kept", not
  colour alone. The mark is stored in the file, so it is a file-level change applied at once on the
  storage worker (`HistoryCommand::Keep`, `Model::keep_version`, `Report::VersionKept`), not a
  document change: it does not dirty the model, is not undoable by Undo (Stop keeping is its
  reverse) and works with unsaved changes, since the versions are the file's. While it runs the
  row shows a spinner and the other buttons are disabled (`VersionHistory::start_keeping`); the
  list is read again when it finishes, and a failure is a notice naming the reason. Like Restore,
  Keep is a button in each row reached by Tab, with no palette command of its own.
- Restore loads in the background and applies `Document::transaction_to` as one undoable change,
  so Undo brings back what was there; the next save keeps the replaced state as a version. A
  version that proves damaged when restored (`LoadError::VersionUnavailable`) is marked from then
  on (`VersionHistory::finish_restoring`). A restore finishing after another model opened says it
  was not restored.

## Preferences and units

- Preferences is a dialog of `PreferencesTab`s (`widgets::tabs`). The tab shown is
  `Workspace::preferences_tab`, kept for the session but not saved. Ctrl+Tab and Ctrl+Page Down
  switch tabs from anywhere in the dialog unless the shortcut editor is open over it. The body is
  as tall as the tallest tab shown so far, so switching never resizes it, capped by
  `dialog_parts::BodyRoom`.
- Choices of two to four options are `widgets::segmented` with hover text on every option
  (`preferences::choice`).
- The input mode is a `Navigation` preference (key `navigation.input_mode`); the Navigate tip and
  the Preferences hover texts describe the chosen mode (`InputMode::navigation_tip`).
- Length units are SI only (`ux.md`). The projection preference changes only when asked (perspective, orthographic or automatic, which
  `Command::AutomaticProjection` and Preferences choose and O leaves for perspective); standard
  views and sketching never change it. The interface scale is the egui zoom factor, with egui's keyboard
  zoom and quit shortcut off (`app::apply_appearance`).
- Graphics settings (`graphics::Graphics`, keys `graphics.*`) are read clamped to what is offered.
  An option the adapter cannot do is disabled with the reason on hover, never hidden, and a stored
  level the adapter lacks gets an info callout naming the one used. The tab ends with the adapter
  details (`graphics::Hardware`), the adapter preference (power saving or performance, applied at
  once by opening a new device) and Copy details for bug reports. Defaults are in code; the
  renderer side is in `render.md`.
- Restore defaults resets only the open tab (`PreferenceChange::Defaults`; General leaves
  shortcuts and tips alone). It asks first with a callout repeating what it restores, then offers
  Undo, which `Workspace::restored` backs (`Preferences::restoring`, `Preferences::undo`) until
  the tab changes, another preference changes or the dialog closes. The shortcut editor's reset
  works the same way.
- Changes apply at once and save on the files worker (a slider on release,
  `PreferencesCommand::Preview` until then).
- `Workspace` owns the `Preferences`; `Model` carries the length unit; `app::apply_preferences`
  hands the model its share at startup and after every change, and the session passes graphics
  settings to the renderer after each frame's actions (`Renderer::set_graphics`).
- The units are for display and input only; models stay unit-explicit. `Preferences::unit` is the
  `LengthUnit` and `Preferences::angle` an `AngleUnit` (degrees or radians, key `units.angle`);
  `Model::units()` pairs them as `Units`, which derefs to the length unit. A plain number typed for
  a length or an angle gets the chosen unit attached (`Units::attach_plain`; `Units::show` shows
  computed values in them), a plain value for an angle parameter gets the angle unit
  (`field::parameter_expression`), and a new angle dimension, the measure tool and the drawing
  readout use it too. A dimension measured from drawn geometry (`LengthUnit::measured`,
  `AngleUnit::measured`) is rounded to the model's precision, a micrometre or a thousandth of a
  degree, written in the chosen unit (six decimals in metres, five in radians), so creating it
  never moves the geometry it measured. Defaults that are round numbers of degrees (a revolve's turn, a pattern's
  angle) and mesh angles stay in degrees.
