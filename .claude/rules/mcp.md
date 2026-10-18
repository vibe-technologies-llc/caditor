---
paths:
  - "crates/caditor/src/mcp.rs"
  - "crates/caditor/src/mcp/**"
  - "crates/caditor/guide/mcp.md"
---

# MCP server

`caditor mcp MODEL [--folder DIR]...` (`cli::Serving`, `mcp::run`) serves one model to an AI
assistant over the Model Context Protocol on stdin and stdout, with no window, logging to stderr
and the session log as the app does. The user guide page is `guide/mcp.md`.

## Protocol (`json_rpc.rs`, `server.rs`)

- The protocol layer is our own, not the `rmcp` SDK: MCP over stdio is newline-delimited JSON-RPC
  that `serde_json` and one reader thread serve, while `rmcp` would bring tokio and macros into a
  workspace with no async runtime. The price is following each protocol revision by hand
  (`PROTOCOL_VERSIONS`). It lives in the app crate, not a crate of its own, since every tool reuses
  the app's headless, field, sketch placement, extrusion detection and hole code.
- Hand-written JSON-RPC 2.0 on `serde_json`, one message per line. `json_rpc::read_line` reads at
  most `MAX_LINE_BYTES` a line (a longer one is dropped to its newline and refused as a parse
  error); `json_rpc::parse` turns a line into `Incoming` or a typed `Rejection` (parse error,
  invalid request; batches are refused), never panicking. The `mcp` fuzz target feeds it bytes and
  decodes tool arguments from them (`caditor::fuzzing::mcp_lines`).
- A reader thread (`read_messages`) parses lines and hands them to the serving thread over a
  channel. It answers `notifications/cancelled` itself by setting the request's flag in
  `Cancellations` (registered when the request is read, so a queued request is cancelled too); a
  cancelled request gets no response, as the protocol asks. Everything else runs in order on the
  serving thread, the only writer of stdout.
- `initialize` agrees on the client's version when it is one of `PROTOCOL_VERSIONS`, else offers
  the newest; `structuredContent` is sent only from `2025-06-18` on. Offered: `ping`,
  `tools/list`, `tools/call`, `resources/list`, `resources/templates/list` (empty),
  `resources/read`. An unknown tool is `-32602`, an unknown resource `-32002`.
- A call with `_meta.progressToken` gets `notifications/progress` with a step count that only
  grows and a message ("Recomputing: 3 of 7 features", "Measuring Plate (1 of 2)").
- A panic in a call is caught (`catch_unwind`): the answer is an internal error, the evaluation is
  forgotten so the next call recomputes, and the model keeps every change made before the call
  (`Editor::apply` is atomic).

## Session (`session.rs`)

- `Session` is the one layer every front shares: an `Editor` over the document, a `Recompute`
  cache with the last `Evaluation`, and a `Storage` worker journaling every change as the window's
  `Model` does (`JournalEntry::Apply`, `Undo`, `Redo`; restarted when it stops; rebases and saves
  followed from its reports). Changes go only through `Session::perform(Change)`, the headless
  counterpart of `Action::Apply`, `Undo` and `Redo`, so a tool cannot break the model's rules.
  The automation entry in `docs/FEATURES.md` is meant to reuse it.
- Opening refuses anything but a `.caditor` path in an existing folder, a model whose journal
  another caditor holds (`FileJournal::InUse`) and one with recoverable changes
  (`FileJournal::Recoverable`), so nothing is overwritten that the window would offer to restore.
  A path that does not exist starts an empty, unsaved model.
- `evaluate` recomputes only when the revision moved, with display meshes (rendering and extrusion
  detection need them), under the call's cancel token; a cancelled recompute leaves the session
  unevaluated, so the next call starts again from the cache. Every call that answers about the
  model, writes included, waits for the recompute it caused.
- `save` sends a `SaveRequest` to the storage worker (atomic, keeping the replaced version, the
  Version history preference's retention, refusing a file changed on disk since it was opened)
  and waits for its report. Closing discards the journal only when the model is saved; otherwise
  it stays for the window to recover.

## Tools (`tools.rs`, `reading.rs`, `writing.rs`, `sketching.rs`, `sketch_edits.rs`, `sketch_moves.rs`, `sketch_cuts.rs`, `sketch_projections.rs`, `sketch_additions.rs`, `features.rs`, `solids.rs`, `sweeps.rs`, `reshaping.rs`, `datums.rs`, `mates.rs`, `curves.rs`, `edits.rs`, `tree_edits.rs`, `edges.rs`, `files.rs`)

- `Tool` lists every tool (`all_variants!`); `Tool::decode` turns arguments into a typed `Call`
  through `serde` (`deny_unknown_fields`), and `Call::run` does it. Errors are `ToolError` and the
  per-area enums (`ReferenceError`, `SketchingError`, `ModellingError`, `FileError`,
  `RenderError`, `ChangeError`, `PlaceError`, `FeatureError`, `MeasuringError`, `TreeError`,
  `SweepToolError`, `ReshapeError`, `FeatureEditError`, `SketchEditError`, `DatumError`,
  `MateError`, `CurveToolError`),
  sent as a tool result with `isError` and the message as a sentence; the app's own refusals
  (field parsing, attachment, constraint conflicts) keep their words.
- References are by stable identity only (`references.rs`): features and bodies by id or name,
  sketch entities by id or `origin`, `horizontal_axis`, `vertical_axis`, faces by a key of their
  `FaceName` digest and occurrence (`face_key_text`), edges by their `EdgeName` digest
  (`places::edge_key_text`), resolved on the body as it stands now, or as a feature received it when
  editing that feature or listing with `before` (below). `places.rs` turns a plane
  (`XY`/`XZ`/`YZ`, `{body, face}` through `attachment_in`, `{datum}`, `{frame, plane}`), an axis
  (`X`/`Y`/`Z`, `{body, edge}`, `{body, face}`, `{sketch, line}`, `{datum}`, `{frame, axis}`) or a
  point (`"origin"`, a corner `{body, corner: [x, y, z]}` found among the shown body's vertices
  and named by `vertex_names`, a round edge's centre `{body, edge}`, a sphere's or torus's centre
  `{body, face}`, `{sketch, point}`, `{datum}` for a datum point or a coordinate system's origin)
  into a `PlaneReference`, `AxisReference` or `PointReference` as the app's pickers capture them:
  an edge or face captured on the shown body, resolved on the body's state at the bar
  (`body_state_in`, `face_at_bar`, `edge_at_bar`) and captured there; a datum or sketch only when
  it stands above the bar. These are serde untagged enums of `deny_unknown_fields` structs.
- Every write is one transaction, one undo step, built with the app's own pieces: sketches as New
  sketch makes them (`sketch_placement::attachment_in` on a face), geometry and constraints after
  settling the sketch to its last result as sketch tools do, constraints tried first with
  `constraint_trial::conflict`, extrusions through `extrusion_detection::extrusion` unless an
  operation is given (a cut from a sketch on the cut body's face runs into it), holes from
  `hole_tools::new_hole`, the swept sketch hidden as the tools hide it. Values are parsed by
  `field::parse_expression` in millimetres and degrees. A write answers with its change's label
  and every failing feature with the tree's reason and remedy, marked `new` when the change made it
  fail.
- Feature writes build the feature the app's tool would (each `*_built` function returns a
  `Built`: kind, default title, asked name, sketches to hide, answer details) and `features::added`
  adds it as one `Create …` transaction, named as the tool names it
  (`editing::next_feature_name`): a revolve (`RevolveAxis::Sketch` or `Model`, full turn without an
  angle, `up_to` a plane (`RevolveExtent::UpTo`, `reversed` kept on an edit), `TwoSides` with `second_angle`, `solid_tools::default_operation` without an operation),
  fillets and chamfers (`EdgeReference::capture_in` per edge key,
  `EdgeGroup::Face` per face key, `EdgeGroup::Body` for `all_edges`), shell (`face_thicknesses`, replaced as a list), offset face,
  primitives (`primitive_tools::default_shape` with the given sizes, at a face's middle
  `hole_tools::middle_of` and adding to that body by default), mirror (of a body, of features or of `faces`) and linear or circular
  pattern of a body or of repeatable features (`pattern_tools::repeatable`, `skipped` copies as
  `Instance`s, the original refused), combine and move. Values are
  `Amount`s, a number of millimetres or degrees or an expression, parsed by
  `field::parse_expression`; offsets, moves and turns may be negative. The kernel's refusals arrive
  as the new feature failing, in the tree's words.
- `solids.rs` holds extrusions and holes. An extrusion's end is one of a distance, `through_all`,
  `up_to` (`"next"` or a plane, `ExtrudeEnd::UpToNext`/`UpToFace`, with `end_offset`) or
  `up_to_surface` (`{body, face}` of any shape through `places::face_at_bar`,
  `ExtrudeEnd::UpToSurface`, no offset), and it takes `second_distance` or the object
  `second_end` (`TwoSides`, the same ends and its own `end_offset`, the two refused together),
  `start_offset` (`SolidStart::Distance`), `taper`, `direction` (an axis as
  `places::axis` takes it, `"square"` clears it) and `thin_wall` with `wall_side` (`Wall`,
  centred by default; `"off"` clears it, shared with the revolve through `solids::wall_of`).
  `regions` (extrusion and revolve, `features::regions_chosen`) are points `[x, y]` in the
  sketch's millimetres, each choosing the region containing it (`RegionChoice::Chosen`, the
  point first found in `selection::sketch_regions`), or `"all"`; `get_sketch` lists each region
  with an `inside` point, its area and holes. A hole takes the same ends but the curved face, a `style` (plain,
  counterbore, countersink or stepped with `steps`, clearing the standard as the panel does), `shape`
  slot with `slot_length` and `slot_angle` (either implies it), a `bottom` drill point with
  `point_angle` (implied by it), `sizing` by circles, head sizes, a `standard` metric size and `fit` through `hole_tools::with_standard` (a diameter clears
  the standard), and a tapped standard's `thread_form`, `thread_hand` and `thread_depth`.
- `sweeps.rs` adds sweeps (with a `twist` angle and an end `scale`), pipes (path as `{sketch, curves}` checked by
  `sweep_tools::is_path_curve`, `{body, edges}` or `{helix}`, a `SweepPath`), lofts through whole
  sketches or `{sketch, inside}` single regions (hidden as the tool hides them; an `end` point
  as `places::point` takes it, `"off"` clearing it, refused with `closed`; `closed`, and a `start_weight` or `end_weight` above 0
  leaving that end square, 0 freeing it), coils (two of turns, pitch and height as a
  `CoilLength`), ribs (square to a face's sketch, facing the body by `rib_tools::facing_body`; a web's `draft` and
  `ends`) and
  threads (the face's `Bore` picks the nearest size and default class, named
  `thread_tools::unique_name`); `reshaping.rs` adds split face (its `carry` square, wrapped or along an axis, the last two
  needing curves of a sketch), delete face, emboss (`add_emboss`: faces, a sketch, a `depth` and a `form` of `raise` or
  `recess`), move face (`add_move_face`: a signed `distance`
  along `direction`, `"normal"` or an axis as `places::axis` takes it, and an `angle` about `axis`,
  `"none"` or an axis; an edit keeps what it is not given), split (along a plane,
  `{sketch}` or `{tool}` body) and scale, each with the app tool's defaults.
- `datums.rs` places datums one way each, the way chosen by which arguments are given (two ways at
  once refused, `DatumError::TwoWays`; an edit keeps the way unless another is given): planes
  offset and turned (`DatumPlane`) or `PlaneThrough` three points, midway, two lines, an axis and
  a point, square to an axis at a point, tangent to a curved face toward a point, or square to an
  edge at a distance (`CurveStation`); axes `Along`, `Intersection`, `Points`, `NormalTo` a plane
  or `SquareToFace`; points at a point with an offset (`DatumPoint`) or `PointBy` crossing lines,
  an axis and a plane, three planes, a distance along an edge, an edge's middle or a face's
  centre; coordinate systems (`DatumFrame`) from an origin, an X axis and a plane, the missing one
  chosen as the tool does (`datum_tools::least_steep`). `mates.rs` builds a `Mate` whose
  `MatePair` follows from the parts given (flush, concentric, both, an angle, a round face resting
  on a plane, a flat face on a round one, a point on a point or plane), the moving parts on the
  moved body and the targets off it; an edit keeps the pair's kind (`MateError::KindKept`).
  `curves.rs` adds helices (`Helix`, two of turns, pitch and height through `coil_length`, a
  `start_angle` or a `start_point` that replaces it, each given clearing the other), curve patterns (`sketch_pattern_tools::kind` for a sketch passing
  `is_curve_path`, spread or `spacing`, following or keeping the turn) and point patterns
  (`PointsPattern` from a base point, the sketch needing lone points).
- `edit_feature` (`edits.rs`) edits a feature with the arguments of the tool adding its kind
  (`edits::tool_for`; sketches, imports, removed bodies, measurements and reference pictures are
  refused in words). The arguments are decoded with that tool's struct, whose fields are all
  optional so an edit gives only what changes: the same `*_built` function runs with the feature's
  kind as its base, keeping every part not given (references too, so a fillet's edges need not be
  named again), and a missing required argument is `FeatureError::Missing` only when adding. The
  edit first evaluates, then runs the rebuild inside `Session::looking_before(feature, …)`:
  `Session::document()` and `evaluation()` then answer with the document rolled back to before the
  feature (`Document::rolled_back_before`) and the evaluation with every body the feature uses
  pointed at the state it saw (`Evaluation::standing_before`, from the `seen_bodies` the recompute
  records for every feature, so a mirror's `faces`, a pipe's `{body, edges}` and an extrusion's
  `up_to_surface` count as much as a fillet's edges), so every resolver (`references::face`,
  `places::edge`, `face_at_bar`, datums above the bar) finds faces and edges as the app's open
  feature does, an edge a fillet rounded away included. A key the input lacks is
  `FeatureEditError::NoFaceBefore` or `NoEdgeBefore`, pointing at `list_faces` and `list_edges`
  with `before` (a feature, `edits::viewed_before`; `NothingBefore` for one using no body). A
  feature using no body resolves on the body as it stands. The change is the app's own
  `edit` transaction for the kind (`SetFeatureKind`, carrying parameter owners where the app does), labelled `Edit <name>`, plus `Edit::RenameFeature` for a new
  `name`, one undo step.
- Tree edits (`tree_edits.rs`) use the tree's own transactions: `Edit::RenameFeature`,
  `Document::suppression`, `Document::move_row` before or after another feature (its
  `check_moves` refusal is the answer) and `Document::deletion`, which refuses while
  `dependents_of` finds users unless `dependents` says `delete` or `keep`, as the delete dialog asks.
- Sketching beyond the first slice lives in `sketching.rs`: `add_sketch_geometry` also draws arcs
  (about a centre, or through three points), slots, arc slots, regular polygons (by vertex, by
  the middle of the first side, or on a side from `from` to `to`, `shapes::polygon_on_side`), fit
  or control splines, conics (start, apex, end and `rho`, a `SplineKind::Conic`), ellipses and
  elliptical arcs (start and end taken toward their positions, `shapes::toward_on_ellipse`), each
  with the joints, tangents, equal arcs and construction circle the app's `Draft` recipes in
  `drawing.rs` add (a slot's top line is held level or upright when its centres line up), from the
  `shapes.rs` geometry; a tangent arc leaves a point ending a line, arc or spline of the sketch as
  shown (`drawing::continuing`, from `from_point`) or the end of an earlier item of the same call
  (`after_item`: `Pen` records every entity it places and `Pen::drawn_so_far` is the shown sketch
  with them, so `continuing` sees them), joined to it by `Coincident` and `Tangent`
  (`shapes::tangent_from`); a conic's `rho` is a number or an expression (`RhoValue`), and an
  expression using parameters adds the `Rho` dimension holding it, as the app's typed rho does;
  `add_sketch` takes a datum plane (`datum`) or a plane of a coordinate system (`frame` with
  `plane`) through `DatumTarget`, as New sketch on a selected datum does; `remove_sketch_items`
  removes constraints and entities (`Transaction::remove_sketch_items` expands an entity to its
  users, and the answer lists everything gone); `edit_sketch_constraint` changes a dimension
  through `field::dimension_transaction` (the dimension field's parsing, units and parameters) and
  turns a constraint on or off, both after settling the sketch. Their failures are
  `SketchToolError`.
- `modify_sketch` (`sketch_edits.rs`, `sketch_moves.rs`, `sketch_cuts.rs`, `sketch_projections.rs`,
  `sketch_additions.rs`) is one tool for the sketch tools that
  change geometry, an `operation`-tagged `Modification` whose variants each name the sketch, so the
  registry holds one entry. Each plans one transaction on a working copy (the definition with the
  shown positions, as `trimming::working`) through `reshape_sketch`, settled first, with the
  sketch crate's operations and the app tools' labels: trim (pieces found again by their middle on
  whichever curve carries them, `trimming::carrier`, as a trim path), extend (by the end point or a
  position by it), offset (any number of chains, `whole_chain` as a click takes it, each side by `toward` or its
  default, a negative distance the other side, `both_sides` with round or flat caps), mirror,
  rectangular and circular patterns (`patterning::turned`, `square_to`), fillet and chamfer of
  corners named by their point or a pair of curves (`ChamferSize`). Move, rotate and scale build
  the app's `sketch_drag::Moving` or `Transforming` drags (an `about` point overrides the pivot)
  and solve them from the shown sketch with `solve_geometry_from`, committed as `settle_sketch`:
  constraints hold as in a drag, the answer says whether every point `reached` its target, and a
  change the constraints hold entirely is refused. Project (`sketch_projections.rs`) collects
  `projecting::edge_projection` and `face_projections` of a body's edges and faces in its state
  seen by the sketch (`body_result_seen_by`), and `projecting::projections_of` for a body corner
  (`corners`, found among the seen body's vertices by position and told apart by `vertex_keys`),
  curves of another sketch above it (`source_sketch` with `source_curves`, else
  `projecting::sketch_candidates` for the whole sketch), principal `axes`, datum axes and points
  (`datums`) and the `origin`, into one transaction leaving out what the sketch already projects.
  `projecting.rs` takes a `Surroundings` (the document, the evaluation and the shown sketches;
  the window's `Model` and the MCP `Seen` both are one) and fails with `ProjectionError`, which
  the window shows as its sentence. Intersect is `projecting::intersect` for a face of a body (one
  cut), a body (the cut through all of it, found through its first face key, as Shift-click), a
  datum plane or a principal plane (a construction line). Construction (`ConstructionChange`,
  switching every curve the other way unless `construction` says), split (`check_split`,
  `split_at`), break (`break_curves`) respace (`respace_fit_spline`, only for splines still
  evenly spaced) and insert_control_point (`splines`, and `point` with a single one;
  `check_point_insertion`, `insert_control_point`) are `sketch_cuts.rs`; tangent circle (two curves and a `radius`, or three),
  blend (`first` and `second` ends, `continuity`), gear (a gear, `pair` or `sprocket` as
  `gearing::Part`, at `center` or `at`, defaults of the Spur gear panel; a single gear is stored
  with literal expressions, a pair and a sprocket drawn as curves; `get_sketch` does not list
  stored gears or texts) and copy (`Sketch::clip`
  then `paste`, within the sketch shifted by `clipboard::same_sketch_shift` or from `from_sketch`
  in place, `by` or `to` moving it) are `sketch_additions.rs`, each with its tool's transaction
  label. Lengths and angles are
  `Dimensioned` from `field::parse_expression` (a plain angle in degrees), kept as expressions where
  the operation keeps one. The answer lists the entities added and removed and the sketch status.
- Reads: the feature tree with `state_of`'s words, parameters, bodies with `exact_mass` and
  `extent` under the kernel interrupt (`interruptible`), faces (at most `MOST_FACES`), edges with
  `edge_measure`'s kind and length (at most `MOST_EDGES`), the distance and angle between two
  points, faces or edges by the kernel's `distance` and `angle`, a sketch with `SketchSummary`
  (a spline with its form (control, fit, conic, piece or knotted), a fit spline's spacing and a conic's rho, an ellipse or elliptical arc with its major end and
  minor radius), and a picture through `snapshot::looking_from` and `png_bytes`, sized `SMALLEST_SIDE` to
  `LARGEST_SIDE`.
- Files: `save` writes the model's own file; `save_as` sends the same `SaveRequest` for a new
  `.caditor` path (`Session::save_to`, refusing a file that exists), and the session follows the
  saved path from the worker's report, as the window's Save as does; `export` writes
  `headless::deliver`'s formats; `import` reads a STEP model or mesh through `import::read_model`
  and `bodies_transaction`, or a DXF or SVG drawing through `read_drawing` and
  `drawing_transaction` into a new sketch on a principal plane or an existing sketch, with the import
  dialog's `DrawingOptions` (`unit`, `scale`, `recentre`, `left_out_layers` by name, the answer
  listing the drawing's `layers`), refused as the app refuses it (`import::outgrows`, `outnumbers`, nothing imported). All of them reach only
  the model's folder, the `--folder` folders and folders below them (`files::allowed`: canonical
  parent, no link as the target). Nothing reaches the network.

## Resources (`resources.rs`)

- `caditor://model/summary` and `caditor://model/parameters` (JSON, the `get_model` and
  `list_parameters` answers) and `caditor://guide/<page id>` for every guide page, rendered from
  the parsed page to Markdown with commands as their titles and links as guide URIs.

## Tests

- `mcp/tests.rs` drives the server over in-memory pipes (`Client`, a thread serving
  `serve_watching`), and the cancellation test holds the output at the first progress
  notification until the cancel is registered (`Gate`), so it never races.
  `mcp/end_to_end_tests.rs` builds a plate with a pocket and two holes by tool calls alone and
  checks its volume, undo, redo, a parameter change, save, export and the saved file.
  `mcp/feature_tests.rs` checks each feature tool by the volumes it leaves (fillets, shells,
  revolves, primitives, mirrors, patterns, combines, moves), the edge listing and measurements, the
  tree edits with undo, and the refusals.
  `mcp/sketching_tests.rs` covers each sketching tool, extruding what the shapes draw to check
  their regions, and sketches on a datum plane and a coordinate system of a saved model.
  `mcp/sketch_edit_tests.rs` checks elliptical arcs, conics, tangent arcs and polygons on a side,
  and each `modify_sketch` operation by positions, counts and extruded areas, with undo,
  constraints holding a move, projections following an edited body, and the refusals.
  `mcp/sketch_cut_tests.rs` checks construction, split, break and respace (a saved model holding an
  evenly spaced spline); `mcp/sketch_source_tests.rs` projecting corners, axes, datums, the origin
  and other sketches, intersecting a body, a face and planes, and copying within and between
  sketches; `mcp/sketch_shape_tests.rs` tangent circles, blends, gears, pairs and sprockets, a
  tangent arc continuing an item of the same call and a conic's rho held by a parameter.
  `mcp/edit_tests.rs` edits primitives, extrusions, patterns, revolves, fillets and holes by
  volume, with undo, renaming and refusals, and checks extrusion ends (curved faces, second sides), regions, walls, directions, offsets and tapers,
  revolves up to a plane, edits naming the edges and faces a fillet or shell received, and the edge,
  face and mirrored faces a pipe, an extrusion up to a face and a mirror received, and hole standards, heads, threads, steps, slots, drill points and circle sizing. `mcp/sweep_tests.rs` checks pipes, sweeps, lofts, coils,
  threads, scales, splits, split faces and deleted faces by volumes, body and face counts.
  `mcp/datum_tests.rs` places datum planes, axes, points and coordinate systems every way and
  edits them, probing each with a box placed on, mated to or turned about it, and mates bodies by
  their bounds, with undo and refusals. `mcp/curve_tests.rs` winds and edits helices (read back
  from the saved file) and pipes one, patterns along curves and at points by volume, imports a
  STEP file and an SVG drawing, saves under another name and checks the refusals.
