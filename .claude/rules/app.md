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
  - "crates/caditor/src/measurement_tools.rs"
  - "crates/caditor/src/measurement_panel.rs"
  - "crates/caditor/src/interference.rs"
  - "crates/caditor/src/interference_panel.rs"
  - "crates/caditor/src/comb.rs"
  - "crates/caditor/src/comb_panel.rs"
  - "crates/caditor/src/isocurves.rs"
  - "crates/caditor/src/isocurve_panel.rs"
  - "crates/caditor/src/section.rs"
  - "crates/caditor/src/section_panel.rs"
  - "crates/caditor/src/analysis.rs"
  - "crates/caditor/src/analysis_panel.rs"
  - "crates/caditor/src/reach.rs"
  - "crates/caditor/src/guide.rs"
  - "crates/caditor/src/guide_panel.rs"
  - "crates/caditor/src/defender.rs"
  - "crates/caditor/guide/**"
  - "crates/caditor/src/view_aids.rs"
  - "crates/caditor/src/pair_reading.rs"
  - "crates/caditor/src/scene_description.rs"
  - "crates/caditor/src/startup_check.rs"
  - "crates/caditor/src/variants.rs"
---

# App shell, model and bodies

## Structure

- `app.rs` is the winit `ApplicationHandler`; `overlay.rs` the egui integration over the viewport,
  keeping a CPU copy of every egui texture so a new renderer generation (lost device) rebuilds
  egui and re-uploads them. `scene.rs` converts documents and results to a `Scene`.
- Selectables are `Pickable`s built from stable IDs. A click that depends on the hover (anything
  but a drawing tool's) keeps the cursor and view it was made at (`PendingClick`) and waits for the
  pick made there, so it never acts on a stale hover. Picks run one at a time: once a pick for a
  newer pointer spot has arrived, or the pointer left the view, the click's own never comes, so it
  is resolved on the CPU (`ViewportState::seen_through`: the scene's `hits_through`, keeping what
  lies no deeper than the nearest face allows, then `best_hit`) and performed with cursor, sketch
  cursor and hover put back to that spot. A drag starting before the press's pick arrived takes what
  lies under the press the same way (`caught_up`).
- `Model` (`model.rs`) owns the `Editor`, the `Recomputer` and the length unit; the UI gets
  `&Model` and returns `Action`s the app performs after its pass. Features draw from the last good
  result, tinted if failed or outdated; suppressed and rolled-back ones are neither drawn nor
  picked (`Document::active_features`) and leave the selection. A sketch's curves and regions are
  available only while it is active (`Document::is_active` in `Pickable::is_available`) or edited,
  as faces and edges only while the evaluation has them.
- `Model` also owns the file session: path, last saved document (unsaved exactly when the document
  differs from it, and always when unknown, e.g. after restoring a rebased journal), the journal's
  base and entries since it, and the `Storage` worker every change is recorded to. While the
  journal cannot be written the status bar shows Not protected.

## Scene

- `ViewportState::build_scene` hands the renderer a cached scene (`scene_cache.rs`). The base scene
  (bodies, sketches, references, hover and selection colours, a `PickTable`) is rebuilt only when
  its content or highlight changes. Content is everything that can change the drawing (document
  revision, evaluation, the open feature's draft, displayed sketches, body meshes, display style,
  `Contrast`, editing `Context`, faceting level); anything new that affects the drawing must feed
  that key. The hovered `Pickable`s and chosen rows (with their bodies,
  `viewport::with_their_bodies`) in `Highlight` are sorted and deduplicated, since items look
  themselves up by binary search.
- Each shown body is a `BodyPart` the cache keeps (`scene::Parts`): mesh instances, silhouette,
  its own batch for edges and vertices, and a `PickBlock` of pick ids from `PART_PICKS_FROM` up. A
  rebuild reuses a part whose `BodyLook` (mesh, colour, face looks, opacity, dashes, placement,
  pickability, display style, palette, analysis, reflection) and `PartLit` (selected and hovered
  items, whether its tree row is chosen; worked out once per rebuild by `LitParts`, keyed by
  `PartKey`) are unchanged, restyles one whose highlight alone changed within the same pick ids
  (keeping its batch's `Arc` when it looks the same), and gives a changed look a new block. A hover
  or selection change restyles and uploads only the bodies it touches and no pick id moves when
  bodies come, go or change
  (`each_body_draws_its_own_batch_and_a_hover_or_a_hidden_body_moves_no_other_pick_id`).
- Each drawn sketch is kept the same way (`Parts::keep`, a `DrawnPart`): its curves and points in a
  batch and block of their own, keyed by a `SketchLook` (its faceted shapes' `Arc`, `Presence`,
  control polygons, palette) and `PartLit`; a background sketch while another is edited takes no
  highlight, so it is never restyled. Its closed regions are a second part (`RegionsLook`: the
  sketch's `SketchSource`, whether the regions were found, pickability, layer, palette). So a drag
  solution, an edit, an evaluation or a hover touching one sketch refacets, restyles and uploads
  that sketch alone
  (`each_sketch_draws_its_own_batch_and_a_change_to_one_refacets_and_uploads_only_it`).
- Batch order: the scene's own (references, datums, threads, the open feature), the sketches', the
  bodies' in body order, the regions', the overlay. Pickables are listed in the order they were
  before parts existed, which the highlight commands step through: the own block's references and
  datums, the sketches' blocks, the bodies', the rest of the own block, the regions' blocks. The
  `PickTable` resolves an id by binary search over the blocks' first ids. The allocator restarts,
  rebuilding every part, past `PART_PICKS_RESTART`. Body health is worked out for all bodies in one
  pass (`Healths`). Image export builds from a copy of the kept parts and sketch shapes.
- Previews, the trim preview and the measured line form a second batch with no pick ids, rebuilt
  only when it differs. A pick is asked for when the cursor, the view or the base's generation
  differs from the last (`PickKey`), never while the camera's viewpoint moves.
- `SketchShapes` caches each sketch's faceted outlines and each entity's `SketchState` under its
  `SketchSource`: the feature's `Arc` in the document, its evaluation state and result `Arc`, and
  the dragged sketch's `Arc` (`DisplayedSketches::dragged`), which together decide what is shown.
  Unchanged sketches keep their shapes across content changes; a new faceting level refacets all.
- Image export, the version preview, the headless PNG and the 3MF thumbnail build their own scene,
  without highlights and in the standard palette (high contrast is a viewing aid): image export and
  version preview in the palette of the 3D view's canvas (`Background::Viewport`), the headless PNG
  and 3MF thumbnail in the dark one (`snapshot::of_bodies` takes the canvas).
- Every scene colour and line weight comes from the `ScenePalette` of the viewport's `Contrast`
  (`scene_palette.rs`, set from the High contrast preference each frame), never a constant in
  `scene.rs`. Colours shared with canvas labels (hover, selection, snap, measure, problems) stay
  `canvas` colours in both palettes.

## Sketch faceting

- Sketch curves, previews and trim pieces are faceted to a chord tolerance (`faceting.rs`): a
  fraction of a pixel at the view's target distance, rounded down to a power of two of millimetres
  (`FacetLevel`), kept while the wanted chord stays within a band above it (never coarser than
  wanted), so zooming back and forth never refacets. `scene::drawn_faceting` coarsens the chord
  while the curves would need more than `SKETCH_SEGMENT_BUDGET` segments.
- Picks, hover highlights and box selection (`sketch_drag::within`) use the drawn polylines;
  snapping stays exact on the curves. Bounds and fitting use the sketch crate's default segment
  angle, not the view's faceting.

## Startup, sessions and crashes

- The app is a library with a thin binary (`main.rs` calls `caditor::run`), so the fuzz workspace
  reaches app code through the `fuzzing` feature. `run` starts with an empty model.
- Release builds unwind (`panic = "unwind"`), since containment relies on it; `recompute.rs` fails
  to compile otherwise.
- The viewport fits once the first recompute of a newly opened model (new `Model::session`) is up
  to date and its bodies are meshed (`Model::bodies_pending`). Fitting and the reference size count
  only what is drawn: sketches among `active_features` and not hidden, and shown bodies.
- IDs restart per document, so a new session clears everything holding them: viewport selection,
  hover, highlight and drawing state (`Workspace::sync`), the tree's selection and rename, the
  export dialog's left-out bodies. Anything new that holds IDs must be cleared there.
- `crash::protect` installs the panic hook and a SIGTERM/SIGHUP/SIGINT handler that flush the
  journal before exit; on Windows a console close handler, plus `crash::flush_when_the_session_ends`
  once the window exists (`windows.md`). `tests/crash_flush.rs` re-runs its binary as a child that
  panics, raises SIGTERM (Unix only) or is killed, and checks a recovery scan restores every change;
  it runs a real storage worker, not the app, which `packaging/check-run.sh` kills (`ci.md`).
- `startup_check.rs` is the app's hook for that script. Every run logs "the first frame was drawn"
  and `Files::offer` logs "recovery offered for <journal> with N recoverable changes".
  `CADITOR_STARTUP_CHECK` (`Mode::parse`), acting on the first frame not blocked by a dialog or an
  opening: any other non-empty value applies one transaction (a parameter `startup_check`), flushes
  the journal (`Model::flush_journal`) and logs "startup check: an edit is in the recovery
  journal"; `save:<path>` replaces the model with the Plate sample, saves it and logs "startup
  check: saved the model to <path>"; `restore` performs `FileCommand::Restore` once a recovery is
  offered (`Files::first_recoverable`) and logs "startup check: restored N changes of <name>" if the
  restored document holds the `startup_check` parameter (an error line otherwise). It has no
  command-line form and is not documented to users.
- A panic while handling a window or user event is caught in `App::contained` after the hook
  flushed the journal. The window gets a fresh egui context, `after_failed_frame` drops transient
  state (selection, tools, sketch editing, palette, dialogs except prompts guarding unsaved work)
  and a notice says so; the model and undo history stay. A held sketch drag is cancelled
  (`Model::abandon_held_drag`); a constraint trial in flight is kept, being a user's edit. A second
  failed frame in a row suppresses every feature as one undoable change, saying how to find the
  culprit; `GIVE_UP_AFTER_FAILED_FRAMES` in a row exits with the journal kept. The recovery card's
  Restore suppressed is the same restore followed by suppressing every feature.
- `logging.rs`: each run logs to stderr and its own owner-only file under the state directory
  (`caditor_file::SessionLog`, `MAX_LOG_SIZE`, `LOGS_KEPT` newest kept), with an end line on a
  normal exit, reported failure or termination signal. At start a log whose process is gone and
  which has no end line is named in a notice, then marked reported.
- A failure `run` returns is logged and, when stderr is not a terminal, shown through `zenity`,
  `kdialog` or `notify-send` (a message box on Windows); `failure_text` names the cause, the log,
  and for a `RenderError` the `WGPU_BACKEND` workaround (`gl` on Linux, `vulkan` or `gl` on
  Windows).

## Redraws and frame pacing

- caditor draws only when something asks (input, a worker's wake, egui's repaint request, or a
  frame that must follow: an action, a camera animation). A pick in flight wakes the app when the
  device answers and is also checked every `PICK_FALLBACK` from `about_to_wait`
  (`Renderer::is_pick_answered`); an exported image's tiles advance on the wake its writer sends
  per buffer. While the renderer uploads meshes (`Renderer::is_uploading`) a frame follows each
  frame (`app-look.md`), and the frame that sees the uploads finish asks for a pick again
  (`ViewportState::pick_again`). Every request goes through `Session::request_redraw`, never
  straight to the window. Animated widgets (`widgets::spinner`) and progress readouts repaint on a
  timer (`request_repaint_after`), never every frame; the text caret does not blink.
- `FramePacer` (`graphics.rs`) holds requests to the frame limit: the chosen rate's interval, or for
  Match the display's refresh rate only while vsync is off (`Fifo` already paces); none when
  unlimited or unknown. An early request is scheduled for the next slot (`next_repaint`, which
  `about_to_wait` turns into `ControlFlow::WaitUntil`). The first frame after an idle spell is never
  delayed.

## Bodies

- Body meshes come from the recompute worker at its `MeshQuality` (`document-recompute.md`). The
  Curve smoothness preference reaches it through `Model::set_mesh_quality` at startup and on each
  change (`app::apply_preferences`), followed by a new submission.
- `BodyMeshing` (`bodies.rs`) converts each body's final mesh (and the state before the open blend
  or shell and the open cut's tools, `Model::mesh_before`) into a `ShadedMesh` with edge polylines
  on its own `Pool` (threads up to the available parallelism less one, conversions before mass
  properties), once per result keyed by its `Arc`'s address. Nothing is copied: a `DisplayedMesh`
  holding the result's `Arc` is the `MeshSource` (`render.md`;
  `a_body_mesh_reads_the_kernel_mesh_rather_than_holding_a_copy_of_it`) and a `BodyEdge` is its name
  plus the index of the kernel mesh's polyline, read through the `Arc` by `BodyMesh::points`
  (`body_edges_read_their_points_from_the_kernel_mesh`). A panic leaves that body meshless; without
  a worker it runs on the UI thread.
- Arrivals are held and shown together: the first at once, then all that arrived whenever
  `SHOWN_TOGETHER_FOR` has passed since the last showing or nothing is left to convert, so a large
  import is drawn in a few batches. `BodyMeshes` takes each batch, keeping the previous mesh until
  then, and walks the bodies only when the evaluation (`Model::evaluation_generation`) or the
  converted meshes (`BodyMeshing::conversions`) changed (`BodyMeshes::update_when_changed`). While
  bodies are pending the status bar says how many are shown (`Model::bodies_preparing`).
- A face is `Pickable::Face` with a `FaceKey` (`FaceName` plus occurrence among same-named faces),
  an edge `Pickable::Edge` with its `EdgeName`, a vertex `Pickable::Vertex` with a `VertexKey`,
  found through the result's `NameIndex` and described in words from the `FaceOrigin`, never by
  index. Vertices are colourless markers: they pick (winning over nearby edges and faces) but show
  only when hovered or selected.
- A body's `BodyMass` (volume, area, centroid, second moments, bounding-box size) and its faces'
  areas are worked out exactly from the kernel (`mass_properties`, `extent`), independent of the
  mesh, only once something asks (`BodyMesh::mass`, `BodyMesh::face_area`): the first asking queues
  the work on the pool behind the conversions (`MassSlot`, shared by clones of the `BodyMesh`;
  inline when the pool is gone), since adaptive quadrature over a large import (185 s for the VZ330
  assembly on one thread) must not hold meshes back. Until it arrives Measure's card says so, the
  centre-of-mass aid leaves the body out and a face's area falls back to its mesh triangles (≈);
  each arrival bumps `Model::masses_measured`, which offers, Measure and (with the aid on) the scene
  key (`Revisions::masses`) watch. A face the kernel could not integrate takes the mesh's triangles
  and the body is marked approximate (`MassAccuracy::Mesh`).
- A body is drawn in its appearance colour, else `body_appearance::DEFAULT_COLOUR`; a failed or
  outdated feature on it tints it. In high contrast every body and face colour passes through
  `ScenePalette::body` (`app-look.md`). While a sketch is edited, bodies are dimmed and not
  pickable.
- Every failed feature whose error has a `place` gets a marker there in the error colour, in front
  of the model, with a "<feature> failed here" label that screen readers read.

## Offers

- `offers.rs` works out what the selection offers the toolbar and status bar only when the
  selection, model revision, evaluation, units (length and angle) or the edited sketch and opened
  feature change. It describes only the first `MAX_DESCRIBED` items and counts the rest, since
  describing an item can scan its body. Offers are worked out again when meshes finish
  (`Model::bodies_pending`) and masses arrive (`Model::masses_measured`).
- A lone selected face, edge, sketch curve or region, or a lone body chosen in the tree, also gets
  its size in `Offers::size` (`measure::size_text`, `body_size_text`, marked ≈ when approximate):
  circle Ø, R and circumference; arc R, length, sweep; line length; a flat face or region whose
  outer loop is four right-angled straight sides width × height (longer first), else perimeter, then
  area; cylinder, sphere, cone and torus radii or angle, then area; a body its Size.
- Two selected items (`measure::is_pair`) get their reading in the status bar in the size's place
  (`StatusContext::between`, `pair_reading::PairReading`): "Distance 12.5 mm" and, where the pair
  has one, "Angle 90°", from the Between them card of a `Readout` that its own `Measurements` works
  out on its worker, in the world frame, whether or not Measure is open. It restarts only when the
  selection, revision, evaluation or masses change and keeps the previous reading until the new one
  arrives.
- `Selection::generation` is globally unique per content change, so caches key on it rather than
  comparing the set: the offers, the scene cache's highlight key, the tree's selected-row check,
  Measure, the panels' Use selected offers (`feature_fields::offered_change`, kept per feature and
  slot in egui's memory), and the viewport's availability check (`retain_available`).

## Measure

- The Measure command toggles `MeasureTool` in the `Workspace`; while open, `measure_panel.rs`
  draws a right-hand panel. Measuring never changes the document; only a reading's New parameter
  from this value and Keep this measurement do, each as an ordinary change.
- `Measurements` resolves the selection on the UI thread into points, edges and faces (with their
  result's `Arc`), sketch curves (`profile_curve` on the sketch's solved plane), principal and datum
  planes and axes, and a coordinate system's axes, planes and origin, then measures on its own
  worker (newest job wins, a panic becomes a failed reading) only when the selection, revision,
  evaluation or the coordinate system read in changes. Until the current result arrives the
  previous readout stays, dimmed (`Freshness::Stale`) under a Measuring header; the measured line
  in the view comes only from the current readout.
- Selected sketch regions (`Pickable::SketchRegion`) gather into one item per sketch
  (`Subject::Regions`, one item for `item_count`) and read their summed section
  (`caditor_kernel::section_of`, `kernel-profile.md`): area, perimeter, centroid, Ix, Iy, Ixy, J,
  principal moments and angle (`Value::SecondMoment`, `LengthUnit::measured_second_moment`); a
  spline side marks every value ≈. A region is never one side of a "Between them" card.
- A `Readout` is a card per item and, for two items, a "Between them" card; more than two asks for
  fewer without measuring any. Approximate values get a note callout under their card. Mass
  properties are read each frame from `BodyMass` for the selected items' bodies and the rows chosen
  in the tree, else every shown body, with material and mass from the density (No density set
  without one) and, with a density, inertia about the centroid along the model axes and the
  principal moments (`MassProperties::second_moment`). Two or more bodies also get an "All N
  bodies" card first: summed volume and area, centroid weighted by mass (by volume unless every
  body has a density), total mass and inertia only when every body has a density.
- Every row of the reading and mass cards (not Interference's, which share `show_card`) has a menu
  on its "⋯" button ("More for <label>") and a right-click of its value (`measure_panel::row_menu`):
  Copy value (as shown, unit and ≈ included); New parameter from this value, for a reading's
  `Value::Length`, `Angle`, `Area` and a mass card's `Volume`, `Area`, `Mass` only, adds a
  parameter through `parameter_table::add_with` in one transaction, named after the label
  (`parameter_stem`, giving `along_x1`), holding the value in the model's unit
  (`LengthUnit::measured`, `AngleUnit::measured`, `measured_area_expression`,
  `measured_volume_expression`; a mass as grams), name field focused.
- Relative to (shown once the model holds a coordinate system, `MeasureTool::relative_to`, World by
  default, forgotten when another model opens) reads every position, direction and Along offset in
  the chosen system (`measure::Relative`, also `mass_cards`' centroids); distances, angles, the
  line and inertia stay world geometry. A gone system, or one with no result, reads as the world
  with a warning callout (`measure_panel::relative`).
- Closest points are drawn on the front layer (`scene::add_measurement`) with a label in
  `Chrome::measure`.
- Keep this measurement (`measurement_tools::KEEP`) is in the row menu of a Between them card's
  Distance, angle and Along rows (`Keepable::Along`), of one item's Length, Radius, Sweep, Area and
  Perimeter (`Keepable::of_row`; `face_perimeter`), of a Position row as three entries along X, Y,
  Z (`Keepable::Position`, `Reading::Position`, named like `position_z1`) and of a body's mass card
  (`Keepable::of_mass_row`: Volume, Surface area, Mass with a density, Centroid along X, Y, Z, each
  `MeasuredItem::Body`). It needs every selected item to have a stable reference
  (`measurement_tools::item_of`, carried as `Readout::kept`; regions have none, a centre of mass is
  its body). One transaction adds a parameter named after the row, a `Measurement` feeding it at
  the bar ("Measurement N") and the parameter's owner (`ParameterOwner::Feature` with
  `measurement_tools::READING`), checked first; a refusal (a reference made below the bar) is a
  warning notice.
- Every shown, up-to-date measurement is drawn outside the panel too (`measurement_tools::shown`,
  each frame like the failure markers): its line in the overlay batch (`Overlay::kept`) and a label
  "<parameter> = <reading>" in `Chrome::measure`. A measured parameter's expression field in
  Parameters shows the reading and refuses edits in words; other parameters may read it.
- Its card (`measurement_panel.rs`, the open feature's panel) has Reads, a combo of the quantities
  of its arity (`measurement_tools::Quantity::offered`), each refused with the reason where its
  items do not allow it (`read_as`); a row per item with Use selected or Choose in the view
  (`Slot::MeasuredItem(MeasuredPart)`, `measurement_tools::item_change`), which takes the one
  selected item found where the measurement sits (`item_at`: made above it, resolving in the body's
  state there, an edge through `trace_edge` so one cut into pieces by a sketch edit is found as its
  pieces and one only partly traced is refused in words; `body_at` for a body item), keeping the quantity while the new item allows it, else
  the first it allows; an Along row is a combo of X, Y, Z over the same picker
  (`datum_tools::axis_reference`); then its Reading (`reading_text`) and the parameter it is Named
  by. Each change is one "Edit <name>" `SetFeatureKind`, checked first.
- Every evaluation of the current revision that arrives (`Model::poll_recompute`) runs
  `Editor::follow_readings`, so measured parameters' stored values follow the readings outside the
  undo history (`document.md`); revision and unsaved state do not change for it.
- Show or hide centres of mass (`Command::ToggleCentresOfMass`) is a view aid
  (`ViewAids::centres_of_mass`, session only, in `ViewportState::aids`; reaches the scene through
  `Sources::aids` and `Revisions::aids`). Outside sketch editing it marks each shown body's
  centroid with a ring on the front layer, a `Pickable::CentreOfMass(body)` on a dark outline
  (`ScenePalette::hole`) holding the ring at 3:1. Measure reads it as a point (`Subject::CentreOfMass`).
  Switching the aid off drops it from the selection.

## Interference

- Check interference toggles `InterferenceTool`; while open, `interference_panel.rs` draws a
  right-hand panel beside any other. It never changes the document.
- `Bodies::of` picks the pairs: every pair of shown bodies with nothing chosen, one chosen body
  (from selected faces, edges, vertices or chosen tree rows) against every other, or every pair
  among several chosen.
- `Interference` checks pairs on its own worker with `caditor_kernel::interference`, one message per
  pair so findings appear as they come. A new basis (pairs, revision, evaluation, mesh quality)
  bumps a shared ticket that interrupts the pair in flight. Findings are cached per pair with both
  bodies' result `Arc`s, so editing one body rechecks only its pairs; closing the panel or a new
  session forgets everything. A panic becomes an unchecked finding.
- `Interference::refresh` works out the pairs only when the selection's generation, the chosen tree
  bodies, revision or evaluation change, and returns a new `Report` only when the basis changed or a
  finding arrived. The panel lists and marks at most `MAX_LISTED` contacts, Measure at most
  `MAX_MASS_CARDS` bodies' masses, each saying how many were left out.
- An overlap is meshed at the model's mesh quality (`BodyMass::of`) and its edges drawn on the front
  layer; every finding with a place gets a marker and label (error for overlaps, the measure colour
  for touches, warning for unchecked pairs) and a Show where button.

## Face analysis

- Analyse draft, minimum radius, tool reach, curvature, Show zebra stripes and Show a chrome
  reflection (`AnalysisCommand::Draft`, `Radius`, `Reach`, `Curvature`, `Zebra`, `Chrome`) toggle
  `AnalysisTool` in the `Workspace` (`Kind` is which; the panel's segmented control changes it too);
  while open, `analysis_panel.rs` draws a right-hand panel. A session view aid, never changing the
  document: `ViewAids::analysis` (a `FaceAnalysis`) or `ViewAids::reflection` (a
  `caditor_render::Reflection`), worked out each frame by `AnalysisTool::analysis`, handed over with
  `ViewportState::set_analysis` and reaching the scene through `Sources::aids` and
  `Revisions::aids`. A limit that cannot be read or a pull that is gone shows a warning callout
  (`Problem`) and nothing is coloured.
- Draft colours each triangle (mean of its corners' normals) by the angle between normal and pull:
  at or above the limit `Band::Drafted`, below minus the limit `Band::Undercut`, between
  `Band::TooLittleDraft` (the parting line). The pull is an axis or a direction from an axis,
  straight edge, sketch line, flat face (outward normal) or round face (its axis) chosen with Use
  selected (`AnalysisCommand::UseSelected`; `Pull::Picked` is resolved again each frame through
  `measure::direction_of` and dropped back to Z when another model opens) or the X, Y, Z buttons;
  `AnalysisCommand::Reverse` flips it. The limit is an angle expression (3° at first, 0° to 90°).
- Reach judges undercuts for a three-axis machine (`Kind::Reach` shares the pull, Use selected and
  Reverse with Draft). Per triangle, a face turned from the tool (`FACING_SLACK`) is
  `Band::FacesAway`; otherwise its centroid, nudged off along the normal, is `Band::Blocked` when any
  shown body's triangle lies over it along the direction, else `Band::Reachable`. `reach.rs`
  projects every triangle into a 2D uniform `Grid` (at most `MOST_CELLS_ACROSS` cells a side;
  edge-on triangles cast no shadow) with slacks of `SLACK_PER_SIZE` of the bodies' size. As the result
  depends on every shown body, `Analyses::prepare` keeps one `Occluders` (direction plus weak
  references to the shown meshes, `analysis::shown_meshes`) and stamps its generation into
  `FaceAnalysis::Reach`; the grid is built once on first use, on the analysis thread past
  `INLINE_TRIANGLES`. Any other analysis, or none, drops the occluders.
- Curvature colours each triangle against a Reference radius R (10 mm at first): `ShapeOperator::of`
  fits the 2×2 shape operator to how the corners' normals turn along the edges (positive where the
  face bulges out), giving Gaussian and principal curvatures (`Measure`). Five steps each, from
  `Measure::bands`: beyond −1 (scaled by R, or R² for Gaussian), below −`FLAT_SHARE`, within
  ±`FLAT_SHARE`, above, beyond +1; colours `ScenePalette::bands.curvature`, a diverging scale.
- Zebra and Chrome change no colours: the scene puts every coloured, unmoved body mesh into
  `Scene::reflective_meshes` with its usual face styles and sets `Scene::reflection` (`render.md`).
  Stripes run along X, Y or Z with `MIN_STRIPES` to `MAX_STRIPES` per turn (12 at first).
- Minimum radius colours `Band::TooTight` the concave triangles whose surface curves tighter than
  the Smallest radius (2 mm at first), read along each triangle edge as `(n₂ − n₁)·d / d²`
  (negative where the face curves away from its outward normal); a face is flagged when its radius
  is more than 0.1% under the limit. Sharp inside corners are not flagged.
- `Analyses` (`analysis.rs`, owned by `ViewportState`, passed as `Sources::analyses`) caches one
  `Analysed` per body mesh and analysis: `ShadedMesh::divide` splits each face into a piece per band
  (`render.md`), carrying its source face and area, and the scene draws it with a `FaceStyle` per
  piece (`Builder::analysed_faces`): the band's colour from `ScenePalette::bands` (Reach reuses
  drafted and undercut, with `blocked` of its own) or the face's own colour for no band, the source
  face's pick id registered once so hover, selection and picking read as before. Up to
  `INLINE_TRIANGLES` the work is done in the frame; a larger mesh on a thread that wakes the app and
  bumps `Analyses::finished` (watched by `Revisions::analysed`), the body drawn plain meanwhile.
  The cache is keyed by mesh address, one entry per mesh, so a lookup is O(log B); entries of gone
  meshes are dropped at each `prepare`, all of them when no analysis is shown (`prepare(None, ..)`).
  The panel and the app share one `shown_meshes` a frame (`AnalysisContext::shown`).
  Moved or see-through bodies, X-ray and wireframe are drawn as before.
- The legend names each band with its area from the same cache (`Tally`, approximate), so no band
  is told by colour alone.

## Curvature comb

- Show or hide the curvature comb (`AnalysisCommand::Comb`, no default key) toggles `CombTool` in
  the `Workspace`; while open, `comb_panel.rs` draws a right-hand panel. Session only; a new
  session forgets its curves (`CombTool::forget`).
- The comb follows the selection: each time `Selection::generation` changes, the selected body edges
  and sketch curves (`comb::is_combable`) become the combed curves, at most `MOST_COMBED`. A
  selection holding none keeps the last ones, so selecting a point to drag it keeps its spline
  combed. The curves are `Pickable`s resolved again when the revision, evaluation or displayed
  sketches change (`Basis`); one that no longer resolves is counted as gone in a warning callout.
- Each curve gets `teeth` + 1 teeth (`MIN_TEETH` to `MAX_TEETH`) at equal lengths, both ends
  included, from a polyline of `STEPS_PER_TOOTH` steps per tooth, each carrying the kernel's
  curvature vector (`CurveDerivatives::curvature`). A tooth points away from the centre of
  curvature, its length the curvature times one reach for the whole comb: the longest tooth is
  `LONGEST_TOOTH` of the combed curves' size times the Scale (`MIN_SCALE` to `MAX_SCALE`), so
  curves combed together compare directly.
- Ends of two combed curves within `JOINT_GAP` make a `Joint`, judged from the tangents and
  curvature vectors (`Continuity`: a corner, tangent with a curvature jump, or curvature continuing
  within `CURVATURE_SLACK`); the envelope steps between their end tips. The panel lists each
  curve's smallest radius and each joint's grade (G0, G1, G2) and radii in words.
- The drawing (`CombDrawing`, rebuilt only when the comb or scale changes) goes into the overlay
  batch on the front layer: every segment first as a wider line in `ScenePalette::hole`, then teeth
  and envelope in `ScenePalette::comb`, holding 3:1 over bodies and canvas in both palettes.

## Isocurves

- Show or hide isocurves with combs (`AnalysisCommand::Isocurves`, no default key) toggles
  `IsocurveTool` in the `Workspace`; while open, `isocurve_panel.rs` draws a right-hand panel.
  Session only; forgets its faces in a new one.
- It follows the selection like the comb: the selected body faces (at most `MOST_FACES`) become the
  chosen faces, and a selection holding none keeps them. Lines along (Both, U, V) and Lines each way
  (`MIN_LINES` to `MAX_LINES`, 5 at first) set how many runs `Solid::isoparametric_runs` finds per
  direction (`kernel-intersect.md`); every run becomes a polyline of `STEPS_PER_LINE` steps with a
  comb from the surface's derivatives (`comb::spaced_teeth_along`). All lines share one comb
  (`Isocurves::comb`) scaled by the comb's Scale; joints are not judged.
- The work runs on an `isocurves` thread whenever the faces, settings, revision or evaluation change
  (`Basis`); a newer basis cancels the older through the kernel interrupt, the last finished lines
  stay drawn meanwhile, and the finished result wakes the app. A face that no longer resolves, or a
  panic in the thread, is a warning callout. The panel lists each face with its smallest U and V
  line radius in words.
- The drawing (`IsocurveDrawing`) goes into the overlay batch on the front layer like the comb: each
  line over a wider `ScenePalette::hole` halo in `CombLook::isocurve`, then the comb's teeth and
  envelope (`CombDrawing::add_to`); the palette test holds the isocurve colour to 3:1 over the halo
  and canvas.

## Section view

- Show or hide the section view (`SectionCommand::Toggle`, no default key) toggles `SectionTool` in
  the `Workspace`; while open, `section_panel.rs` draws a right-hand panel and the model is cut. It
  never changes the document. The planes are session view state, not saved with the model or
  preferences (they only look into the model; saving would need a format change and an undo entry
  for a non-edit). A new session forgets the planes (`SectionTool::forget`).
- A plane (`Cut`) starts from a base: a principal plane, a datum plane, a coordinate system's plane,
  a flat face (outward normal, by `FaceKey`) or a sketch's plane, taken with Use selected
  (`SectionCommand::UseSelected`, `section::base_from`) and resolved again each frame
  (`section::base_plane`), so it follows edits; a gone base shows a warning callout and that plane
  cuts nothing. Offset moves it along the base normal into the side it keeps, Tilt and Turn turn it
  about the base's own x then y axis (length and angle expressions), Cut away the other side
  (`SectionCommand::Flip`) flips the side, and Cut faces chooses hatched or filled
  (`caditor_render::CutFace`). The cut side is the base normal's: above XY, in front of XZ, right of
  YZ, outside a face.
- Opening the view with no plane, and Add a plane (`SectionCommand::Add`), adds one through the
  centre of the model's bounds (offset rounded to 0.1 mm) on the first principal plane not in use
  (XZ, YZ, XY), up to `MAX_SECTION_PLANES`; `SectionCommand::Remove` drops one. Commands act on the
  current plane (added or edited last); removing a plane above it moves `current` down with it
  (`SectionTool::remove`), removing the current one makes the next current.
- `SectionTool::planes` resolves the planes each frame into `ViewportState::set_section`;
  `ViewportState::shown_section` adds the sketch slice (`app-sketching.md`) and hands them to
  `SceneCache::set_section`, which sets `Scene::section` without rebuilding the cached scene and
  bumps its generation, so a moved plane costs a uniform write (`render.md`) and the hover is picked
  again. Picking, paint selection and box selection (the `seen` closure of `select_in_model` treats a
  cut point as unseen) reach only what is shown, so Measure does too. An exported image keeps the
  section (`ViewportState::image`); thumbnails and the headless PNG have none.

## Samples

- `samples.rs` builds parametric models through the document API (so always the current format),
  fully constrained with dimensions naming parameters; a test recomputes each and checks its
  volume. Open sample opens one untitled and unmodified.

## User guide

- The guide ships inside the binary: one page per tool or subject in `crates/caditor/guide/<id>.md`,
  embedded by `include_str!` through `guide::Page::source`, parsed once (`guide::GUIDE`, a
  `LazyLock`). `Page` is the only list of pages; `Chapter` groups them in `Page::ALL` order.
- Pages use a small markup of our own, not Markdown: `# ` the title (first line), `## ` a heading,
  `- ` a list item (indented lines continue it), blank lines between paragraphs, and inline
  `**strong**`, `` `code` ``, `[text](page-id)` links and `{command:id}`, which renders the
  command's title with the user's current keys (`guide_panel::command_text`), so pages never repeat
  default shortcuts. Tests hold every link and command id to one that exists, every page to a title,
  content and an incoming link; `no_page_has_a_line_the_parser_does_not_read` fails on a line that
  starts like unread markup (`###` headings, fences, quotes, `*` or `+` items, tables, indented
  items), so an example is a code span, not a fenced block.
- `guide_panel.rs` draws it as a right-hand side panel (`Guide` in the `Workspace`, reset by a
  failed frame): back, contents and close buttons, a search field (`guide::search`, title matches
  first), the contents by chapter, and a page with a Next link. Links are underlined `accent_text`
  labels, focusable and named as links.
- Help › User guide (`Command::Guide`, F1) opens the page of the current context
  (`guide::context`): the active sketch tool other than Select, else the open feature
  (`Page::of_feature`), else the first open side panel, else the edited sketch or plane choice, else
  the tree's selected row; with none, the contents with the search field focused. Run again on the
  page it would open, it closes the guide.
- Each side panel's header has a help button (`guide_panel::help_button`) and an open feature's card
  ends with "Help on <page>" (`help_link`); both ask through egui temp data (`guide::ask`, taken by
  `app::guide_commands` next frame). A new tool, feature kind or side panel gets a page (the
  exhaustive `Page::of_tool`, `of_feature` and `of_panel` fail to compile otherwise), and a change
  to what a tool does updates its page in the same commit.

## About, command line, accessibility, packaging

- `cli.rs`: `caditor [FILE]`; a `.dxf` or STEP file (by extension or header) goes to import as if
  dropped, anything else to Open. `caditor --export OUT [--resolution ...] [--size WxH]
  [--configuration NAME | --all-configurations] MODEL` (`headless.rs`) opens no window: it loads a
  caditor model or imports a STEP file, recomputes without display work, exports every body that
  built to the format `OUT`'s extension names and prints a summary. A `.png` instead recomputes with
  display work and draws the scene as image export does (`snapshot.rs`: fitted viewpoint, no grid or
  highlights, 1920×1080 unless `--size`) on an `OffscreenRenderer`, so it needs a graphics adapter;
  `--size` is refused for any other format. Load issues and left-out bodies are warnings, a failed
  feature a warning that also makes the exit status 2 (`PARTIAL_EXIT_STATUS`); a model with no body,
  a DXF or SVG drawing, an unreadable file or an unknown extension is an error that writes nothing.
- `--configuration NAME` (`ConfigurationChoice::Named`, case-insensitive) exports a copy switched to
  that configuration (`Document::activating`); an unknown name or a model without configurations is
  refused naming the ones it has. `--all-configurations` (`ConfigurationChoice::Every`) exports each
  to the file `configuration_export::files_beside` plans next to `OUT`, keeping `OUT`'s extension;
  one that cannot be switched to or has no body is a warning making the status 2
  (`Converted::configurations_left_out`), and only none exported an error. The desktop entry also
  offers STEP (`model/step`) and DXF (`image/vnd.dxf`).
- AccessKit (`egui-winit`'s `accesskit` feature): the window is created hidden, the adapter
  attached, then shown. The status bar notice and the failed-features pill are live regions
  (`widgets::announced`: assertive for an error, polite otherwise). The viewport is named "3D view"
  and described in words (`scene_description.rs`, rebuilt only when the revision, evaluation, units
  or edited sketch changes): bodies, sketches and datums by name and count, the first
  `NAMED_AT_MOST` bodies with size and lowest corner, or while a sketch is edited its curves,
  points, constraints, how constrained it is and the span of its points. Its children are one label
  node per item (`SceneDescription::items`, `viewport::scene_items`, built only while AccessKit is
  on, at most `ITEMS_AT_MOST`; a body's faces beneath it, at most `FACES_AT_MOST`; while a sketch is
  edited each curve and point), zero-sized and sensing only hover, so they take no room, pointer or
  Tab stop. Text painted over it (prompt, hover description, snap and measure labels) is also a
  `Label` node (`canvas::announce`), the prompt and keyboard-highlight description polite live
  regions.
- The about dialog names `THIRD-PARTY-NOTICES.txt`, the licences of the other fonts and of the
  linked libraries that every package installs beside `LICENSE` (`notices.md`); a test in `about.rs`
  holds the packaging scripts, the PKGBUILD, the MSI and their checks to naming it.
- Wayland app ID, X11 class and Windows window class are `about::APP_ID`, which must match the
  desktop entry's name.
- The logo is `packaging/caditor.svg`; `packaging/render-icons.sh` renders it into
  `packaging/icons/`, committed, so neither build nor release needs an SVG renderer. `logo.rs`
  embeds some renders and `logo::show` draws the smallest covering the requested physical size,
  hidden from AccessKit. The window icon is a fixed render (X11 and Windows, which also gets the
  largest as its taskbar icon; Wayland takes it from the desktop entry). The Windows executable's
  icon is built from the same renders (`windows.md`). A render that fails to decode logs a warning;
  tests decode every render.
