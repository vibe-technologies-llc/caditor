---
paths:
  - "crates/caditor/src/files.rs"
  - "crates/caditor/src/onboarding.rs"
  - "crates/caditor/src/export.rs"
  - "crates/caditor/src/import.rs"
  - "crates/caditor/src/history.rs"
  - "crates/caditor/src/preferences.rs"
  - "crates/caditor/src/units.rs"
  - "crates/caditor/src/model.rs"
---

# Files, import, export, history, preferences

## File workflow

- `files.rs` is the file workflow: the File menu and shortcuts, the recovery offer and the load
  report, native dialogs through the XDG desktop portal (`rfd`) on their own thread; loading and
  recovery scans on a background worker (each job under `catch_unwind`, a panic becoming its
  failure).
- The unsaved-changes prompt precedes New, Open, Open Sample, Restore and Quit; Quit waits for the
  storage worker without blocking the UI (Quit Anyway after a few seconds).
- Save As appends `.caditor` to other names, checks the target on the worker (refused while open in
  another window or holding unrecovered changes) and asks before replacing a file the dialog did not
  name.

## Onboarding

- `onboarding.rs`. The welcome dialog shows until closed once (`onboarding.welcomed`) and from Help
  › Welcome and samples…; it offers an empty model (New, through the unsaved-changes prompt, unless
  the model already is empty and untitled), the samples and Open.
- Tips show one at a time as a card at the bottom centre of the viewport when no dialog is open, the
  first of those that apply and were not dismissed: start with a sketch (empty model), draw (edited
  sketch with no geometry), constrain (edited sketch that can still move), extrude or revolve (a
  sweepable sketch and no body), navigate (a body exists), the palette. Got it dismisses one
  (`onboarding.dismissed_hints`, unknown ids kept), Hide tips turns them off (`onboarding.hints`);
  Preferences turns them back on or restores the dismissed ones.

## Export

- `export.rs`. File › Export… (Ctrl+E): format (STL, 3MF or STEP), for meshes the resolution
  (showing the resulting deviation in millimetres), a checkbox per body, all on by default. It waits
  for a running recompute and warns when features failed (bodies export as last good). After the
  save dialog it runs on its own thread, with Cancel in the status bar. A path lacking the format's
  extension gets it appended, so an export never replaces a model file (`.stp` counts as STEP). The
  outcome is a notice with the body count and, for meshes, the triangle count.

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

- `history.rs`. File › Version History… (for a saved model) lists the file's versions newest first
  on the files worker as "Saved 2 hours ago (29 Sep 2026 at 14:03) after “Edit width”" (date in the
  system's time zone via `jiff`), marking damaged ones.
- Restore loads it in the background and applies `Document::transaction_to` (remove every feature
  and parameter, then insert the version's, keeping ID counters) as one "Restore earlier version"
  change (a refused one keeps its error; a late one, arriving after another model was opened, says
  it was not restored), so Undo brings back what was there. The next save keeps the replaced state
  as a version.

## Preferences and units

- `preferences.rs`, `units.rs`. File › Preferences… (Ctrl+,) sets the length unit (µm, mm, cm or m;
  SI only, `ux.md`); the `Appearance` (theme system, dark or light, the 3D view staying dark;
  interface size 75% to 200% in eighths, Ctrl+Plus/Minus/0, the egui zoom factor with egui's
  keyboard zoom and quit shortcut off; high contrast); orbit and zoom speed with the zoom direction;
  the projection (`navigation.projection`, perspective by default, also switched by Switch between
  perspective and orthographic, O, in the View menu and the palette; standard views and sketching
  never switch it by themselves, so the view only changes when asked); and opens the shortcut
  editor.
- Every text colour of the theme is tested against its background (4.5:1, and 7:1 for body text and
  pills in high contrast, whose button and focus outlines reach 3:1).
- Changes apply at once and save on the files worker (a slider on release,
  `PreferencesCommand::Preview` until then).
- `Workspace` owns the `Preferences`, `Model` carries the length unit so every panel can use it, and
  `Action::Preferences` is performed with the workspace.
- The unit is for display and input only; models stay unit-explicit. Values show in it
  (`LengthUnit::show`); a plain number typed for a length gets it attached (`2` becomes `2 cm`), as
  does a plain value for a length parameter (an angle one gets degrees,
  `field::parameter_expression`). Measured dimensions are written in it, new features start from
  round numbers in it, and the cursor readout resolves a hundredth of a millimetre in it.
