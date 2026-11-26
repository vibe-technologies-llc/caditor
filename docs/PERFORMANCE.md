# caditor performance

Where caditor is slower or heavier than it should be, with the measurements that show it.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Kernel

- [medium · hard] A modelled thread is slow to cut: in release an M4×20 shaft threaded its whole
  length takes 0.7 to 0.9 s (about a third classifying fragments by rays through a hundred
  one-turn flank faces, a third imprinting, a fifth validating the volume) and an M8 bore 12 mm
  deep 0.35 s. With the groove's far face exactly on the shaft (no 5 % reach) the M4 took 0.5 s,
  the crest then being the groove's own helix rather than a marched one, but coincident faces were
  left out until they are known to hold on every face kind.
- [medium · hard] A boolean still makes passes over every face, edge and loop of the body that the
  tool leaves alone, though the geometry work on them is carried, so a sequence of hole features
  stays quadratic: in release the 144th hole of a block takes 4.45 ms against 1.0 for the first
  (11.7 before a plane given a new loop was meshed with only the carried loops beside it, 12.5
  before quiet loops were carried through untangling and validation;
  `carry_tests::holes_drilled_one_by_one_and_blocks_joined_one_by_one_take_bounded_time`). What
  remains is the input's face boxes (0.6 ms), imprinting, which splits every edge and pools every
  vertex (1.1), the top and bottom faces' quiet loops copied with their areas, grouped and
  sharpened (0.65 each), and boxed once more in validation to find those beside the new loops,
  selecting and healing every face (0.45) and `Plan::build`, whose vertex merging and naming visit
  every edge (0.8). A face of another surface than a plane given a new loop, such as a tube's wall drilled
  across, is still triangulated whole. A union touching one of 300 separated blocks takes 2.6 ms
  (18.2 before the blocks it does not reach were left alone), mostly boxing, grouping and copying
  those blocks. A result of several shells, such as a block with a void inside, is still meshed
  whole for its volumes and placement.
- [low · medium] A lofted blend's boolean is still most of its cost: in release the fillet round
  the eight-edge rim of a pocket with rounded corners in a cylinder takes about 49 ms
  (`lofted_tests::rounding_the_rim_of_a_pocket_with_rounded_corners_is_timed`, ignored; 57 ms on
  the same machine before subdivisions were shared and flat spline spans seeded as runs), 30 ms of
  it the boolean, 10 ms rounding and 8 ms planning. About 16 ms of the boolean are the lofted
  faces' intersections with the cylinder and walls: 5 ms seeding, 5 ms marching and building the
  intersection curves, the rest clipping and recognising them; edge hits take about 6 ms,
  fragment selection and splitting about 3.5 ms each, and validating volumes by meshing about
  3.5 ms. Seeding is now mostly boxing and pruning subdivision pieces met for the first time
  (each lofted face meets two or three faces, so sharing its subdivision saved about 2 ms) and
  about 2 ms of leaf solves: 376 leaves gave 362 seeds for the 12 branches of 86 marched pairs.
  Skipping the leaves of a branch already found needs that branch traced during subdivision,
  ahead of the hints and of the isolated-touch search, which today see every seed before
  anything is traced, so it was left for the 2 ms it would save at most.
- [low · medium] Exact mass along a marched boundary still spends a 15-point rule per piece:
  `measure/swept_tests.rs` measures a threaded M4×20 shaft in about 615 thousand evaluations
  (3.0 million while strips took a 15-point rule too), half of them the nodes of its 20,500
  boundary estimates, mostly along the crest edges of its flanks and crest band (150 marched pieces
  a turn), the other half the one-evaluation ruled strips at those nodes. A smaller Gauss–Kronrod
  pair along intersection curves, whose marched pieces are short, might halve that again if it
  keeps the boundary tolerance. The pipes along three helix turns (751 thousand, from 1.4 million)
  and a flat spiral (1.5 million, from 2.8) are the exact 8-point strips across the 256 cubic spans
  (512 on the spiral) between each turn's middle and the 120 nodes of its section edges.
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

- [high · hard] Reading a large STEP file is bound by its largest parts: the VZ330 reads in 38 s
  and its lead screw (406 faces, two helical spline flanks of 4 by 2519 control points) in 8.3 s
  alone, 1.6 s of it an exact build the file refuses, 3.5 s healing it extended (tracing its edges,
  loops and gap checks, each global projection onto a flank still about 265 evaluations) and 2.9 s
  in `find_crossing`, nearly all testing the thread's crest and root edges against the opposite
  flank. Opening the saved model takes 14 s on the computer that imported it (5.3 s to save), its
  1,200 texts read in parallel and trusted as checked (`checked_texts.rs`); on another computer
  each text is checked once, about 75 s, measured before the lead screw got cheaper.

## Interface

- [low · hard] Each sketch is a kept part (`app.md`), but within one sketch a hover or selection
  change restyles and uploads the whole sketch, about 1.9 ms to rebuild on the 24,000-curve
  `large_sketch` in a release build (`frame_costs_on_a_large_sketch_and_a_large_model`), and each
  drag solution refacets the dragged sketch whole. Hovered and selected
  entities drawn as a small batch over the kept one would limit a hover to what it lights, once
  lines of equal depth drawn twice are settled.
- [medium · hard] A still frame of a large assembly is bound by its edges on the GPU: on the VZ330
  (3,977 bodies, 17.8 million triangles, 5.8 million edge segments, fitted at 1920×1080, 4x, RX
  9070 XT) a frame turning a little takes 21 to 25 ms on the GPU (8 to 10 without edges, 3 to 4
  with faces alone, coarser edge levels included) and 3.5 to 6 ms on the UI thread
  (`frame_costs_on_a_large_assembly`), against 29 to 30 and 16.5 to 18 ms before a mesh, a
  silhouette and a batch's lines each became one draw from shared slabs (`render.md`). Each batch
  is still a draw of its own, since its coarser levels lie after its full one and apart from its
  neighbours', edges of bodies large on screen still draw every segment, and meshes and
  silhouettes have no coarser level for bodies a few pixels across, where 1.9 million of the shown
  segments lie in 3,006 batches under 8 pixels. Levels laid out so that neighbouring batches draw
  as one, and a coarser mesh without a silhouette for bodies small on screen, would cut the rest.
- [low · hard] Snapping projects every point and curve of the sketch on every hover frame
  (`snap.rs`), a cost linear in the sketch that is most of the frame for tens of thousands of lines,
  mostly walking the entities, and a line or slot end walks every line once more to find the nearest
  for parallel and perpendicular inference (`drawing::nearest_lines`: one allocation-free pass, but
  still 4.6 ms for 30,000 lines in release, against 5.2 ms before); a drag of several points still
  walks every entity for the moving point nearest one (`snap::nearest_moved`), which builds no
  target off screen of the moved points (centres aside) but cannot skip an entity without a
  screen-space index. The body items are bounded (`body_snap.rs` `Bounds`, 83 µs against 677 µs for 200
  bodies of 100 edges each) but a body or arc edge crossing the camera's near plane is scanned
  whole. A screen-space index of the sketch would need the preimage of the snap radius on the
  sketch plane, unbounded near the horizon.


## Memory

- [medium · hard] The VZ330 assembly (3,967 bodies, 16.3 million triangles) holds about 5.4 GiB
  once imported (`import_costs`, `CADITOR_IMPORT_IN_THE_APP_ONLY=1`, resident after settling; a
  jemalloc heap profile attributes it). The largest part left is solids: each copy of a part keeps
  its own `Solid` (`Solid::imported` names faces after the copy's feature, and placed copies are
  `transformed` with new curves and surfaces), about 1.4 GiB beside one canonical solid per part
  when measured with 320-byte faces. Boxing the extrusion, revolution and B-spline surfaces has
  since brought a `Face` to 192 bytes (`Surface` 96, sized by cones and tori); an `Edge` is 160
  (`Curve` 96, its variants 48 to 88 bytes, nothing worth boxing). Sharing geometry between copies
  (names and placement over one solid) would save most of the rest but reaches every kernel reader.

## Sketch solver

- [medium · hard] Swinging or folding a 2,000-line zig-zag chain far from where it was drawn
  still costs 40 to 70 million work units in its first frame, nearly all of it `FROZEN`
  (`reach_tests.rs` keeps the 500-line cases): pulled behind its anchor past its reach 68
  million (`FROZEN` 63 million), folded back to a pointer within reach 59 million, straight up
  past its reach 52 million, down 39 million, against 14 million pulled ahead past its reach.
  There the cost is not convex, the Newton Hessian factors only under a shift and the steps gain
  a few percent each for most of `MAX_ITERATIONS`. Whether such a drag lands in its first frame
  hangs on `TURNING_SHIFT`: at 2e2 the swing behind the anchor runs out of steps 3,200 mm off the
  ray after 300 million units, while from 5e2 up the conflict diagnosis of a thousand-line chain
  stretched between held ends (`diagnosis_tests.rs`, which already needed all of
  `DIAGNOSIS_WORK`) runs out of budget.
