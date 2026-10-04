---
paths:
  - "crates/caditor/src/app.rs"
  - "crates/caditor/src/overlay.rs"
  - "crates/caditor/src/main.rs"
  - "crates/caditor/src/lib.rs"
  - "crates/caditor/src/fuzzing.rs"
  - "crates/caditor/src/model.rs"
  - "crates/caditor/src/bodies.rs"
  - "crates/caditor/src/offers.rs"
  - "crates/caditor/src/samples.rs"
  - "crates/caditor/src/about.rs"
  - "crates/caditor/src/logo.rs"
  - "packaging/**"
  - "crates/caditor/src/cli.rs"
  - "crates/caditor/src/scene.rs"
  - "crates/caditor/src/scene_cache.rs"
  - "crates/caditor/src/faceting.rs"
  - "crates/caditor/src/selection.rs"
  - "crates/caditor/src/measure.rs"
  - "crates/caditor/src/measure_panel.rs"
---

# App shell, model and bodies

## Structure

- `app.rs` is the winit `ApplicationHandler`; `overlay.rs` the egui integration drawn over the
  viewport, keeping a CPU copy of every egui texture so that, when the renderer's generation changes
  after a lost device, it builds a new egui renderer on the new device and uploads them whole again.
  `scene.rs` converts documents and results to a `Scene`.
- The UI is `menu_bar.rs`, the tool ribbon `toolbar.rs`, `status_bar.rs`, the side panel `panels.rs`
  with the feature tree (`feature_tree.rs`) and parameter table (`parameter_table.rs`), the viewport
  widget (`viewport.rs`: navigation, hover, selection) and the view cube (`view_cube.rs`).
- Selectables are `Pickable`s built from stable IDs. The hover remembers the cursor position and
  view its pick result was made for; a click that depends on it (anything but a drawing tool's)
  waits until a result for the current cursor and view has arrived, so it never acts on a stale
  hover.

## Model

- `Model` (`model.rs`) owns the `Editor` and `Recomputer` and carries the length unit; the UI gets
  `&Model` and returns `Action`s the app performs after its pass, so the UI never mutates the
  document directly.
- Changes submit a snapshot to the worker; features draw from the last good result, tinted if failed
  or outdated. Suppressed features and those below the rollback bar are not drawn or picked
  (`scene.rs` walks `Document::active_features`), so the view shows the model as of the bar.
- `Model` owns the file session: the path, the last saved document (the model is unsaved exactly
  when its document differs from it, and always when it is unknown, after restoring a journal that
  had been rebased), the journal's base and the entries since it (kept to restart a stopped
  `Storage`, and dropped up to a `Report::Rebased`) and the `Storage` worker, to which every change
  is recorded. While the journal cannot be written the status bar shows Not protected.

## Scene

- `ViewportState::build_scene` hands the renderer a cached scene (`scene_cache.rs`), never one
  built afresh each frame. The base scene (bodies, sketches, references, with their hover and
  selection colours and a `PickTable`) is rebuilt only when its content or its highlight changes:
  content is the document revision, the evaluation generation, `DisplayedSketches::generation`
  (a drag shown or dropped, `forget`), `BodyMeshes::generation` (a mesh arrived, changed or went,
  or the open blend or shell changed), the editing `Context` and the faceting level; highlight is
  the selection and the hovered items. An idle frame or a camera move reuses it whole.
- Previews, the trim preview and the measured line form a second batch, the overlay, rebuilt only
  when it differs from the last one; it carries no pick ids, so it never asks for a pick.
- Each rebuild of the base bumps a generation; a pick is asked for when the cursor, the view or
  that generation differs from the last pick's (`PickKey`), and the pick table travels with it as
  an `Arc`. No pick is asked for in a frame where the camera's viewpoint differs from the last
  request's (orbit, pan, zoom, an animation), so hover stays as it was while the view moves and one
  pick follows when it settles. The keyboard highlight's list of distinct pickables is worked out only when first
  needed after a rebuild.
- `SketchShapes` keeps each drawn sketch's faceted outlines and constraint palettes for the
  current content, so a hover or selection change only restyles and re-registers them.
- Image export builds its own scene without highlights at a level fitting the image's size,
  following the window's level where that is fine enough.

## Sketch faceting

- Sketch curves, previews and trim pieces are faceted to a chord tolerance (`caditor_sketch::
  Faceting`, `sketch.md`) set from the view (`faceting.rs`): a quarter of a pixel at the view's
  target distance, rounded down to a power of two of millimetres (`FacetLevel`). A level is kept
  while the wanted chord stays between it and four times it (never coarser than wanted), so
  curves are faceted anew once per halving while zooming in, once per two doublings while zooming
  out, and never on a small zoom back and forth. Without a view a fixed level (1/32 mm) is used.
- `scene::drawn_faceting` doubles the chord while the drawn sketches' curves would need more than
  `SKETCH_SEGMENT_BUDGET` (2^19) segments, so zooming far into a sketch of thousands of circles
  never exhausts memory.
- Picks come from the drawn polylines, hover highlights draw the same polylines wider, box
  selection tests the same faceting (`sketch_drag::within`), and snapping stays exact on the
  curves. Bounds and fitting sample curves at a fixed 3°.

## Sessions and startup

- The app is a library with a thin binary: `main.rs` only calls `caditor::run` in `lib.rs`, which
  holds the modules and the startup, so the fuzz workspace can reach app code through the
  `fuzzing` feature (`fuzzing.rs`: preferences read from settings, stored shortcuts).
- `run` starts with an empty model.
- Release builds unwind (`panic = "unwind"`), since containment relies on it, or `recompute.rs`
  fails to compile.
- The viewport fits (`BuiltScene::fit_all`) once the first recompute of a newly opened model
  (another `Model::session`) is up to date: to sketches and bodies if any, else the reference
  planes.
- IDs restart per document, so a new session clears the viewport's selection, hover, highlight,
  drawing state (`Workspace::sync`, at the start of each frame), the tree's selection and rename,
  the export dialog's left-out bodies.
- `run` calls `crash::protect`, which installs the panic hook and a SIGTERM/SIGHUP/SIGINT handler
  (`signal-hook`) flushing the journal before exit; the handler then stops the process by that
  signal. `tests/crash_flush.rs` re-runs its own binary as a child that records changes to a real
  `Storage` and panics, raises SIGTERM or is killed, and checks that a recovery scan restores
  every change.
- A panic while handling a window or user event (a frame in `Session::redraw`: polling, egui,
  `Model::perform`, scene building, drawing; or closing, a dropped file, an accessibility event) is
  caught in `App::contained` after the hook has flushed the journal. The window gets a fresh egui
  context (`Overlay::new`, accessibility again), `after_failed_frame` drops the workspace's
  transient state (selection, tools, sketch editing, palette, dialogs) and `Files::close_dialogs`
  the file dialogs (export, image, version history, reports, recovery; prompts guarding unsaved
  work stay), and a notice says so; the model and its undo history stay. A second failed frame in a row suppresses
  every feature as one undoable change (`Model::suppress_every_feature`), saying how to find the
  one at fault; after `GIVE_UP_AFTER_FAILED_FRAMES` in a row the app exits with the journal kept,
  so the next start offers it.
- `logging.rs`: each run logs to stderr and to its own file,
  `$XDG_STATE_HOME/caditor/logs/caditor-<seconds>-<pid>.log` (`caditor_file::SessionLog`,
  owner-only, cut at `MAX_LOG_SIZE`), ending with an end line on a normal exit, a reported
  failure or a termination signal. The panic hook logs the panic with a backtrace (release
  builds keep symbol names, `strip = "debuginfo"`). At start, a log whose process is gone and
  which has no end line is reported in a notice naming it, then marked reported; only the ten
  newest logs are kept (`LOGS_KEPT`).
- A failure `run` returns (no window, no renderer, a window failing frame after frame) is
  logged and, when stderr is not a terminal (a desktop launch), shown through `zenity`, `kdialog`
  or `notify-send`, whichever works first; `failure_text` names the cause, the `WGPU_BACKEND=gl`
  workaround when a `RenderError` is behind it, and the log.
- The recovery card offers Restore suppressed beside Restore: the same restore followed by
  suppressing every feature, for a model that made caditor stop.

## Redraws and frame pacing

- caditor draws only when something asks: an input or window event, a worker's wake, egui's
  requested repaint, or a frame that must follow (an action, a camera animation, a pick or an
  exported image in flight). Every such request goes through `Session::request_redraw`, never straight to the window.
- `FramePacer` (`graphics.rs`) holds them to the frame limit: `Graphics::frame_interval` is the
  chosen rate's interval, or for Match the display the monitor's refresh rate
  (`MonitorHandle::refresh_rate_millihertz`, read at startup and on move, resize and scale change)
  only while vsync is off, since `Fifo` already paces to it; None when unlimited or the rate is
  unknown. A request earlier than the next slot is scheduled for it (`next_repaint`, which
  `about_to_wait` turns into `ControlFlow::WaitUntil` and requests once due), otherwise drawn at
  once. Slots keep a steady cadence when frames arrive on time and restart from the frame after an
  idle spell, so the first frame after idling is never delayed and idle stays idle.

## Bodies

- Body meshes come from the recompute worker at its `MeshQuality` (`document-recompute.md`), smooth
  by default. The Curve smoothness preference (`graphics.curve_quality`, coarse or smooth) reaches
  it through `Model::set_mesh_quality`, at startup and on each change (`app::apply_preferences`):
  a different quality is sent with `Recomputer::set_mesh_quality`, also to a worker spawned again
  later, followed by a new submission, so every body is meshed again at it.
- `BodyMeshing` (`bodies.rs`, in `Model`) converts each body's final mesh (and the state before the
  open blend or shell, once `Model::mesh_before` finds it meshed) into a `ShadedMesh` with edge
  polylines (no seams) on its own worker, once per result keyed by its `Arc`; requested on each
  evaluation, pruned to results still shown. A panic leaves that body meshless; without a worker the
  conversion runs on the UI thread.
- `BodyMeshes` (viewport) takes each conversion on arrival, keeping the previous mesh until then, so
  the UI thread only uploads buffers; the first fit of a newly opened model waits for them
  (`Model::bodies_pending`).
- A face is `Pickable::Face` with a `FaceKey` (`FaceName` plus occurrence among same-named faces, in
  solid order), an edge `Pickable::Edge` with its `EdgeName`, a vertex `Pickable::Vertex` with a
  `VertexKey` (`VertexName` plus occurrence, like faces); found through the result's `NameIndex`
  (`find_face`, `find_edge`, `find_vertex` on the `SolidResult` from `bodies::shown`/`input`) and
  described in words from the `FaceOrigin` (for example "Extrude 1 side from Line 3"; a vertex as
  where its faces meet).
- Vertices are markers with no colour, so they pick (as points, winning over edges and faces
  nearby) but show only when hovered or selected. The conversion worker also lists each body's
  vertices and works out its `BodyMass` (the mesh's volume, area and centroid; `Exact` for flat
  faces and straight edges only, else the mesh's chord and the volume's bound, curved area times
  chord).
- While a sketch is edited, bodies are dimmed and not pickable.

## Offers

- `offers.rs` works out what the selection offers the toolbar and status bar (the flat face for New
  sketch, the model axis for Revolve and patterns, the datum plane and axis, the faces to shell,
  the body to pattern, the selection's descriptions) only when the selection, the model's
  revision, its evaluation (`Model::evaluation_generation`) or the length unit changes, not every
  frame.

## Measure

- The Measure command (I, the ribbon, View menu, palette) toggles `MeasureTool` in the `Workspace`;
  while open, `measure_panel.rs` draws a right-hand panel before the viewport. Measuring never
  changes the document.
- `Measurements` resolves the selection on the UI thread into points (vertices, sketch points, the
  origin) and edges or faces with their result's `Arc`, then measures on its own worker (newest job
  wins, a panic becomes a failed reading, without a worker it runs inline) only when the
  selection, revision or evaluation changes; an empty selection is read at once. Until the result
  for the current selection arrives the previous readout stays in the panel, dimmed
  (`Measurements::shown`, `Freshness::Stale`), and the header says Measuring…, so nothing jumps; the
  measured line in the view comes only from the current readout (`Measurements::readout`).
- A `Readout` is a card per item (position; length, radius, diameter, centre; area, exact for flat
  faces from the kernel, else from the display mesh; round faces' radii) and, for two items, a
  "Between them" card: distance and its X, Y and Z parts, centre to centre for two circles, axis to
  axis, the gap between parallel planes and the angle. More than two asks for fewer. Values are
  formatted in the length unit (`LengthUnit::measured_*`, a micrometre's resolution), approximate
  ones prefixed ≈ with a note. Values are selectable text and the header's Copy all copies the
  whole panel as text.
- The panel opens with `panel_header` (Measure, Copy all, Close); with nothing selected it shows the
  hint alone. Notes (approximations, a body waiting for its mesh) are callouts under their card,
  never inside it. Mass properties is a collapsible section counting its bodies. egui keeps the
  panel's width for the session; it is not stored in the preferences.
- Mass properties are read each frame from the bodies' `BodyMass`: the bodies of the selected
  faces, edges and vertices and the feature selected in the tree, else every shown body.
- The closest points are drawn in the view on the front layer (`scene::add_measurement`) with a
  label of the distance at their middle in `canvas::MEASURE`.

## Samples

- `samples.rs` builds three parametric models through the document API (always the current format):
  a two-hole plate (extrude), a flanged spool (full revolve about the sketch's vertical axis), an
  angle bracket (symmetric extrude, hole removed by a second one); fully constrained, dimensions
  naming parameters; a test recomputes each and checks its volume. Open sample (menu, palette,
  welcome dialog) opens one untitled and unmodified after the unsaved-changes prompt.

## About, command line, accessibility

- `about.rs`: Help › About caditor shows the logo beside the name and tagline, then version and
  licences. `cli.rs`: `caditor [FILE]`;
  `--version`, `--help` print and exit; unknown options, several paths refused. A `.dxf` or STEP
  file (by extension or header) goes to import as if dropped, anything else to Open.
  `caditor --export OUT [--resolution coarse|standard|fine] MODEL` (`headless.rs`) opens no window:
  it loads the model (`caditor_file::load`, load issues become warnings on stderr), recomputes it
  without display work (`Recompute::run_without_display`), exports every body that built to the
  format `OUT`'s extension names (STL, 3MF, STEP; bodies left out and failed features are warnings)
  and prints a summary; a model with no body, an unreadable file or an unknown extension is an
  error and writes nothing.
- AccessKit (`egui-winit`'s `accesskit` feature): the window is created hidden, the adapter attached
  (`Overlay::enable_accessibility`), then shown; `AppEvent::Accessibility` carries the adapter's
  requests to the overlay.
- Wayland app ID and X11 class are `about::APP_ID` (`caditor`), which must match the desktop entry's
  name.
- The logo is `packaging/caditor.svg`; `packaging/render-icons.sh` renders it with `rsvg-convert`
  into `packaging/icons/caditor-<size>.png`, committed beside it, so neither the build nor the
  release needs an SVG renderer. `logo.rs` embeds the 32 to 256 px renders: the 128 px one is the
  window icon (shown by X11 window managers; Wayland compositors take the icon from the desktop
  entry through the app ID), and `logo::show` draws the smallest render covering the requested
  size in physical pixels, one texture per size cached in egui's memory, hidden from AccessKit.
  A render that fails to decode logs a warning and leaves the window without an icon or the space
  blank; tests decode every render.
