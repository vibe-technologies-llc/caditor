# Audit

Direction pass against commit `81ff7c4c` (master, 2026-10-15). The question was what is
missing, or needlessly awkward, across sketching and the solid workflow that follows it.

`docs/FEATURES.md`, `docs/DECISIONS.md` and `docs/BUGS.md` are the project's own remainder.
This pass does not restate them. The six items below are the ones a modelling session hits
that those files either omit or leave unordered. Each was read in the current tree.

The earlier correctness audit that this file replaced is the history of `docs/AUDIT.md` at
`b1d3d3bd`. Those seven items were not re-checked here.

Verification for a change, matching this repo: `cargo test -p <crate> <test>`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `rust-formatter`.
A sketch or feature change also updates its page under `crates/caditor/guide/` and the rules
file that describes it, in the same commit.

## Findings

Ordered by leverage: how often the workflow pays for the gap, divided by how much of the
model has to change. The first four are absent from `FEATURES.md`. The last two are already
specified there; they are here because they are the next capabilities the same workflow runs
into, and the small items above them should land first.

| # | Finding | Kind | Impact | Effort | Risk | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Project one curve at a time, one undo each | Convenience | Reusing a profile is a click per curve | S | Low | `projecting.rs`, `viewport.rs` |
| 2 | A projection cannot be turned into ordinary geometry | Convenience | A section or an outline cannot be edited | S | Low | `edit.rs`, `project-and-intersect.md` |
| 3 | A circular pattern of points lays out no holes | Missing behaviour | A bolt circle drawn in the sketch does not drill | M | Medium | `pattern.rs`, `hole.rs` |
| 4 | New features remember only eleven starting values | Convenience | Revolve, pipe, rib, wall, offset and spacing start over | S | Low | `last_values.rs` |
| 5 | A sketch cannot carry text | Missing tool, already tracked | No engraving, no label that extrudes | L | Medium | `FEATURES.md` Sketching |
| 6 | A sweep path cannot be drawn in space | Missing tool, already tracked | A handle or a pipe run has to be edges of a body | L | High | `FEATURES.md` Sketching |

### 1. Project a whole sketch, or the current selection, in one step

- **Evidence**: Project accepts one pick at a time. `projectable` (`crates/caditor/src/projecting.rs`)
  allows a single `Pickable::SketchEntity`, and `projections_of` returns one outline for it. A
  face is the exception: the same function calls `face_projections` and returns every boundary
  edge. The click path (`crates/caditor/src/viewport.rs`, `pick_action` while
  `active.tool.projects()`) applies that one transaction immediately, so each curve is its own
  undo step. The guide (`crates/caditor/guide/project-and-intersect.md`) describes "a curve or
  point of another sketch", one at a time. Nothing reads the selection, and a sketch row is not
  a projectable pick.
- **Impact**: Copying a mounting profile, a logo outline or a pocket from one sketch onto a
  parallel face is a click and an undo step per curve. A face outline is already one step. The
  two gestures do the same job.
- **Effort**: S. Coarse: a day, including the UI test.
- **Risk**: Low. Each curve is still the same `add_projection` the click uses. A curve the
  sketch already projects is skipped today (`already_projected`); a batch must skip the same way
  and say what it skipped.
- **Confidence**: High.
- **Trade-off**: One transaction is the right size, matching a face. Projecting every curve of
  a sketch that was not meant (construction axes of that sketch, a gear's pitch circles) needs
  the same filter Project uses now: skip reference geometry, and leave construction curves in,
  since a click on one projects it.
- **Fix sketch**: Add a command, offered while Project is active and while curves of one earlier
  sketch or edges of one body are selected: one transaction, the label "Project N curves of
  <name>", built by calling `projections_of` per pick and skipping sources already present.
  Selecting the other sketch's row projects its non-reference curves. Intersect stays per face;
  Shift-click already intersects the whole body.

### 2. Break a projection or an intersection into ordinary curves

- **Evidence**: `Edit::SetSketchProjection` (`crates/caditor-document/src/edit.rs`) already takes
  `source: Option<ProjectionSource>`. `source: None` is applied when a projected entity is
  deleted (`edit/sketch.rs`, `remove_sketch_items`) and is tested on its own
  (`projection_tests.rs`, the test that unprojects one of two curves and leaves the shared
  points projected while the other curve still carries them). No command sends that edit for a
  curve the user wants to keep. The guide says projected geometry cannot be dragged and follows
  its source. Trim, extend, split and the sketch fillet refuse it (`guide/trim-and-extend.md`).
- **Impact**: Intersect and Project are how a sketch starts from a body: the section of a turned
  part, the outline of a face, the curves of an earlier sketch. The result is locked. Changing
  one length means deleting the projection and redrawing it, and the dimensions on it go with
  the deletion. Breaking the link keeps the curve, its points and its dimensions, and later
  edits to the source no longer move it.
- **Effort**: S.
- **Risk**: Low. The edit exists. The subtle case is a point shared with a projection that is
  still linked: the point stays projected, which the existing test already requires.
- **Confidence**: High.
- **Trade-off**: After the break, the curve is a snapshot. Say so in the notice ("Line 3 no
  longer follows the body"). Offer it on the selected projected curves, from the sketch menu,
  the palette and the context menu. Do not break a link as a side effect of trimming; trimming
  a projection stays a refusal until the user breaks the link.
- **Fix sketch**: One "Make ordinary" transaction of `SetSketchProjection { source: None }` per
  selected projected entity, settled like any other sketch edit. A UI test projects a face,
  breaks one edge, moves a point of it, recomputes, and checks the other edges still follow the
  face.

### 3. A circular pattern of points does not become holes

- **Evidence**: A hole drills `free_points` plus the centre of every circle that is not
  construction (`crates/caditor-document/src/hole.rs`, `centres`). `free_points`
  (`crates/caditor-sketch/src/sketch.rs`) is every point no curve uses; a constraint does not
  count. A rectangular pattern ties points with horizontal and vertical distances only
  (`pattern.rs`, `tie` through `shift_constraints`), so those points stay free and a hole
  drills each copy. A circular pattern ties them with construction lines from the hub to each
  point (`pattern.rs`, `tie_turns` / `ray_to`: `Entity::Line { start: hub, end: point }` then
  `set_construction`). Those lines use the points, so `centres` drops every copy and the hub.
  The hole guide says the tool drills loose points and circles. It never says a circular
  pattern of points is not loose. Separately, a point cannot be construction at all
  (`Sketch::set_construction` returns `PointAsConstruction`; the test
  `curves_become_construction_geometry_and_points_never_do`). The hole list already skips an id
  that is in the construction set, but no point can get there.
- **Impact**: Six points around a centre is the usual way to draw a bolt circle. Drawn with the
  circular pattern, then handed to Hole, it drills nothing. The same six points in a rectangular
  pattern drill. The workaround is to pattern circles instead of points, and there is no way to
  mark the centre as a layout point so that, once the copies do drill, the centre stays undrilled.
- **Effort**: M. The pattern and the hole rule are each small; the behaviour change for existing
  hole sketches is the part that needs care.
- **Risk**: Medium. If every point that only a construction curve uses becomes a hole, a
  construction line drawn as a guide in a hole sketch gains a hole at each loose end. A corner
  that also belongs to an ordinary line stays as it is, because an ordinary curve still uses it.
- **Confidence**: High that the circular pattern drills nothing. Medium on which rule should
  change; the escape hatch below is the decision.
- **Trade-off**: Construction lines are the solver's way of holding an angle about a point
  (`Angle` is between lines). Deleting the rays would drop the constraint that keeps the copies
  spaced. The rays should stay. Hole placement is what should look through them.
- **Fix sketch**: Treat a point as free for `centres` when every curve that uses it is
  construction. Allow a point to be construction, drawn as a layout point, and keep it out of
  `centres` (the filter is already there). The hub of a bolt circle is then marked construction
  and is not drilled; the copies, used only by the pattern's construction rays, are drilled.
  Before changing the rule, search hole sketches in the tests for a construction line with a
  loose end. If any shipped behaviour depends on that end not drilling, stop and special-case
  the pattern rays instead of every construction curve. Update `guide/hole.md`,
  `guide/sketch-patterns.md` and `guide/sketches.md`, and the sketch and document rules.

### 4. Remember the starting value of the tools that still start from a constant

- **Evidence**: `Remembered` (`crates/caditor/src/last_values.rs`) has eleven slots: fillet and
  chamfer size, chamfer form and second distance or angle, shell thickness, hole diameter and
  blind depth, extrusion distance, linear count, circular count. `app-modelling.md` lists those
  and no others. A new revolve is always a full turn (`solid_tools.rs`, `RevolveExtent::Full`).
  A thin wall is always 1 mm centred (`default_wall`, `DEFAULT_WALL_THICKNESS`). A pipe is always
  10 mm with the default bend (`pipe_tools.rs`, `Pipe::new` from `DEFAULT_SIZE`). A rib is always
  2 mm (`rib_tools.rs`, `DEFAULT_THICKNESS`). An offset face is always the default distance
  (`offset_face_tools.rs`, `DEFAULT_DISTANCE`). A pattern's spacing is computed from the body's
  size on each create (`pattern_tools.rs`, `direction`) and is not one of the remembered slots;
  only the count is.
- **Impact**: The second bracket, the second pipe and the second rib in a session start as if
  the first had not been set. Extrude, hole, fillet and shell already carry the last value
  across sessions, through the preferences file, including a value that names a parameter.
- **Effort**: S per slot. The mechanism is `Remembered`, `Starts::of` and `last_values::changes`.
  Doing the set above together is still S.
- **Risk**: Low. A stored text that no longer parses, or names a missing parameter, already
  falls back to that slot's default (`Starts::of`). A full-turn revolve has no angle to store;
  remember the angle only when the user set a partial turn.
- **Confidence**: High.
- **Trade-off**: Remembering a spacing fights the current default, which scales with the body so
  the copies start clear of it. Keep that as the default the first time, and remember only after
  the user has edited a spacing. Do not remember twist, end scale or a loft weight until someone
  asks; those were just added and their defaults (0°, 1, free ends) are the common case.
- **Fix sketch**: Add slots for the revolve angle, wall thickness, pipe size, pipe bend, rib
  thickness and offset distance, on the same path fillet size already uses. Cover each in
  `ui_tests/remembered_starts.rs`: set a value, make another feature, see the value, restart
  the harness from the stored preferences, see it again.

### 5. Sketch text, then emboss

- **Evidence**: `docs/FEATURES.md` (Sketching, "Tools missing") names text: a font, a height,
  bold and italic, set along a curve, letters becoming closed regions that extrude. The drawing
  tools on `guide/sketches.md` list every other shape and do not list text. Emboss and deboss of
  that profile onto a flat or curved face is a separate modelling entry, blocked by this one.
  SVG import already turns text into outlines through Inter (`file-import-export.md`), so the
  outline conversion exists for files brought in, and a letter typed in a sketch does not use it.
- **Impact**: A part number, a logo or a scale mark cannot be typed. It is either imported as
  frozen outlines from an SVG or drawn curve by curve. Neither follows a parameter, and neither
  sits on a curve the way the roadmap describes.
- **Effort**: L. A new sketch entity (or a group of regions owned by one text record), a file
  record, the solver holding the outlines fixed the way projected geometry is held, and the
  emboss feature after that. Coarse: more than a week for text alone.
- **Risk**: Medium. Letter outlines that overlap, and a count of curves that changes when the
  string changes, are the same problem the parametric gear entry already names. Regions have to
  survive a one-character edit without renaming every face of the extrusion.
- **Confidence**: High that it is missing. The scope is the roadmap's, not a new design.
- **Trade-off**: Ship the sketch text first, on a plane, extrudable as ordinary regions.
  Emboss onto a curved face is the blocked entry and should stay blocked until text has a stable
  region identity. Use the Inter faces the SVG importer already takes, so a typed label and an
  imported label match.
- **Fix sketch**: This is a design task, not a small patch. Start from `Sketch::add_gear`'s
  "draw outlines once, held fixed" and from `import/svg/lettering.rs`, and write the file record
  before the emboss feature. Stop if the region keys change when the string is edited by one
  character; that has to be settled before an extrusion can depend on the text.

### 6. A sweep path drawn in space

- **Evidence**: `docs/FEATURES.md` (Sketching) states that a sketch lies on one plane, so a path
  that bends in space cannot be drawn. A sweep or a pipe can follow curves of one sketch, edges
  of one body, or a helix (`app-modelling.md`, Sweep and Pipe). The helix, the twist and the end
  scale landed in `81ff7c4c`. What remains for a handle, a cable or a pipe run is a path the user
  draws, rather than edges of a body built only to carry the path. Guide rails and a non-even
  twist are the other half of that same roadmap entry; they matter after a path can leave the plane.
- **Impact**: Every 3D path is either a helix (one pitch, one radius) or a chain of edges, which
  means an extra body. The planar sketch tools cannot place a point off the plane.
- **Effort**: L. The roadmap names the solver work: 3D points, and coincident, distance,
  parallel, perpendicular and fix in space. Coarse: a multi-week feature. It is blocked by no
  decision in `DECISIONS.md`; it is simply large.
- **Risk**: High. A second kind of sketch touches the solver, the file format, picking, the
  drawing tools and every feature that stores a sketch id. A path-only subset (lines in space,
  used by sweep and pipe, no regions) is smaller and still does the job.
- **Confidence**: High that the workflow stops at planar sketches and body edges.
- **Trade-off**: A full 3D sketch with regions, splines and dimensions is the roadmap's text.
  The part a sweep needs first is an open chain of lines and arcs with typed coordinates. Ship
  that, and leave sketch regions on a plane. Do not fold this into the helix; a helix is one
  curve with a pitch, and a handle is not.
- **Fix sketch**: Design the path-only subset against `SweepPath` before adding entities. If the
  file format cannot tell a path sketch from a profile sketch without a new record, add the
  record so an older reader reports the feature as from a newer version (`file-format.md`).

## Already tracked, and left there

These are real gaps in the same workflow. `FEATURES.md` already says what they are and how big
they are, so they are not findings of this pass:

- Direct editing of an imported face (move, tilt, replace). This is the top modelling gap for a
  STEP body with no history.
- Spline handles, inserting a spline point, and offset or fillet of a spline.
- A gear that rebuilds when its module or its tooth count changes.
- A sketch pattern along a path.
- The silhouette of a body brought into a sketch, and 2D drawings. Both wait on vector
  hidden-line removal.
- Guide rails, round sweep corners, and a loft tangent to a face.
- Assemblies, deriving another file, surfaces and sheet metal. Each is an open scope decision
  in `DECISIONS.md`.

## Considered and rejected

- Tracing a reference image into curves automatically. The picture is there to draw over by
  eye (`document.md`, Reference image). A bitmap fitter would produce curves the solver then has
  to hold, and nothing in the image code samples pixels as snap targets. Not worth a first version.
- A pierce constraint. A datum point where an axis meets a plane, then Project, already places
  that point.
- A rounded-rectangle tool. A rectangle and a sketch fillet of its four corners stay parametric
  through the radius dimension.
- Turning an extrusion into a revolve in place. `SetFeatureKind` refuses a kind change, so
  dependents keep a meaningful body. The user deletes and recreates.
- Chain and ordinate dimensions in the sketch. Dimension fully from a datum point already adds
  the horizontal and vertical distances that define each free point (`Sketch::datum_dimensions`).
- Remembering twist, end scale and loft weights. Their defaults are the common case, and they
  were added in `81ff7c4c`.

## What this pass did not do

Kernel booleans, shells, blends and STEP healing were not re-read; their failures stay in
`BUGS.md`. The seven items of the previous `docs/AUDIT.md` were not re-verified. No UI session
was driven. Fuzz targets were not run.
