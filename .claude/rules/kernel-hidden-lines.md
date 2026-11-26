---
paths:
  - "crates/caditor-kernel/src/hidden_lines/**"
---

# Hidden lines (`hidden_lines/`)

- `hidden_lines(solids, view, tolerance)` returns a `HiddenLines`: every edge and silhouette of the
  solids, and every crossing of two solids' faces, as `ViewPiece`s in the view's plane, each a
  `Curve2` over a range, `Visible` or `Hidden`, carrying its `ViewSource` (an edge with its
  `EdgeName` and `ViewEdgeKind`, the silhouette of a named face, or the crossing of two named faces
  of two solids) and whether it is `exact`. A `HiddenLineView` is a look direction and an up
  (`new`), or a plane seen square (`square_to`); either way a `Plane` whose normal points at the
  viewer, so its local coordinates are the view's. Orthographic only. It runs on
  the calling thread and the scoped threads it fans out (below): callers keep it off the UI thread
  or under a deadline, and it polls `interrupt::check` per edge, face, ray and every `POLL_EVERY`
  segments (`HiddenLineError::Cancelled`).
- Pipeline: a `Track` per curve to draw (`tracks_of_scene`), its splits (below), each stretch
  between splits judged at its middle (`visibility::verdict`), neighbours of one class merged, a
  closed conic's last and first pieces joined round its seam so a whole circle is one piece. Every
  piece of a curve keeps one `Curve2`; a fold makes separate tracks, so nothing merges across one.
- Both halves of the work are fanned out with `fan_out::each_in_order` (scoped threads up to what the
  helper budget spares, `kernel.md`, at least a few items each, each worker taking the next item, the caller's
  interrupt installed on every one); each item's result is its own, kept in item order and joined
  after, so the pieces, the `undecided` count and the first error (a cancel from any thread) are
  those of one thread whatever the thread count; `hidden_lines_on` and `outer_outline_on` take the
  count for the tests. Building the tracks (`tracks_of_scene`) fans out over the edges (flattening,
  roots on curved faces, each returning its tracks and the boundary points it found, merged in edge
  order), over the faces (contours), over chunks of the sampled segments (crossings) and of the
  tracks (track ends, piercings); a chunk returns its splits as `(track, parameter)` pairs applied
  in chunk order, so every track's splits come in the order of the serial run. Only the
  segment tree, turn-backs and the merges are serial. The judging of tracks, the part that casts
  rays, fans out over the tracks (`TRACKS_PER_THREAD` each). A perforated plate
  (`fixtures::perforated_plate`, release, a machine at about twice its cores in load, 16 threads,
  median of 15, `hidden_line_costs` ignored) took, 202 faces: building tracks 16.5 ms on one
  thread against 5.2 on 16, `hidden_lines` 30.3 against 12.4 and `outer_outline` 33.6 against
  12.8; 70 faces 10.3 against 4.3 and 10.8 against 4.8; 22 faces 2.2 against 1.3 and 2.4 against
  1.6.

## Projecting (`flatten.rs`)

- A line stays a line (`Mapping::AlongLine`), a line seen end-on and any curve within `VANISHED`
  of one point gives no track at all. A circle or ellipse is `c + a cos t + b sin t` with `a`, `b`
  its frame axes projected: equal and square (`ROUND`) a `Circle2` with `t` unchanged, else the
  `Ellipse2` of the turn `0.5 atan2(2 a·b, a² − b²)` that makes them square (`Mapping::Offset`).
  A minor axis under `EDGE_ON` is a conic seen edge-on: a line folded where `t − turn` is a
  multiple of π, one track per fold. A spline maps its control points with weights and knots, so
  it is exact whatever its degree. An intersection curve is a degree-one spline through
  `Curve::sample`, `sampled`, its parameters the edge's.

## Silhouettes (`contour.rs`, `trace.rs`)

- A contour is where `n·d = 0` on a curved face (`facing`). Closed form: a cylinder's two rulings; a
  cone's two rulings where `tan α (axis·d) = |d square to the axis| cos(u − φ)`, none when
  the view is inside the cone's angle; a sphere's circle square to the view. A torus's contour is
  `v(u) = atan2(−(d·radial), d·axis)` and that plus π (`torus_branch`), halved until within the
  chord, `sampled`; seen along its axis it is its two equator circles, seen square to it its top
  and bottom circles and two tube circles, all exact. An extrusion's are rulings at the roots of
  `(profile′ × direction)·d` along the profile, exact lines at sampled roots; none seen along the
  direction.
- A revolution or spline face is traced (`traced_contours`): signs of `n·d` on a grid over the
  face's uv box (`CELLS_PER_SPAN` a span, `MIN_CELLS` to `MAX_CELLS`), marching squares (`links`,
  a saddle decided by its centre, keys wrapped across a periodic side so a contour closes round a
  seam) into chains, each crossing bisected on its grid side and corrected by Newton on
  `(Su × Sv)·d` (`on_contour`), and each chord halved while the corrected middle lies past the
  tolerance (`refined`, `REFINE_DEPTH`, `MAX_CHAIN_POINTS`). A flat region counts as positive
  (`FLAT_FACING`), so a face seen wholly edge-on has no contour and a partly flat one is outlined
  where it curves away.
- What lies between the grid's nodes is looked for before marching (`small_loops.rs`): a row and a
  column are put through each point found, so a node carries it (the grid's lines need not be
  evenly spaced, `step` is the widest gap). On a spline face `n·d` times positive factors is a
  polynomial on each knot span, `(Su × Sv)·d` from the span's Bezier net
  (`BSplineSurface::bezier_net`), `w (Au × Av)·d − wu (A × Av)·d + wv (A × Au)·d` in the homogeneous
  `A`, `w` of a rational one, kept in Bernstein form (`bernstein.rs`), so its coefficients bound
  its sign on any part of the span; `Quadrants` halves it, memoised, only where that bound is
  mixed. A span is first settled from its B-spline net, every pair of the two hodographs' control
  vectors (`net_bound`, non-rational), and its polynomial built only where those pairs disagree
  (measured against one evaluation: a net bound costs about 1.5, a polynomial 10, a restriction 2).
  A cell whose surrounding 4×4 nodes all share a class, so no contour crosses it or a neighbour
  (where refining already follows a contour bulging past a grid line), and whose bound allows the
  other class is searched by quarters to `MAX_SEARCH_DEPTH`, each centre evaluated, the first of
  the other class kept. Bounds allow `BOUND_SLACK` of the factors' magnitudes, so a face seen
  edge-on is not searched. On a revolution `n·d` round a parallel is `α + β cos(u − φ)`, so every
  region of one class reaches a meridian facing the view most or least, or an end of a partial
  turn: those meridians are grid columns (`meridians`), and each gap between two rows whose nodes
  on them share a class is searched by halves under the profile's polynomial
  `r (z′ (R(u) e_r)·d − r′ (axis·d))` (`R (c (Z′W − ZW′) − d_z (R′W − RW′))` for a rational profile)
  when the profile is a B-spline, by the turning points of `n·d` between rows (below) otherwise.
  Budgets: `MAX_SEARCH_POINTS` evaluations and `MAX_FOUND` points a face, and no search on a spline
  face of more than `MAX_BOUNDED_SPANS` spans in its box. `tally::measure_searched` counts this work
  and the turning points'.
- Contours run past the face and are cut to it (`kept_ranges`): the roots of `n·d` along every
  bounding edge (`roots_among`, bisected) are the only places a contour can cross the boundary, so
  they are mapped onto it (when within reach) and the stretch between two is kept when its middle
  lies strictly inside the face. Besides sign changes at the samples, a gap where `|n·d|` falls
  leaving one sample and rises into the next (slopes by a forward difference of `SLOPE_STEP` of the
  gap, `turning_points`) holds a turn, found by safeguarded cubic Hermite steps on the two values
  and slopes (`TURN_STEPS`, `TURN_GUARD`); a turn of the other sign gives a root either side, so a
  contour folding across the edge between two samples is cut at the edge. A contour lying along an edge reads `OnBoundary` and is dropped,
  since the edge draws it. The same roots split the edge.

## Crossings (`crossings.rs`)

- Solids overlapping without a boolean meet where their faces cross, a line of the union neither
  solid has as an edge. `face_pairs` takes each pair of solids whose boxes overlap (`Scene::tree`),
  each face of the first overlapping the second's box and each face of the second overlapping that
  face's box (`SolidClassifier::faces_overlapping`, `face_bounds`); a view of separated solids does
  one box-tree look-up a solid and nothing else
  (`separated_blocks_add_no_crossing_lines_and_no_work_beyond_finding_them_apart`). Pairs fan out
  (`PAIRS_PER_THREAD`), each polling the interrupt, under `intersect_surfaces` of the two faces'
  patches with its own budgets; a failing pair counts in `undecided`.
- A coincident pair or a tangent branch (faces touching) gives no line. A branch is cut to both
  faces: its range, sampled (`CROSSING_SAMPLES`) and seeded with where each face's boundary edges
  meet the other's surface (`inside_intervals`), keeps what lies strictly inside both faces, each
  end snapped onto the seed it bisected towards. So a line along either face's boundary is left to
  the edge drawing it: two blocks sharing a face plane, touching, or one resting on the other add
  nothing. A closed conic cut at its seam is joined round it.
- A crossing track belongs to the first solid with `Own::Crossing` naming the second and both
  faces. Locally it is hidden when the view enters either face (`face_hides`, along a face the
  bend as for an edge), since the material of each solid lies behind its own face; its ray skips
  near hits on both faces, and piercings skip both solids.

## Splitting (`events.rs`, `add_piercings`)

- Visibility can only change along a curve where it passes over another curve's projection, ends on
  one, turns back in the view, or enters a solid, since the projection of a face is bounded by the
  projections of its edges and contour. So splits are the roots above; proper crossings of the
  sampled polylines refined by Newton on both curves; every track end lying within twice the chord
  of a segment, refined by its nearest point on that curve (a silhouette meets a rim tangentially,
  where a crossing test sees nothing); reversals of the sampled direction (`add_turn_backs`, a
  contour's cusp); and, for several solids, intersections with the faces of other solids whose boxes
  it overlaps (for a crossing, solids other than its two). A failed piercing counts in
  `undecided`. The segments share one `BoxTree`; spare splits only cost a ray and merge away. Splits within `SAME_SPLIT` in the view are one.

## Visibility (`visibility.rs`, `SolidClassifier::ray_view`)

- A stretch is hidden when material or a face lies between it and the viewer. First locally, at
  the curve itself, since a ray starting on a face cannot see it reliably: for an edge, the angle
  of the view about the edge against the sector of material between its two faces' inward
  directions; a view along a face's tangent plane (`ALONG_FACE`) runs along that face and is hidden
  unless the face curves away from the ray (`bend_along`, the second fundamental form along the
  view). So the back top edge of a block seen level is hidden behind the top face it lies in, the
  front one is not, and the far half of a rim seen edge-on is hidden behind the plane it lies in.
  For a contour, the same bend alone: a ray entering the surface (the inner half of a torus's tube
  circle, a bore's lines) is hidden there. A sampled contour's point is first put back on the
  surface (`on_contour`), since its chord lies up to the chord off.
- Then `ray_view` along the view over the solids the line reaches (`Scene::tree`): a hit strictly
  beyond `RAY_TOUCH`, or an overlap holding a probe of `OVERLAP_PROBES` beyond it, on or inside a face
  covers. The stretch's own faces skip point hits within `OWN_FACE_REACH` (a tangent ray's double root
  lands a little past zero), and a stretch's own plane skips point hits altogether. Coincident
  lines are therefore hidden behind a visible one, not doubled as visible, and a line's sides
  never decide it. A ray that cannot be decided is tried at a quarter and three quarters; a piece
  still undecided is kept visible and counted in `HiddenLines::undecided`, never guessed silently.
- `meets_smoothly` flags `ViewEdgeKind::Tangent`, a face used twice `Seam`, so a drawing can leave
  them out; the pieces are all there, so visible plus hidden length is each edge's projected length.

## Outer outline (`outline.rs`)

- `outer_outline(solid, view, tolerance)` is the outer boundary of the solid's projection as
  `OutlinePiece`s, for Project (`document.md`). A visible stretch (seams left out) belongs when its
  two sides differ: `Sides::material_on_left` casts a line along the view `SIDE_STEP` either side of
  its middle, from behind the whole solid. Before merging, since a line may run along the boundary
  for part of its length. Coincident stretches facing one way keep the best ranked, a sharp edge
  over a tangent one over a silhouette (`without_coincident`), so a cylinder's silhouette line and
  seam are one line. Stretches are oriented with the material on their left and walked into loops
  (`outer_loops_of`): a positive area is an outer boundary and a negative one a hole, left out, so a
  block seen along its bore is four lines and a torus along its axis its rim alone. From a
  stretch's tip the walk goes on to the start within reach (twice the chord plus `EXACT_SAME_LINE`)
  turning furthest right, the loop's own first stretch among them (`Walk::next_after`, nearest then
  first as ties): a hole meeting the outline at one point is walked as its own loop and left out,
  not run through as a figure eight with it. Directions are taken to the first sample beyond reach.
  Starts sit in a `BoxTree`, so a step looks only at those near its tip, and every step polls the
  interrupt. A stretch that cannot be decided is `OutlineUndecided`, one that closes no loop
  `OutlineOpen`.
- Pieces of one source are numbered by `occurrence`, of `of`, along their middles' x then y, so a
  cylinder's two silhouette lines are told apart while the face's name is the same.

## Tests

- `tests.rs` holds the fixture solids to known answers, `edge_totals` to the sum over each edge,
  `verdicts_agree_with_rays_cast_at_a_fine_mesh` every fixture to rays against a fine mesh wherever
  the mesh decides clearly, and `work_on_a_row_of_blocks_grows_about_linearly` the cost by `tally` on one thread (tally is
  per thread). `lines_found_on_many_threads_are_those_of_one_thread` and
  `it_can_be_cancelled_anywhere_on_many_threads` hold the fan-out to the serial answer and to
  cancelling at every poll. A ring of bars whose hole meets the outside at one corner, built in
  every order, and checkerboards with their edges shuffled hold the outline walk to the outside
  alone; `walking_the_outline_grows_about_linearly` holds its cost by `tally`. A pin through a
  block holds crossings to the lines on its front face seen and on its back face hidden, blocks
  sharing face planes, touching or resting on each other and cylinders against or lying on a block
  to none, and `crossings_are_found_on_many_threads_as_on_one_and_can_be_cancelled_anywhere` the
  crossing stage to the serial answer and to cancelling at every poll. A bump on a spline sheet
  between the grid's nodes, a dip of a turned profile between its rows, and that dip crossing the
  rim between two of the rim's samples hold the search and the turning points to closed outlines
  and to a contour stopping at the rim; `small_loops.rs` holds the span and profile polynomials to
  the sign of `facing` on plain and rational surfaces, and
  `looking_for_small_loops_and_folds_costs_normal_curved_parts_a_small_share_of_their_work` keeps
  that work within a fifth of the tally on every curved fixture from five views.
