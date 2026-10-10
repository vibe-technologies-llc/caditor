# caditor performance

Where caditor is slower or heavier than it should be, with the measurements that show it.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Kernel

- [medium · medium] A lofted blend's boolean is most of its cost: in release the fillet round the
  eight-edge rim of a pocket with rounded corners in a cylinder takes about 0.57 s, 0.43 s of it
  the boolean against the chord faces, whose sections are as dense as the fillet's (up to 129 a
  piece, so the rows hold `FOOT_FIT`) because the boolean's vertices are kept for the fillet's
  edges; the unoptimised test build takes 14 s
  (`lofted_tests::the_rim_of_a_pocket_with_rounded_corners_is_rounded_and_chamfered_all_round`).
  Lofting the chord through a few of the sections and moving the vertices onto the fillet's rows
  afterwards (along a row onto an end face where one meets it) would cut the spans the boolean
  marches through several times.
- [medium · hard] A boolean still validates the volume of every shell it touches by meshing the
  whole shell, and passes every face of the body through `Plan::build` and the cheap checks, so a
  sequence of hole features stays quadratic with a smaller constant: in release the 144th hole of
  a block takes about 21 ms (14 of them the volume check's mesh) against 1 ms for the first
  (`carry_tests::holes_drilled_one_by_one_and_blocks_joined_one_by_one_take_bounded_time`).
  Shells whose volume and placement could be known without meshing, and lumps of a many-lump body
  that the tool does not reach (a union touching one of 300 separated blocks takes 18 ms), are not
  carried yet.
- [low · hard] More strips than six do not pay on the top of a plate with 113 holes (6,784
  points) on 16 threads of 8 cores, though a reach sized by the face's widest gap now keeps 95% of
  its triangles in twelve: each strip triangulates its core and a reach of about the widest empty
  disc each side, so twelve strips triangulate about two and a half times the face's points, and
  the serial steps (sorting along x with the gap estimate beside it, then the remainder, about
  0.3 ms together) bound the face at about 1.4 ms against 3 ms whole. Hard because a remainder made
  in parallel would need its own decomposition, and a narrower reach loses the triangles bridging
  between holes. The remaining exact in-circle predicates (under a tenth of tessellation time) are
  on cocircular boundary samples of mirror-symmetric holes, which no helper reaches.
- [low · hard] `SolidResult::cuts`/`joins` keep every tool solid with every history entry,
  though only patterns, mirrors and an open feature read them; a removal's tool shares little
  with the result, since a difference reverses the tool's kept faces and their pcurves (an
  addition's tool shares its kept faces' geometry, which the history counts once). Dropping them
  needs the feature's key to say whether anything wants its tools: a later pattern or mirror
  repeating it, and the feature open in the app, whose tools `Recomputer::mesh` meshes on request
  without recomputing. Adding a pattern or opening a hole would then evaluate the feature again
  and, as `same_shapes` compares tools, everything below it; the tools are usually a few faces
  next to the body, so this waits for a model where they weigh.
- [low · hard] The face grid is graded per direction but still a tensor product, so a bump divides
  the whole rows and columns through it, and curvature is sampled only on the lattice, so a feature
  narrower than a lattice span is refined only if a checked cell lands on it. Cells split where
  they bow past the chord (a quadtree) would keep the division local and find narrow features.

## STEP import

- [high · hard] A large STEP import still stalls the interface while its bodies arrive: every
  showing rebuilds the base scene, whose one batch holds every body edge as line segments (13.2
  million for the VZ330 assembly before edges were sampled sparingly, 0.25 to 0.4 s a rebuild on
  the UI thread in release, and the renderer uploads the whole batch again unbudgeted). Drawing a
  still frame of the VZ330 then took about 190 ms on the GPU (RX 9070 XT, 1920×1080, 4x): 100 ms of
  edge lines, 28 ms of meshes, 20 ms of silhouettes; time it again, and consider a coarser mesh for
  bodies small on screen (batches are culled by their bounds, which only pays once bodies are
  batches of their own). Each body's edges and vertices could be a
  batch of its own, cached while its `BodyMesh`, style and highlight are unchanged, with pick ids
  that do not shift when other bodies come and go; hovering over a large model rebuilds the same
  way. Reading is now bound by single parts: the VZ330's lead screw (406 faces, helical splines of
  7 by 1441 control points) takes most of the 71 s alone, in healing and `find_crossing`; opening a
  saved model reads every import text with `read_step` again, crossing check included
  (`step_cache.rs`).

## Interface

- [medium · easy] Frames and scene rebuilds walk linear lists once per item, unmeasured but
  quadratic on large imports (one feature per body, so a 954-body assembly is about 954 features):
  `Highlight::is_of_chosen_row` scans the tree's chosen rows with `contains` for every face and
  edge of every body on each base-scene rebuild; the Bodies tree lays out every body row each frame
  with no off-screen skip, looking each up with `Document::feature` and building a visibility
  `Transaction` and label per row; the feature tree tests `chosen.contains` per row and runs
  `group_run` (a fresh `Vec` of every feature) per group header; `BodyMeshes::update` rebuilds the
  whole body map every frame though a generation would skip it; the keyboard highlight is checked
  with `highlightable().contains` over every pickable each frame; and with a face analysis open
  `Analyses::of` retains and scans every entry once per body, so the legend is O(B²) a frame and
  computes `shown_meshes` again. Sorted sets, generation keys and the feature tree's off-screen
  rows would make each a lookup; time them on an import first.
- [medium · medium] `Document::feature` and `feature_index` scan the feature list, and are called
  inside per-body and per-item loops: `body_snap::Basis::of` runs `is_shown` for every body every
  frame a sketch is edited, `whole_bodies` every frame the Bodies filter hovers a face,
  `Selection::is_available` once per selected item when the selection or revision changes (whole
  bodies of an import are tens of thousands of faces), and `InsertFeature` checks ids and names in
  O(F), so a transaction adding N imports is O(N²). An index from id to position, kept by every
  structural edit (or rebuilt lazily), makes them logarithmic.
- [medium · hard] The cached scene is one batch: any change to its content (each drag solution, an
  edit, an evaluation, a new faceting level) facets every drawn sketch again, and a hover or
  selection change restyles and uploads all of it, over a millisecond to rebuild and about half of
  that to upload for a sketch of 24,000 curves in a release build. A batch per feature, with pick
  ids of its own, would limit both to what changed.
- [low · easy] The sketch fillet clones the whole sketch every frame it aims at a crossing:
  `Filleting::hovered` calls `aim.found_in(&mut sketch.clone())` while a first curve is chosen and
  the pointer is over a second, and `prepared` clones it again every frame once a crossing corner
  is chosen and its radius dragged, O(sketch) a pointer move on a large sketch. Cache the prepared
  sketch by the displayed sketch's generation and the aims.
- [low · hard] Snapping projects every point and curve of the sketch on every hover frame
  (`snap.rs`), a cost linear in the sketch that is most of the frame for tens of thousands of lines,
  mostly walking the entities, and a line or slot end walks every line again to find the nearest for
  parallel and perpendicular inference (`drawing.rs` `guides`); a screen-space index would need the
  preimage of the snap radius on the sketch plane, unbounded near the horizon.


## Sketch solver

- [medium · hard] A drag frame solves geometry only (`solve_geometry_from`, no rank or
  degrees-of-freedom analysis), but the dragged part is still never memoised and the solve itself is
  the cost: dragging an end of a fully dimensioned chain of 2,000 lines to a point it cannot reach
  takes seconds a frame in a release build, where a chain joined only by `Coincident` takes tens of
  milliseconds.
- [low · medium] A drag past reach ends at the closest least-squares pose only in parts of at
  most `DENSE_LIMIT` variables (`numeric/reach.rs` uses a dense SVD of the null space and one
  Jacobian per free direction); a larger part keeps the `STIFF` solve's pose, or the last frame's
  when that fails, so a long chain dragged past its reach stops instead of following. A sparse
  null-space basis, or Hessian-vector products inside CGLS, would lift the limit.

## Recompute

- [low · easy] A recompute builds a glimpse before every feature it evaluates: `offer_glimpse`
  returns early only when nothing was recomputed since the last glimpse, which is never true once
  a feature was evaluated, so `glimpse` clones the statuses, bodies and inputs and walks the rest
  of the tree before each feature, and `into_evaluation` builds `Measured::of` again though the run
  holds it, O(F²) for a cold open of a long tree while the presenter keeps only the latest glimpse
  and reports none before `FEATURES_DONE_AFTER`. Build one only once a report is due.
