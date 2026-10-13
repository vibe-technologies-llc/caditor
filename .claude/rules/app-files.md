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
  - "crates/caditor/src/version_preview.rs"
  - "crates/caditor/src/preferences.rs"
  - "crates/caditor/src/preferences/**"
  - "crates/caditor/src/graphics.rs"
  - "crates/caditor/src/units.rs"
  - "crates/caditor/src/model.rs"
  - "crates/caditor/src/drop_target.rs"
  - "crates/caditor/src/import_options.rs"
  - "crates/caditor/src/model_properties.rs"
  - "crates/caditor/src/window_export.rs"
---

# Files, import, export, history, preferences

## File workflow

- `files.rs` owns the File menu and shortcuts; `files/dialogs.rs` draws its dialogs (opening, the
  unsaved-changes prompt, both "Closing…" waits, replacing, changed on disk, the load report and the
  recovery offer). Native dialogs
  run on their own thread; recovery scans and other file checks run on the files worker, each job
  under `catch_unwind` so a panic becomes its failure. Opening and imports never use the files
  worker: each runs on a thread of its own (`Files::spawn_own`, named `open` or `import`, on the UI
  thread when none can start), so a slow one never queues a save.
- `portal.rs` holds the request and error types; `portal/xdg.rs` shows Unix dialogs through the XDG
  portal over `zbus` (no `libdbus`) with a `zenity` fallback, `portal/windows.rs` through rfd. The
  dialog's parent is `x11:<id>` on X11 (`portal::own_dialogs`) and an `xdg-foreign` handle
  `wayland:<handle>` on Wayland, exported by `caditor-wayland` (`wayland.md`) and handed over by
  `window_export.rs` once the compositor answered (none before, or without the protocol). A dialog
  that cannot be shown or that dies (`PortalClosed`, `ZenityKilled`) is a `DialogError` whose
  `notice` says what to install, never a silent Cancel; only the portal's cancel response and
  zenity's status 1 are cancels, which is what clears a save waiting to continue.
- While a native dialog is open (`Files::picking`: ticket, purpose, the model's `Since`) the window is
  blocked; Stop waiting (and Esc) abandons the pick as a cancel and a late answer carries an old
  ticket and is ignored. Close is drawn over dialogs (`window_frame::over_dialogs`), so the
  unsaved-changes prompt can be raised meanwhile; its Save abandons the pick first (`stop_picking`).
  A save waiting to continue (`after_save`) waits only for a save that was sent or a Save as pick that
  started (`pick`, `pick_save_path` drops the continuation otherwise); only the Save as pick's cancel
  drops it, so a later save never quits or opens for a long-answered prompt.
- Unix filters match either case (`*.[sS][tT][eE][pP]`; GTK's portal is case-sensitive); Open and
  Import end with All files (`Filter::any`), the content check deciding. Every save dialog confirms
  replacing itself (zenity gets `--confirm-overwrite`).
- The unsaved-changes prompt precedes every action that replaces or ends the document (Save primary,
  the discard a `danger_button` far left, `widgets::footer_split`). Quit waits for the storage worker
  in a "Closing…" `widgets::waiting_dialog` (no close button, Escape ignored); after
  `QUIT_ANYWAY_AFTER` it says the disk is slow and offers Quit anyway at the far left. While a model or image export or a drawing, parameter
  configuration or template write runs, `run(Intent::Quit)` closes nothing: `Files::waiting_for_exports` shows
  `finishing_exports` (a dialog: Don't quit is the primary, as are its close button and Escape) and `poll` closes storage once none is left (`stop_waiting_for_exports`), unless
  the model gained unsaved changes, which raises the prompt for `Intent::Quit`. Writes run on the
  files worker, which `App::finish` waits for only `SETTINGS_FLUSH`, so each is counted from
  `Files::spawn_write` to its event (`Files::writing`, `Event::written`). Cancel export and quit
  raises the exports' cancel; a write cannot be cancelled, so while only writes remain Quit anyway
  appears after `QUIT_ANYWAY_AFTER`. The close button and Quit command both reach the wait through
  `FileCommand::Quit`, which it ignores. `Files::hold_jobs` (test only) blocks the files worker.
- Opening shows a cancellable dialog. Each open counts an attempt (`Files::open_attempt`); an
  abandoned one's result is ignored. Cancelling, or starting another open, raises the open's
  `CancelToken`, which `caditor_file::load_cancellable` checks after reading, between records and
  while completing reference origins (`LoadError::Cancelled`). Closing the window mid-open abandons
  it (with a notice) before the prompt is raised.
- A result arriving after a pick, check or wait is held against the model as it was when the intent
  ran (`Since`: session and revision, taken by `run`, `pick` and `list_templates`). An open or
  template load (`Event::Opened`, `Event::TemplateLoaded`) replacing a model with new unsaved changes
  (`Since::has_new_unsaved_changes`) raises the prompt with `Intent::Replace` first. The Save As
  target check (`Event::SaveTargetChecked`), an output's check (`Event::OutputChecked`, Export again
  included) and the "Replace …?" confirmation (`confirm_replace`) carry the pick's session; arriving
  after another model opened they write nothing and say so (`not_written_for_another_model`).
- A recovery offer leaves the list only when its discard succeeded; a failure keeps the card, worded
  through `caditor_file::DiscardError` (`DiscardFailure` adds a crashed job). A journal already gone
  counts as discarded.
- A failed save (`Report::SaveFailed`, or a writer that stopped) raises its notice through
  `Model::fail_save`; the next successful save takes that notice off the screen when it is still the
  one shown, and leaves any other notice alone.
- Save As appends `.caditor`, checks the target on the worker (refused while open in another window
  or holding unrecovered changes) and asks before replacing a file the dialog did not name.
- The open thread and the Save As check wait for the model's storage handover
  (`Model::storage_handover`, `await_handover`, `file-journal.md`), then read journals through
  `Journals::of`, which re-asks `journal_for` every `IN_USE_POLL` while it answers `InUse` and the
  startup scan runs (`Files::scanning`, cleared by the scan job's `Scanning` guard), or within
  `IN_USE_RETRY` of the first ask (another window's scan), stopping on the open's cancel; only a
  journal still held after that is refused as open in another window.
- The startup scan is reconciled with the window's actions: `Event::Started` skips journals restored
  or discarded this run (`Files::settled_journals`) and any of the model open now (by its
  `Recovered::file`), so a stale copy never replaces later edits.
- A save over a file another program changed (`Report::ChangedOnDisk`) asks: Save a copy primary,
  Replace far left but not a danger button (the outside contents become a version). A save started
  from the prompt continues after either and is dropped on Cancel. The prompt keeps the refused
  model's session (`FileEvent::ChangedOnDisk`, `Files::changed_on_disk`); once another model is open
  `poll` drops it with a notice (`not_saved_for_another_model`), so Replace never writes another model
  over that file.
- Each dialog purpose remembers the folder last picked for the app session (`Folder`: open, import,
  export, drawing, parameter and configuration files; the image export keeps its own), else the model's folder, else
  the most recent file's. Save as and templates keep their own.
- Revert to the saved version (`Command::RevertToSaved`) applies `Document::transaction_to` the last
  saved document (`Model::saved_version`) as one undoable change; unavailable with the reason when
  never saved, unchanged, or the saved document is unknown after a recovery.
- Open recent entries are `widgets::removable_menu_item`s (`FileCommand::ForgetRecent`, palette
  `Command::ForgetRecent` per slot) that drop the entry, never the file. "Removed …" is posted only
  after the write succeeded (`Event::RecentForgotten`); a failed recent-list or preferences write is
  a notice (`Event::RecentNotSaved`, `PreferencesNotSaved`, worded through
  `caditor_file::UpdateFailure`). Missing entries are found on the files worker
  (`FileCommand::CheckRecent`, not found only, so a permission error does not count) at startup and
  on each File menu open, drawn muted "(not found)" but openable. Opening a gone recent model forgets
  it; `open_file` judges with the same `is_gone`, so an offline share stays listed.
- Files dragged over the window (`drop_target.rs`, egui's `hovered_files`: winit on X11 and Windows,
  `file_drops.rs` on Wayland, `wayland.md`) outline the view and card what the drop does, mirroring
  `Files::dropped`, or warn why it is refused. Judged by extension only
  (`has_importable_extension`), never read on the UI thread; the import worker decides by content.

## Templates

- Templates are model files in the config directory's `templates` folder (`files/templates.rs`,
  `Templates`), listed on the files worker at startup, on opening the File menu or Preferences and
  after a save; the folder is made only before a dialog that needs it.
- `Command::NewFromTemplate` and `Command::SaveAsTemplate` have no default keys. Starting from a
  template passes the prompt, reads on an `open` thread (`Origin::Template`) and replaces the model
  with an untitled, unchanged copy: no path, lock or journal on the template, not in recent files,
  so editing never writes it. Unreadable parts go in the report; an unreadable template leaves a new
  empty model with a notice.
- Save as template writes a copy without versions (`templates::save`, files worker); an appended
  extension asks before replacing, naming the open model's own file is refused in words, and the
  model keeps its path.
- Preferences › General › New models › Start from (`files.default_template`, a file name, empty by
  default) makes New model start from it (`Origin::DefaultTemplate`); `Files` reads it from every
  `Settings` it is handed. A missing or unreadable default starts an empty model with a notice
  pointing to Preferences, which shows "(not found)". The welcome's Start with an empty model
  (`FileCommand::NewEmpty`) and the startup model stay empty.

## Onboarding

- The welcome dialog shows once (`onboarding.welcomed`) and from the Help menu: an empty model
  (through the prompt unless already empty and untitled), Open, recent files and samples.
- Tips show one at a time at the viewport's bottom centre while no dialog is open: the first `Hint`
  that applies and was not dismissed. Dismissed ids are kept even when unknown
  (`onboarding.dismissed_hints`); Hide tips is `onboarding.hints`; Preferences restores both. Tip
  text uses the names the UI uses.
- The start card on an empty model (`app-input.md`) is `onboarding.start_card` (on by default),
  Preferences › General › Starting and recent files › Show the start card on empty models, reset
  with that section.

## Export

- Export waits for a running recompute and warns when features failed (bodies export as last good)
  or a stopped recompute left features outdated; warnings and blockers are callouts shared with the
  image export (`export::warnings`).
- The dialog stays open behind the save dialog so cancelling keeps the choices, and closes when the
  export starts, which runs on its own thread, cancellable from the status bar and palette.
- A binary STL (the default encoding; text keeps each body a named solid) moved near the origin says
  by how much and how to move it back, in a notice that outlasts edits (`export::moved_note`; the
  command line prints a warning).
- A 3MF export first has its thumbnail drawn: the `Exporter` holds the job (`thumbnail_job`) until
  the session draws the chosen bodies alone, framed from the initial view on a transparent
  background at `THUMBNAIL_SIZE` (`ViewportState::thumbnail`, `snapshot::of_bodies`) with
  `Renderer::render_image`; the export thread reads the bands before writing. A busy or failing
  renderer means no thumbnail. The command line draws it offscreen when an adapter exists.
- With two or more configurations (`document.md`) the dialog adds Configurations: This configuration
  or Every configuration (`Exporter::every_configuration`), one file per configuration beside the
  chosen one, its name added to the stem (unsafe characters replaced, an empty name becoming
  "Configuration <position>"). `configuration_export::files_for` plans every file before the job
  starts, case-insensitively apart (" (2)", " (3)" on clashes). The plan travels in
  `Output::Export` from check to job (`Exporter::configuration_files`, `ConfigurationJob::files`), so
  checked files are written files; existing ones are asked about once ("Replace N files?",
  `Replacement::Output`'s `replaced`) and the chosen path itself is never written. The
  `ConfigurationJob` on the export thread switches a copy of the document to each configuration
  (`Document::activating`), recomputes without display data (one `Recompute`, reusing unchanged
  features) and exports the bodies not left out; Cancel stops between and inside configurations,
  keeping files written. The model never switches. One notice lists the files and per-configuration
  refused switches, failed features and left-out bodies. Such a 3MF has no thumbnail.
- With modelled threads in the model the dialog adds Threads (`ExportThreads`, in `ExportChoices`):
  Modelled (the default) or As shown. Modelled while the session shows them as cosmetic
  (`Exporter::recomputes_threads`) hands the job the document as stored and the export thread
  recomputes it without display data before exporting the chosen bodies (`Job::modelled`,
  `ExportReport::Modelled`, whose notice turns to a failure naming the features that failed while
  cutting). As shown exports the evaluation shown; every configuration recomputes the document as
  stored or as shown (`Exporter::exported_document`). The headless `--export` never shows threads
  as cosmetic, so it exports them modelled.
- A path lacking the format's extension gets it appended, so an export never replaces a model file;
  an existing appended name asks "Replace …?" as Save As does, and Cancel returns to the dialog.
- Every model export records a `LastExport` in `Files` (path, `ExportChoices`, model session) and, on
  success, each written file's size and modification time (`Stamp`, `ExportReport::written`). Export
  again (`Command::ExportAgain`, titled through `Command::title_with`) restores the choices and
  repeats the export without a dialog; the targets, planned again from the model as it is, are
  checked on the files worker, and a file whose stamp differs from what this export left is asked
  about with "Replace …?". Unavailable with the reason before an export in this session, while an
  export runs or the model recomputes, and for the model's own file.

## Drawing export

- Export sketch (`Command::ExportSketch`, `feature_tree::commands`) is offered for the sketches among
  the features chosen in the tree, else the current or edited one
  (`drawing_export::exportable_sketches`), once every one has solved geometry. Export face
  (`Command::ExportFace`, `drawing_export::face_commands`) is offered while faces are selected, all
  flat (a curved one refuses in words). Each takes the other's source too: Export sketch the flat
  faces selected in the view (`DrawingSource::with_faces`, `DrawingSource::Both`), Export face the
  sketches chosen in the tree (`drawing_export::face_source`); the dialog offers the faces as a
  checkbox, on each time it opens, and the files worker writes both into one drawing
  (`caditor_file::export_drawing`), announced by `drawing_export::both_finished`.
- Both send `FileCommand::ExportDrawing` with a `DrawingSource`, opening the dialog
  (`drawing_export::dialog`, `DrawingExporter` in `Files`, `FileCommand::DrawingExport`): Layout side
  by side (default, a single sketch keeping its coordinates) or nested on a sheet (width 600 mm and
  spacing 5 mm, units and expressions; turns on: quarter turns and the 15° step boxing a part
  smallest); Dimensions and names (off by default); for sketches the construction choice. Choices
  last for the session. Export… picks a `.dxf` or `.svg` path (`Purpose::Drawing`, `.dxf` appended to
  any other); the dialog stays open behind the picker and a refused replacement. The files worker
  writes a clone of each displayed sketch, or each face found again by key in the shown body's result
  (an `Arc`), named after the feature or face. Notices count what was written, what was left out or
  kept, polylines and parts wider than the sheet (`drawing_export.rs`).
- Keep construction geometry in drawings (`Command::KeepDrawingConstruction`, File menu, palette, the
  dialog's checkbox) writes construction curves on their own dashed layer; kept in `Files` for the
  session, not saved.

## Image export

- Choices (size, scale, background) last for the session. A side beyond `MAX_IMAGE_SIDE` disables
  Export with the reason on hover.
- The image is the scene drawn again at the image's size (`ViewportState::image`) with the camera,
  graphics settings and section planes in use (`app.md`), in the scene palette of the 3D view's
  canvas, without hover, selection, keyboard highlights, grid, drawing preview or egui overlays. A
  wider image shows more at the sides; sketch curves are faceted for the image's size.
- After the save dialog the job waits in `Files` (`image_job`); the session starts it with
  `Renderer::render_image` before the frame's own drawing, hands its `ImageBands` (as `RenderedRows`)
  to the PNG writer and calls `advance_image` every frame (UI tests stand in with rows of their own).
  The PNG streams band by band on its own thread under `catch_unwind`, cancellable from the status
  bar and palette; cancelling drops the bands, stopping the tiles still to draw.
- Failures say what to change (size, scale, anti-aliasing, folder).

## Import

- Import picks a DXF or SVG drawing (`DRAWING_IMPORT_EXTENSIONS`; `read_drawing` tells them apart by
  extension, else content, and gets `fonts::INTER` and `fonts::INTER_ITALIC` as
  `TextOutlines::InFont` so SVG text becomes outlines), a STEP model or a mesh, and reads it on its
  own thread. A drawing becomes one change to the edited sketch if the command came from there, else
  to a new, entered sketch named after the file. A STEP model becomes one change adding an import
  feature per body. `import::place_bodies` refuses in a notice, before it lands, a model whose text
  passes `MAX_MODEL_RECORDS` (`outgrows`) or whose bodies pass `MAX_RECORDS` features and parameters
  (`outnumbers`), since the rest would be lost on reopening; `Document::apply` refuses the same for
  other changes (`TooManyRecords`, `document.md`), this check only words the import notice.
- A read drawing opens the import options dialog (`import_options.rs`, `Files::arranging`) before
  anything is planned: unit (`DrawingUnit`, as the file says by default, SI only), scale, centring on
  the origin, for a new sketch the plane (`Arrangement`), and for several layers which to import
  (Import disabled while none). It shows the resulting size and warns when the chosen layers pass
  `MAX_DRAWING_CURVES` (`Drawing::chosen_curve_count`); Cancel adds nothing, a queue of dropped files
  waits behind it.
- An import's details (`import_panel.rs`) end with its placement: Placed in (World or a coordinate
  system, `feature_fields::frame_row`), Turn about and Move along X, Y, Z, then Scale (a factor above
  zero), all expressions (key `import-field`, `turn` or `offset` and the axis index, or
  `("scale", 0)`), each entered value one undoable `SetFeatureKind` named "Place <name>".
- Replace from file (`Command::ReplaceImport`) picks a STEP or mesh file, reads it on an import thread
  and applies one `SetFeatureKind` putting its body in place of the import, keeping its placement, so
  dependants keep it and find faces again by name. A file of several bodies gives the one named like
  the feature (or like it before a " 2" suffix), else nothing with the reason; a drawing is refused.
  Reload import from its file (`Command::ReloadImport`) does the same with the kept path
  (`import::kept_source`, made absolute by `import::read_model`) without a dialog; a body imported
  before paths were kept points to Replace. The UI thread touches no file for either: whether the
  kept file exists and is a model (`ReplacementSource::check`) is decided in the job, whose refusal
  (`SourceRefusal`, `Event::ReplacementRefused`) becomes the notice, so a gone share cannot freeze
  the window.
- Every import step (reading, replanning after `Placement::Stale`, Replace, Reload) is one job on a
  thread of its own (`Files::start_import`, named `import`, spawned through `run_where_possible`,
  which runs it on the UI thread, logged, if no thread starts). Each counts an attempt
  (`Files::import_attempt`) carried in its event; a cancelled job's result is ignored.
- The status bar shows progress and Cancel: each job carries a `ReadingProgress`
  (`Importing::progress`) the reader fills (`ModelReader`, `read_step_file_reporting`), then parts
  stored (`files::import_text`); `Command::CancelImport` (offered only while a file is being read)
  does the same. Cancelling raises the job's `CancelToken` (`file-import-export.md`), forgets the job
  at once, drops its result even if the reader ignores the token, clears queued dropped files and
  says so. A pick still on the file dialog is stopped with Stop waiting. Opening another model
  meanwhile is allowed; the result is dropped as from another session.
- The worker plans the change on the model as it was at the start (`import::plan_drawing` with the
  `Arrangement`, `Model::base`); the UI thread only commits it (`Model::commit`) and replans on
  `Placement::Stale`. Results arriving after another document opened are dropped.
- The file's notes (units, left-out objects, fitted curves, repaired edges) go in the report dialog,
  also when nothing imported.
- Dropped files (`FileCommand::Drop`): one `.caditor` model opens; otherwise they queue as imports
  into what was edited at the drop, each after the previous report closed. Mixed or multi-model
  drops, or drops during an import or dialog, are refused with a notice.

## Reference image files

- `FileCommand::InsertImage` (`files/pictures.rs`) picks a picture (`Purpose::Picture`, the import
  folder; `Dialogs::pick_picture` filters on `PictureFormat::extensions_of_all`, then All files) and
  reads it on an `import` thread (`read_picture_file`): the bytes, the format by content, the pixels
  and the `PictureTexture` for showing, so the UI thread never decodes. The answer
  (`Event::PictureRead`, dropped after another model opened) inserts the image (`app-modelling.md`)
  or says in a notice why the picture cannot be used ("Could not insert the picture “…”: it is an
  HEIC picture, which caditor cannot read yet; save it as PNG or JPEG.").

## Parameter files

- `Command::ImportParameters` and `ExportParameters` (`files/parameters.rs`) move the model's
  parameters through a CSV file (`file-import-export.md`). Export is offered only while the model has
  parameters; the text is made on the UI thread and written on the files worker, `.csv` appended,
  a notice counting what was written.
- Import reads on an `import` thread (a result after another model opened is dropped). The rows open
  a preview (`ParameterImportDraft`, a modal counted by `is_blocking`) planned by
  `Document::plan_parameter_import`: per parameter a pill (New, Replaces, Kept, Same, Left out), the
  expression and why it is replaced or left out, a warning callout counting values the model has
  differently and an error callout counting rows left out. "Replace values the model already has" (on
  by default) replans with conflicts kept instead. Import replans against the model as it is then
  and applies one transaction ("Import parameters from <file>") with a notice summing up added,
  changed, kept and left out (a warning when any row was left out); disabled with the reason when
  nothing would change. An unreadable file is a notice.

## Configuration files

- `Command::ImportConfigurations` and `ExportConfigurations` (`files/configurations.rs`) move the
  configurations table through a CSV file (`file-import-export.md`), from the palette and from the
  Configurations dialog's footer (Import from CSV…, Export to CSV…, `configurations::Outcome::File`).
  They share the parameter files' remembered folder. Export is offered only while the model
  has configurations (the reason is the disabled tooltip and the palette's), makes its text on the
  UI thread and writes it on the files worker (`WriteKind::Configurations`, counted by the quit
  wait), `.csv` appended, a notice counting the rows.
- Import reads on an `import` thread and opens a preview (`ConfigurationImportDraft`, a modal
  counted by `is_blocking`, drawn over the Configurations dialog) planned by
  `Document::plan_configuration_import`: a pill per column (Configured, New value, Left out with
  the reason, so unknown names are listed), a pill per row (New, Replaces, Kept, Same, Left out)
  with the cells that change as "Parameter: width: 70 mm (was 60 mm)", a warning callout counting
  configurations already in the model with other values and error callouts counting columns and rows
  left out. "Replace configurations the model already has" (on by default) replans with them kept.
  Import replans against the model as it is then and applies one transaction ("Import
  configurations from <file>"), with a notice summing up added, changed, configured, kept and left
  out (a warning when any row or column was left out); disabled with the reason when nothing would
  change. A file without a Configuration column is a notice.

## Model properties

- Model properties… (File menu, the details under the title, palette) opens a modal
  (`model_properties.rs`, `Workspace::model_properties`, dropped with the session): a property grid
  of the one-line fields, focus on Title, and a multi-line Notes field. Save properties applies one
  `SetModelProperties` only when something changed; Cancel and Escape drop the draft.

## Version history

- File › Version history lists the file's versions newest first, read on the files worker, each with
  Restore, Keep and Preview (per-row buttons reached by Tab, no palette commands); a damaged one
  shows a "Damaged" `status_pill` with the reason on hover and no button. No footer,
  `DialogWidth::Wide`.
- Preview (`HistoryCommand::Preview`; `version_preview.rs`) shows a card under the list: a picture of
  the bodies, feature and body counts, failing features and unreadable parts (warning callouts), and
  how it differs from the model now by stable id (`Changes::between`; lists cut at `MAX_NAMED`),
  plus Restore this version. It never touches the document or undo history. The version is read
  (`caditor_file::load_version_cancellable`), recomputed with display data and meshed
  (`snapshot::take_unless`) on a `preview` thread (`Files::spawn_own`) with a spinner, the stage
  (`version_preview::Progress`) and Cancel (`HistoryCommand::StopPreview`). Choosing another version,
  Hide preview, closing, or a listing where the version is gone or changed raises its `CancelToken`;
  a result with an older ticket is dropped. Failures are in words (`PreviewFailure`,
  `PictureFailure`) and the rest of the card still shows.
- The picture is drawn like the 3MF thumbnail: the prepared `Snapshot` (initial viewpoint fitted to
  the bodies, no grid or highlights) waits until the session draws it at `PICTURE_SIZE` with
  `Renderer::render_image`, in the scene palette of the canvas in use when Preview was pressed
  (carried by `HistoryCommand::Preview` to `version_preview::prepare`; `Files::preview_picture`,
  `preview_rendered`; a busy renderer leaves it waiting). Bands are read on a `preview` thread into
  an egui texture (`read_picture`), named for screen readers. Thumbnails are computed on demand, not
  stored: storing would cost every save a render and a format change, while any restorable version
  can be recomputed.
- `Files::poll` compares again whenever the session or revision changes
  (`VersionHistory::compare_preview`), and says it is not compared while another model is open.
- Keep / Stop keeping marks a version so age-based thinning never removes it (`file-format.md`),
  shown by `icons::KEPT_VERSION` and the word "Kept". The mark lives in the file, so it is a
  file-level change applied at once on the storage worker (`HistoryCommand::Keep`,
  `Model::keep_version`, `Report::VersionKept`), not a document change: it does not dirty the model,
  Undo does not undo it (Stop keeping reverses it) and it works with unsaved changes. The row shows a
  spinner and other buttons are disabled (`VersionHistory::start_keeping`); the list is re-read on
  finish, a failure is a notice.
- One restore or Keep runs at a time (`VersionHistory::is_idle`): meanwhile every Restore and Keep,
  the preview's included, is disabled with `history::ANOTHER_BEING_CHANGED`, and `start_restoring`
  and `start_keeping` refuse, so `finish_restoring` always marks the version it started.
- Restore loads in the background and applies `Document::transaction_to` as one undoable change; the
  next save keeps the replaced state as a version. A version that proves damaged when restored
  (`LoadError::VersionUnavailable`) is marked from then on (`VersionHistory::finish_restoring`). A
  restore finishing after another model opened says it was not restored.

## Preferences and units

- Preferences is a dialog of `PreferencesTab`s (General, Appearance, Navigation, Sketching,
  Graphics; `widgets::tabs`), drawn by `preferences/dialog.rs` (General and Appearance) and
  `preferences/motion.rs` (Navigation and Sketching), with Graphics in `graphics::tab`;
  `preferences.rs` holds the settings and their keys, `preferences/choices.rs` the choices read
  from text (`Choice`: `ALL`, `key`, `label`, `description`, `read` falling back to the default for
  a missing or unknown key). The tab shown is `Workspace::preferences_tab`, kept for the session,
  not saved. Ctrl+Tab and Ctrl+Page Down switch tabs unless the shortcut editor is open. The body
  is as tall as the tallest tab shown so far, capped by the room the dialog leaves it
  (`widgets::DialogBody::min_height`), with the tabs and search field above and the undo note,
  confirmation and footer below staying in view. Choices of two to four options are
  `widgets::segmented` with hover text on every option (`preferences::choice`, `pick` for a
  `Choice`), on and off settings `preferences::toggle`, which keeps a label that fits beside its
  box unwrapped (`fits_on_one_line`): a wrapping egui checkbox in a property grid makes its row
  taller than it draws.
- Every block of settings is a `preferences::Section` (`preferences/sections.rs`: tab, id, title,
  the words a search matches, and `copy`, which moves exactly its settings from one `Preferences`
  to another), drawn through `preferences::section` as a `widgets::section` holding a card. The
  search field under the tabs (`SEARCH_FIELD`, `Workspace::preferences_search`, cleared by a tab
  change or closing) shows, from every tab in turn under the tab's name, the sections whose title
  or words hold every term typed (`Section::matches`), else an empty state with Clear the search.
  A resettable section that differs from its defaults (`Section::is_default`) has a reset action in
  its header (`reset_name`, `icons::RESET_SECTION`), sending `PreferenceChange::SectionDefaults`
  at once; Keyboard, Tips and Saving on Windows have none.
- The input mode is a `Navigation` preference (`navigation.input_mode`); the Navigate tip and
  Preferences hover texts describe it (`InputMode::navigation_tip`), the canvas key hint names its
  buttons (`InputMode::navigation_hint`).
- Length units are SI only (`ux.md`). The projection preference (perspective, orthographic or
  automatic via `Command::AutomaticProjection` and Preferences; O leaves for perspective) changes
  only when asked; standard views and sketching never change it. The interface scale is the egui zoom
  factor, with egui's keyboard zoom and quit shortcut off (`app::apply_appearance`).
- Graphics settings (`graphics::Graphics`, keys `graphics.*`) are read clamped to what is offered. An
  option the adapter cannot do is disabled with the reason on hover, never hidden, and a stored level
  the adapter lacks gets an info callout naming the one used. The tab ends with the adapter details
  (`graphics::Hardware`), the adapter preference (Auto default, power saving or performance, applied
  at once by opening a new device when what it resolves to differs; Auto says what it chose and why,
  `graphics::auto_summary`) and Copy details for bug reports. Defaults are in code; the renderer side
  is in `render.md`.
- Restore defaults resets only the open tab (`PreferenceChange::Defaults`: its resettable sections,
  `Section::restored_with`; General leaves shortcuts and tips alone). It asks first with a callout
  repeating what it restores, then offers Undo, which `Workspace::restored` backs
  (`Restored::Defaults` with a `Restoring` tab or section, `Preferences::restoring`,
  `Preferences::undo`) until the tab changes, another preference changes or the dialog closes. A
  section's reset offers the same Undo without asking first. The shortcut editor's reset works the
  same way.
- Keys added with the settings a broad user base expects, each read leniently (a missing, unknown
  or mistyped value is today's behaviour) and each wired end to end: `navigation.zoom_toward`
  (`pointer`, `centre`), `navigation.orbit_around` (`pointer`, `centre`),
  `navigation.invert_orbit_x` and `_y`, `navigation.transitions` (`off`, `quick`, `standard`,
  `slow`: `Transitions::scale` of the camera's transitions), `navigation.field_of_view`
  (`narrow`, `standard`, `wide`: 20°, 30°, 45°), `appearance.tooltip_delay` (`at_once`,
  `standard`, `later`, applied over the installed styles by `app::apply_appearance`),
  `view.edges` (`thin`, `standard`, `thick`), `view.outlines`, `view.view_cube`,
  `view.principal_when_needed` (on unless stored off, `app-modelling.md`), `sketch.look_at`,
  `sketch.escape_finishes`, the starting states `sketch.glyphs`, `sketch.snapping`,
  `sketch.grid_snapping`, `sketch.typed_dimensions` and `model.dimensions`
  (`StartingToggles`, set on the viewport at startup and again when one of them changes, the
  toggles staying session state), `files.startup` (`empty`, `last`), `files.recent_listed` (`3`,
  `5`, `10`), `tree.double_click` (`edit`, `rename`) and `modelling.new_bodies` (`one_body`,
  `body_each`: `NewBodies`, shown in the Feature tree section as New bodies from separate pieces,
  handed to `Model::set_new_body_pieces` by `apply_preferences`; whether a new extrusion, revolve or
  sweep starts with `Pieces::BodyEach`, `app-modelling.md`). Recovery's journal and atomic saving have
  no setting (`reliability.md`).
- `Files` reads `files.startup` and `files.recent_listed` from every `Settings` it is handed, as it
  does the default template. `Files::recent` lists only the newest `RecentListed::count` (the menu,
  the welcome and the palette's recent slots); the stored list keeps `RECENT_LIMIT`. With The last
  model, `Files::start` without a file to open marks the newest recent model to open, and
  `Event::Started` opens it (`open_last_model`) only while the model is untitled and unchanged,
  nothing is opening and no recovery was offered.
- Changes apply at once and save on the files worker (a slider on release,
  `PreferencesCommand::Preview` until then). The palette's Recent commands are kept the same way
  (`palette.recent`, `app-input.md`) but are no preference: Restore defaults leaves them alone.
- Appearance keys: `appearance.theme` (`system`, `dark`, `light`, `midnight`, `graphite`, `paper`,
  `slate`, `studio` or `user:<file stem>`), `appearance.accent` (`theme` or a hue),
  `appearance.view` (`theme` or a canvas key: `dark`, `grey`, `studio`, `blue_grey`, `light`,
  `white`) and `appearance.grid` (`automatic` or `#rrggbb`), read leniently: an unknown or missing
  value is the default. Restore defaults on Appearance resets them all. The themes folder is listed
  like the templates (`app-look.md`).
- Lenient keys, all left alone by Restore defaults and handed over by `apply_preferences`: recent
  custom colours (`Preferences::recent_colours`, `body.recent_colours`, hex strings newest first, at
  most `RECENT_COLOURS_LIMIT`, set by `PreferenceChange::RecentColour`, which leaves out the fixed
  swatches and moves a repeated colour to the front; unreadable or repeated entries skipped, written
  only when any exist; read by the body card through `Model::recent_colours`, `app-look.md`), and the
  starting values of new features (`Preferences::last_values`, `last_values::LastValues`, keys
  `modelling.last.<slot>` such as `modelling.last.fillet_size`, `app-modelling.md`; the text as typed,
  set by `PreferencesCommand::RememberLast` from each applied edit; a missing or empty key is no
  value; read through `Model::last_values`).
- `Workspace` owns the `Preferences`; `Model` carries the length unit; `app::apply_preferences` hands
  the model its share at startup and after every change, and the session passes graphics settings to
  the renderer after each frame's actions (`Renderer::set_graphics`). `Workspace::follow_preferences`
  hands the viewport its share after every change and undo (`ViewportState::set_navigation`,
  `set_view_details`, `set_sketching`, and `start_with` when a starting state changed); `app::show`
  hands `PanelState::double_click` over each frame.
- Units are for display and input only; models stay unit-explicit. `Preferences::unit` is the
  `LengthUnit`, `Preferences::angle` an `AngleUnit` (degrees or radians, `units.angle`);
  `Model::units()` pairs them as `Units`, which derefs to the length unit. A plain number typed for a
  length or angle gets the chosen unit (`Units::attach_plain`; `Units::show` shows computed values in
  them), a plain value for an angle parameter gets the angle unit (`field::parameter_expression`),
  and new angle dimensions, the measure tool and the drawing readout use it too. A dimension measured
  from drawn geometry (`LengthUnit::measured`, `AngleUnit::measured`) is rounded to the model's
  precision (a micrometre, a thousandth of a degree) in the chosen unit, so creating it never moves
  the geometry. Defaults that are round numbers of degrees (a revolve's turn, a pattern's angle) and
  mesh angles stay in degrees.
