---
paths:
  - "crates/caditor/src/app.rs"
  - "crates/caditor/src/overlay.rs"
  - "crates/caditor/src/main.rs"
  - "crates/caditor/src/model.rs"
  - "crates/caditor/src/bodies.rs"
  - "crates/caditor/src/offers.rs"
  - "crates/caditor/src/samples.rs"
  - "crates/caditor/src/about.rs"
  - "crates/caditor/src/cli.rs"
  - "crates/caditor/src/scene.rs"
  - "crates/caditor/src/selection.rs"
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
  or outdated.
- `Model` owns the file session: the path, the last saved document (the model is unsaved exactly
  when its document differs from it), the journal entries since then and the `Storage` worker, to
  which every change is recorded. While the journal cannot be written the status bar shows Not
  protected.

## Sessions and main.rs

- `main.rs` starts with an empty model.
- Release builds unwind (`panic = "unwind"`), since containment relies on it, or `recompute.rs`
  fails to compile.
- The viewport fits (`BuiltScene::fit_all`) once the first recompute of a newly opened model
  (another `Model::session`) is up to date: to sketches and bodies if any, else the reference
  planes.
- IDs restart per document, so a new session clears the viewport's selection, hover, highlight,
  drawing state (`Workspace::sync`, at the start of each frame), the tree's selection and rename,
  the export dialog's left-out bodies.
- `main.rs` installs the panic hook and a SIGTERM/SIGHUP/SIGINT handler (`signal-hook`) flushing the
  journal before exit.

## Redraws and frame pacing

- caditor draws only when something asks: an input or window event, a worker's wake, egui's
  requested repaint, or a frame that must follow (an action, a camera animation, a pick in
  flight). Every such request goes through `Session::request_redraw`, never straight to the window.
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
  solid order), an edge `Pickable::Edge` with its `EdgeName`; found through the result's `NameIndex`
  (`find_face`, `find_edge` on the `SolidResult` from `bodies::shown`/`input`) and described in
  words from the `FaceOrigin` (for example "Extrude 1 side from Line 3").
- While a sketch is edited, bodies are dimmed and not pickable.

## Offers

- `offers.rs` works out what the selection offers the toolbar and status bar (the flat face for New
  sketch, the model axis for Revolve and patterns, the datum plane and axis, the faces to shell,
  the body to pattern, the selection's descriptions) only when the selection, the model's
  revision, its evaluation (`Model::evaluation_generation`) or the length unit changes, not every
  frame.

## Samples

- `samples.rs` builds three parametric models through the document API (always the current format):
  a two-hole plate (extrude), a flanged spool (full revolve about the sketch's vertical axis), an
  angle bracket (symmetric extrude, hole removed by a second one); fully constrained, dimensions
  naming parameters; a test recomputes each and checks its volume. Open Sample (menu, palette,
  welcome dialog) opens one untitled and unmodified after the unsaved-changes prompt.

## About, command line, accessibility

- `about.rs`: Help › About caditor shows version, licences. `cli.rs`: `caditor [MODEL]`;
  `--version`, `--help` print and exit; unknown options, several paths refused.
- AccessKit (`egui-winit`'s `accesskit` feature): the window is created hidden, the adapter attached
  (`Overlay::enable_accessibility`), then shown; `AppEvent::Accessibility` carries the adapter's
  requests to the overlay.
- Wayland app ID and X11 class are `about::APP_ID` (`caditor`), which must match the desktop entry's
  name.
