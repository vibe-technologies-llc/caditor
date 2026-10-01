# caditor roadmap

## Direction

caditor is a parametric CAD application for Linux. The project is judged on two things before
anything else:

1. **User experience.** Modelling should feel direct and predictable, without the failure modes
   that FreeCAD is known for: broken references after an upstream edit, workbench and mode
   juggling, opaque errors and a UI that freezes. The concrete requirements are in
   `.claude/rules/ux.md`.
2. **Never losing work.** Crashes and data loss are catastrophic failures. The concrete
   requirements are in `.claude/rules/reliability.md`.

Features are added only once they meet both bars; an unpolished feature is not shipped. An item
is implemented when it works end to end in the app, not when the APIs exist. Implemented items
and resolved decisions are removed from this file, and a milestone disappears once it is empty.
Git history is the record of what was done.

Categories below are ordered so that earlier ones unblock or protect later ones, and entries
within a category run from most to least important.

## Checks and CI

- No fuzz target covers the kernel or sketch operations, though "valid or an error, never a
  panic" is the central invariant: random profiles through `Profile::new`, `extrude` and
  `revolve` hit about 0.3% `Invalid` results (see Kernel correctness). Add seeded targets for
  profile sweeps, booleans, the solver and `Document::apply`, an unsealed `journal` target for
  torn tails, targets for `Settings::load`, recent files and `commands::parse_stored`, and a
  save-load round trip.
- The boolean tests use general-position placements, integer grids of aspect at most 4 and exact
  coincidence only, so the near-coincident bands and long-edge clipping losses in Kernel
  correctness were invisible. Add randomised aligned contacts with high aspect ratios and offsets
  of 1e-6 to 1e-4 mm.
- The ignored `random_placements_of_every_fixture` and
  `a_conflict_across_hundreds_of_entities_is_named_within_seconds` guard the boolean failure rate
  and the diagnosis budget but run nowhere; run them in release on a schedule with a threshold.
- The panic hook and signal flush in `main.rs`, the data-loss backstop, have no test: spawn the
  binary, kill it, and recover its journal. `check-install.sh` only runs `--version`; start the
  packaged binary to a first frame under Xvfb and lavapipe, check its linked libraries and highest
  glibc symbol against `docs/RELEASING.md`, and run the offscreen tests once more on the GL
  backend that `packaging/INSTALL.md` promises.
- Two solver tests assert under 5 seconds in a debug build and a document test waits on a
  20 second deadline; use work budgets as `DIAGNOSIS_WORK` does.
- Action SHAs, toolchains and tool versions are bumped by hand and duplicated between `ci.yml`
  and `release.yml`, and `rustup` is fetched by an unpinned `curl | sh`.
- `build-release.sh` accepts an empty changelog section and any `appstreamcli` failure that is
  not an `E:` line, the release has no signature or attestation beside its `.sha256`, and the
  metainfo feature list lags the changelog (no patterns, trim, offset, mirror or PNG export).
- Slow tests to keep an eye on: about 40 UI tests at over a second each.

## Persistence and recovery

- Saving over a file that went bad after it was loaded (bit rot, a sync client) silently drops
  its damaged chunks and every version delta that depended on an unreadable head, without a
  `.damaged` copy, since `keep_original` comes only from the load's own report. Count damaged
  and unreachable pieces in `Parsed::of(previous)` and keep the backup whenever there are any.
- A save does not notice that the file changed on disk since it was opened; the outside change
  survives only as a version until retention thins it. Remember the head digest at load and
  report "changed since you opened it" so the UI can offer a copy or an overwrite.
- The 2 GiB decompression `Budget` is cumulative over a whole version walk, so with a snapshot of
  about 20 MB a year of default retention exhausts it: `history()` then marks intact versions as
  unrecoverable, and `write_versions` keeps a delta whose newer neighbour it just dropped,
  leaving that version permanently undecodable. Bound live memory per step instead.
- A save never checks its size against the loader's 2 GiB `MAX_FILE_SIZE`, and retention caps
  age, not bytes, so a large history can produce a file that no longer opens and refuses the
  next save. Thin harder as the file nears the limit, and say so.
- The recovery journal grows without bound until a save: every apply, undo and redo appends the
  whole transaction (undoing and redoing an import repeats its STEP text), the worker's
  `entries` grow all session, and a journal over 2 GiB cannot be recovered. Rebase the snapshot
  in the background past a size, and journal undo and redo by reference.
- Temporaries left by a power cut in an earlier boot, and those of untitled journals, are never
  removed, since cleanup matches only the current boot's tag and the path just written. Tag them
  by machine as well and sweep the recovery folder in `recovery::scan`.
- Records that use an unknown codec, ran out of memory or exceeded the budget are all reported as
  "damaged", because `Budget::unpack` drops the `UnpackError`; an unknown codec should read like
  a chunk from a newer version.
- One flipped byte in the 12-byte magic refuses the whole file as not a model even when every
  chunk checksum is intact; offer a salvage open.
- Load and save hold five or six copies of the model (every record unpacked before any is
  parsed, the unchanged-record check keyed by full bytes), contradicting "records decode one at
  a time" in `file-format.md`. Opening Version History decompresses every version to verify it.
- An unreadable recent-files list is silently reset without the backup that preferences keep;
  preferences and recent files are read, modified and written with no lock, so two windows can
  lose an update; and set-aside `.unreadable` journals are never offered or pruned.

## Reliability and diagnostics

- A panic on the UI thread ends the process: `Session::redraw` (egui, `Model::perform`, scene
  building) has no `catch_unwind`, and a document whose display code panics crashes again each
  time its journal is restored. Contain a failed frame, drop transient UI state and say so, and
  offer a safe open (features suppressed) after a crash.
- Logs go only to stderr and the desktop entry has `Terminal=false`, so a panic or a startup
  failure ("could not start the renderer") leaves no trace for a desktop launch, and stripped
  symbols make backtraces empty. Write a log in the state directory, show startup errors in a
  native dialog naming the cause and the `WGPU_BACKEND=gl` workaround, and say where the log is
  after an unclean exit.
- GPU allocations are not in error scopes outside image export and surface configuration: a mesh,
  batch, pick target or 4K MSAA 8x target that does not fit leaves the encoder the UI shares
  invalid, and every later frame fails. Scope `ViewportRenderer::upload` and `ensure_targets`,
  drop what failed or step MSAA down, and report it.
- A persistent `SurfaceValidation` error never heals, because `begin_frame` neither
  reconfigures nor recreates the surface; fall back to a conservative configuration.
- Device-loss recovery runs `pollster::block_on(Gpu::open(..))` on the UI thread at every retry
  while the GPU resets; open the replacement on a worker.
- A single surface `Timeout` is treated as occlusion and blanks the viewport for five seconds,
  since cursor movement does not count as showing the window; treat it as a skipped frame.
- A NaN depth read back from a pick reaches `unproject` unchecked and becomes the zoom anchor or
  orbit pivot, poisoning the camera until the next fit; reject non-finite hits and anchors.
- `rfd`'s portal backend `dlopen`s `libdbus` (C) and, when it is missing, returns `None`, which
  the app treats as the user cancelling, so Open and Save As do nothing without a word. Use a
  `zbus` portal call (zbus is already in the tree) and report a missing portal.

## Model stability

- A `PieceId` occurrence on a closed curve is counted from the curve's parameter origin, so when
  a crossing moves past it the two arcs between the same cutters swap names, and side faces and
  edges named from them silently rewire (a circle cut by a chord moved from just above to just
  below the centre). Count occurrences from something an edit cannot reorder.
- `EdgeReference` has no origin fallback, and cap names digest the region's whole boundary, so
  adding a hole inside an extruded rectangle makes every fillet or chamfer on a cap edge fail,
  while the cap faces themselves still resolve through `FaceOrigin`. Resolve both faces with the
  `FaceReference` fallback and take the edge between them.
- A chosen region's `RegionKey` digests its boundary, so adding a hole or a splitting line inside
  it fails the extrusion or revolve with "no longer exists", and `Profile::select` fails the whole
  feature on the first stale key. Fall back to the region sharing most boundary pieces or
  containing an anchor point, keep the surviving regions, and fail only on a tie.
- Pruned dangling pieces still cut the curves they touch, so a stray line touching an outline
  adds a vertex and a coplanar side face and renames that side. Merge pieces of one entity
  meeting at a degree-2 vertex after pruning, and build `PieceBound::Cut` only from surviving
  curves.
- Deleting, suppressing or moving a feature does not count features that hold its faces or edges
  as dependents: `dependents_of` follows only `FeatureKind::features()`, so deleting a boss
  extrusion that a fillet uses gives no prompt and the fillet then fails. Derive dependents from
  the origin feature of each held reference too.
- A reference resolved through the fallback recomputes as plain `UpToDate`; nothing tells the user
  it now points at another face, and the stored name is never refreshed, so small edits can
  drift it until it fails. Add a healed state shown as a warning and an undoable "update
  references".

## Kernel correctness

- Shell cannot split a corner whose offsets do not meet when its convex and concave edges
  alternate (two ridges of different slopes crossing) or one convex edge meets concave ones (a
  cavity whose ridge runs over its inside corner): the offset there joins faces the body keeps
  apart, or runs an edge between another pair of faces, which splitting the corner into several
  cannot give. Corners of more than eight faces are not split either. All are refused as
  `Corner` (`shell::tests::a_corner_where_ridges_and_valleys_alternate_is_named`,
  `a_cavity_whose_ridge_runs_over_its_inside_corner_is_named`).
- A face the shell's thickness closes up is dropped only when it has one loop and keeps two
  single edges apart from each other, or none; a band whose side is a chain of edges (a rim split
  by another face's seam) is refused as `EdgeCollapses`.
- About 2% of booleans between the fixture solids in random placements still fail
  (`boolean::tests::random_placements_of_every_fixture`, ignored, best run in release): mostly
  `Open` and `Ambiguous`, then nearly coincident tori and cones that are too intricate to
  intersect. Most of the `Open` and `Ambiguous` cases are short intersection pieces lost by
  sampled clipping: surface–surface branches are clipped to both patches with 32 to 4096 samples
  (`surface_surface/mod.rs`), and raising the counts removes 55–80% of the failures at a large
  cost, so clip exactly against the patch bounds instead. Intersection curves crossing at a
  tangent point (tori touching along their equators, a face touching a torus's inner equator)
  cannot be split, and a result whose pcurves stray past the resolution (a cylinder against a
  tilted torus) or with a lump too thin for the validation mesh is refused as invalid; each of
  the three has a test pinning its error.
- An edge overlapping a face is clipped to the face's box by `CLIP_SAMPLES = 32` samples
  (`curve_surface.rs`), so an overlap shorter than a thirty-second of the edge is lost: a block
  flush with the side of a 200 mm plate unions at x = 100 but fails as `Open` at x = 33 or 77.7,
  and about 1 in 200 aligned cylinder bosses fail likewise. `curve_curve.rs` clips the same way.
- Faces or axes apart by more than `LINEAR_RESOLUTION` but by less than a few micrometres are
  neither coincident nor separate: coaxial cylinders of radii 5 and 5 + 2e-6, a plug offset 1e-5
  in its bore, or blocks of heights differing by 5e-6 fail as `Open`, `Ambiguous` or
  `Invalid(LoopOrientation)`, which sloppy STEP imports will hit. `SAME_EDGE`, `NEAR_BOUNDARY`,
  `PCURVE_TOLERANCE` and the coaxial offset are unrelated absolute values; derive them from one
  tolerance model and snap or refuse within a documented band.
- A crescent (a circle with an internally tangent circle) fails to extrude as `Invalid` ("the
  boundary of face … crosses itself") unless the tangent point is at a multiple of 90°, and a
  partial revolve of it fails too; a full revolve of a profile tangent to the axis fails with a
  loop winding the wrong way.
- Two blends whose feet cross on one face are accepted: both rims of a cylinder 10 tall filleted
  at 5.5 give a valid solid of intersecting tori with no side face left, and all horizontal edges
  of a 40×40×10 box at 6 likewise. `fits` checks each edge alone; refuse as `TooLarge` when feet
  on one face cross.
- `volumes()` retries a finer validation mesh only after `VoidOutside`, so a thin valid region
  (a 0.05 mm² lens) fails the whole extrusion as "encloses no volume"; retry on `EmptyVolume` too.
- `validate` accepts lumps that overlap, nest or coincide (two boxes in one solid double their
  volume), since `volumes_at` never tests outward shells against each other, and accepts a
  dangling edge used twice inside a planar face, since a repeated edge is never checked to be a
  seam one period apart on a periodic surface.
- `same_surface` samples a 7×7 grid, so a spline patch with a bump narrower than a seventh of
  it is declared coincident with a plane and booleans treat it so.
- Sphere and torus meshes deviate about 1.7 times the requested chord, since only spline,
  revolution, extrusion and cone faces get the `grid_deviation` refinement, so the volume bound
  the app shows (`BodyMass::volume_within`) is violated. Add them to `measured` and test the
  deviation of every surface kind, not only the cylinder.
- Up to next samples at most about 256 rays over the profile, so a feature covering under about
  1% of it is never seen and the extrusion passes through it to the far plane without a word.
- Near-duplicate lines in a profile make phantom sliver regions, because a face counts as real
  when its area exceeds tolerance² though a sliver thinner than the tolerance can be far larger;
  judge it by its width.
- Pattern copies that touch only along a line or at a point fail the union as `NonManifold`, so
  round parts spaced one diameter apart cannot be patterned; keep such copies as separate shells
  of one body, and name the copies in `PatternError::Union`.
- The spline-surface projection seed grid is capped at 48 samples per direction, so on dense
  imported nets an on-surface point's foot is missed (28 in 600 at 60×60 control points).
- `select::classify` takes the first coincident sample of a fragment and never checks the others,
  so a partly coincident fragment is classified by one point.
- Meshes fold where two faces meet at a very small dihedral (lens tips, a plane nearly tangent to
  a torus), giving self-overlapping triangles that `validate` does not see.
- `Revolution::new` does not check that the profile lies in a meridian plane, which
  `project_seed` assumes; a skew STEP `SURFACE_OF_REVOLUTION` is accepted and seeded wrongly.
- `parallel_within` compares `|a×b|`, so a knife edge between anti-parallel faces counts as
  smooth.

## Kernel feedback

- `BooleanError::Split`, `Open`, `Ambiguous`, `NonManifold` and `Intersection` carry no data, and
  the document reduces five of them to one message blaming "faces or edges that exactly touch",
  wrong for the near-coincident cases. Carry the face or edge names, or a model point, so the
  error can name and highlight them.
- `ProfileError::NoClosedProfile` names no curves although the pruned open ends are known; report
  each open end with its nearest candidate and gap, as `kernel-profile.md` says errors should.
  `SweepError::Invalid` cannot say which region failed, since all regions build in one `Plan`.
- `BlendError::Boolean`, `Sweep` and `Profile` return no edge although `applied` knows which tool
  failed, contrary to `kernel-operations.md`. Shell discards the inward attempt's specific error
  and reports `TooThick`, and `ShellError::Walls` names nothing. `InvalidSize` says "not above
  zero" for sizes up to 1e-6 mm.

## Kernel performance

- Extruding a spline profile is quadratic in its control points (320 take 3.4 s to extrude, and
  as long again to validate and to mesh), because `Extrusion::project` searches the whole profile
  each call and `minimize_near` runs the global search before the hint. Try the hint first, as
  `Revolution` does.
- `find_crossing` rebuilds `edge_uses` for every neighbouring face pair and compares boundary
  edges pairwise, quadratic in the edges of one face: reading an 8,000-face prism from STEP takes
  9 s, of which 60% is `edge_uses`. Compute it once per face and use the `BoxTree`.
- Point-in-face is a linear parity scan over every pcurve sample, repeated per hit and ray, and
  `trace::group` rebuilds the outer polygon per hole, so a boolean of two plates with 12×12
  holes takes 1.3 s and grows as about F^1.8. Index each face's boundary once per classifier,
  and put faces in a `BoxTree` for `classify_along`, `cast` and `first_crossing`.
- Every boolean clones, splits, reselects and revalidates every face of the body even when the
  tool touches two, so a sequence of hole features is quadratic (43 ms for the 144th hole). Carry
  untouched faces through by id, and return disjoint operands without the pipeline: a pattern of
  1,600 separated copies takes 2.9 s.
- Selecting regions is quadratic in outer loops: every hole is tested against every outer loop,
  so 1,600 washers take 2.8 s to select. Use the face assignment the arrangement already has.
- The face grid is uniform in uv and sized by the worst curvature anywhere, so one small bump
  multiplies a whole face's triangles, and straight directions are capped at `FLAT_ASPECT` times
  the curved one, so a 1×1000 cylinder gets 121k triangles where 120 would do.
- Blending scales worse than linearly: `crosses_boundary` tests every boundary edge with no box
  filter and recounts uses inside the loop, and tools are unioned pairwise even when disjoint.
- Pcurve fitting, `validate::coedge_geometry` and `boundary_loops` never poll `interrupt::check`,
  so cancelling an extrusion of a large spline waits seconds; `cast` and `first_crossing` treat
  `Cancelled` as a doubt and keep trying every direction.
- Marched curves' `closest_parameter` and `length` reseed over all nodes on every call, from
  loops over nearby vertices in `imprint.rs`.

## Document and recompute

- A wedged recompute cannot be recovered: `cancel` only flips an atomic, the worker's
  `JoinHandle` is not kept, `Action::Recompute` submits to the same thread, and dropping a
  `Recomputer` does not cancel its job. Orphan a stuck worker after a grace period and start a
  fresh one, and report which feature is running and for how long, since `Progress` is only a
  count.
- Export after a cancelled or stopped recompute writes the outdated features' previous results
  as current with no warning, since the dialog checks only for `Running` and failures.
- A recompute reports once, after every feature, region and mesh, so one slow late feature hides
  every finished body. Send an update after the feature loop and per mesh.
- The cache keeps one result per feature, so changing a depth and undoing recomputes everything
  after it; it also has no byte budget, holding every intermediate `Solid`. Keep a small,
  size-bounded history per feature.
- Recompute is single-threaded: independent bodies and the final meshing of each body could run
  in parallel over the dependency data the document already has.
- Parameter lookups by ID and by name scan the whole list (`Document::parameter`,
  `parameter_named`, `parameter_position`), so a transaction touching every parameter is
  quadratic: restoring a chain of 1,200 takes about 0.1 s in a debug build. Index them.
- `Recomputer::cancel()` after a job has finished marks that sequence cancelled, so every later
  mesh request returns at once and a fillet or shell panel's "before" mesh never appears; mesh
  requests are also served oldest first, finishing stale states ahead of the current one.
- "Recompute" cannot force a re-run: a cached failure, including the internal error from a caught
  panic, matches its key and is reused until the feature is edited.
- The `Editor` records no-op edits (setting a parameter to its own expression, a flag to its
  value) as undo steps that bump the revision and recompute.
- A sketch that re-solves to identical geometry gets fresh `OnceLock`s, so its profile
  arrangement and region triangulation are rebuilt for display.
- The undo size estimate counts only inline sizes, not spline control lists, expression trees or
  reference neighbour sets, so heavy histories exceed the 256 MiB budget.
- `SetFeatureKind` refuses an import although `document.md` says an import stays an import;
  allowing it would also give re-import.

## Sketch solver and expressions

- The rank and null-space analysis (`analyze_sparse`, `Echelon::spans_unit` once per column) is
  near cubic on closed chains and never checks `cancelled`: solving the sketch left by offsetting
  a closed 3000-line chain takes 218 s, and Cancel does nothing meanwhile. Use a sparse
  factorisation with a fill-reducing order, and poll inside `analyze_component`.
- `solve_dragging` runs the full rank and DOF analysis every drag frame though the drag worker
  uses only the geometry and memo, and the dragged component is never memoised, so a drag frame
  of a 2000-line chain takes 660 ms. Add a geometry-only drag solve.
- `spans_within` scans every span of the sketch for each part, and `components()` is rebuilt
  with `BTreeMap`s several times per solve, so solving is quadratic in independent parts: 6,400
  dimensioned rectangles take 0.9 s, and a warm re-solve recalling every part costs the same.
- Conflict diagnosis confirms each constraint of a conflict with a Gauss–Newton step over the
  whole part, so a conflict running through a part of more than about five hundred entities still
  runs out of budget and is reported as not solving; one factorisation of the Jacobian, updated
  per constraint left out, would make each confirmation cheap.
- A solve with several conflicting parts reports only the one owning the newest constraint
  (`max_by_key` in `diagnose_failure`), so users fix one and meet the next.
- When conflict diagnosis finds that a part which failed from its drawn shape holds after all (a
  chain whose line must fold back, reached from a solution of all but one constraint), the solve
  still fails; the solution found could be offered instead.
- A point on a line segment or arc is held to the infinite line or full circle, so it can solve
  beyond the segment's ends or outside the sweep, and the line rotates to meet it; bound it or
  say so.
- Horizontal and vertical distances accept a negative value when added and fail only at solve;
  allow signed values to flip the side, or refuse them up front.
- Several function errors say "compared" (`hypot(1 mm, 1 deg)` reads "an angle cannot be compared
  with a length"), `0^-1` reports "too large" rather than division by zero, `1/2 mm` gives no hint
  that the unit binds to the 2, and `format_number`'s fixed six decimals show `1e-7` as `0` and
  print `1e300` in full.
- `10 mm^2` means (10 mm)², since a power binds to the measure before it; stored text relies on
  that reading, so changing it needs a new spelling or a format change.
- `Sketch::remove_entity` has no non-test caller and scans every entity and constraint per call,
  contrary to `sketch.md`'s "in one pass"; remove it or index it.

## STEP import and export

- Healing covers only edges with exactly two distinct faces, so real Fusion 360 exports are
  refused whole over a vertex a few micrometres off its edge though the file declares 0.01 mm: a
  cylinder seam (one face, used twice) or the line where two half-cylinders meet cannot be
  rebuilt by `IntersectionCurve::through`. Project the vertices onto lines, circles and ellipses
  within the declared precision instead. Faces that meet only within a coarse declared precision
  are refused rather than refitted to each other.
- One unsupported surface or curve loses the whole body: an `OFFSET_SURFACE` of a spline,
  extrusion or revolution (it would need a surface fitted within tolerance), `PARABOLA`,
  `HYPERBOLA` and the `*_REPLICA` forms. Fit a spline within the declared precision, or keep the
  other faces and say which were lost. Colours and layers are not read.
- Import canonicalises each placement by writing and re-reading it, parses every import again on
  each model load, journal replay and recovery scan, and stores every placement of a product as
  its own STEP text. Build each representation once, store each product once with placements,
  and cache solids by text digest.
- A damaged header or a missing `END-ISO-10303-21;` trailer refuses the whole file, though the
  header is never used and data entries already recover one by one.
- A file with no closed solids always reads "holds no solid bodies", whether it is IFC,
  tessellated AP242, a surface model or a wireframe; report the schema and what it holds.
- The parse tree still holds about three times the file size (a boxed slice per record and per
  list); a flat arena of values would bring it near the file size.
- The writer shares nothing: an 8,000-sided prism writes 48,003 `CARTESIAN_POINT`s and 40,006
  `DIRECTION`s (13 MB), which also bloats the STEP text stored in models. Deduplicate points,
  directions and placements.
- The writer puts all bodies in one product with no colours, holding the output twice in memory.
- Placements that scale or mirror are left out with a note; a uniform scale could be applied, and
  a mirror once the kernel can reflect.
- Imports cannot be positioned (`Import` has no placement) or refreshed from their source file:
  the path is not kept, so a changed STEP file means deleting the feature and breaking what
  references it.
- Only `.step` and `.stp` are recognised (no `.p21` or `.stpz`), and names in raw Latin-1 become
  U+FFFD without a note.

## Drawing import and export

- Sketches cannot be exported: there is no DXF or SVG output of a sketch or flat face, though
  laser and CNC work need it and `DrawingCurve` already models what it would write.
- DXF import has no options: units come only from `$INSUNITS` and `$MEASUREMENT` with no override
  or scale (templates commonly default to inches), coordinates are not recentred, and a new
  sketch always lands on the XY plane although `SketchTarget::New` takes a plane.
- Curves carry no layer, so the import cannot offer a layer choice, and over 20,000 curves the
  whole file is refused as `TooLarge` with nothing imported.
- Linetypes and the `DEFPOINTS` layer are ignored, so centrelines, hidden lines and dimension
  points import as profile geometry and add regions; map them to construction geometry.
- Damage anywhere refuses the whole DXF, losing everything read before it, and CR-only line
  endings read as damaged at line 1.

## Mesh export

- One body that cannot be meshed or written aborts the whole export (`?` per body in
  `export_bodies`); export the others and name the one left out.
- STL uses absolute f32 coordinates, which lose about 0.06 mm at 10^6 mm, and merges all bodies
  into one surface; `stl::encode` holds every triangle as f64 before writing.
- 3MF has no colours, materials or thumbnail, builds everything in memory and cannot exceed
  4 GiB without ZIP64; it prints vertices with up to 17 digits, about doubling the XML.
- No mesh import (STL, 3MF, OBJ), though the STEP reader already builds `FACETED_BREP`s from
  polygons; it needs the STEP storage item above first.

## Interface performance

- While orbiting, panning or zooming, a GPU pick is issued every frame (the view is part of
  `PickKey`) and each change of the item under the moving cursor rebuilds and uploads the whole
  scene. Freeze hover during camera moves and pick once when they settle.
- Select all on a large sketch does quadratic work every frame: `sketch_tools::fixed` checks
  `points.contains` on a `Vec` per selected point, every constraint tool rebuilds its candidates
  each frame, and `select_all` builds a set of every entity just to show availability.
- Snapping projects every point and curve of the sketch on every hover frame (`snap.rs`), about
  1 ms for 20,000 lines in a release build, most of it walking the entities, and a line or slot
  end walks every line again to find the nearest for parallel and perpendicular inference
  (`drawing.rs` `guides`); a screen-space index would need the preimage of the snap radius on the
  sketch plane, unbounded near the horizon.
- Every frame `Marks::collect` formats every constraint's description, evaluates every dimension
  and registers an `interact` per glyph, and an expanded sketch in the tree does the same with an
  O(n²) `involved` check; the `large_sketch` benchmark has no constraints. Cache per generation,
  cull off-screen marks and virtualise the tree.
- The cached scene is one batch: any change to its content (each drag solution, an edit, an
  evaluation, a new faceting level) facets every drawn sketch again, and a hover or selection
  change restyles and uploads all of it, about 1.2 ms to rebuild and 0.5 ms to upload for a
  sketch of 24,000 curves in a release build. A batch per feature, with pick ids of its own,
  would limit both to what changed. Face styles are likewise rewritten whole on every highlight
  change.
- Zooming in five times reanchors and re-uploads every batch, since the anchor reach is four view
  distances; derive it from the f32 error budget.
- Meshes are uploaded whole on the UI thread in the frame that first shows them, and never
  culled in the main or pick pass although each keeps its bounds; the frame-cost benchmark has no
  meshes, picking or hover.
- Image export submits every tile's readback buffer at once and assembles the image beside them,
  about 540 MB at 8192², contrary to `render.md`'s bounded memory; reuse a few buffers and stream
  bands to the encoder.
- A pick or image readback in flight redraws full frames until polled complete; vertex records
  repeat per-layer data and both ends of shared segments; invisible vertex markers go through the
  colour pass; each mesh's placement uniform is written every frame; resizing recreates the MSAA
  targets per pixel; and an MSAA change rebuilds all ten pipelines with no pipeline cache.
- The feature tree lays out every row each frame, which a STEP import of hundreds of bodies
  makes long.

## Sketching

- No projection of model edges or other sketches into a sketch, and bodies and other sketches
  are unpickable while editing.
- Every dimension drives: there are no reference (driven) dimensions and no way to disable a
  constraint, so dimensioning determined geometry adds a redundant constraint instead of a
  measurement.
- Typed lengths and angles (`@40, 20`, `25 < 30`, `width / 2, 10`) are evaluated once and place
  free points, keeping neither a dimension nor the parameter link; offer to create the
  dimensions.
- Trimming a line whose removed end is shared with another curve leaves its length dimension
  between the old corner and the far end, since `keeps_length` filters only constraints on the
  curve itself.
- Duplicate or contradicting constraints are accepted and reported afterwards (Horizontal twice
  on a line adds a redundant one; Vertical on it fails the sketch); refuse duplicates and trial-
  solve before committing.
- A drag to a position with no solution freezes the geometry without a cue, and in a conflicting
  sketch every drag does nothing and then blames the move.
- While a drawing tool is active, a click that moves 6 px or is held 0.8 s is dropped silently;
  place the point anyway, and allow press-drag-release to draw a line, rectangle or circle.
- Tools missing: ellipse (a new entity kind across the solver, kernel and file format), sketch
  chamfer, rectangular and circular patterns, rotate, scale and copy of a selection, split at a
  point, text, and fit-point, closed or periodic splines (`BSpline::interpolate` and `through`
  serve only DXF import, and the control polygon is not drawn).
- Splines cannot be trimmed or extended (`TrimError::Spline`), trim ignores collinear and
  co-circular overlaps as cutters, and the sketch axes are not cutters. Offset takes one chain at
  a time, leaves the free ends of an open chain sliding along their curves and cannot offset
  splines; a sketch fillet cannot round a spline and drops equal lengths and midpoints of the
  lines it shortens, as trim does.
- Constraint kinds missing: distance between circles, line and circle, or to a spline; arc length
  and sweep; angle or perpendicular to an arc; arc midpoint; equal splines; spline–spline
  tangency; curvature continuity; symmetric curves. Coincident, perpendicular and tangent take
  exactly two items where parallel and equal chain.
- Spline intersections sample sign changes, so a near-tangent crossing between samples is missed.
- No live length, size or radius readout while drawing, no closed-region or open-end feedback
  while sketching, and no smart-dimension tool that takes the entities after the command.
- Snapping has no midpoints, intersections, spline targets, grid or inference lines to other
  points, cannot be suppressed by a modifier or toggle, and dragged geometry does not snap at
  all.
- Dimensions all sit at one fixed offset, so collinear chains overlap, and labels cannot be
  dragged. Glyphs stack uncapped (a 64-gon puts 63 `=` glyphs on its first side) and cannot be
  hidden.
- Backspace in a line chain removes only the anchor and Ctrl+Z ends the chain; double-click does
  not select a connected chain; a tangent arc cannot continue a line chain without switching
  tools; the polygon side count changes only by one per key.
- Clicked points are not bounded like typed ones, so an edge-on view can place a point at an
  enormous distance.

## Modelling features

- Feature kinds missing: mirror (the kernel has no reflecting transform), hole, draft, sweep,
  loft, split, and move or copy body.
- Datums cannot be built from points: no datum point, plane through three points, mid-plane,
  plane through an axis and a point, plane normal to an edge at a point, or axis through two
  points. Model vertices are named and pickable but only the measure tool uses them, and datums
  and pattern axes cannot take sketch geometry.
- Extrusions always start on the sketch plane, with no start offset or face, taper angle or thin
  wall, though `LinearExtent::between` accepts any bounds; revolve has the same gaps.
- Patterns repeat a whole body: no pattern of chosen features (a row of holes cut into a plate),
  no instances left out, no pattern along a curve or driven by sketch points, no linear "total
  length" mode, and a copy's faces are described as the face they copy.
- Extrusions end only on flat faces and planes: up to face and up to next refuse a curved face,
  and up to next needs one flat face that the whole profile meets first.
- No feature combines two existing bodies, and a cut affects only one body.
- Bodies have no colour, material or density, so a multi-body model is one grey until picked and
  mass cannot be shown; this blocks coloured STEP and 3MF export.
- Mass properties are volume, area and centroid from the display mesh, biased low on curved
  bodies, with no mass, inertia, bounding size or total. Integrate exactly over the trimmed
  faces, as `planar_area` already does for planes.
- Blends: only line and circle edges along planes, parallel cylinders and coaxial surfaces; no
  ellipse, spline or intersection edges, not even a straight edge beside a spline extrusion face;
  ends at steps and T-junctions refused; no variable radius, two-distance or distance-angle
  chamfer; corners only for three convex straight edges.
- Shell: no spline, extrusion or revolution faces, only line and circle edges, only flat faces
  open, one thickness for the whole body.
- Revolve cannot keep the part of a region on one side of the axis.
- Several features chosen in the tree cannot be dragged together; each moves on its own.
- No live preview of a fillet, chamfer or shell while its panel is open, and no viewport handles
  for extents.
- Parameters cannot be reordered or given a note, show no "used by" or unused flag, cannot be
  deleted by inlining their value, and expressions cannot refer to measured values or sketch
  dimensions.
- The measure tool cannot take planes, axes, datums or sketch curves, so a hole axis to a datum
  or a circle's radius cannot be measured.

## Viewer

- Display styles: wireframe, hidden line and shaded without edges, plus isolate or hide others
  and look normal to a face.
- Silhouette edges on curved bodies.
- Section planes.
- Transparent or X-ray bodies.
- Line caps, joins and anti-aliasing without MSAA.
- A selection filter, so a click takes only faces, edges, vertices or sketch geometry.
- Edge lines can be eaten by faces at grazing angles, since depth bias is a constant factor with
  no slope term, and the grid and reference fills share the mesh's bias, so a face on the XY
  plane can speckle with the grid. Neither has a test.
- Zoom to fit uses the bounding sphere, wasting about 30% on wide flat parts, and the perspective
  branch uses the sine of the half angle although `render.md` says tangent.
- No touchpad navigation: orbit is right-drag, pan needs a middle button or Shift, and two-finger
  scroll always zooms.
- Adapter choice is only the `WGPU_POWER_PREF` environment variable, so users of hybrid laptops
  or broken drivers cannot pick another adapter from Preferences.
- Lighting and the MSAA resolve happen in gamma space; the model keeps no saved view.

## Accessibility

- Everything drawn in the viewport is invisible to screen readers: the keyboard highlight
  description, tool prompts, snap labels and measure labels are painter text, and the viewport is
  an unnamed `interact`. Notices and recompute failures are not live regions, so a failed save is
  never announced.
- Constraints and dimensions are not scene pickables, so N never reaches them and a dimension can
  be re-edited only by double-click or from the tree; with a drawing tool active, Space toggles
  the selection instead of placing at the highlight, so keyboard drawing cannot start from
  existing geometry.
- View cube labels sit on the cell fill with no tested backdrop (about 1.2:1 on the hovered amber
  cell), and high contrast reaches neither the scene colours nor the colour-only sketch states.
- Dialogs set no initial focus and bind no Enter to the primary action, contrary to
  `app-input.md`.

## Application

- The modelling tools borrow Phosphor glyphs that mean something else (`icons.rs`): fillet is the
  full-screen corners, chamfer a generic polygon, revolve the refresh arrows, circular pattern a
  loading spinner, shell a see-through cube, and the sketch fillet shares the fillet's. Draw
  caditor's own icons for fillet, chamfer, shell, extrude, revolve and both patterns, on
  Phosphor's grid and stroke weight so they sit beside it;
  undecided whether they ship as glyphs added to the `icons` font family or as painted shapes.
- One files worker runs everything and Open and Import cannot be cancelled, so a slow STEP import
  blocks Open behind a modal, and the opening modal is drawn before the unsaved-changes prompt,
  so closing the window during a load hides the prompt until the load ends. Give imports their
  own cancellable job. When a worker thread cannot be spawned the job runs on the UI thread.
- Export and image export append the extension after the save dialog returns, so an existing
  `part.stl` is replaced without the confirmation Save As gives; cancelling their file picker
  also drops the dialog and its settings.
- A STEP or DXF path on the command line goes to Open and fails as "not a caditor model", though
  dropping it imports; the desktop entry registers only `application/x-caditor`, and there is no
  headless export or conversion.
- Text outside Latin, Greek and Cyrillic shows as missing glyphs in feature and file names, since
  only Inter and egui's defaults are loaded.
- A corrupt preferences file resets the keymap with only a log line, and a failed preference save
  is never shown.
- Notices are one slot: an info notice replaces a save or export failure, with no history.
- Version history shows only "saved N ago" with no summary, preview or way to keep a version.
- The palette finds commands only, not features or parameters, and omits commands that are out of
  context instead of explaining them as the menus do; the tree has no filter or groups.
- Angles display only in degrees though `ux.md` allows radians; Open Recent shows bare file names
  and cannot be cleared; core modelling commands have no default shortcuts.
- `Command::all()` is a hand-kept array of 80, so a new variant can miss the keymap and shortcut
  editor; test that every variant is listed.
- Dropping files on the window works only under X11, since winit 0.30 has no drag and drop on
  Wayland, and nothing shows where a drop will go while files are dragged over the window.
- One document per process.
- No clipboard for sketch geometry or features, no parameter import or export.
- No localisation.

## Dependencies

- `wgpu` builds `dx12`, `metal`, `webgl`, `webgpu` and `renderdoc` by default, `egui-winit`'s
  `links` pulls `webbrowser` though no URL is opened, and `env_logger` pulls `regex`; turn off
  default features and list what is used.
- `libzstd-rs-sys` assembles a C file on x86_64, contrary to `zstd.md`'s "pure-Rust port", and is
  a pre-release decoding untrusted data; record it and add `gcc` to the PKGBUILD.
