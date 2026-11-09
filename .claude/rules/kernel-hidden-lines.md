---
paths:
  - "crates/caditor-kernel/src/hidden_lines/**"
---

# Hidden lines (`hidden_lines/`)

- `hidden_lines(solids, view, tolerance)` returns a `HiddenLines`: every edge and silhouette of the
  solids as `ViewPiece`s in the view's plane, each a `Curve2` over a range, `Visible` or `Hidden`,
  carrying its `ViewSource` (an edge with its `EdgeName` and `ViewEdgeKind`, or the silhouette of a
  named face) and whether it is `exact`. A `HiddenLineView` is a look direction and an up
  (`new`), or a plane seen square (`square_to`); either way a `Plane` whose normal points at the
  viewer, so its local coordinates are the view's. Orthographic only. It runs on
  the calling thread and the scoped threads it fans out (below): callers keep it off the UI thread
  or under a deadline, and it polls `interrupt::check` per edge, face, ray and every `POLL_EVERY`
  segments (`HiddenLineError::Cancelled`).
- Pipeline: a `Track` per curve to draw (`tracks_of_scene`), its splits (below), each stretch
  between splits judged at its middle (`visibility::verdict`), neighbours of one class merged, a
  closed conic's last and first pieces joined round its seam so a whole circle is one piece. Every
  piece of a curve keeps one `Curve2`; a fold makes separate tracks, so nothing merges across one.
- The judging of tracks, the part that casts rays, is fanned out over the tracks
  (`fan_out::each_in_order`: scoped threads up to the available parallelism, at least
  `TRACKS_PER_THREAD` tracks each, each worker taking the next track, the caller's interrupt
  installed on every one). Each track's result is its own, kept in track order and joined after,
  so the pieces, the `undecided` count and the first error (a cancel from any thread) are those of
  one thread whatever the thread count; `hidden_lines_on` and `outer_outline_on` take the count for
  the tests. Building the tracks (`tracks_of_scene`: flattening, silhouettes, splits) stays on the
  calling thread. A perforated plate (`fixtures::perforated_plate`, release, a machine at twice its
  cores in load, 16 threads, median of 15, `hidden_line_costs` ignored) took, building tracks
  apart: 22 faces 1.8 ms with `hidden_lines` 3.6 ms on one thread against 6.1 ms on 16; 70 faces
  7.9 ms, 15.1 against 11.3; 202 faces 22.5 ms, 42.2 against 32.6, and `outer_outline` 47.6 against
  35.5. So small bodies lose to the cost of spawning and a large one gains a quarter, with the
  serial tracks about half the one-thread time bounding it near two.

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
  tolerance (`refined`, `REFINE_DEPTH`, `MAX_CHAIN_POINTS`). A closed contour smaller than a grid
  cell can be missed. A flat region counts as positive (`FLAT_FACING`), so a face seen wholly
  edge-on has no contour and a partly flat one is outlined where it curves away.
- Contours run past the face and are cut to it (`kept_ranges`): the roots of `n·d` along every
  bounding edge (`roots_among`, bisected) are the only places a contour can cross the boundary, so
  they are mapped onto it (when within reach) and the stretch between two is kept when its middle
  lies strictly inside the face. A contour lying along an edge reads `OnBoundary` and is dropped,
  since the edge draws it. The same roots split the edge.

## Splitting (`events.rs`, `add_piercings`)

- Visibility can only change along a curve where it passes over another curve's projection, ends on
  one, turns back in the view, or enters a solid, since the projection of a face is bounded by the
  projections of its edges and contour. So splits are the roots above; proper crossings of the
  sampled polylines refined by Newton on both curves; every track end lying within twice the chord
  of a segment, refined by its nearest point on that curve (a silhouette meets a rim tangentially,
  where a crossing test sees nothing); reversals of the sampled direction (`add_turn_backs`, a
  contour's cusp); and, for several solids, intersections with the faces of other solids whose boxes
  it overlaps. A failed piercing counts in `undecided`. The segments share one `BoxTree`; spare
  splits only cost a ray and merge away. Splits within `SAME_SPLIT` in the view are one.

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
  (`outer_loops`): a positive area is an outer boundary and a negative one a hole, left out, so a
  block seen along its bore is four lines and a torus along its axis its rim alone. A stretch that
  cannot be decided is `OutlineUndecided`, one that closes no loop `OutlineOpen`.
- Pieces of one source are numbered by `occurrence`, of `of`, along their middles' x then y, so a
  cylinder's two silhouette lines are told apart while the face's name is the same.

## Tests

- `tests.rs` holds the fixture solids to known answers, `edge_totals` to the sum over each edge,
  `verdicts_agree_with_rays_cast_at_a_fine_mesh` every fixture to rays against a fine mesh wherever
  the mesh decides clearly, and `work_on_a_row_of_blocks_grows_about_linearly` the cost by `tally` on one thread (tally is
  per thread). `lines_found_on_many_threads_are_those_of_one_thread` and
  `it_can_be_cancelled_anywhere_on_many_threads` hold the fan-out to the serial answer and to
  cancelling at every poll.
