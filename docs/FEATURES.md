# caditor features

What caditor cannot do yet: missing tools, feature options and exchange formats. An entry here is
new capability; a wrong result or a refusal of something that should work belongs in `BUGS.md`.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Modelling

- [high · hard] Bodies cannot be edited directly beyond offsetting and deleting faces: no moving,
  tilting or replacing a face, so a hole in the wrong place or a wall at the wrong angle on an
  imported STEP body (which has no feature history) still means remodelling it. Delete face closes a
  gap only from the faces around it: not a face whose sides lie beyond parallel faces (a step's
  riser), one face of a tangent chain whose ends meet other blends (the chain must go whole), a face with several loops that are not whole loops of the faces around it (a hole
  breaking out through a side), nor a gap that only faces further away could close. Direct edits
  become features of their own, named from the faces they move, so they stay parametric and
  undoable.
- [medium · hard] Offset face moves planes, cylinders, cones, spheres and tori only: a spline,
  extrusion or revolution face becomes its offset surface (which needs a surface fitted within
  tolerance), and a fillet moved beside a plane that stays is refused rather than re-blended.
- [medium · hard] An end up to the next face or a curved face follows curved or several faces
  only on one side and without an offset (two sides or an offset need one flat face). An end
  cannot end on a whole body (where the profile last leaves it); an extrusion along a direction
  takes no taper, up to next or curved face; a revolve turns up to a face or plane only when it
  holds the axis, never up to a curved face or the next face it meets; and a hole stops at the
  next face only where that face is flat.
- [medium · hard] Blends: ends at steps and T-junctions refused, and a lofted chain (an edge that is
  no line along its faces nor a circle about their axis, and the tangent chain holding it) refused
  where it ends at a rounded corner; a lofted chain's feet are not checked against other blends'
  feet; no variable radius; a round corner only for three convex straight edges meeting at three
  planes (other corners mitre). Missing as shapes
  of their own: a full-round fillet across a narrow face between two others, a fillet sized by
  chord length, a fillet that runs by a rule over every edge of a kind, setback corners where three
  fillets meet, a tangency weight, and a curvature-continuous (G2) fillet.
- [medium · hard] Shell: no spline, extrusion or revolution faces, as walls or openings (the kernel
  has no fitted offset surface); a curved face (cylinder, cone, sphere, torus) opens only on the
  body's outside where sharp edges part it from the closed faces, since a smooth or void opening
  needs the opening prism only flat faces give.
- [medium · hard] Sweep and loft: a sweep carries a sketch's regions along a chain of sketch
  curves or model edges, square to the path or parallel to the profile, but takes no guide rail,
  taper or twist (so a closed path out of one plane, which brings the profile back turned, is
  refused), mitres only sharp corners between two lines (other sharp corners are refused: no
  round corners), and sweeps no thin wall of open curves. A loft passes through closed sketch
  profiles without holes in order, smooth or ruled, and may end at a point, but takes no faces of
  bodies as profiles, no guide rails or centreline, no tangent or curvature-continuous conditions
  at its ends, and cannot close back to its first profile.
- [medium · hard] No flat spiral or varying helix: Helix gives a sweep or pipe a helix of one
  pitch (tapered or not, started at any angle round its axis) to follow, but a flat spiral (a clock
  spring) has no tool at all, and a helix cannot change its pitch along the way, so a closed-end
  spring's closed and ground ends cannot be swept. A helix's start is chosen by angle only, never
  by picking a point, and turns cannot touch (a closed coil, or a profile as tall as the pitch swept
  along a helix, needs a pitch a hair larger).
- [medium · hard] Patterns stop short of what other modellers repeat: a curve pattern follows only
  the curves of one sketch, never a chain of model edges (a 3D path, the copies then turning with
  its tangent and normal), and a pattern repeats features or whole bodies but never chosen faces.
  A feature pattern or mirror repeats a shell only when nothing else changes the body between the
  shell and it (`repeat_shell.rs` hollows again from the body before the shell, so later features
  would be lost), refusing otherwise.
  Every copy is the original's tool placed again, never recomputed where it lands (a copy of an
  extrusion up to next stops where the original did, not on the face it meets), and one pattern
  cannot chain a shift, a turn and a mirror.
- [low · medium] Ribs (`rib.rs`) follow only chains that do not branch: two ribs meeting in a T
  are refused (crossing chains work), a web is thickened only along lines and arcs
  (`wall_regions`), free ends run on along their tangent only (no natural continuation of an arc),
  and a rib takes no draft angle.
- [low · medium] A chamfer's feet on a curved face stand on the face's tangent line rather than on
  the face, and a curve pattern's arrow (`value_gauges.rs`) slides along the curve's tangent while
  it is dragged, rejoining the curve when the copies are placed again. Each would be a `Track`
  that follows a curve.
- [low · medium] Mirror faces closes an opening only in one plane or on one elementary face beside
  it: an opening on an extrusion, revolution or spline face, one running all the way around a
  round face (a collar or a groove), or one spanning several curved faces is refused, since
  closing it needs a freeform patch, a ring face or a surface fitted across it.
- [low · medium] A pipe's bend radius arrow stands only at a corner between two straight lines
  (`PipeStand::corner`, a gauge linear in the radius): a path whose sharp corners all touch an arc,
  spline or edge curve is bent but has no arrow to drag its bend radius.
- [low · medium] A body of each separate piece is keyed by the lowest sketch curve that drew it, so
  deleting and redrawing that one curve of a shape makes its piece a new body; a configuration
  file's colour column cannot name a piece (`plan_configuration_import` finds bodies by making
  feature); Hide others keeps every piece of the kept piece's feature shown.
- [low · hard] Split face wraps closed outlines only within once round a cylinder or cone, and a
  wrapped open chain may not run right round and back to the edge of the faces it started from:
  a helical stripe drawn as one outline running round more than once, a band whose ends meet
  after one turn and a chain turning back after going round all need a tool that crosses the seam
  of the unrolled window in one piece (its two edges glued into one solid, the caps then whole
  rings), which the window's separate regions cannot build; a sketch of two or more open chains
  (a stripe between two helices) is refused too. Spheres stay refused in words: a sphere has no
  flat unrolling, so wrapping onto one first needs a chosen projection (stereographic, equal area
  or along its axis) and the distortion it brings, a design decision before any kernel work.
- [low · hard] Scale is uniform: a body cannot be stretched by different factors along the three
  axes (a plane stays a plane, but a cylinder becomes an elliptical one, which the kernel's
  surfaces do not have).
- [low · hard] No silhouette split: dividing a body's faces along its outline seen from a chosen
  direction (a split face whose tool is that outline), so the parting line of a moulded or cast
  part can be a face boundary for a draft to start from.
- [medium · hard · blocked by: direct face edits ("Bodies cannot be edited directly")] Draft angle
  on existing faces: a fixed angle from a plane, a split at a parting line with an angle on each
  side, and an angle per face, following tangent faces as one chain.
- [low · hard · blocked by: sketch text ("Tools missing" under Sketching)] Emboss or deboss sketch
  text, or any sketch profile, onto a face, flat or curved (wrapped around it), raised or
  recessed by a depth.
- [low · hard · blocked by: draft angle (above)] No plastic-part features:
  the screw boss with its ribs, a lip and groove along a seam, snap fits (hook, loop, groove) and a
  rest (a flat seat on a curved face), which moulded parts need.

## Sketching

- [medium · hard] Body items (`body_snap.rs`) are snapped to by drawing and grabs, pulled by the
  held snap and picked by Select and Smart dimension, from the pointer or the highlight keys, but
  they are not acquired for tracks and never crossed by a direction or track, since they are not in
  the sketch until a point lands on them: acquiring one would need tracks that hold a point the
  shape has yet to project.
- [medium · hard] A projected edge (`projection.rs`) that an upstream trim, split or break cut into
  pieces with a gap between them fails the sketch in words rather than projecting each piece as a
  curve of its own: a projection is one sketch curve with the points it was made with, and the
  recompute that places it cannot add the curves for the other pieces. Offering them from Update
  references (the pieces projected as new curves, the first keeping the id constraints and
  dimensions hang on) needs that healing to add sketch geometry, which it does not yet.
- [medium · hard] Tools missing: a pattern of sketch geometry along a path (copies tied to the
  path would need a vector-equality or along-the-curve spacing the solver lacks), and text (a
  font, a height, bold and italic, set along a curve, its letters becoming closed regions that
  extrude).
- [medium · hard] A spline is only the control points it was drawn with: a point has no tangent or
  curvature handle to set the direction and pull of the curve there, which would be stored as
  constraints on the point rather than as positions so the solver and dimensions keep reading
  them; the degree cannot be chosen; and no point can be inserted or removed while keeping the
  shape.
- [medium · hard] A spur gear (`sketch.md`, Spur gears) is drawn once: its curves are free sketch
  geometry, not tied to the values in the Spur gear panel (expressions using parameters are read
  when it is drawn), so changing the module, teeth or pressure angle, or a parameter they use,
  means deleting it and drawing it again, and nothing holds its shape when one of its points is
  dragged. Keeping it parametric needs a gear stored in the sketch (its expressions and the curves
  it made, saved in the file) that recompute draws again from the evaluated values, holding its
  curves fixed like projected geometry; the teeth changing the number of curves is the hard part.
- [medium · medium] An outline of odd sides with an arc among them has no centre to snap or track
  to: the centre of an odd outline is the centre of its corners, which reads as its middle only
  for straight sides, and nothing yet says where the middle of such an outline is. A drag snaps
  only its handle (the moving point nearest the press), not whichever moving point comes near a
  target.
- [medium · hard] Offset takes one chain at a time, leaves the free ends of an open chain sliding
  along their curves and cannot offset splines; a sketch fillet cannot round a spline and drops
  equal lengths and midpoints of the lines it shortens, as trim does. A trimmed fit-point spline is
  refitted through its remaining points rather than kept exactly (its knots follow its points, so
  no fit spline holds the piece), and a spline cannot be split or broken at a point.
- [medium · hard] Sketches lie on one plane, so a sweep path or rail that bends in space (a cable
  run, a handle) cannot be drawn; only a chain of model edges gives the sweep one. A 3D sketch of
  lines, arcs and splines, with points placed by typed coordinates in space or moved off the
  sketch plane, would be the path; with it a sketch curve projected onto a curved face along a
  direction or to the nearest point, and the curve where two faces meet. The solver then holds 3D
  points and directions and the constraints that mean something there (coincident, horizontal and
  vertical in space, parallel, perpendicular, distance, fix).
- [low · hard] An ellipse's offset is a free fit-point spline that does not follow the ellipse:
  holding it would need a constraint keeping each fit point on the ellipse's normal at its own
  parameter (a `Distance` from the ellipse lets every fit point slide along the offset and would
  put a dimension on each), a new constraint kind with its solver form, file record and glyph.
  An ellipse also cannot be offset within a chain of lines and arcs, whose joints would need the
  spline to meet them.
- [low · hard · blocked by: vector hidden-line removal ("Technical drawings")] Project takes edges,
  corners and the boundaries of faces, but not the outline of a body seen square to the sketch
  plane: the silhouette lines of a cylinder lying along the plane or the circle of a sphere cannot
  be brought in, so a sketch following a body's outline is drawn by hand.

## STEP import and export

- [low · easy] STEP styling is read and written in every form the standard and the tests
  cover (`step-read.md`), but has been checked only against a handful of real files with plain
  colours. Read files with see-through bodies and faces and with copies coloured on their own from
  SolidWorks, CATIA, Creo, NX, Fusion and FreeCAD (`STEP_CORPUS`), and support any styling the
  import report names as not understood.
- [low · hard] No IGES import or export, though older CAM software and many suppliers still exchange
  it.

## Drawing import and export

- [low · hard] SVG text comes in as outlines in Inter (upright and italic): vertical writing
  modes and shaping beyond pair kerning (ligatures, marks, right-to-left scripts) are not applied,
  which needs a shaping engine (rustybuzz) and bidirectional reordering rather than the per-letter
  layout `lettering.rs` does, and letters of different glyphs that overlap are not merged into one
  outline (`overlap.rs` merges each glyph alone; merging across letters needs the profile built
  over the whole text). `textPath`'s `method` and `spacing` are ignored, and `sub`, `super` and
  percentage baseline shifts use fixed shares of the font size rather than the font's own
  subscript and superscript metrics.

## Mesh import and export

- [medium · hard] Mesh import (STL, OBJ, 3MF) repairs simple defects and joins flat areas into
  planar faces, but curved areas stay faceted, a sealed hollow is filled, non-manifold edges and
  gaps of more than `MAX_FILLED_HOLE_EDGES` edges lose their whole shell, and a mesh of more than
  `MAX_FACETED_FACES` faces after joining (most scans and organic prints) is refused. An imported
  solid is stored as STEP text like any import, and the source mesh is not kept. What remains:
  - Rebuild real faces: group triangles into regions and recognise planes, cylinders, cones,
    spheres and tori within a tolerance the import chooses from the mesh's own noise (adjustable
    with a live preview), fit splines to what remains, and join them into a B-rep whose faces and
    edges are named, so a flat side takes a sketch, a hole edge a fillet and a bore its axis.
  - Fall back gracefully: what cannot be fitted stays faceted but still part of a valid solid that
    booleans, shells and direct edits (Modelling) accept, so the import never fails as a
    whole over a bad region.
  - Stay quick on scans and printer files of millions of triangles, in the background with
    progress and Cancel, and keep the source mesh in the model so the conversion can be redone at
    another tolerance later without breaking what references its faces.
  - Units: STL has none, so the import guesses from the size (a part 0.05 mm across is likely in
    metres) and offers a scale before committing, rather than leaving it to the scale item.

## Technical drawings

- [medium · hard] Vector hidden-line removal: the hidden-lines-removed display style hides edges
  by the depth buffer, which gives pixels, not curves. A drawing view needs each edge of a body
  split where faces cover it, as 2D curves in the view's plane with their visible and hidden
  pieces (silhouettes of curved faces included), computed off the UI thread and cancellable.
- [medium · hard · blocked by: vector hidden-line removal (above)] No 2D drawings
  at all: no sheet with a title block, no projected front, top, side and isometric views of the
  bodies, no section or detail views, no dimensions or notes taken from the model, and no PDF, SVG
  or DXF output of a sheet, though parts made for a workshop need one. Views update with the model
  and their dimensions refer to edges by name, so they survive edits as features do.
- [medium · hard · blocked by: 2D drawings (above)] Annotations beyond plain dimensions and notes:
  centre marks and centrelines on holes and round edges, hole and thread callouts read from the
  hole and thread features, ordinate and baseline dimensions, tolerances (plus and minus, limits,
  ISO 286 fits), geometric tolerances with datum features (ISO 1101), surface texture symbols
  (ISO 21920) and weld symbols (ISO 2553), and a title block filled from the model properties.
- [low · medium · blocked by: 2D drawings (above)] Views and sheets beyond the standard ones:
  auxiliary views square to an inclined face, broken views shortening a long part, broken-out and
  half sections, cropped views, first- or third-angle projection, ISO 5457 sheets from A4 to A0 and
  several sheets per drawing, a revision table, and section hatching chosen per material.

## Application

- [low · medium] The rendered view's bounced light (`render.md`, the light pass) gathers from a
  reflective shadow map, so it reaches only `BOUNCE_REACH` of the light frame, comes only from
  surfaces the sun sees, and its visibility test (`bounce_blocked`) sees only those surfaces: a
  wall the sun does not light (fully in shadow, or edge-on to it) still lets bounced light
  through.
- [low · medium] The rendered view's surroundings are a procedural sky and a shadow-catching
  ground; a user cannot load an HDR image as the backdrop and light, choose a backdrop or ground,
  nor give a material texture, bump or clear-coat maps, and the ground bounces no light or occlusion
  onto the bodies.
- [medium · hard] The MCP server (`caditor mcp`, `mcp.md`) builds only part of the model: an
  assistant can read the tree, parameters, bodies, faces, edges and sketches, measure between
  points, faces and edges, draw a picture, set parameters, sketch points, lines, polylines,
  rectangles, circles, arcs, slots, polygons, splines and ellipses with constraints and dimensions
  (adding, removing and editing them) on a principal plane, datum plane, coordinate system or flat
  face, extrude, revolve, drill holes, fillet, chamfer, shell, offset faces, add primitives,
  mirror, pattern in a row, grid or circle, combine and move bodies, add datum planes and axes,
  sweep, pipe, loft, coil, rib, thread, split and delete faces, split and scale bodies, edit any
  of those features, rename, suppress, reorder and delete features, undo, redo, save and export.
  What remains, through the same `Session` layer:
  - A write and edit tool per remaining feature kind (mate, curve and point patterns, datum
    points, the other datum plane and axis constructions, coordinate systems, helices).
    Extrusions lack regions, an end up to a curved surface, a second side other than a distance,
    a direction other than square, and taking a thin wall off again; revolves lack up-to ends,
    regions and thin walls; holes lack stepped styles, slots, drill points and sizing by circles;
    shells lack per-face thickness, mirrors take no faces, lofts take no single regions or end
    point, and split faces neither carry nor wrap curves. Faces and edges named in an edit are
    found on the body as it stands now, so an edge a fillet rounded away cannot be named again.
  - Sketching: elliptical arcs, conic splines, polygons on a side, tangent arcs, and projections
    of body edges; the other sketch tools of the app (trim, extend, offset, mirror, patterns,
    fillet) and moving or resizing existing entities.
  - Import, and saving under another name, within the allowed folders.
- [medium · hard · blocked by: the in-app assistant connection decision (`DECISIONS.md`)] An
  assistant cannot work on the model open in the window: the headless server owns its own
  document. Connecting to the running app needs a transport the app listens on (off by default),
  the user's approval of each client, a status pill naming the connected client with a Disconnect,
  its changes landing in the user's undo history interleaved with their own, and its tool calls
  queued like `queued.rs` actions while the model recomputes, never blocking the UI thread.
- [low · medium] A copy from another model keeps none of its references outside the copied set. A
  sketch's section projection of a copied body is still left out, its edge being named by the
  sketch's own cut (`section.rs`), and a corner picked on a body outside the copy is not followed
  onto copied faces around it.
- [low · hard] No automation: nothing can be driven by a script or macro, as Fusion's scripts and
  add-ins do, to make repetitive geometry, run a batch over files or add a tool. An interface
  would go through `Action`s and `Transaction`s like the UI, so scripts cannot break the model's
  rules, sharing the MCP server's `Session` layer (`mcp.md`), and would need a decision on the
  language and on safety (a script cannot reach files or the network unasked).
- [low · hard] One document per process.
- [low · hard] No localisation.

