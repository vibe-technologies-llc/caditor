# caditor bugs

Wrong results, silent losses and refusals of shapes and operations that should work today, each with
its reproduction or pinning test where one exists.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Saving and recovery

- [medium · small] A saved copy kept aside after a failed replace (`save::KeptAside`, named in
  the save's error) keeps its temporary's name, so once that caditor has exited, the next
  successful save to the same path removes it as an orphan (`remove_orphaned_temporaries`), with
  the earlier versions only it holds. Recording the kept name (in the journal or a marker beside
  it) and sparing it in the sweep would close it.
- [medium · small] Keep and Restore name a version by its index (`with_version_kept`,
  `load_version`) while a save can be in flight (`history.rs` and `files.rs` allow both): Keep
  queues behind the save, which puts a new version at index 0 and may thin others, and Restore
  reads the file directly, so either can act on another version than the one clicked, and the one
  meant to be kept can later be thinned away. Carry the version's digest with the index and refuse
  on a mismatch.
- [medium · small] Quitting while a save is in flight asks "Save changes?" (`dirty` clears only on
  `Report::Saved`): Save is refused as still saving, and Close without saving queues
  `Close { discard: true }` behind the save, which removes the journal even when that save then
  fails (a full disk). Wait for the save (`after_save = Some(Quit)` with a saving note) instead of
  asking.
- [medium · medium] Running out of memory during a save aborts the process instead of failing the
  save, and an abort skips the journal flush: `caditor-zstd` allocates its output with an
  infallible `vec![0; n]`, stored chunks are copied with `to_vec` (`binary/mod.rs`), and the
  read-back allocates the whole written file (`save.rs`) while the encoded bytes, the previous file
  and the snapshots are still held. Only `try_reserve_exact` for sliced content yields the
  `UnpackError::OutOfMemory` retention is written to handle. Allocate fallibly there and drop the
  held buffers before the read-back.
- [low · small] A change landing after Quit has closed storage is neither journaled nor asked
  about: `Model::close` takes the storage and `record` then does nothing, `Files::poll` quits when
  closing finishes without checking `is_dirty()` again, and `run_due` still applies due reruns
  (`Rerun::Editing`, `OwnThickness`, `OwnAngle`) while closing. Drop the model's queue in
  `close_storage` and check dirtiness again before quitting.
- [low · small] On Windows, Save As to the open file spelled in another letter case compares paths
  as text: `storage.rs`'s `owns` takes its own journal for one locked elsewhere and refuses
  (`OpenInAnotherWindow`), and `save.rs`'s `reads_target` skips the changed-on-disk guard and the
  `.damaged` copy when the journal is in the recovery folder. Open canonicalises the whole path and
  Save As only its folder (`files.rs`), so a model reached through a symbolic link differs the same
  way. Compare by `FileKey` (`os::same_file_path`) at every such site.
- [low · small] Reading a journal has no memory budget (`journal.rs` unpacks each entry with
  `unpack(None)` and keeps it), unlike loading a model, and journals are read on every open and at
  startup, so a damaged or crafted journal beside a downloaded model may exhaust memory. How large
  restored transactions can get is unchecked; charge a `Budget` across entries if they can.

## Cloud and the MCP server

- [medium · small] The sign-in listener binds 127.0.0.1 (`loopback.rs`) but sends the browser to
  `http://localhost:<port>/callback`, which browsers often try as `[::1]` first, where any local
  process can listen on the same port and take the code; with PKCE that only blocks the sign-in,
  but Onshape's PKCE is unconfirmed (`DECISIONS.md`). Listen on both, or register `127.0.0.1` as
  RFC 8252 §8.3 recommends.
- [medium · small] The sign-in listener can be held or ended by anyone: connections are answered
  one at a time with a 3 s timeout per read, so a client sending a byte every few seconds holds it
  for hours, the stop flag and deadline checked only between connections; any `?error=` ends the
  sign-in without checking `state`, and the first `code` and `state` end the wait with the
  signed-in page before the state is compared (`client.rs`). It stays bound through the keyring
  save (`cloud/jobs.rs`, an unlock prompt of up to 2 minutes), blocking the other service's sign-in
  on the shared port. Give each connection a deadline, ignore callbacks of another state, and drop
  the listener once the callback is taken.
- [medium · medium] A sign-in or token refresh finishing after Cancel, Sign out or turning cloud
  off stores its session anyway: `cloud/jobs.rs` saves after the token exchange without checking
  `stop` again, Cancel queues a separate forget that can run before that save (and with `revoke`
  forgets nothing), and a refresh in flight (`client.rs`, `renew`) saves after sign-out's forget,
  so the session comes back unrevoked at the next start, against `cloud.md`. A forgotten
  generation the store checks on saving, and revoking a session that arrives late, close it;
  nothing tests these orders (`FakeProvider` holding the exchange and `FakeKeyring` can).

## Kernel

- [high · hard] Booleans between the fixture solids in random placements all succeed on the survey's
  seed (`boolean::tests::random_placements_of_every_fixture`, ignored), and the failures other seeds
  (`Random::new(n)`, 7 to 19, 48, 52, 57, 76, 79, 82, 85, 160, 161, 163, 178, 214, 255, 291, 325,
  403, 405, 432, 486, 632, 724, 767, 1115, 1135, 1136, 1214, 1225, 1272, 1344, 1383 and 1402) gave
  are pinned by tests and combine. Seeds 20 to 1114 all combine, those from 61 to 985 surveyed
  before the last marching change and not again. Seeds 1115 to 1420 were not surveyed again after
  the fixes for 1135, 1136 and the others; the survey's own seed 2 and seed 1422 combined
  throughout on the build before the fix for 1136 and were not run again since, and seeds beyond
  1422 are unsurveyed.
  Tori that nearly coincide are surveyed only down to turns of 1e-5 radians: a torus and its
  copy turned 1e-4 about nine chosen axes, X and Y, and 1e-5 about six more,
  combine (the four `a_torus_and_its_copy_turned_…_combine` tests), as
  they do about all ten axes `Random::new(7)` draws (components in -1 to 1) at 1e-4 and the next
  ten at 1e-5. At 1e-6, seven of the ten axes `Random::new(11)` draws fail (`Split`, `Open`), and
  the other three combine into wrong volumes: their union plus intersection falls 4.6e-5 to 6.8e-5
  short of both tori, the union ending about 1e-7 beyond one torus where the difference says 3e-5,
  which the turned tori test's bound (1e-6 of a torus) does not see. Sines there stay below
  `STOP_SINE` nearly everywhere, so most sections are never seeded (`TOUCH_SINE`); three of the
  seven (the second, third and seventh axes) combined into such wrong volumes before branches
  stalling far from a touch marched on to `FAR_STOP_SINE`, and now fail `Split` beside the
  u seam on the inner equator, at (4, 0, ±1e-6).
  A torus and its copy shifted 1e-5 along X or Y combine
  (`a_torus_and_its_copy_shifted_a_hundred_thousandth_along_x_combine_exactly`, `_along_y_combine`),
  as they do along Z and along all three axes at once, and along Y by 3e-5 and 1e-3; by 2e-5 along Y
  they are `Split` in every operation at (-0.083, 5.9994, 2), on the top circle. Shifts of 1e-4 and
  3e-6 along Y did not finish all three operations within a minute, so are unjudged.
  Lowering `STOP_SINE` itself to 1e-7 or 1e-8 fails
  the X shift again (`Invalid`: an edge used twice at one place in a face's domain) or a 1e-5 turn
  (`Split`), and polishing near-tangent contacts on one surface by the other's signed distance
  changed none of these.
- [medium · hard] Near-tangent booleans can turn on the last bit of a result. The kernel and the
  geometry crate now take their maths from `libm`, so Linux and Windows agree bit for bit, but a
  plug 2e-6 off its bore's axis at 45 degrees and 3e-6 wider still fails as `Open` at the plate's
  start face in the intersection (it combined with the C library's results moved one ulp up or at
  random), and the turned tori tests (`a_torus_and_its_turned_copies_combine`)
  fail `Open` with every result moved one ulp up or down, so such tori still turn on the last
  bit. The combine tests of a failing place cut a plug 1.5e-6 off at 112.5 degrees and 1e-6 wider
  from the plate, its wall 5e-7 inside the bore's over a wide arc on the near side (`Split` at the
  plate's start face), whose outcome survived every result moved one ulp up, down or at random; the
  plug they used before (8.03e-6 off, 7.76e-6 wider, its rim dipping 2.7e-7 through the bore's wall)
  combines now that such a dip is two crossings. Plugs 8e-6 to 2e-5 off with their wall dipping
  2.7e-7 to 9e-7 inside the bore's failed `Split` in every operation at 9.6 to 80 degrees; they now
  combine, but at 45 degrees their intersection is `NonManifold` at mid-height, as it already was for
  plugs 5e-6 off at every angle and for walls just clear of the bore's. A
  combine failing `NonManifold` names no place, whatever the reason, since
  `CombineCause::Edgewise` drops the kernel's `BooleanSite`.
- [medium · hard] Shell cannot split a corner whose offsets do not meet when its convex and concave
  edges alternate (two ridges of different slopes crossing) or one convex edge meets concave ones (a
  cavity whose ridge runs over its inside corner): the offset there joins faces the body keeps
  apart, or runs an edge between another pair of faces, which splitting the corner into several
  cannot give. Corners of more than `MAX_SPLIT_FACES` (24) faces are not split either. All are
  refused as `Corner` (`shell::tests::a_corner_where_ridges_and_valleys_alternate_is_named`,
  `a_cavity_whose_ridge_runs_over_its_inside_corner_is_named`). In the cavity, roofs sloping 3/5
  and 3/7 put the offset ridge 0.076 of the thickness to the front of the wall below it, so the
  ridge runs on straight past the corner and that wall's top edge now meets the back roof: the
  edge between the front roof and the wall has to become a ridge piece plus a new edge between the
  wall and the back roof. A mirrored `ConvexButFold` (concave but one convex edge, residual signs
  flipped) was tried and finds no triangulation of that cycle, as expected. Between the crossing
  ridges, the lower one stops short on the higher roof, and the two faces of that roof's plane
  that the lower roof kept apart (touching only at the vertex) become one face. Both need the
  corner rebuilt from the arrangement of its offset planes under its own union and intersection
  formula (convex edges intersect, concave ones unite), with an edge allowed to change its pair of
  faces and faces of one plane allowed to merge; the plan builds one face per original face.
- [low · hard] Fills that use up a whole face lose the edges on it: a 4 mm pocket's twelve edges
  chamfered 2 mm (`fills_using_up_a_pocket_s_walls_lose_its_rim`) is `Lost` for the rim, since the
  corner edges' bevels meet exactly across each wall and take it away; cutting first instead leaves
  the corner edges ending at the rim chamfers' mitre, the concave mirror of a boss's upright, which
  `carried_round` handles only for concave edges after convex ones. The rim would have to be
  carried onto the edges between the top and the corner bevels.
- [medium · hard] An edge ending at a corner earlier fillets made meets them along a crease, or is
  refused. The uprights of a 40×30×20 box whose top edges were filleted 2 mm take any fillet or
  chamfer, but only a 2 mm fillet ends in a smooth ball corner (`corner::after_blends`); a 1 mm one
  meets the top fillets along their intersection, a crease of about 50° where the three faces meet
  (`out_through_blends`), where other modellers roll the ball on along the seam between the top
  fillets (a pipe surface pinched to a point at the top face; the seam is smooth there, so
  `blend_chain` never follows it and the lofted path cannot end on it). The seam, the two
  cylinders' intersection in the bisecting plane, leaves the upright's top vertex along the
  upright (so the chain would continue tangentially) and turns from 90° there to tangent at the
  top face; the missing pieces are a sharpness test that does not judge the seam by its smooth end
  and a lofted run whose sections narrow to nothing at its end. A pocket's corners over its
  rounded floor likewise crease at any size but the floor's radius, where they end in a concave
  ball corner (`a_pocket_s_corners_filleted_as_its_floor_was_end_in_the_ball_corners_of_a_pocket_filleted_at_once`).
  The upright of a boss whose foot an earlier fillet rounded is still `UnsupportedEnd` at
  its foot, the refusal naming the earlier blend and saying to move the feature above it
  (`an_upright_of_a_boss_whose_foot_was_rounded_is_refused_at_its_foot`,
  `an_upright_rising_from_a_foot_an_earlier_blend_rounded_names_it_and_says_to_move_above_it`):
  the foot's fills meet in a concave seam under a convex edge, and extending the edge's tool
  down would cut into the plate.
- [medium · hard] The top rim of a box tapered to widen upwards, whose slanted corner edges were
  filleted barely larger than the rim fillet, is refused as `TooLarge`: under a 2 mm rim fillet a
  40×40×30 box tapered -4° needs corners of 2.5 mm (2.4 mm is refused), at -8° 2.7 mm (2.5 mm is
  refused), at -2° 2.4 mm (2.2 mm is refused)
  (`a_rim_fillet_whose_ball_pivots_inside_the_corners_of_a_box_widening_upwards_is_too_large`).
  The ball's side contact lies below the top, where the leaning corner cylinder's axis has moved
  in by about the drop times the lean, so the ball centre, `R − r` from that axis, stands almost
  over the rim arc's centre of curvature: the top contact barely moves (or turns back) round the
  corner, `Reach::unfolded` cuts the chord's overhang beyond it below `LEAST_OVERHANG`, and the
  lofted tool's sections would cross there. Rounding these needs a tool whose sections are not
  fanned about the rim's curvature axis, or the fillet surface trimmed where it folds.
- [low · easy] Guards that hold only by what callers do today: `build/loft/skin.rs` raises the
  degree of an empty segment forever (`raised(&[])` returns empty and `segment` answers an empty
  slice out of range), `build/loft/mod.rs` and about 25 other places take a largest value with
  `fold(0.0, f64::max)`, which drops NaN before an `is_finite` check, and `caditor-geometry`'s
  `Plane::new`, `Plane::with_x_axis` and `Ray::new` accept a non-finite origin that
  `Plane::from_frame` refuses. Each is masked by an earlier check (`Profile::new`, the loft's
  construction, `checks::checked_frame`).
- [low · small] A rigid move puts a body anywhere finite: `Solid::transformed` checks only that
  vertices are finite, not `MAX_SIZE` as `Solid::mapped` does, so Move (`movement.rs`, whose
  `MoveCause::TooFar` comes only from a non-finite result) accepts a body 1e11 mm away, where one
  f64 step exceeds `LINEAR_RESOLUTION` and later booleans fail without naming the cause. Refuse
  past `MAX_SIZE` there.
- [low · medium] Two edges alike in every other part of their names are told apart by
  `occurrence` (`naming/mod.rs`), their midpoints rounded to a 1e-4 grid and ordered by x, then y,
  then z: a midpoint within an ulp of a cell boundary can round either way after an upstream edit,
  and turning the sketch plane changes which world axis orders them, so a fillet's reference can
  resolve to the other edge without `Ambiguous` (`kernel-naming.md` calls the grid coarse enough
  never to swap them). Unreproduced; an order from the feature itself (the profile entity, the
  parameter along the generating curve) would not depend on placement.
- [low · medium] `outer_outline` (`hidden_lines/outline.rs`) walks on to the nearest chain start,
  so where a hole meets the outline at one point the walk can run through both as a figure eight
  and keep the hole's runs in the outline Project makes; the walk also scans every chain at each
  step without polling the interrupt. Unreproduced; choose by turning angle at a tip with several
  candidates, and index the starts.
- [low · hard] `hidden_lines` draws edges and silhouettes only, so two bodies overlapping without a
  boolean show no line where their surfaces cross (each hides its part inside the other, and the
  stretches are right). A traced silhouette on a revolution or spline face misses a closed loop
  smaller than a cell of its grid (`hidden_lines/trace.rs`, `MAX_CELLS` a side), and a contour
  folding in the gap between two samples of a bounding edge can run past the edge there
  (`roots_among` brackets by sign changes at sampled points).

## Modelling

- [medium · medium] A configuration can become one that cannot be switched to: a row's cells are
  checked for cycles only when set (`configurations.rs`, `check_row_cycle`), so a live expression
  set later (`c = a` while a row holds `a = c`) makes `applying` that row a cycle, refused, and
  deleting the active configuration fails when that row comes next. Check a live expression
  against each inactive row overlaid on the graph, naming the row.
- [medium · medium] Pattern and mirror copies that land a hole or extrusion end (`landing.rs`)
  evaluate the original with the pattern's parameter values (`LandingInputs::inputs`), and
  `Measured::overlay` fills only the measurements the pattern reads, so an original sized by a
  measurement fails `CopyEnd` in the copies or lands with other values. Keep the original's values
  in `LandingInputs` when captured; untested with a measured diameter.

## Sketch solver

- [medium · hard] A sketch with a closed centripetal spline through one point twice moves when
  solved again from its own result (seeds 269470, 306204 and 173746 of
  `a_solved_sketch_does_not_move_when_solved_again`, by 0.34, 0.14 and 0.004 mm; seed 257788 of
  `a_solved_sketch_solves_again_from_its_own_result` now solves again). In seed 269470 (a distance
  from an axis to a spline through four fit points, one repeated, and nothing else) no spacing
  round's solve with the fit points frozen holds, so all eight rounds and then the
  `PARAMETER_ROUNDS` re-measurements run (16 in the first solve, 6 in the second): the loop through
  the repeated point has two bulges near the dimension and each round holds one while the other
  comes nearer, as in the alternating projection `sketch-solver.md` describes, and the solve
  ends on whichever bulge the last round held, sliding the whole sketch up to 0.3 mm along the
  directions the dimension leaves free. `a_solved_sketch_does_not_move_when_solved_again` skips
  these splines.
- [low · hard] A sketch with a closed degree-one spline (a polygon of its control points) and a
  `Distance` dimension from it to another curve moves by 0.4 mm when solved again from its own
  result (seed 1555 of `a_solved_sketch_of_splines_of_chosen_degrees_does_not_move_when_solved_again`
  with `regraded` allowed degree one for closed splines, which it no longer does: `regraded`
  gives a closed spline at least degree two). The cause was not worked out.
- [medium · hard] An open fit spline through three coincident points and a fourth, held a distance
  from a line (`a_spline_with_three_of_four_points_coincident_solves_again_from_its_own_result`),
  moves by 0.05 mm when solved again from its own result for about one rounding in ten of an
  input by up to three ulps (the Windows CI run fails it where Linux passes). The hidden controls
  beside the coincident points sit 1e-6 mm apart, so the end tangent's direction is finer than a
  coordinate of 10 mm can hold: the first attempt stalls at a residual of 2e-9 against a tolerance
  of 1.6e-9 and a perturbed retry lands elsewhere. Weighting the tangent form by the tangent's
  length, or the interpolation rows by 1e3, each removed the jump and left a drift of 2e-6 mm or
  moved it to the closed spline; neither is kept.

## Sketching and expressions

- [medium · easy] Split, trim and break give a dimension they move to the far piece a new id
  (`trim.rs`, `move_constraints` re-adds it through `add_detached_constraint`), so a model
  parameter owned by that angle dimension (`ParameterOwner::Dimension`, by `ConstraintId`) keeps an
  owner that no longer exists: the parameters table cannot focus it, and deleting the dimension
  later leaves the parameter behind. Re-attach under the old id (`attach_constraint`), which the
  restructure has already freed.
- [medium · easy] A sketch's id counter can reach `FIRST_UNSTORABLE_ID` (`reserve_ids_below` caps
  `next_id` at exactly it and `allocate` never checks), so after loading an entity at 2^63 − 1
  every new one gets a reserved id in the letter and gear curves' range, and the saved file is
  refused on reloading (`ReservedId`). Refuse in `allocate`, as the document refuses its own
  counters.
- [medium · easy] Spelling an imperial unit out ("2 inches", "3 feet") reads as `Unit::Inch` or
  `Unit::Foot` and is refused with the hint to write `in` or `ft` (`parse.rs`), offering an
  imperial unit against `ux.md` and leading to a second refusal; `UnitWithoutNumber` can suggest
  "10 in" too. Send spelled imperial names to `ImperialUnit`.
- [low · easy] `tan` at a pole gives about 1.6e16 and `atan2(0 mm, 0 mm)` gives 0°
  (`expression.rs`), both finite, so a parameter built on them takes a meaningless value where
  `asin`, `acos`, `ln` and `sqrt` refuse as `OutOfDomain`.
- [low · easy] A stored expression can display longer than typed text may be: `display_number`
  writes `1e-300` positionally, about 300 characters, where stored text takes the shorter
  scientific form, so confirming such a field unchanged is refused as too long. Display numbers the
  shorter way.
- [low · easy] `relation.rs`'s `contradicting` does not oppose a horizontal line and
  `VerticalPoints` on its own ends (or the reverse), though `subject` treats them as one relation,
  so the pre-check lets through a pair the solver then reports as a conflict.
- [low · easy] Extend removes every constraint on the end it moves (`trim.rs`): `check_free_end`
  refuses a shared end, a `Fix` and coincidence with a point, but a distance dimension, a
  `Symmetric` or a point-on-curve `Coincident` there vanish without a word (`sketch.md` names the
  refusals, not this). Refuse an end a dimension holds, or name what was dropped.
- [low · easy] Adding a dimension driven by a parameter never asks whether it would only measure
  (`solve/determined.rs` answers false for any expression needing parameters), so one on geometry
  already held is added driving and then reported redundant.
- [low · easy] The parse depth bound (`deepened`) is checked for sums, products and comparisons
  but not for negation, powers, units and calls, and `parse_with` checks no final depth, so a tree
  can pass `MAX_TREE_DEPTH` by the nesting allowed; the recursive print, clone and drop are tested
  only at the bound.

## STEP import

- [high · hard] A body whose faces meet only within the file's declared precision (CATIA and
  Autodesk exports whose tangent fillet splines sit micrometres apart) is bent where it can be
  (`step-read.md`, "Bent faces") and otherwise imports as flat facets. On a CATIA V5 export of 11
  bodies at 0.01 mm, bending makes one of the seven loose bodies exact. Two bend every side but
  leave one traced edge off its face at validation (3.6e-6 and 8.2e-5 mm); one fails tracing an
  end-to-end join of two fillets slanted on both (2.6e-3 rad apart); one has a side that is a pole
  or seam; one bends a face whose boundary then crosses itself in uv. In the VZ330 assembly 63
  bodies import as facets. The bent build refuses them for these reasons:
  - 25 have a corner of faces with no common point within the resolution. "Pulley (1)", "Pulley
    (2)", "Pulley (1) (1)", "Pulley(Mirror) (1)" and "X motor mount part A" and its mirror (3.4e-6
    mm) join two equal tooth-root cylinders 9.6e-6 mm apart, each exactly tangent to its own fillet,
    so snapping holds both (`round_faces`); moving one needs its fillet and the flank beyond to
    follow, or a rigid face turned into a spline it can bend. "Part 9 (1)" to "Part 14 (1)" miss a
    spline they lie on no line of by 2e-5 to 5e-5 mm; the Raspberry Pi connectors, "top (1)" and
    "powerswitch bracket" miss planes and cylinders by 1e-4 to 5e-4 mm.
  - 13 keep a traced edge that misses its vertex, tracing having failed across a near tangency:
    "gantry bottom" and "CPAP duct 15mm" (0.006 mm), "bottom (1)" (2.5e-5 mm).
  - 6 ("M3 Nut", the knobs) fail validation by 2e-6 to 2e-4 mm after every side bends.
  - 7 of the "PCB (1)" and "Blades (1)" splines have a target that jumps along a side (up to 0.0035
    mm) or a corner off the face it bends onto, as has "CPAP duct 18mm (1)" within 3.8e-7 mm.
  - 4 ("front handle1", "front handle2", "X motor mount part B (top)" and its mirror) have a vertex
    whose common point lies farther along the tangency than the precision.
  - "Micro Switch" and its copy settle two vertices onto one point (an edge of no length), and
    "Lead Screw lower bracket" has two edges overlapping along a side.
- [medium · medium] The VZ330 assembly (an Autodesk export at 0.01 mm, 3,980 copies) leaves 2
  parts out, refused as "a vertex 0.00003 to 0.00008 mm away from the end of its edge", the exact
  build's refusal at faces meeting tangentially further apart than the resolution, where the
  faceted fallback then fails on its own: "front handle1" meshes a sliver 1.2e-6 mm wide whose
  edges three triangles share (welding wider only opens more), and "Component25" (a thread whose
  helical edges its exact build cannot follow) would need 20,266 flat faces. "Lead Screw lower
  bracket" and "goliath heatsink fan duct (1)" import only as facets: exact, a thin spline strip
  meets its neighbours nearly tangentially, and tracing its edges across them zigzags (a 0.057 mm
  edge traced 0.34 mm long, whose pcurve then needs more than 65,536 samples). "M3 Nut" also imports
  only as facets (three faces meeting so nearly tangentially that their common point lies 0.0115 mm
  along, on the next vertex, so the ellipse edge between them has no length; bending gets within
  2e-6 mm). Reproduce by reading the file with `STEP_CORPUS` in release (about 65 s).
- [medium · medium] Assembly placement paths copy the whole class of representations per placement
  (`structure.rs`, `Placements::paths`) and `placements()` copies them again per solid
  (`Arc::unwrap_or_clone`), so 100k representations joined by plain relationships and placed by
  10k `MAPPED_ITEM`s (within `MAX_INSTANCES`) need about 10^9 ids from a 15 MB file, with no
  interrupt polled. Paths should hold the class key and test context membership through `Classes`.
  The same file starts a fresh `Work::new(MAX_WORK)` per item, not the one budget per file
  `step-read.md` describes.
- [low · hard] A face whose surface cannot be read is left out (`step-read.md`, "Unreadable
  faces"), but from the outer shell that turns the whole body into flat facets, since the kernel
  holds only closed solids, and a lost face with holes is not closed at all; an edge whose curve
  cannot be read still loses the whole body. An offset that folds anywhere in its basis's domain
  is refused even where the face's own region is clear: fit over the region the face uses.

## Drawing and mesh import

- [low · easy] Reference pictures shown by a version preview or snapshot are decoded on the calling
  thread without `catch_unwind` (`image_textures.rs`, `decoded_for`), while the background path in
  the same file wraps its decode, so a decoder panic on a crafted picture would end that thread.
  Unchecked which thread that is; wrap it the same way.

## Application

- [medium · small] A model export whose thread panics stays running for the session: `export.rs`
  runs the single and every-configuration exports without `catch_unwind` (writing contains a
  body's panic, but recomputing and the thumbnail do not), its `finished` callback is dropped and
  `running` clears only on that event, so Export, Export again, Open in OrcaSlicer and Open in
  Fusion 360 are refused and Quit waits on it until Quit anyway. A frame failing between taking the
  3MF thumbnail job and handing back its pixels (`waiting.rendering`, not reset by
  `close_dialogs`) leaves it stuck the same way. Every `Files` job is contained; one helper for the
  thread-and-slot jobs (`export.rs`, `image_export.rs`, `files.rs`) would keep them alike.
- [low · easy] `about_to_wait`'s session work (`window_export.poll`, `check_pick_fallback`,
  `redraw_when_due`) and `exiting` run outside `App::contained`, and `Session::recover` can itself
  panic in `Overlay::new`, so a panic there ends the process instead of skipping the frame
  (`reliability.md`); the hook still flushes the journal. The containment path is tested only by
  calling `after_failed_frame` directly (`ui_tests.rs`).
- [low · small] The Lighting panel's sun turn, sun height and picture turn are egui sliders whose
  number box takes plain numbers (`lighting_panel.rs`), not units (`0.5 rad`) as every numeric
  field should (`ux.md`).
- [low · small] The failed-frame steps count caught events, not frames (`FrameFailures::failed` per
  event, reset only after a redraw), so a panic in input handling could reach suppressing every
  feature, and then exiting, within milliseconds without a redraw between. Unreproduced.
- [low · medium] When the storage worker cannot be started (no thread can be spawned), Save says to
  try again every time (`model.rs`, `restart_storage`) and has no fallback; a contained
  synchronous save, worded as slow, would still save.
- [low · medium] Two safe functions of the `unsafe` crates rest on their callers: the Wayland
  `DropTarget::attach` and `ToplevelExport::attach` wrap winit's `wl_display` and keep the window
  alive, but the connection belongs to winit's event loop, so their soundness rests on the drop
  order `wayland.md` describes rather than on types; and `caditor-windows`'s `DialogParent::new`
  takes any `NonZeroIsize` and hands out a borrowed window handle claiming a live window. Make them
  `unsafe` with a stated contract, or tie them to what owns the handle; unchecked whether winit
  holds the display strongly.

## Viewer

- [low · small] An image export of 2048 pixels or more in the rendered style with ambient occlusion
  is refused on a device limited to 2048-pixel textures: the tile side is `TILE_SIDE.min(largest)`
  and the occlusion targets add `2 × MOST_PIXELS` unclamped (`viewport.rs`, `occlusion.rs`), and an
  occlusion fault fails the whole image where the window only drops occlusion.
- [low · small] A lost surface whose replacement lacks the current format fails every frame without
  stepping up to a new device (`caditor-render`'s `lib.rs`; only `Validation` climbs the ladder),
  and the image writer waits for its mapping with no deadline (`image.rs`), so a hang that never
  reports the device lost pins its thread. Neither the recovery ladder nor surface healing has a
  test (`offscreen_tests.rs` covers `DeviceLoss` and a fresh device only).
- [low · medium · blocked by: wgpu's GL backend] On GL and other devices without texture view
  formats the multisample resolve still averages in gamma space. A resolve of its own (a pass
  reading the samples through a `texture_multisampled_2d` and averaging them in linear light) was
  tried: it matches the view-format resolve on Vulkan, but wgpu 30's GL backend binds a
  multisampled texture as `TEXTURE_2D` (`gles::Texture::get_info_from_desc` never chooses
  `TEXTURE_2D_MULTISAMPLE`), so every sample reads as zero there and the frame comes out black. It
  needs that fixed in wgpu, or a GL-only blit resolve into an sRGB texture.
