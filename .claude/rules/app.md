---
paths:
  - "crates/caditor/src/app.rs"
  - "crates/caditor/src/overlay.rs"
  - "crates/caditor/src/main.rs"
  - "crates/caditor/src/lib.rs"
  - "crates/caditor/src/fuzzing.rs"
  - "crates/caditor/src/crash.rs"
  - "crates/caditor/src/logging.rs"
  - "crates/caditor/src/headless.rs"
  - "crates/caditor/src/model.rs"
  - "crates/caditor/src/bodies.rs"
  - "crates/caditor/src/offers.rs"
  - "crates/caditor/src/samples.rs"
  - "crates/caditor/src/about.rs"
  - "crates/caditor/src/logo.rs"
  - "crates/caditor/tests/**"
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

- `app.rs` is the winit `ApplicationHandler`; `overlay.rs` the egui integration over the viewport,
  keeping a CPU copy of every egui texture so a new renderer generation (lost device) rebuilds
  egui and re-uploads them whole. `scene.rs` converts documents and results to a `Scene`.
- Selectables are `Pickable`s built from stable IDs. The hover remembers the cursor position and
  view its pick was made for; a click that depends on it (anything but a drawing tool's) waits
  for a result for the current cursor and view, so it never acts on a stale hover.
- `Model` (`model.rs`) owns the `Editor`, the `Recomputer` and the length unit; the UI gets
  `&Model` and returns `Action`s the app performs after its pass. Features draw from the last good
  result, tinted if failed or outdated; suppressed and rolled-back ones are neither drawn nor
  picked (`Document::active_features`).
- `Model` also owns the file session: path, last saved document (unsaved exactly when the
  document differs from it, and always when unknown, e.g. after restoring a rebased journal), the
  journal's base and entries since it, and the `Storage` worker every change is recorded to.
  While the journal cannot be written the status bar shows Not protected.

## Scene

- `ViewportState::build_scene` hands the renderer a cached scene (`scene_cache.rs`), never one
  built afresh each frame. The base scene (bodies, sketches, references, hover and selection
  colours, a `PickTable`) is rebuilt only when its content or highlight changes; an idle frame or a
  camera move reuses it whole. Content is everything that can change what is drawn (document
  revision, evaluation, displayed sketches, body meshes, editing `Context`, faceting level);
  anything new that affects the drawing must feed that key.
- Previews, the trim preview and the measured line form a second batch, rebuilt only when it
  differs; it has no pick ids, so it never asks for a pick.
- A pick is asked for when the cursor, the view or the base's generation differs from the last
  (`PickKey`). None is asked while the camera's viewpoint moves, so hover stays put and one pick
  follows when it settles.
- `SketchShapes` caches faceted outlines and constraint palettes, so a hover or selection change
  only restyles them. Image export builds its own scene, without highlights.

## Sketch faceting

- Sketch curves, previews and trim pieces are faceted to a chord tolerance (`faceting.rs`) set
  from the view: a fraction of a pixel at the view's target distance, rounded down to a power of
  two of millimetres (`FacetLevel`). A level is kept while the wanted chord stays within a
  band above it (never coarser than wanted), so zooming back and forth never refacets.
- `scene::drawn_faceting` coarsens the chord while the drawn curves would need more than
  `SKETCH_SEGMENT_BUDGET` segments, so zooming into thousands of circles never exhausts memory.
- Picks, hover highlights and box selection (`sketch_drag::within`) use the drawn polylines;
  snapping stays exact on the curves. Bounds and fitting use the sketch crate's default segment
  angle, not the view's faceting.

## Startup, sessions and crashes

- The app is a library with a thin binary (`main.rs` calls `caditor::run`), so the fuzz workspace
  reaches app code through the `fuzzing` feature. `run` starts with an empty model.
- Release builds unwind (`panic = "unwind"`), since containment relies on it; `recompute.rs`
  fails to compile otherwise.
- The viewport fits once the first recompute of a newly opened model (new `Model::session`) is up
  to date and its bodies are meshed (`Model::bodies_pending`).
- IDs restart per document, so a new session clears everything holding them: viewport selection,
  hover, highlight and drawing state (`Workspace::sync`), the tree's selection and rename, the
  export dialog's left-out bodies. Anything new that holds IDs must be cleared there.
- `crash::protect` installs the panic hook and a SIGTERM/SIGHUP/SIGINT handler that flush the
  journal before exit. `tests/crash_flush.rs` re-runs its binary as a child that panics, raises
  SIGTERM or is killed, and checks a recovery scan restores every change.
- A panic while handling a window or user event (a frame, closing, a dropped file, an
  accessibility event) is caught in `App::contained` after the hook flushed the journal. The window
  gets a fresh egui context, `after_failed_frame` drops transient state (selection, tools, sketch
  editing, palette, dialogs, except prompts guarding unsaved work) and a notice says so; the
  model and undo history stay. A second failed frame in a row suppresses every feature as one
  undoable change, saying how to find the culprit; `GIVE_UP_AFTER_FAILED_FRAMES` in a row exits with
  the journal kept, so the next start offers it. The recovery card's Restore suppressed is the
  same restore followed by suppressing every feature.
- `logging.rs`: each run logs to stderr and its own owner-only file under the state directory
  (`caditor_file::SessionLog`, bounded by `MAX_LOG_SIZE`, `LOGS_KEPT` newest kept), with an end
  line on a normal exit, reported failure or termination signal. At start a log whose process is
  gone and which has no end line is named in a notice, then marked reported.
- A failure `run` returns (no window, no renderer, a window failing frame after frame) is logged
  and, when stderr is not a terminal, shown through `zenity`, `kdialog` or `notify-send`;
  `failure_text` names the cause, the log, and the `WGPU_BACKEND=gl` workaround for a
  `RenderError`.

## Redraws and frame pacing

- caditor draws only when something asks (input, a worker's wake, egui's repaint request, or a
  frame that must follow: an action, a camera animation, a pick or image in flight). Every request
  goes through `Session::request_redraw`, never straight to the window.
- `FramePacer` (`graphics.rs`) holds requests to the frame limit: the chosen rate's interval, or
  for Match the display the monitor's refresh rate only while vsync is off (`Fifo` already paces
  to it); none when unlimited or unknown. An early request is scheduled for the next slot
  (`next_repaint`, which `about_to_wait` turns into `ControlFlow::WaitUntil`). The first frame
  after an idle spell is never delayed and idle stays idle.

## Bodies

- Body meshes come from the recompute worker at its `MeshQuality` (`document-recompute.md`). The
  Curve smoothness preference reaches it through `Model::set_mesh_quality` at startup and on each
  change (`app::apply_preferences`), followed by a new submission so every body is meshed again.
- `BodyMeshing` (`bodies.rs`) converts each body's final mesh (and the state before the open blend
  or shell, `Model::mesh_before`) into a `ShadedMesh` with edge polylines on its own worker, once
  per result keyed by its `Arc`. A panic leaves that body meshless; without a worker it runs on
  the UI thread. `BodyMeshes` takes each conversion on arrival, keeping the previous mesh until
  then, so the UI thread only uploads buffers.
- A face is `Pickable::Face` with a `FaceKey` (`FaceName` plus occurrence among same-named
  faces), an edge `Pickable::Edge` with its `EdgeName`, a vertex `Pickable::Vertex` with a
  `VertexKey`, found through the result's `NameIndex` and described in words from the
  `FaceOrigin`, never by index.
- Vertices are colourless markers: they pick (winning over edges and faces nearby) but show only
  when hovered or selected. The conversion worker also computes each body's `BodyMass` (volume,
  area, centroid, bounding-box size), exact only for flat faces and straight edges, else the
  mesh's chord approximation, which the UI marks as approximate.
- While a sketch is edited, bodies are dimmed and not pickable.

## Offers

- `offers.rs` works out what the selection offers the toolbar and status bar only when the
  selection, the model revision, its evaluation or the length unit changes, not every frame.

## Measure

- The Measure command toggles `MeasureTool` in the `Workspace`; while open, `measure_panel.rs`
  draws a right-hand panel. Measuring never changes the document.
- `Measurements` resolves the selection on the UI thread into points, edges and faces (with their
  result's `Arc`), then measures on its own worker (newest job wins, a panic becomes a failed
  reading) only when the selection, revision or evaluation changes. Until the current result
  arrives the previous readout stays, dimmed (`Freshness::Stale`) under a Measuring header, so
  nothing jumps; the measured line in the view comes only from the current readout.
- A `Readout` is a card per item and, for two items, a "Between them" card; more than two asks for
  fewer. Approximate values are marked with a note, a callout under its card. Mass properties are
  read each frame from `BodyMass` for the selected items' bodies, else every shown body.
- The closest points are drawn on the front layer (`scene::add_measurement`) with a distance
  label in `canvas::MEASURE`.

## Samples

- `samples.rs` builds parametric models through the document API (so always the current format),
  fully constrained with dimensions naming parameters; a test recomputes each and checks its
  volume. Open sample opens one untitled and unmodified.

## About, command line, accessibility, packaging

- `cli.rs`: `caditor [FILE]`; a `.dxf` or STEP file (by extension or header) goes to import as if
  dropped, anything else to Open. `caditor --export OUT [--resolution ...] MODEL`
  (`headless.rs`) opens no window: it loads a caditor model or imports a STEP file, recomputes
  without display work, exports every body that built to the format `OUT`'s extension names and
  prints a summary; load issues and left-out bodies are warnings, a failed feature a warning that
  also makes the exit status 2 (`PARTIAL_EXIT_STATUS`), and a model with no body, a DXF drawing,
  an unreadable file or an unknown extension an error that writes nothing. The desktop entry also
  offers STEP (`model/step`) and DXF (`image/vnd.dxf`).
- AccessKit (`egui-winit`'s `accesskit` feature): the window is created hidden, the adapter
  attached, then shown. The status bar notice and the failed-features pill are live regions
  (`widgets::announced`: assertive for an error, polite otherwise). The viewport is named "3D
  view", and text painted over it (prompt, hover description, snap and measure labels) is also a
  `Label` node (`canvas::announce`), the prompt and keyboard-highlight description polite live
  regions.
- Wayland app ID and X11 class are `about::APP_ID`, which must match the desktop entry's name.
- The logo is `packaging/caditor.svg`; `packaging/render-icons.sh` renders it into
  `packaging/icons/`, committed, so neither build nor release needs an SVG renderer. `logo.rs`
  embeds some renders and `logo::show` draws the smallest covering the requested physical size,
  hidden from AccessKit. The window icon is a fixed render (X11 only; Wayland takes the icon from
  the desktop entry). A render that fails to decode logs a warning; tests decode every render.
