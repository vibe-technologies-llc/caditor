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
  revision, evaluation, the open feature's draft, displayed sketches, body meshes, display style,
  `Contrast`, editing `Context`, faceting level); anything new that affects the drawing must feed
  that key.
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
  returns. Neither draws frames while it waits. While the renderer is uploading meshes
  (`Renderer::is_uploading`) a frame follows each frame, and the frame that sees the uploads
  finished asks for a pick again (`ViewportState::pick_again`), since picks made meanwhile could
  not pick the replaced bodies. Every request goes through `Session::request_redraw`, never straight to the window.
  Animated widgets (`widgets::spinner`) and progress readouts repaint on a timer
  (`request_repaint_after`), never every frame, and the text caret does not blink (`app-look.md`).
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
  area, centroid, second moments and bounding-box size) and each face's area (`BodyFace::area`)
  exactly from the kernel (`mass_properties`, `extent`), independent of the mesh. A face the
  kernel could not integrate takes the mesh's triangles, and the body is then marked approximate
  with the mesh's chord (`MassAccuracy::Mesh`).
- A body is drawn in its appearance colour, else the default (`body_appearance::DEFAULT_COLOUR`);
  a failed or outdated feature on it tints it as before. While a sketch is edited, bodies are
  dimmed and not pickable.
- Every failed feature whose error has a `place` gets a marker there in the error colour, drawn in
  front of the model with a "<feature> failed here" label that screen readers read.

## Offers

- `offers.rs` works out what the selection offers the toolbar and status bar only when the
  selection, the model revision, its evaluation, the length unit or the edited sketch and opened
  feature change, not every frame (the Hole tool's start is one of the offers). It
  describes only the first `MAX_DESCRIBED` items and counts the rest (the status bar's tooltip
  ends "and N more"), since describing an item can scan its body.
- A lone selected face, edge or sketch region, or a lone body chosen in the tree with nothing
  selected, also gets its size in `Offers::size` (Area, Length or Size, `measure::size_text` and
  `body_size_text`, marked ≈ when approximate), which the status bar shows beside the selection;
  the offers are worked out again when the bodies' meshes finish (`Model::bodies_pending`), since
  areas and sizes read the converted body (a face's area falls back to its mesh triangles,
  marked ≈, until then).
- `Selection::generation` is globally unique per content change, so caches key on it rather than
  cloning and comparing the set: the offers, the scene cache's highlight key, the check that
  clears the tree's selected row when the view's selection changes, Measure, the panels' Use
  selected offers
  (`feature_fields::offered_change`, kept per feature and slot in egui's memory), and the
  viewport's check that the selection is still available (`retain_available`, rerun only when the
  selection, revision, evaluation or editing context changed).

## Measure

- The Measure command toggles `MeasureTool` in the `Workspace`; while open, `measure_panel.rs`
  draws a right-hand panel. Measuring never changes the document; only a reading's New parameter
  from this value does, as an ordinary change.
- `Measurements` resolves the selection on the UI thread into points, edges and faces (with their
  result's `Arc`), sketch curves (`profile_curve` lifted onto the sketch's solved plane), and the
  principal and datum planes and axes, a coordinate system's axes and planes and its origin as a
  point (a plane reads through a point with its normal, an axis
  through a point with its direction, a circle or arc its radius, diameter, centre and sweep), then measures on its own worker (newest job wins, a panic becomes a failed
  reading) only when the selection, revision, evaluation or the coordinate system read in
  changes. Until the current result
  arrives the previous readout stays, dimmed (`Freshness::Stale`) under a Measuring header, so
  nothing jumps; the measured line in the view comes only from the current readout.
- Selected sketch regions (`Pickable::SketchRegion`) gather into one item per sketch
  (`Subject::Regions`, the regions cloned from the result's display regions with the sketch's
  solved plane), counted as one item (`item_count`), titled "<sketch> › N regions" when several.
  They read their summed section (`caditor_kernel::section_of`, `kernel-profile.md`): area,
  perimeter of the union's outline, centroid in the world, Ix, Iy and Ixy about the centroid along
  the sketch's x and y, the polar moment J, the principal moments I1 and I2 and, unless the two
  agree, the principal angle from the sketch's x to I1's axis and that axis as a world direction.
  Second moments are `Value::SecondMoment` (`LengthUnit::measured_second_moment`, length⁴); a
  spline side marks every value ≈. A region is never one side of a "Between them" card.
- A `Readout` is a card per item and, for two items, a "Between them" card; more than two asks for
  fewer, without describing or measuring any. Approximate values are marked with a note, a callout under its card. Mass properties are
  read each frame from `BodyMass` for the selected items' bodies and those of the rows chosen in
  the tree, else every shown body, with the
  body's material and its mass from the density (No density set without one, a warning note when
  the density cannot be evaluated) and, with a density, the moments of inertia about the centroid
  along the model axes and the principal moments (`MassProperties::second_moment`). Two or more bodies also get an "All N bodies" card first: summed
  volume and area, the centroid weighted by mass (by volume unless every body has a density), and
  the total mass and inertia about that centroid only when every body has a density.
- Every row of the reading and mass cards (not Interference's, which share `show_card`) has a menu
  on its "⋯" button (named "More for <label>", so Tab and Enter reach it) and on a right-click of
  its value (`measure_panel::row_menu`): Copy value puts the text as shown, unit and any ≈
  included, on the clipboard (`egui::Context::copy_text`, as Copy all does) with a notice; New
  parameter from this value, offered for a reading's `Value::Length`, `Angle` and `Area` only (a
  position, direction or second moment is no one value the expression language holds), adds a
  parameter through `parameter_table::add_with` in one transaction: named after the reading's
  label (`parameter_stem`, "Along X" giving `along_x1`, the first free number), holding the value
  in the model's unit as `LengthUnit::measured`, `AngleUnit::measured` or
  `measured_area_expression` (`mm²` and the like, rounded to the model's precision), with its
  name field focused for renaming and a notice saying so.
- Relative to (shown once the model holds a coordinate system, `MeasureTool::relative_to`, World
  by default, kept for the session) reads every position and direction and the Along X, Y and Z
  offsets in the chosen coordinate system (`measure::Relative`, its frame's inverse); distances,
  angles and the line in the view stay world geometry. A chosen system that is gone or has no
  result is read as the world with a warning callout saying so (`measure_panel::relative`).
- The closest points are drawn on the front layer (`scene::add_measurement`) with a distance
  label in `canvas::MEASURE`.
- Show or hide centres of mass (`Command::ToggleCentresOfMass`, View menu, palette) is a view aid
  (`ViewAids::centres_of_mass`, kept for the session in `ViewportState::aids`, not saved; it
  reaches the scene through `Sources::aids` and `Revisions::aids`). Outside sketch editing it
  marks each shown body's centroid (`BodyMass`) with a ring on the
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
  (`BodyMass::of`) and its mesh edges drawn on the front layer; every
  finding with a place gets a marker and a label in the view (error for overlaps, the measure
  colour for touches, warning for unchecked pairs) and a Show where button on its card.

## Face analysis

- Analyse draft, Analyse minimum radius, Analyse tool reach, Analyse curvature, Show zebra stripes
  and Show a chrome reflection (`AnalysisCommand::Draft`, `Radius`, `Reach`, `Curvature`, `Zebra`,
  `Chrome`, View menu, palette) toggle `AnalysisTool` in the `Workspace` (`Kind` is which one, which
  the panel's segmented control, a menu once it is too wide, changes too); while open, `analysis_panel.rs` draws a right-hand panel
  beside any other. Like
  centres of mass it is a view aid: `ViewAids::analysis` (a `FaceAnalysis`) or
  `ViewAids::reflection` (a `caditor_render::Reflection`), worked out each frame from the tool by
  `AnalysisTool::analysis` as an `Analysis` and handed over with `ViewportState::set_analysis`,
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
- Reach judges undercuts for a three-axis machine: the tool comes from the direction (`Kind::Reach`
  shares the pull, Use selected and Reverse with Draft, captioned Reach from), and per triangle a
  face turned from it (`FACING_SLACK`, so walls square to it are reached) is `Band::FacesAway`;
  otherwise the triangle's centroid, nudged off its face along its normal, is `Band::Blocked` when
  any triangle of the shown bodies lies over it along the direction, else `Band::Reachable`.
  `reach.rs` projects every triangle along the direction into a 2D uniform `Grid` (at most
  `MOST_CELLS_ACROSS` cells a side; triangles seen edge-on cast no shadow) with slacks of
  `SLACK_PER_SIZE` of the bodies' size, so a face sharing an edge with the sample never blocks it.
  Since the result depends on every shown body, `Analyses::prepare` keeps one `Occluders` (the
  direction and weak references to the shown meshes, `analysis::shown_meshes`) and stamps its
  generation into `FaceAnalysis::Reach`, so a change of bodies, visibility or direction is a new
  analysis; the grid is built once on first use, on the analysis thread when the mesh and the
  occluders together pass `INLINE_TRIANGLES`. Any other analysis, or none, drops the occluders.
- Curvature colours each triangle by a curvature of the display mesh against a Reference radius R
  (a length expression, 10 mm at first, above zero): `ShapeOperator::of` fits the 2×2 shape
  operator to how the corners' normals turn along the three edges (least squares in the triangle's
  plane, positive where the face bulges out of its outward side), giving the Gaussian curvature
  and the largest and smallest principal curvatures (`Measure`). Five steps each, from
  `Measure::bands`: beyond −1 (scaled by R, or R² for Gaussian), below −`FLAT_SHARE`, within
  ±`FLAT_SHARE` (flat for the principal ones, flat or bent one way, so developable, for Gaussian),
  above, beyond +1; their colours are `ScenePalette::bands.curvature`, a diverging scale.
- Zebra and Chrome change no colours: the scene puts every coloured, unmoved body mesh into
  `Scene::reflective_meshes` with its usual face styles (so hover, selection and picking read as
  before) and sets `Scene::reflection` (`render.md`). Zebra's stripes run along the X, Y or Z axis
  (Stripes along) with `MIN_STRIPES` to `MAX_STRIPES` stripes per turn (12 at first); the panel
  explains how stripes and reflections show G0, G1 and G2 joints instead of a legend of areas.
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
  `ScenePalette::bands` (Reach reuses the drafted and undercut colours, with `blocked` of its own),
  or the face's own colour for no band, and the source face's pick id
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

## Isocurves

- Show or hide isocurves with combs (`AnalysisCommand::Isocurves`, View menu, palette, no default
  key) toggles `IsocurveTool` in the `Workspace`; while open, `isocurve_panel.rs` draws a
  right-hand panel beside any other. It never changes the document, is kept for the session and
  forgets its faces in a new one.
- It follows the selection like the comb: the selected body faces (at most `MOST_FACES`) become
  the chosen faces, and a selection holding none keeps them. For each, Lines along (Both, U, V)
  picks the directions and Lines each way (`MIN_LINES` to `MAX_LINES`, 5 at first) how many runs
  `Solid::isoparametric_runs` finds per direction (`kernel-intersect.md`); every run becomes a
  polyline of `STEPS_PER_LINE` steps and a comb of Teeth per line + 1 teeth from the surface's
  derivatives along it (`comb::spaced_teeth_along`, so teeth sit at equal lengths as on curves).
  All lines share one comb (`Isocurves::comb`) scaled by the comb's Scale, so faces and directions
  compare directly; joints are not judged.
- The work runs on an `isocurves` thread whenever the faces, the settings, the revision or the
  evaluation change (`Basis`); a newer basis cancels the older thread through the kernel
  interrupt, the last finished lines stay drawn until the new ones arrive (the panel says it is
  working), and the finished result wakes the app. A face that no longer resolves is counted as
  gone in a warning callout; a panic in the thread is a warning callout too.
- The panel lists each face with the smallest radius of its U lines and of its V lines in words
  (or straight), so nothing is told by the drawing alone.
- The drawing (`IsocurveDrawing`, rebuilt only when the lines or the scale change) goes into the
  overlay batch on the front layer like the comb: each line over a wider `ScenePalette::hole`
  halo in `CombLook::isocurve`, then the comb's teeth and envelope (`CombDrawing::add_to`); the
  palette test holds the isocurve colour to 3:1 over the halo and the canvas.

## Section view

- Show or hide the section view (`SectionCommand::Toggle`, View menu, palette, no default key)
  toggles `SectionTool` in the `Workspace`; while open, `section_panel.rs` draws a right-hand panel
  beside any other and the model is cut. It never changes the document. The planes are a view
  state kept for the session, not saved with the model or in the preferences: they only look into
  the model, a saved one would mean a format change and an undo entry for what is not an edit, and
  sections set up for one look are rarely wanted when the file is opened again. A new session
  forgets the planes, which may refer to its IDs (`SectionTool::forget`).
- A plane (`Cut`) starts from a base: a principal plane (the XY, XZ and YZ buttons), a datum plane,
  a coordinate system's plane, a flat face (its outward normal, by `FaceKey`) or a sketch's plane,
  taken with Use selected (`SectionCommand::UseSelected`, `section::base_from`) and resolved again
  each frame (`section::base_plane`), so it follows edits; a base that is gone shows a warning
  callout and that plane cuts nothing. Its Offset moves it along the base normal into the side it
  keeps, Tilt and Turn turn it about the base's own x and then y axis (length and angle expressions
  of named parameters), Cut away the other side (`SectionCommand::Flip`) flips the side in place,
  and Cut faces chooses hatched or filled (`caditor_render::CutFace`). The cut side is the base
  normal's: above the XY plane, in front of the XZ plane, right of the YZ plane, outside a face.
- Opening the view with no plane, and Add a plane (`SectionCommand::Add`), adds one through the
  centre of the model's bounds (offset rounded to 0.1 mm) on the first principal plane not in use
  (XZ, YZ, XY), up to `MAX_SECTION_PLANES`; Remove (`SectionCommand::Remove`, or the card's bin)
  drops one. The commands act on the current plane, the one added or edited last, marked Current.
- `SectionTool::planes` resolves the planes each frame into `ViewportState::set_section`;
  `ViewportState::shown_section` adds the sketch slice (`app-sketching.md`) and hands them to
  `SceneCache::set_section`, which sets `Scene::section` on the cached scene without rebuilding it
  and bumps its generation, so a moved plane costs a uniform write (`render.md`) and the hover is
  picked again. Picking, the pick list, paint selection and box selection (the `seen` closure of
  `select_in_model` treats a cut point as unseen) reach only what is shown, so Measure does too.
  An exported image keeps the section shown (`ViewportState::image`); thumbnails and the
  headless PNG have none.

- `samples.rs` builds parametric models through the document API (so always the current format),
  fully constrained with dimensions naming parameters; a test recomputes each and checks its
  volume. Open sample opens one untitled and unmodified.

## User guide

- The guide ships inside the binary: one page per tool or subject in `crates/caditor/guide/<id>.md`,
  embedded by `include_str!` through `guide::Page::source`, parsed once (`guide::GUIDE`, a
  `LazyLock`), so it reads offline and never touches the disk. `Page` is the only list of pages and
  `Chapter` groups them into the contents in `Page::ALL` order.
- Pages are a small markup of our own, not Markdown read by a crate: `# ` the title (first line),
  `## ` a heading, `- ` a list item (indented lines continue it), blank lines between paragraphs,
  and inline `**strong**`, `` `code` ``, `[text](page-id)` links and `{command:id}`, which renders
  the command's title with the user's current keys (`guide_panel::command_text`), so pages never
  repeat default shortcuts. Unclosed marks stay text. Tests hold every link and command id to one
  that exists, every page to a title, content and an incoming link, and the markup's reading.
- `guide_panel.rs` draws it as a right-hand side panel sharing the side panels' room (`Guide` in
  the `Workspace`, reset by a failed frame): back, contents and close buttons, a search field over
  the titles and text (`guide::search`, title matches first, with a snippet around the first word),
  the contents by chapter, and a page with a Next link to the following one. Links are underlined
  `accent_text` labels, focusable and named as links.
- Help › User guide (`Command::Guide`, F1) opens the page of the current context
  (`guide::context`): the active sketch tool other than Select, else the open feature
  (`Page::of_feature`), else the first open side panel (Constrain automatically, face analysis,
  comb, isocurves, section view, interference, Measure), else the edited sketch or plane choice, else the tree's
  selected row; with none, the contents with the search field focused. Run again on the page it
  would open, it closes the guide. The palette's detail names the page.
- Panels link to their page: each side panel's header has a help button
  (`guide_panel::help_button`) and an open feature's card ends with "Help on <page>"
  (`help_link`); both ask through egui temp data (`guide::ask`, taken by `app::guide_commands`
  next frame), so panels need no new parameters. A new tool, feature kind or side panel gets a page
  (the exhaustive `Page::of_tool`, `of_feature` and `of_panel` fail to compile otherwise), and a
  change to what a tool does updates its page in the same commit.

## About, command line, accessibility, packaging

- `cli.rs`: `caditor [FILE]`; a `.dxf` or STEP file (by extension or header) goes to import as if
  dropped, anything else to Open. `caditor --export OUT [--resolution ...] [--size WxH] MODEL`
  (`headless.rs`) opens no window: it loads a caditor model or imports a STEP file, recomputes
  without display work, exports every body that built to the format `OUT`'s extension names and
  prints a summary. A `.png` instead recomputes with display work and draws the scene the way the
  image export does (`snapshot.rs`: the initial viewpoint fitted to the model, no grid or
  highlights, 1920×1080 unless `--size` says otherwise) on an `OffscreenRenderer`, so it needs a
  graphics adapter and refuses `--size` for any other format; load issues and left-out bodies are warnings, a failed feature a warning that
  also makes the exit status 2 (`PARTIAL_EXIT_STATUS`), and a model with no body, a DXF or SVG drawing,
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
