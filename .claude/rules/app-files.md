---
paths:
  - "crates/caditor/src/files.rs"
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

- `files.rs` is the file workflow: the File menu and shortcuts, the recovery offer and the load
  report, native dialogs on their own thread; loading and recovery scans on a background worker
  (each job under `catch_unwind`, a panic becoming its failure).
- `portal.rs` asks the XDG desktop portal's `FileChooser` over `zbus` (pure Rust, no `libdbus`):
  it subscribes to the `Request.Response` signal at the handle its `handle_token` predicts before
  calling `OpenFile` or `SaveFile`, then reads the first `uris` entry as a local path (a remote
  URI is refused in words). When there is no session bus or no portal, it falls back to
  `zenity --file-selection`. A dialog that cannot be shown is a `DialogError`, never a silent
  Cancel: the files workflow shows its `notice`, which says what to install.
- The unsaved-changes prompt precedes New, Open, Open sample, Restore and Quit: Save (or Save as…)
  primary and rightmost, Cancel beside it, and Close without saving or Continue without saving as a
  `danger_button` at the footer's far left (`dialog_parts::split_footer`). Quit waits for the
  storage worker in a "Closing…" `dialog_parts::titled_modal` (no close button, since quitting
  cannot be taken back; a danger Quit anyway after a few seconds) without blocking the UI.
- Opening a model shows an "Opening “name”…" dialog whose Cancel and close button drop the open
  (`FileCommand::CancelOpen`): every open counts an attempt (`Files::open_attempt`) and a result
  from an abandoned one is ignored.
- File › Open recent lists each file name with its folder muted beside it (`~` for the home
  folder, `onboarding::folder_name`) and its path on hover, then Clear recent files
  (`Command::ClearRecent`, `FileCommand::ClearRecent`, also in the palette while there are any).
- A load or import report is one card listing each issue with an info icon, closed by Close.
- Save As appends `.caditor` to other names, checks the target on the worker (refused while open in
  another window or holding unrecovered changes) and asks before replacing a file the dialog did not
  name.
- A save over a file another program changed since it was opened (`Report::ChangedOnDisk`) opens
  "“name” was changed by another program" (`files::changed_on_disk`): Save a copy… (Save As)
  primary and rightmost, Cancel beside it, Replace at the far left (not a danger button, since the
  outside contents become a version). Replace resends the save with `replace_outside_changes`; a
  save started from the unsaved-changes prompt continues after Replace or Save a copy and is
  dropped on Cancel.

## Onboarding

- `onboarding.rs`. The welcome dialog shows until closed once (`onboarding.welcomed`) and from Help
  › Welcome and samples…; it offers an empty model (New, through the unsaved-changes prompt, unless
  the model already is empty and untitled; the primary, rightmost), Open… beside it, the recent
  files (the first five in a card, name and muted folder, path on hover, opening on click, with
  Clear recent files, which keeps the dialog open; left out while there are none) and the samples
  as clickable `card`s with an icon each (plate, revolve, extrude). Its middle scrolls when the
  window is too short (`dialog_parts::BodyRoom`).
- Tips show one at a time as a card at the bottom centre of the viewport when no dialog is open, the
  first of those that apply and were not dismissed: start with a sketch (empty model), draw (edited
  sketch with no geometry), constrain (edited sketch that can still move), extrude or revolve (a
  sweepable sketch and no body), navigate (a body exists), the palette. The card has the dialog
  surface and margins, a Tip title with a close button, and Hide tips left of the primary Got it.
  Got it or the close button dismisses one (`onboarding.dismissed_hints`, unknown ids kept), Hide
  tips turns them off (`onboarding.hints`); Preferences turns them back on or restores the
  dismissed ones. Tips call the bars the ribbon and the sketch ribbon and the parameter list
  Parameters, as the sample descriptions do.

## Export

- `export.rs`. File › Export… (Ctrl+E): format (STL, 3MF or STEP), for meshes the resolution
  (showing the resulting deviation in millimetres, each choice's deviation on its hover), both as
  `segmented` choices, and a checkbox per body, all on by default, in a card that scrolls
  (`list_height`) under Select all and Select none (`ExportCommand::IncludeAll`). It waits for a
  running recompute and warns when features failed (bodies export as last good); the failure and
  whatever blocks Export… (running, recomputing, no body chosen) are warning callouts above the
  footer (`export::warnings`, shared with the image export, as are the section headings and gaps).
  Without bodies it is an empty state closed by its title bar's close button. The dialog
  stays open behind the save dialog, so cancelling the picker keeps it and its choices; it closes
  when the export starts. After the save dialog it runs on its own thread, with Cancel in the status
  bar. A path lacking the format's extension gets it appended, so an export never replaces a model
  file (`.stp` counts as STEP); when that appended name already exists, `files.rs` asks "Replace …?"
  (`Output`, `Replacement`) like Save As does, since the native dialog only confirmed the name it
  was given, and Cancel returns to the export dialog. The outcome is a notice with the body count
  and, for meshes, the triangle count.

## Image export

- `image_export.rs`. File › Export image… (Ctrl+Shift+E): size (the 3D view's size in physical
  pixels, `ViewportState::view_pixels`, or a custom width and height, each a `commit_field` of
  `widgets::FIELD_WIDTH` taking whole pixels from 1 to `MAX_IMAGE_SIDE`), scale (1×, 2×, 4×) and
  background (the 3D view's or transparent), each a `segmented` choice with hover text. The dialog
  shows the resulting size, or a warning callout when a side passes 8192 (Export… disabled with the
  same reason on hover), and warns when features failed as the export does. The choices, and the folder last exported to,
  are kept for the session like the other export's, not saved.
- The image leaves out hover, selection and keyboard highlights, the grid and the drawing preview
  (`ViewportState::image` builds the scene again with an empty `Highlight` and no grid); egui's
  labels, annotations and view cube are never in it, since they are not in the scene. It keeps
  everything else the view shows (bodies, visible sketches, reference planes, axes and datums,
  which can be hidden first), the camera as it is now and the graphics settings in use. Its view
  is the camera at the image's size, so the vertical extent matches the view and a wider image
  shows more at the sides; pixels per point scale with the image's height over the view's, so
  lines and points grow with 2× and 4×. Sketch curves are faceted for the image's size (the
  view's level while that is fine enough, `app.md`), so a larger image shows no facets.
- The dialog stays open behind the save dialog and closes when the image starts, so cancelling the
  picker keeps its choices. After the save dialog (`Dialogs::pick_image_path`, `.png` appended when
  missing, so an export never replaces a model, and an existing file at the appended name asks
  "Replace …?" as the other export does) the job waits in `Files` (`image_job`); the session renders it with
  `Renderer::render_image` before the frame's own drawing and polls it each frame, redrawing while
  it is pending (the UI tests stand in for the renderer). The readback goes to `image_rendered`,
  which reads the pixels and writes the PNG on its own thread under `catch_unwind`, with Cancel
  in the status bar and the palette (`Command::CancelImageExport`).
- The outcome is a notice with the size and file name; failures say what happened and what to do
  next (a smaller size or scale, lower anti-aliasing, another folder, export again after the
  device was recovered). Clipboard copying is not offered: the app has no image clipboard.

## Import

- `import.rs`. File › Import… (Ctrl+I) picks a DXF or STEP file (by extension, else by whether it
  starts like STEP) and reads it on the files worker. A drawing becomes one "Import <file>" change
  to the edited sketch if the command was given there, else to a new XY sketch named after the file,
  which is entered. The files worker builds the change on the model as it was at the start
  (`import::plan_drawing`, through `Model::base`); the UI thread only commits it (`Model::commit`),
  remaking the plan if the model changed (`Placement::Stale`). A STEP model becomes one change
  adding an import feature per body.
- The outcome is a notice with the curve or body count; the file's notes (units, left-out objects,
  fitted curves, repaired edges) go in the report dialog used for a damaged file's problems, also
  when nothing imported. Results arriving after another document opened are dropped.
- Dropped files (`WindowEvent::DroppedFile`, gathered per frame into `FileCommand::Drop`): one
  `.caditor` model opens; otherwise they queue as imports into what was edited at the drop, each
  after the previous report closed; mixed or multi-model drops, or during an import or dialog, are
  refused with a notice.

## Version history

- `history.rs`. File › Version history… (for a saved model) opens "Version history", listing the
  file's versions newest first in a card, read on the files worker, as "Saved 2 hours ago (29 Sep
  2026 at 14:03) after “Edit width”" (date in the system's time zone via `jiff`), each with a
  Restore button, a damaged one with an error "Damaged" `status_pill` saying why on hover. It has
  no footer; its title bar's close button closes it.
- Restore loads it in the background and applies `Document::transaction_to` (remove every feature
  and parameter, then insert the version's, keeping ID counters) as one "Restore earlier version"
  change (a refused one keeps its error; a late one, arriving after another model was opened, says
  it was not restored), so Undo brings back what was there. The next save keeps the replaced state
  as a version.

## Preferences and units

- `preferences.rs`, `units.rs`, `graphics.rs`. File › Preferences… (Ctrl+,) is a dialog of
  `PreferencesTab`s (`widgets::tabs`): General, Appearance, Navigation and Graphics. The tab shown
  is `Workspace::preferences_tab`, kept for the session (`PreferencesCommand::Tab`) but not saved.
  Tabs switch by click, arrows, Home and End on a focused tab, and Ctrl+Tab, Ctrl+Shift+Tab,
  Ctrl+Page Down and Ctrl+Page Up anywhere in the dialog unless the shortcut editor is open over it.
  The body is as tall as the tallest tab shown so far in the session, so switching back never
  shrinks it, capped so the whole dialog stays within 75% of the window (`dialog_parts::BodyRoom`),
  and scrolls beyond that. Each section (`preferences::section`) is a collapsible
  `widgets::section` header over a `card` holding a property grid and a muted note; choices of two
  to four options are `widgets::segmented` with hover text on every option (`preferences::choice`).
  - General: Units (the length unit, µm, mm, cm or m; SI only, `ux.md`), Keyboard (Keyboard
    shortcuts…) and Tips (showing tips; Show dismissed tips again on its own row).
  - Appearance: Colours (theme system, dark or light, the 3D view staying dark; high contrast) and
    Interface (size 75% to 200% in eighths through `widgets::stepper`, named Make the interface
    smaller and larger, Ctrl+Plus/Minus/0, the egui zoom factor with egui's keyboard zoom and quit
    shortcut off; the title bar, caditor's or the system's, `app-look.md`).
  - Navigation: the projection (`navigation.projection`, perspective by default, also switched by
    Switch between perspective and orthographic, O, in the View menu and the palette; standard
    views and sketching never switch it by themselves, so the view only changes when asked); orbit
    and zoom speed with the zoom direction.
  - Graphics: `graphics::Graphics`, under `graphics.*`, read clamped to what is offered: vsync
    (`graphics.vsync`, on), frame rate (`graphics.frame_limit`: `unlimited`, `30`, `60`, `120`,
    `144` or `display`, the default; another number reads as the nearest rate; a combo box, its
    options' hover texts saying what each does), anti-aliasing
    (`graphics.msaa`, samples 1, 2, 4 (default) or 8; another number reads as the largest level
    within it; `dialog_parts::segmented_offered`, which can disable one), shading (`graphics.shading`, `standard` or `enhanced`) and curve smoothness
    (`graphics.curve_quality`, `smooth` or `coarse`). Each option has a caption and hover text;
    one the adapter cannot do (a level it does not offer, vsync off where the surface has only
    `Fifo`) is disabled with the reason on hover, and a stored level the adapter lacks gets an
    info callout naming the one used. The tab ends with the adapter, backend, driver and display
    rate (`graphics::Hardware`, which the session refreshes from the renderer and the monitor) and
    Copy details, for bug reports.
- Restore defaults (a `danger_button` at the footer's far left, Close the primary at its right)
  resets only the open tab (`PreferenceChange::Defaults(tab)`, its hover text listing the values);
  General leaves shortcuts and tips alone. It asks first: a warning callout repeating what it
  restores, over a footer of Restore defaults (danger, left) and Cancel (primary); the question is
  kept in egui memory per tab and dropped on closing. Once restored, a success note with Undo
  (`PreferencesCommand::Undo`) puts the tab's earlier values back: `Workspace::restored` holds them
  (`Preferences::restoring`, `Preferences::undo`) until the tab changes, another preference changes
  or the dialog closes.
- Every text colour of the theme is tested against its background (4.5:1, and 7:1 for body text and
  pills in high contrast, whose button and focus outlines reach 3:1).
- Changes apply at once and save on the files worker (a slider on release,
  `PreferencesCommand::Preview` until then).
- `Workspace` owns the `Preferences`, `Model` carries the length unit so every panel can use it, and
  `Action::Preferences` is performed with the workspace. `app::apply_preferences` hands the model
  its share (length unit, mesh quality) at startup and after every change; the session passes the
  graphics settings to the renderer after each frame's actions (`Renderer::set_graphics`).
- The unit is for display and input only; models stay unit-explicit. Values show in it
  (`LengthUnit::show`); a plain number typed for a length gets it attached (`2` becomes `2 cm`), as
  does a plain value for a length parameter (an angle one gets degrees,
  `field::parameter_expression`). Measured dimensions are written in it, new features start from
  round numbers in it, and the cursor readout resolves a hundredth of a millimetre in it.
