---
paths:
  - "crates/caditor/src/files.rs"
  - "crates/caditor/src/portal.rs"
  - "crates/caditor/src/portal/**"
  - "crates/caditor/src/onboarding.rs"
  - "crates/caditor/src/export.rs"
  - "crates/caditor/src/image_export.rs"
  - "crates/caditor/src/import.rs"
  - "crates/caditor/src/history.rs"
  - "crates/caditor/src/preferences.rs"
  - "crates/caditor/src/graphics.rs"
  - "crates/caditor/src/units.rs"
  - "crates/caditor/src/model.rs"
---

# Files, import, export, history, preferences

## File workflow

- `files.rs` owns the File menu and shortcuts, the recovery offer and the load report. Native
  dialogs run on their own thread; loading and recovery scans run on the files worker, each job
  under `catch_unwind` so a panic becomes its failure.
- `portal.rs` holds the request and error types; `portal/xdg.rs` shows file dialogs on Unix
  through the XDG desktop portal over `zbus` (pure Rust, no `libdbus`) and falls back to `zenity`,
  `portal/windows.rs` through rfd, owned by the main window. A dialog that cannot be shown is a
  `DialogError` whose `notice` says what to install, never a silent Cancel.
- The unsaved-changes prompt precedes every action that replaces or ends the document: Save
  primary and rightmost, Cancel beside it, the discarding choice a `danger_button` at the far left
  (`widgets::footer_split`). Quit waits for the storage worker in a "Closing…" modal without
  blocking the UI and without a close button, since quitting cannot be taken back.
- Opening shows a cancellable dialog. Each open counts an attempt (`Files::open_attempt`) and the
  result of an abandoned one is ignored.
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
  being edited for a DXF), or a warning naming why the drop would be refused.

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
- A path lacking the format's extension gets it appended, so an export never replaces a model
  file; when the appended name exists, `files.rs` asks "Replace …?" as Save As does, and Cancel
  returns to the export dialog.

## Sketch export

- Export sketch (File menu, palette; `Command::ExportSketch`, `sketch_export.rs`) is offered
  by `feature_tree::commands` for the sketch the tree has current or that is edited, and only once
  it has solved geometry. `FileCommand::ExportSketch` picks a `.dxf` or `.svg` path (`Purpose::Sketch`,
  `.dxf` appended to any other and replacing asked as for the other outputs), then writes a clone of the
  displayed sketch on the files worker; the notice counts what was written and the construction
  curves left out.

## Image export

- Choices (size, scale, background) are kept for the session, not saved. A side beyond
  `MAX_IMAGE_SIDE` disables Export with the reason on hover.
- The image is the scene drawn again at the image's size (`ViewportState::image`) with the camera
  and graphics settings in use, without hover, selection, keyboard highlights, grid or drawing
  preview; egui overlays (labels, annotations, view cube) are never in it. A wider image shows
  more at the sides, and sketch curves are faceted for the image's size (`app.md`).
- After the save dialog the job waits in `Files` (`image_job`); the session renders it with
  `Renderer::render_image` before the frame's own drawing and polls it each frame (UI tests stand
  in for the renderer). The PNG is written on its own thread under `catch_unwind`, cancellable
  from the status bar and the palette.
- Failures say what to change (size, scale, anti-aliasing, folder).

## Import

- Import picks a DXF or STEP file (by extension, else by content) and reads it on the files
  worker. A drawing becomes one change to the edited sketch if the command was given there, else
  to a new XY sketch named after the file, which is entered. A STEP model becomes one change adding
  an import feature per body.
- The worker plans the change on the model as it was at the start (`import::plan_drawing`,
  `Model::base`); the UI thread only commits it (`Model::commit`) and replans on
  `Placement::Stale`. Results arriving after another document opened are dropped.
- The file's notes (units, left-out objects, fitted curves, repaired edges) go in the report
  dialog, also when nothing imported.
- Dropped files (`FileCommand::Drop`): one `.caditor` model opens; otherwise they queue as imports
  into what was edited at the drop, each after the previous report closed. Mixed or multi-model
  drops, or drops during an import or dialog, are refused with a notice.

## Version history

- File › Version history lists the file's versions newest first, read on the files worker, each
  with Restore; a damaged one shows a "Damaged" `status_pill` with the reason on hover. It has no
  footer.
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
- Length units are SI only (`ux.md`). The projection changes only when asked; standard views and
  sketching never switch it. The interface scale is the egui zoom factor, with egui's keyboard
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
  readout use it too. Defaults that are round numbers of degrees (a revolve's turn, a pattern's
  angle) and mesh angles stay in degrees.
