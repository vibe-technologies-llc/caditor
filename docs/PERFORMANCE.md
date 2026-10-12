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
- [medium · hard] A modelled thread is slow to cut and to measure: in release an M4×20 shaft
  threaded its whole length takes 0.7 to 0.9 s (about a third classifying fragments by rays
  through a hundred one-turn flank faces, a third imprinting, a fifth validating the volume) and an
  M8 bore 12 mm deep 0.35 s. With the groove's far face exactly on the shaft (no 5 % reach) the
  M4 took 0.5 s, the crest then being the groove's own helix rather than a marched one, but
  coincident faces were left out until they are known to hold on every face kind. Measuring the
  threaded M4's mass exactly (`mass_properties`) gave up on 59 of its 64 faces after about 100 s
  and took them from the mesh: the flank integrals do not converge within `FACE_EVALUATIONS`.
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

- [high · hard] Reading a large STEP file is bound by single parts: the VZ330's lead screw (406
  faces, helical splines of 7 by 1441 control points) takes most of the 71 s alone, in healing and
  `find_crossing`; opening a saved model reads every import text with `read_step` again, crossing
  check included (`step_cache.rs`).

## Interface

- [medium · hard] The cached scene's own batch holds every drawn sketch: any change to its
  content (each drag solution, an edit, an evaluation, a new faceting level) facets every drawn
  sketch again, and a hover or selection change restyles and uploads all of it, over a millisecond
  to rebuild and about half of that to upload for a sketch of 24,000 curves in a release build.
  Bodies are parts of their own with pick ids of their own (`app.md`); a batch per sketch, kept the
  same way, would limit both to what changed.
- [medium · hard] A still frame of a large assembly is bound by its draws: on the VZ330 (3,894
  bodies, 15.6 million triangles, 4.8 million edge segments, fitted at 1920×1080, 4x, RX 9070 XT)
  a frame turning a little takes about 22 ms on the GPU (9 without edges, 4 with faces alone,
  coarser edge levels included) and 12 to 16 ms on the UI thread, most of it encoding three to six
  commands for each mesh, silhouette and body batch (`frame_costs_on_a_large_assembly`). Edges of
  bodies large on screen still draw every segment, and meshes and silhouettes have no coarser
  level for bodies a few pixels across. Body batches packed into shared buffers with one style
  table (a body one draw, neighbours one draw), placements at dynamic offsets of one uniform
  buffer, and a coarser mesh without a silhouette for bodies small on screen would cut both.
- [low · easy] The sketch fillet's crossing is joined on a copy of the whole sketch once a frame
  while the pointer moves along the second curve, since each frame's pick (`Pick::near`) is a new
  aim for the cache in `Filleting::joined`; only a resting pointer, or a chosen crossing whose
  radius is dragged, reuses it. O(sketch) a pointer move on a large sketch.
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

## Hostile files

- [low · easy] The XML reader shared by SVG and 3MF (`xml.rs`) never polls the interrupt, so Cancel
  only abandons the worker of a crafted file up to `MAX_DRAWING_ELEMENTS` elements; poll it every
  few thousand elements and answer a cancellation.
