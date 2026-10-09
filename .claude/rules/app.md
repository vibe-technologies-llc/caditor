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
  - "crates/caditor/src/snapshot.rs"
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
  - "crates/caditor/src/scene_palette.rs"
  - "crates/caditor/src/faceting.rs"
  - "crates/caditor/src/selection.rs"
  - "crates/caditor/src/measure.rs"
  - "crates/caditor/src/measure_panel.rs"
  - "crates/caditor/src/interference.rs"
  - "crates/caditor/src/interference_panel.rs"
  - "crates/caditor/src/comb.rs"
  - "crates/caditor/src/comb_panel.rs"
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
  revision, evaluation, displayed sketches, body meshes, display style, `Contrast`, editing
  `Context`, faceting level); anything new that affects the drawing must feed that key.
- Previews, the trim preview and the measured line form a second batch, rebuilt only when it
  differs; it has no pick ids, so it never asks for a pick.
- A pick is asked for when the cursor, the view or the base's generation differs from the last
  (`PickKey`). None is asked while the camera's viewpoint moves, so hover stays put and one pick
  follows when it settles.
- `SketchShapes` caches faceted outlines and each entity's `SketchState`, so a hover or selection
  change only restyles them. Image export and the headless PNG build their own scene, without
  highlights and in the standard palette, since high contrast is a viewing aid.
- Every scene colour and line weight comes from the `ScenePalette` of the viewport's `Contrast`
  (`scene_palette.rs`, set from the High contrast preference each frame), never a constant in
  `scene.rs`; the overlay batch carries the contrast too. Colours shared with canvas labels
  (hover, selection, snap, measure, problems) stay `canvas` colours in both palettes.

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
  to date and its bodies are meshed (`Model::bodies_pending`), keeping the projection of the
  preferences. Fitting and the reference size count only what is drawn: sketches among
  `active_features` (not suppressed or past the bar) and not hidden, and shown bodies.
- IDs restart per document, so a new session clears everything holding them: viewport selection,
  hover, highlight and drawing state (`Workspace::sync`), the tree's selection and rename, the
  export dialog's left-out bodies. Anything new that holds IDs must be cleared there.
- `crash::protect` installs the panic hook and a SIGTERM/SIGHUP/SIGINT handler that flush the
  journal before exit; on Windows a console close handler, plus `crash::flush_when_the_session_ends`
  once the window exists (`windows.md`). `tests/crash_flush.rs` re-runs its binary as a child that
  panics, raises SIGTERM (Unix only) or is killed, and checks a recovery scan restores every
  change.
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
  and, when stderr is not a terminal, shown through `zenity`, `kdialog` or `notify-send` (a message
  box on Windows); `failure_text` names the cause, the log, and for a `RenderError` the
  `WGPU_BACKEND` workaround of the platform (`gl` on Linux, `vulkan` or `gl` on Windows).

## Redraws and frame pacing

- caditor draws only when something asks (input, a worker's wake, egui's repaint request, or a
  frame that must follow: an action, a camera animation). A pick in flight is checked every
  `PICK_CHECK` from `about_to_wait` (`Renderer::is_pick_answered`) and a frame follows once it is
  answered; an exported image's tiles advance on the wake its writer sends for each buffer it
  returns. Neither draws frames while it waits. Every request goes through `Session::request_redraw`, never straight to the window.
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
  or shell and the open cut's tools, `Model::mesh_before`) into a `ShadedMesh` with edge polylines on its own worker, once
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
- A body is drawn in its appearance colour, else the default (`body_appearance::DEFAULT_COLOUR`);
  a failed or outdated feature on it tints it as before. While a sketch is edited, bodies are
  dimmed and not pickable.
- Every failed feature whose error has a `place` gets a marker there in the error colour, drawn in
  front of the model with a "<feature> failed here" label that screen readers read.

## Offers

- `offers.rs` works out what the selection offers the toolbar and status bar only when the
  selection, the model revision, its evaluation or the length unit changes, not every frame. It
  describes only the first `MAX_DESCRIBED` items and counts the rest (the status bar's tooltip
  ends "and N more"), since describing an item can scan its body.
- A lone selected face or edge, or a lone body chosen in the tree with nothing selected, also gets
  its size in `Offers::size` (Area, Length or Size, `measure::size_text` and `body_size_text`,
  marked ≈ when approximate), which the status bar shows beside the selection; the offers are
  worked out again when the bodies' meshes finish (`Model::bodies_pending`), since curved areas
  and sizes read the mesh.
- `Selection::generation` is globally unique per content change, so caches key on it rather than
  cloning and comparing the set: the offers, Measure, the panels' Use selected offers
  (`feature_fields::offered_change`, kept per feature and slot in egui's memory), and the
  viewport's check that the selection is still available (`retain_available`, rerun only when the
  selection, revision, evaluation or editing context changed).

## Measure

- The Measure command toggles `MeasureTool` in the `Workspace`; while open, `measure_panel.rs`
  draws a right-hand panel. Measuring never changes the document.
- `Measurements` resolves the selection on the UI thread into points, edges and faces (with their
  result's `Arc`), sketch curves (`profile_curve` lifted onto the sketch's solved plane), and the
  principal and datum planes and axes (a plane reads through a point with its normal, an axis
  through a point with its direction, a circle or arc its radius, diameter, centre and sweep), then measures on its own worker (newest job wins, a panic becomes a failed
  reading) only when the selection, revision or evaluation changes. Until the current result
  arrives the previous readout stays, dimmed (`Freshness::Stale`) under a Measuring header, so
  nothing jumps; the measured line in the view comes only from the current readout.
- A `Readout` is a card per item and, for two items, a "Between them" card; more than two asks for
  fewer, without describing or measuring any. Approximate values are marked with a note, a callout under its card. Mass properties are
  read each frame from `BodyMass` for the selected items' bodies and those of the rows chosen in
  the tree, else every shown body, with the
  body's material and its mass from the density (No density set without one, a warning note when
  the density cannot be evaluated) and, with a density, the moments of inertia about the centroid
  along the model axes and the principal moments (`MassProperties::second_moment`, integrated
  over the mesh's tetrahedra). Two or more bodies also get an "All N bodies" card first: summed
  volume and area, the centroid weighted by mass (by volume unless every body has a density), and
  the total mass and inertia about that centroid only when every body has a density.
- The closest points are drawn on the front layer (`scene::add_measurement`) with a distance
  label in `canvas::MEASURE`.
- Show or hide centres of mass (`Command::ToggleCentresOfMass`, View menu, palette) is a view aid
  (`ViewAids::centres_of_mass`, kept for the session in `ViewportState::aids`, not saved; it
  reaches the scene through `Sources::aids` and `Revisions::aids`). Outside sketch editing it
  marks each shown body's centroid (`BodyMass`, so approximate on curved faces) with a ring on the
  front layer, a `Pickable::CentreOfMass(body)` on a dark outline (`ScenePalette::hole`) that keeps
  the ring (`centre_of_mass`, and the hover and selection colours) at 3:1 on bodies and canvas.
  Measure reads it as a point named "Centre of mass of <body>" (`Subject::CentreOfMass`), so the
  distance and the offsets along each axis from any other item come from the ordinary two-item
  readout. Switching the aid off drops it from the selection.

## Interference

- Check interference (Inspect group, View menu, palette) toggles `InterferenceTool`; while open,
  `interference_panel.rs` draws a right-hand panel beside any other. It never changes the document.
- `Bodies::of` picks the pairs: every pair of shown bodies with nothing chosen, one chosen body
  (from selected faces, edges or vertices, or the bodies of every row chosen in the tree) against
  every other shown body, or every pair among several chosen.
- `Interference` checks pairs on its own worker with `caditor_kernel::interference`, one message
  per pair so findings appear as they come. A new basis (pairs, revision, evaluation, mesh quality)
  bumps a shared ticket that interrupts the pair in flight. Findings are cached per pair with both
  bodies' result `Arc`s and reused while those are unchanged, so editing one body rechecks only its
  pairs; entries for results that are gone are dropped. Closing the panel or a new session forgets
  everything. A panic becomes an unchecked finding.
- `Interference::refresh` works out the pairs only when the selection's generation, the tree's
  chosen bodies, the revision or the evaluation changes, and returns a new `Report` only when the
  basis changed or a finding arrived, so an open panel over a thousand bodies does nothing per
  frame. The panel lists and marks at most `MAX_LISTED` contacts, Measure at most
  `MAX_MASS_CARDS` bodies' masses, each saying how many more were left out.
- An overlap is meshed at the model's mesh quality for its volume, size and centroid
  (`BodyMass::of`, approximate on curved faces) and its mesh edges drawn on the front layer; every
  finding with a place gets a marker and a label in the view (error for overlaps, the measure
  colour for touches, warning for unchecked pairs) and a Show where button on its card.

## Face analysis

- Analyse draft and Analyse minimum radius (`AnalysisCommand::Draft`, `Radius`, View menu, palette)
  toggle `AnalysisTool` in the `Workspace` (`Kind` is its Draft or Minimum radius switch, which the
  panel's segmented control changes too); while open, `analysis_panel.rs` draws a right-hand panel
  beside any other. Like
  centres of mass it is a view aid: `ViewAids::analysis` (a `FaceAnalysis`, worked out each frame
  from the tool by `AnalysisTool::analysis` and handed over with `ViewportState::set_analysis`)
  reaches the scene through `Sources::aids` and `Revisions::aids`. It never changes the document and
  is kept for the session. A limit that cannot be read or a pull that is gone shows a warning
  callout (`Problem`) and nothing is coloured.
- Draft colours each face by the angle between the display mesh's normal and the pull direction,
  per triangle (the mean of its corners' normals), so a curved face is banded across its extent.
  A triangle at or above the limit is `Band::Drafted`, below minus the limit `Band::Undercut`, in
  between `Band::TooLittleDraft`, which holds the parting line. The pull is an axis or a direction
  given by an axis, straight edge, sketch line, flat face (its outward normal) or round face
  (its axis) chosen with Use selected (`AnalysisCommand::UseSelected`, `Pull::Picked` is resolved
  again each frame through `measure::direction_of`, so it follows edits) or the X, Y and Z buttons;
  `AnalysisCommand::Reverse` flips it. The limit is an angle expression of named parameters
  (3° at first, 0° to 90°).
- Minimum radius colours `Band::TooTight` the concave triangles whose surface curves tighter than
  the Smallest radius (a length expression, 2 mm at first, above zero), where a cutter or a nozzle
  of that radius cannot reach. The curvature is read from the display mesh along each triangle edge,
  the change of the corners' normals over the chord (`(n₂ − n₁)·d / d²`), negative where the face
  curves away from its outward normal, so planar, convex and gentler faces are left in their own
  colour; a face is flagged when its radius is more than 0.1% under the limit. Sharp inside corners
  between faces have no radius and are not flagged.
- `Analyses` (`analysis.rs`, owned by `ViewportState`, handed to the scene as `Sources::analyses`)
  caches one `Analysed` per body mesh and analysis: `ShadedMesh::divide` splits each face into a
  piece per band (`render.md`), the pieces carrying their source face and area, and the scene draws
  that mesh with a `FaceStyle` per piece (`Builder::analysed_faces`): a band's colour from
  `ScenePalette::bands`, or the face's own colour for no band, and the source face's pick id
  registered once, so hover, selection and picking read as before. Up to `INLINE_TRIANGLES` the
  work is done in the frame; a larger mesh is worked out on a thread that wakes the app and bumps
  `Analyses::finished`, which `Revisions::analysed` watches, the body drawn plain meanwhile. Entries
  of meshes that are gone are dropped. Moved or see-through bodies, X-ray and wireframe are drawn
  as before (the panel says so for a style that does not colour faces).
- The panel's legend names each band with its area from the same cache (`Tally`, approximate
  since it is the mesh's), so no band is told by colour alone; a band's hover says what it means.

## Curvature comb

- Show or hide the curvature comb (`AnalysisCommand::Comb`, View menu, palette, no default key)
  toggles `CombTool` in the `Workspace`; while open, `comb_panel.rs` draws a right-hand panel beside
  any other. It never changes the document and is kept for the session; a new session forgets its
  curves (`CombTool::forget`).
- The comb follows the selection: each time `Selection::generation` changes, the selected body
  edges and sketch curves (lines, arcs, circles, splines; `comb::is_combable`) become the combed
  curves, at most `MOST_COMBED`, the panel saying how many were left out. A selection holding none
  keeps the last ones, so selecting a point to drag it keeps its spline combed. The curves are
  `Pickable`s resolved again whenever the revision, the evaluation or the displayed sketches change
  (`Basis`), so the comb follows recompute and a sketch drag; one that no longer resolves is counted
  as gone in a warning callout.
- Each curve gets `teeth` + 1 teeth (Teeth per curve, `MIN_TEETH` to `MAX_TEETH`) at equal lengths
  along it, both ends included, from a polyline of `STEPS_PER_TOOTH` steps per tooth, each carrying
  the kernel's curvature vector (`CurveDerivatives::curvature`). A tooth points away from the centre
  of curvature, its length the curvature times one reach for the whole comb: the longest tooth is
  `LONGEST_TOOTH` of the combed curves' size times the Scale (`MIN_SCALE` to `MAX_SCALE`,
  logarithmic), so curves combed together compare directly. The envelope joins the tips along each
  curve.
- Ends of two combed curves within `JOINT_GAP` make a `Joint`, judged from the tangents leaving it
  and the curvature vectors (`Continuity`: a corner, tangent with a jump in curvature, or curvature
  continuing within `CURVATURE_SLACK`); the envelope steps from one curve's end tip to the other's,
  so a jump shows as a step and continuing curvature as none. The panel lists each curve's smallest
  radius and each joint with its grade (G0, G1, G2) and the radii either side in words, so nothing is
  told by the drawing alone.
- The drawing (`CombDrawing`, rebuilt only when the comb or the scale changes) goes into the overlay
  batch on the front layer: every segment first as a wider line in `ScenePalette::hole`, then the
  teeth and the envelope in `ScenePalette::comb` over it, so the comb holds 3:1 over bodies and the
  canvas in both palettes.


- `samples.rs` builds parametric models through the document API (so always the current format),
  fully constrained with dimensions naming parameters; a test recomputes each and checks its
  volume. Open sample opens one untitled and unmodified.

## About, command line, accessibility, packaging

- `cli.rs`: `caditor [FILE]`; a `.dxf` or STEP file (by extension or header) goes to import as if
  dropped, anything else to Open. `caditor --export OUT [--resolution ...] [--size WxH] MODEL`
  (`headless.rs`) opens no window: it loads a caditor model or imports a STEP file, recomputes
  without display work, exports every body that built to the format `OUT`'s extension names and
  prints a summary. A `.png` instead recomputes with display work and draws the scene the way the
  image export does (`snapshot.rs`: the initial viewpoint fitted to the model, no grid or
  highlights, 1920×1080 unless `--size` says otherwise) on an `OffscreenRenderer`, so it needs a
  graphics adapter and refuses `--size` for any other format; load issues and left-out bodies are warnings, a failed feature a warning that
  also makes the exit status 2 (`PARTIAL_EXIT_STATUS`), and a model with no body, a DXF drawing,
  an unreadable file or an unknown extension an error that writes nothing. The desktop entry also
  offers STEP (`model/step`) and DXF (`image/vnd.dxf`).
- AccessKit (`egui-winit`'s `accesskit` feature): the window is created hidden, the adapter
  attached, then shown. The status bar notice and the failed-features pill are live regions
  (`widgets::announced`: assertive for an error, polite otherwise). The viewport is named "3D
  view" and described in words (`scene_description.rs`, rebuilt only when the revision, the
  evaluation or the edited sketch changes): the shown bodies, sketches and datums by name and
  count and how many bodies are hidden, the first `NAMED_AT_MOST` bodies with their size and
  lowest corner, sketches with the plane they face and datums with where they lie (a plane as a
  sketch's, an axis along its direction through a point, a point at its position;
  `LengthUnit::spoken_length`, trailing zeros dropped), or while a sketch is edited its curves,
  points, constraints, how constrained it is and the span of its points. Its children are one
  label node per item (`SceneDescription::items`, `viewport::scene_items`, built only while
  AccessKit is on, at most `ITEMS_AT_MOST`): each shown body with its size and face count, its
  faces as nodes beneath it (at most `FACES_AT_MOST`, then how many more), each shown sketch and
  datum; while a sketch is edited each curve and point with where it lies, its size, construction
  and whether it is fully constrained, so a screen reader steps through them one by one. They are
  zero-sized and sense only hover, so they take no room, pointer or Tab stop. Text painted over
  it (prompt, hover description, snap and measure labels) is also a `Label` node
  (`canvas::announce`), the prompt and keyboard-highlight description polite live regions.
- Wayland app ID, X11 class and Windows window class are `about::APP_ID`, which must match the
  desktop entry's name.
- The logo is `packaging/caditor.svg`; `packaging/render-icons.sh` renders it into
  `packaging/icons/`, committed, so neither build nor release needs an SVG renderer. `logo.rs`
  embeds some renders and `logo::show` draws the smallest covering the requested physical size,
  hidden from AccessKit. The window icon is a fixed render (X11 and Windows, which also gets the
  largest as its taskbar icon; Wayland takes the icon from the desktop entry). The Windows
  executable's icon is built from the same renders (`windows.md`). A render that fails to decode
  logs a warning; tests decode every render.
